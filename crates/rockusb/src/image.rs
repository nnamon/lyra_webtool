//! The `update.img` firmware container.
//!
//! An `update.img` is an `RKFW` header followed by two payloads: an embedded
//! [`crate::loader::RkBoot`] loader, and an inner `RKAF` firmware holding the
//! partition images.
//!
//! # Provenance, and why it matters here
//!
//! The outer `RKFW` header is a decode of `CRKImage::ObjectInit` @ `0x444f10`
//! (the tag check is at `0x4451f0`, and the header is `fread` with a fixed
//! length of `0x66`). Every offset below was read off that disassembly.
//!
//! The inner `RKAF` item table is **not** verified the same way. `upgrade_tool`
//! v2.17 contains no `RKAF` tag comparison at all -- it reaches the item table
//! through `CRKImage::GetData` without a magic check -- so the field offsets
//! here follow the public `rkdeveloptool` `RKANDROID_HDR` definition instead of
//! a disassembly of this binary, and they have not been checked against a real
//! `update.img`.
//!
//! Because a wrong offset here would mean writing a payload to the wrong flash
//! address, [`RkafFirmware::parse`] validates aggressively: it requires the
//! tag, rejects an implausible item count, and rejects any item whose payload
//! or target range falls outside the container. A layout mismatch therefore
//! produces an error instead of a bad flash write.

use crate::loader::{LoaderError, RkBoot};
use crate::protocol::SECTOR_SIZE;

/// `RKFW` tag, little endian `0x57464b52`.
pub const TAG_RKFW: [u8; 4] = *b"RKFW";
/// `RKAF` tag of the inner firmware, little endian `0x46414b52`.
pub const TAG_RKAF: [u8; 4] = *b"RKAF";

/// Fixed size of the `RKFW` header, the length `ObjectInit` passes to `fread`.
pub const RKFW_HEADER_LEN: usize = 0x66;
/// Length of the lowercase-hex MD5 string at the end of the file.
pub const MD5_TAIL_LEN: usize = 32;

/// Offset of the inner `RKAF` item array.
const RKAF_ITEMS_OFFSET: usize = 0x88;
/// Stride of one `RKAF` item record.
const RKAF_ITEM_LEN: usize = 0x6c;
/// Item count above which the container is treated as malformed.
const RKAF_MAX_ITEMS: usize = 64;

/// Errors from parsing a firmware container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    TooSmall {
        len: usize,
        needed: usize,
    },
    BadTag {
        found: [u8; 4],
        expected: [u8; 4],
    },
    /// A declared region runs past the end of the container.
    OutOfBounds {
        what: &'static str,
        offset: u64,
        len: u64,
        file_len: u64,
    },
    /// The item count is zero or implausibly large.
    BadItemCount(u32),
    /// The embedded loader did not parse.
    Loader(LoaderError),
    /// The trailing MD5 did not match the content it covers.
    Md5Mismatch {
        stored: String,
        computed: String,
    },
    /// The file carries no trailing MD5 to check.
    NoMd5,
}

impl From<LoaderError> for ImageError {
    fn from(e: LoaderError) -> Self {
        Self::Loader(e)
    }
}

impl core::fmt::Display for ImageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooSmall { len, needed } => {
                write!(f, "file is {len} bytes, needs at least {needed}")
            }
            Self::BadTag { found, expected } => write!(
                f,
                "bad tag {:?}: expected {:?}",
                String::from_utf8_lossy(found),
                String::from_utf8_lossy(expected)
            ),
            Self::OutOfBounds {
                what,
                offset,
                len,
                file_len,
            } => write!(
                f,
                "{what} at {offset:#x} (+{len}) runs past the {file_len}-byte container"
            ),
            Self::BadItemCount(n) => {
                write!(
                    f,
                    "implausible item count {n} (expected 1..={RKAF_MAX_ITEMS})"
                )
            }
            Self::Loader(e) => write!(f, "embedded loader: {e}"),
            Self::Md5Mismatch { stored, computed } => {
                write!(
                    f,
                    "firmware MD5 mismatch: file says {stored}, computed {computed}"
                )
            }
            Self::NoMd5 => write!(f, "container has no trailing MD5"),
        }
    }
}

impl std::error::Error for ImageError {}

fn rd_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn rd_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Read a NUL-terminated, fixed-width ASCII field.
fn rd_str(b: &[u8], at: usize, max: usize) -> String {
    let end = (at + max).min(b.len());
    let slice = &b[at.min(b.len())..end];
    let len = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..len]).into_owned()
}

/// One image inside the inner `RKAF` firmware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareItem {
    /// Logical name, e.g. `uboot`, `boot`, `rootfs`, `parameter`.
    pub name: String,
    /// Original filename inside the packed firmware.
    pub file_name: String,
    /// Payload offset, relative to the start of the `RKAF` blob.
    pub data_offset: u32,
    /// Destination address on flash, in 512-byte sectors.
    ///
    /// `0xffff_ffff` marks an item with no flash destination (it is a
    /// descriptor rather than something to write).
    pub target_sector: u32,
    /// Payload length in bytes.
    pub data_size: u32,
    /// Padded length as stored.
    pub padded_size: u32,
}

impl FirmwareItem {
    /// True when this item has a real flash destination.
    pub fn is_writable(&self) -> bool {
        self.target_sector != 0xffff_ffff && self.data_size > 0
    }

    /// Destination byte offset on flash.
    pub fn target_byte_offset(&self) -> u64 {
        self.target_sector as u64 * SECTOR_SIZE as u64
    }

    /// True for the partition-table descriptor rather than a flashable image.
    pub fn is_parameter(&self) -> bool {
        self.name.eq_ignore_ascii_case("parameter") || self.file_name.contains("parameter")
    }
}

/// The inner `RKAF` firmware.
#[derive(Debug, Clone)]
pub struct RkafFirmware {
    /// The `RKAF` blob, so item payloads can be sliced out.
    raw: Vec<u8>,
    pub model: String,
    pub id: String,
    pub manufacturer: String,
    pub version: u32,
    pub items: Vec<FirmwareItem>,
}

impl RkafFirmware {
    /// Parse an `RKAF` blob, validating every declared range.
    pub fn parse(data: &[u8]) -> Result<Self, ImageError> {
        if data.len() < RKAF_ITEMS_OFFSET {
            return Err(ImageError::TooSmall {
                len: data.len(),
                needed: RKAF_ITEMS_OFFSET,
            });
        }
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&data[0..4]);
        if tag != TAG_RKAF {
            return Err(ImageError::BadTag {
                found: tag,
                expected: TAG_RKAF,
            });
        }

        let count = rd_u32(data, 0x84);
        if count == 0 || count as usize > RKAF_MAX_ITEMS {
            return Err(ImageError::BadItemCount(count));
        }

        let table_end = RKAF_ITEMS_OFFSET + count as usize * RKAF_ITEM_LEN;
        if table_end > data.len() {
            return Err(ImageError::OutOfBounds {
                what: "RKAF item table",
                offset: RKAF_ITEMS_OFFSET as u64,
                len: (count as u64) * RKAF_ITEM_LEN as u64,
                file_len: data.len() as u64,
            });
        }

        let mut items = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let base = RKAF_ITEMS_OFFSET + i * RKAF_ITEM_LEN;
            let item = FirmwareItem {
                name: rd_str(data, base, 0x20),
                file_name: rd_str(data, base + 0x20, 0x3c),
                data_offset: rd_u32(data, base + 0x5c),
                target_sector: rd_u32(data, base + 0x60),
                data_size: rd_u32(data, base + 0x64),
                padded_size: rd_u32(data, base + 0x68),
            };
            // Refuse a payload that does not lie inside the blob. This is the
            // guard that turns an inner-layout mismatch into an error rather
            // than a write to the wrong flash address.
            if item.data_size > 0 {
                let end = item.data_offset as u64 + item.data_size as u64;
                if end > data.len() as u64 {
                    return Err(ImageError::OutOfBounds {
                        what: "RKAF item payload",
                        offset: item.data_offset as u64,
                        len: item.data_size as u64,
                        file_len: data.len() as u64,
                    });
                }
            }
            items.push(item);
        }

        Ok(Self {
            raw: data.to_vec(),
            model: rd_str(data, 0x08, 0x22),
            id: rd_str(data, 0x2a, 0x1e),
            manufacturer: rd_str(data, 0x48, 0x38),
            version: rd_u32(data, 0x80),
            items,
        })
    }

    /// Payload bytes for `item`.
    pub fn item_data(&self, item: &FirmwareItem) -> &[u8] {
        let start = item.data_offset as usize;
        &self.raw[start..start + item.data_size as usize]
    }

    /// Items that should actually be written to flash.
    pub fn writable_items(&self) -> impl Iterator<Item = &FirmwareItem> {
        self.items.iter().filter(|i| i.is_writable())
    }
}

/// A parsed `update.img`.
#[derive(Debug, Clone)]
pub struct UpdateImage {
    pub version: u32,
    pub merge_version: u32,
    pub release_time: crate::loader::ReleaseTime,
    /// SoC family code, same encoding as the loader's field.
    pub chip: u32,
    /// Offset and length of the embedded loader.
    pub boot_offset: u32,
    pub boot_size: u32,
    /// Offset and length of the inner `RKAF` firmware.
    pub fw_offset: u32,
    pub fw_size: u32,
    /// Lowercase-hex MD5 from the last 32 bytes, when present.
    pub stored_md5: Option<String>,
}

impl UpdateImage {
    /// Parse the `RKFW` header only, without touching the payloads.
    ///
    /// This is deliberately cheap: a browser can read the first
    /// [`RKFW_HEADER_LEN`] bytes of a multi-hundred-megabyte file and show the
    /// user what it is before anything else is loaded.
    pub fn parse_header(data: &[u8]) -> Result<Self, ImageError> {
        if data.len() < RKFW_HEADER_LEN {
            return Err(ImageError::TooSmall {
                len: data.len(),
                needed: RKFW_HEADER_LEN,
            });
        }
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&data[0..4]);
        if tag != TAG_RKFW {
            return Err(ImageError::BadTag {
                found: tag,
                expected: TAG_RKFW,
            });
        }
        Ok(Self {
            version: rd_u32(data, 0x06),
            merge_version: rd_u32(data, 0x0a),
            release_time: crate::loader::ReleaseTime {
                year: rd_u16(data, 0x0e),
                month: data[0x10],
                day: data[0x11],
                hour: data[0x12],
                minute: data[0x13],
                second: data[0x14],
            },
            chip: rd_u32(data, 0x15),
            boot_offset: rd_u32(data, 0x19),
            boot_size: rd_u32(data, 0x1d),
            fw_offset: rd_u32(data, 0x21),
            fw_size: rd_u32(data, 0x25),
            stored_md5: None,
        })
    }

    /// Byte range of the embedded loader.
    pub fn boot_range(&self) -> core::ops::Range<usize> {
        self.boot_offset as usize..self.boot_offset as usize + self.boot_size as usize
    }

    /// Byte range of the inner `RKAF` firmware.
    pub fn fw_range(&self) -> core::ops::Range<usize> {
        self.fw_offset as usize..self.fw_offset as usize + self.fw_size as usize
    }

    /// Offset the trailing MD5 covers up to: the end of the firmware payload.
    ///
    /// `ObjectInit` computes `dwFWOffset + dwFWSize` and uses it as the MD5
    /// boundary, then reads 32 bytes from the end of the file.
    pub fn md5_covered_len(&self) -> u64 {
        self.fw_offset as u64 + self.fw_size as u64
    }

    /// SoC family rendered as text, e.g. `"3568"`.
    pub fn chip_name(&self) -> String {
        let s: String = self
            .chip
            .to_be_bytes()
            .iter()
            .filter(|&&b| b.is_ascii_graphic())
            .map(|&b| b as char)
            .collect();
        if s.is_empty() {
            format!("{:#010x}", self.chip)
        } else {
            s
        }
    }
}

/// An `update.img` with its payloads parsed.
#[derive(Debug, Clone)]
pub struct Firmware {
    pub header: UpdateImage,
    /// The embedded loader, when it parsed.
    pub loader: Option<RkBoot>,
    /// The inner firmware, when it parsed.
    pub firmware: Option<RkafFirmware>,
}

impl Firmware {
    /// Parse a whole `update.img` held in memory.
    ///
    /// The loader and inner firmware are parsed independently: a container with
    /// an unreadable inner firmware still yields its header and loader, which
    /// is enough to bootstrap a board out of MaskROM.
    pub fn parse(data: &[u8]) -> Result<Self, ImageError> {
        let mut header = UpdateImage::parse_header(data)?;

        if data.len() >= MD5_TAIL_LEN {
            let tail = &data[data.len() - MD5_TAIL_LEN..];
            if tail.iter().all(|c| c.is_ascii_hexdigit()) {
                header.stored_md5 = Some(String::from_utf8_lossy(tail).to_lowercase());
            }
        }

        let boot = header.boot_range();
        if boot.end > data.len() {
            return Err(ImageError::OutOfBounds {
                what: "embedded loader",
                offset: header.boot_offset as u64,
                len: header.boot_size as u64,
                file_len: data.len() as u64,
            });
        }
        let fw = header.fw_range();
        if fw.end > data.len() {
            return Err(ImageError::OutOfBounds {
                what: "inner firmware",
                offset: header.fw_offset as u64,
                len: header.fw_size as u64,
                file_len: data.len() as u64,
            });
        }

        let loader = if header.boot_size > 0 {
            RkBoot::parse(&data[boot], true).ok()
        } else {
            None
        };
        let firmware = if header.fw_size > 0 {
            RkafFirmware::parse(&data[fw]).ok()
        } else {
            None
        };

        Ok(Self {
            header,
            loader,
            firmware,
        })
    }

    /// Verify the trailing MD5 against the region it covers.
    pub fn verify_md5(&self, data: &[u8]) -> Result<(), ImageError> {
        use md5::{Digest, Md5};
        let stored = self.header.stored_md5.as_ref().ok_or(ImageError::NoMd5)?;
        let covered = self.header.md5_covered_len() as usize;
        if covered > data.len() {
            return Err(ImageError::OutOfBounds {
                what: "MD5 coverage",
                offset: 0,
                len: covered as u64,
                file_len: data.len() as u64,
            });
        }
        let mut h = Md5::new();
        h.update(&data[..covered]);
        let computed = h
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if &computed == stored {
            Ok(())
        } else {
            Err(ImageError::Md5Mismatch {
                stored: stored.clone(),
                computed,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_rkaf() -> Vec<u8> {
        let n = 2usize;
        let table_end = RKAF_ITEMS_OFFSET + n * RKAF_ITEM_LEN;
        let mut buf = vec![0u8; table_end];
        buf[0..4].copy_from_slice(&TAG_RKAF);
        let put_str = |buf: &mut Vec<u8>, at: usize, s: &str| {
            buf[at..at + s.len()].copy_from_slice(s.as_bytes());
        };
        put_str(&mut buf, 0x08, "rk3506_lyra");
        put_str(&mut buf, 0x2a, "LYRA-ZERO-W");
        put_str(&mut buf, 0x48, "Luckfox");
        buf[0x80..0x84].copy_from_slice(&1u32.to_le_bytes());
        buf[0x84..0x88].copy_from_slice(&(n as u32).to_le_bytes());

        let p0 = b"UBOOT-PAYLOAD".to_vec();
        let p1 = b"BOOT-PAYLOAD".to_vec();
        let o0 = table_end;
        let o1 = o0 + p0.len();

        let item = |buf: &mut Vec<u8>,
                    i: usize,
                    name: &str,
                    file: &str,
                    off: usize,
                    size: usize,
                    target: u32| {
            let b = RKAF_ITEMS_OFFSET + i * RKAF_ITEM_LEN;
            buf[b..b + name.len()].copy_from_slice(name.as_bytes());
            buf[b + 0x20..b + 0x20 + file.len()].copy_from_slice(file.as_bytes());
            buf[b + 0x5c..b + 0x60].copy_from_slice(&(off as u32).to_le_bytes());
            buf[b + 0x60..b + 0x64].copy_from_slice(&target.to_le_bytes());
            buf[b + 0x64..b + 0x68].copy_from_slice(&(size as u32).to_le_bytes());
            buf[b + 0x68..b + 0x6c].copy_from_slice(&(size as u32).to_le_bytes());
        };
        item(&mut buf, 0, "uboot", "uboot.img", o0, p0.len(), 0x4000);
        item(
            &mut buf,
            1,
            "parameter",
            "parameter.txt",
            o1,
            p1.len(),
            0xffff_ffff,
        );

        buf.extend_from_slice(&p0);
        buf.extend_from_slice(&p1);
        buf
    }

    #[test]
    fn parses_rkaf_items_and_payloads() {
        let blob = synth_rkaf();
        let fw = RkafFirmware::parse(&blob).expect("parse rkaf");
        assert_eq!(fw.model, "rk3506_lyra");
        assert_eq!(fw.id, "LYRA-ZERO-W");
        assert_eq!(fw.manufacturer, "Luckfox");
        assert_eq!(fw.items.len(), 2);

        let uboot = &fw.items[0];
        assert_eq!(uboot.name, "uboot");
        assert_eq!(uboot.file_name, "uboot.img");
        assert_eq!(uboot.target_sector, 0x4000);
        assert_eq!(uboot.target_byte_offset(), 0x4000 * 512);
        assert!(uboot.is_writable());
        assert_eq!(fw.item_data(uboot), b"UBOOT-PAYLOAD");

        // The parameter descriptor has no flash destination.
        let param = &fw.items[1];
        assert!(param.is_parameter());
        assert!(!param.is_writable());
        assert_eq!(fw.writable_items().count(), 1);
    }

    #[test]
    fn rejects_wrong_inner_tag() {
        let mut blob = synth_rkaf();
        blob[0] = b'X';
        assert!(matches!(
            RkafFirmware::parse(&blob),
            Err(ImageError::BadTag { .. })
        ));
    }

    /// The guard that matters: an item pointing outside the blob must be an
    /// error, never a flash write to a wrong address.
    #[test]
    fn rejects_item_payload_past_end() {
        let mut blob = synth_rkaf();
        let b = RKAF_ITEMS_OFFSET;
        blob[b + 0x64..b + 0x68].copy_from_slice(&0xffff_0000u32.to_le_bytes());
        assert!(matches!(
            RkafFirmware::parse(&blob),
            Err(ImageError::OutOfBounds {
                what: "RKAF item payload",
                ..
            })
        ));
    }

    #[test]
    fn rejects_absurd_item_count() {
        let mut blob = synth_rkaf();
        blob[0x84..0x88].copy_from_slice(&9999u32.to_le_bytes());
        assert!(matches!(
            RkafFirmware::parse(&blob),
            Err(ImageError::BadItemCount(9999))
        ));
        blob[0x84..0x88].copy_from_slice(&0u32.to_le_bytes());
        assert!(matches!(
            RkafFirmware::parse(&blob),
            Err(ImageError::BadItemCount(0))
        ));
    }

    fn synth_rkfw() -> Vec<u8> {
        let rkaf = synth_rkaf();
        let mut buf = vec![0u8; RKFW_HEADER_LEN];
        buf[0..4].copy_from_slice(&TAG_RKFW);
        buf[0x04..0x06].copy_from_slice(&(RKFW_HEADER_LEN as u16).to_le_bytes());
        buf[0x06..0x0a].copy_from_slice(&0x0000_0101u32.to_le_bytes());
        buf[0x0a..0x0e].copy_from_slice(&0x0100_0000u32.to_le_bytes());
        buf[0x0e..0x10].copy_from_slice(&2025u16.to_le_bytes());
        buf[0x10] = 6;
        buf[0x11] = 16;
        buf[0x12] = 9;
        buf[0x13] = 30;
        buf[0x14] = 0;
        // "3506" in reverse byte order.
        buf[0x15..0x19].copy_from_slice(b"6053");

        // No embedded loader in this fixture; firmware follows the header.
        let fw_offset = RKFW_HEADER_LEN as u32;
        buf[0x19..0x1d].copy_from_slice(&0u32.to_le_bytes()); // boot offset
        buf[0x1d..0x21].copy_from_slice(&0u32.to_le_bytes()); // boot size
        buf[0x21..0x25].copy_from_slice(&fw_offset.to_le_bytes());
        buf[0x25..0x29].copy_from_slice(&(rkaf.len() as u32).to_le_bytes());
        buf.extend_from_slice(&rkaf);

        // Trailing MD5 over everything the header says it covers.
        use md5::{Digest, Md5};
        let covered = fw_offset as usize + rkaf.len();
        let mut h = Md5::new();
        h.update(&buf[..covered]);
        let hex = h
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        buf.extend_from_slice(hex.as_bytes());
        buf
    }

    #[test]
    fn parses_rkfw_header() {
        let buf = synth_rkfw();
        let h = UpdateImage::parse_header(&buf).expect("parse header");
        assert_eq!(h.chip_name(), "3506");
        assert_eq!(h.release_time.year, 2025);
        assert_eq!(h.release_time.to_string(), "2025-06-16 09:30:00");
        assert_eq!(h.fw_offset, RKFW_HEADER_LEN as u32);
    }

    #[test]
    fn header_parses_from_a_truncated_prefix() {
        // A browser should be able to identify a huge file from its first bytes.
        let buf = synth_rkfw();
        let prefix = &buf[..RKFW_HEADER_LEN];
        assert!(UpdateImage::parse_header(prefix).is_ok());
    }

    #[test]
    fn full_parse_finds_inner_firmware_and_md5_verifies() {
        let buf = synth_rkfw();
        let fw = Firmware::parse(&buf).expect("parse firmware");
        assert!(fw.header.stored_md5.is_some());
        fw.verify_md5(&buf).expect("md5 must verify");
        let inner = fw.firmware.expect("inner firmware");
        assert_eq!(inner.items.len(), 2);
        assert_eq!(inner.model, "rk3506_lyra");
    }

    #[test]
    fn md5_mismatch_is_detected() {
        let mut buf = synth_rkfw();
        // Corrupt a payload byte inside the MD5-covered region.
        buf[RKFW_HEADER_LEN + 0x90] ^= 0xff;
        let fw = Firmware::parse(&buf).expect("parse firmware");
        assert!(matches!(
            fw.verify_md5(&buf),
            Err(ImageError::Md5Mismatch { .. })
        ));
    }

    #[test]
    fn rejects_rkfw_with_regions_past_eof() {
        let mut buf = synth_rkfw();
        buf[0x25..0x29].copy_from_slice(&0xffff_0000u32.to_le_bytes());
        assert!(matches!(
            Firmware::parse(&buf),
            Err(ImageError::OutOfBounds { .. })
        ));
    }

    #[test]
    fn rejects_non_rkfw_file() {
        let buf = vec![0u8; 200];
        assert!(matches!(
            UpdateImage::parse_header(&buf),
            Err(ImageError::BadTag { .. })
        ));
    }
}
