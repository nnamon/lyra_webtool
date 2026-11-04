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
//! The inner `RKAF` layout was recovered from a real Luckfox Lyra Zero W
//! `update.img` (Buildroot 261020, FIRMWARE_VER 8.1) rather than from the
//! vendor binary, which contains no `RKAF` tag comparison at all. The decoded
//! targets cross-check against that image's `parameter.txt`: `uboot` at
//! `0x2000`, `boot` at `0x4000` and `rootfs` at `0x10000`, with the partition
//! sizes in each item matching the `mtdparts` line.
//!
//! A wrong offset here would write a payload to the wrong flash address, so
//! [`RkafFirmware::parse`] validates hard: it requires the tag, rejects an
//! implausible item count, and rejects any item whose payload falls outside
//! the container.

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
pub(crate) const RKAF_ITEMS_OFFSET: usize = 0x8c;
/// Stride of one `RKAF` item record.
pub(crate) const RKAF_ITEM_LEN: usize = 0x70;
/// Offset of the item count.
pub(crate) const RKAF_COUNT_OFFSET: usize = 0x88;
/// Offset of the packed firmware version, e.g. `0x08010000` for 8.1.
pub(crate) const RKAF_VERSION_OFFSET: usize = 0x84;
/// Sentinel in an item's flash address meaning "no fixed LBA".
pub const NO_FLASH_ADDRESS: u32 = 0xffff_ffff;
/// Sentinel in an item's partition size meaning "grow to fill the device".
pub const PARTITION_GROWS: u32 = 0xffff_ffff;
/// Item count above which the container is treated as malformed.
pub(crate) const RKAF_MAX_ITEMS: usize = 64;
/// Every payload starts on a 2048-byte boundary, and so does the first one.
///
/// Read off a real Luckfox `update.img`: the header and table occupy 812
/// bytes, the first payload sits at 2048, and each later payload starts at the
/// previous one's end rounded up to 2048.
pub const RKAF_PAYLOAD_ALIGN: usize = 2048;

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

/// How an item reaches the device.
///
/// Only [`ItemKind::Image`] is a plain copy to an LBA. The others need
/// transforms this crate does not implement, so they must not be written as
/// raw bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A partition image, written to `target_sector` verbatim.
    Image,
    /// The partition table. For a `TYPE: GPT` device the host builds a GPT from
    /// `parameter.txt`; writing the text file to LBA 0 would destroy the GPT.
    Parameter,
    /// The bootloader. It becomes an IDBlock, which is RC4'd and laid out
    /// across sectors 0, 2 and 3, not copied to an LBA.
    Loader,
    /// A manifest listing the other items. Never written.
    Manifest,
}

impl ItemKind {
    /// Why this item cannot be written as a plain copy, if it cannot.
    pub const fn unsupported_reason(self) -> Option<&'static str> {
        match self {
            Self::Image => None,
            Self::Parameter => Some("needs a GPT built from parameter.txt"),
            Self::Loader => Some("needs IDBlock generation"),
            Self::Manifest => Some("manifest, not written to flash"),
        }
    }
}

/// One image inside the inner `RKAF` firmware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareItem {
    /// Logical name, e.g. `uboot`, `boot`, `rootfs`, `parameter`.
    pub name: String,
    /// Original filename inside the packed firmware.
    pub file_name: String,
    /// Declared partition size in sectors, matching the `mtdparts` entry.
    /// [`PARTITION_GROWS`] means the partition fills the rest of the device.
    pub partition_sectors: u32,
    /// Payload offset in bytes, relative to the start of the `RKAF` blob.
    pub data_offset: u32,
    /// Destination address on flash, in 512-byte sectors.
    /// [`NO_FLASH_ADDRESS`] means the item has no fixed LBA.
    pub target_sector: u32,
    /// Payload length rounded up to 2048-byte pages.
    pub padded_pages: u32,
    /// Payload length in bytes.
    pub data_size: u32,
}

impl FirmwareItem {
    /// Classify how this item reaches the device.
    pub fn kind(&self) -> ItemKind {
        if self.name.eq_ignore_ascii_case("parameter") {
            return ItemKind::Parameter;
        }
        if self.target_sector == NO_FLASH_ADDRESS {
            if self.name.eq_ignore_ascii_case("bootloader") {
                return ItemKind::Loader;
            }
            return ItemKind::Manifest;
        }
        ItemKind::Image
    }

    /// True when this item is a plain copy to an LBA.
    pub fn is_writable(&self) -> bool {
        self.kind() == ItemKind::Image && self.data_size > 0
    }

    /// Destination byte offset on flash.
    pub fn target_byte_offset(&self) -> u64 {
        self.target_sector as u64 * SECTOR_SIZE as u64
    }

    /// True when the payload is larger than the partition declared for it.
    pub fn overflows_partition(&self) -> bool {
        if self.partition_sectors == PARTITION_GROWS {
            return false;
        }
        let capacity = self.partition_sectors as u64 * SECTOR_SIZE as u64;
        capacity > 0 && self.data_size as u64 > capacity
    }
}

/// The inner `RKAF` firmware.
#[derive(Debug, Clone)]
pub struct RkafFirmware {
    /// The bytes we hold. This may be only a prefix of the blob.
    raw: Vec<u8>,
    /// Total blob length, from the header at `0x04`.
    ///
    /// Item payloads are bounds-checked against this rather than against
    /// `raw.len()`, so a browser can parse the item table from a short prefix
    /// of a 115 MiB image.
    pub declared_len: u64,
    pub model: String,
    pub id: String,
    pub manufacturer: String,
    pub version: u32,
    pub items: Vec<FirmwareItem>,
}

impl RkafFirmware {
    /// Parse a complete `RKAF` blob.
    pub fn parse(data: &[u8]) -> Result<Self, ImageError> {
        let fw = Self::parse_table(data)?;
        if (data.len() as u64) < fw.declared_len {
            return Err(ImageError::OutOfBounds {
                what: "RKAF blob",
                offset: 0,
                len: fw.declared_len,
                file_len: data.len() as u64,
            });
        }
        Ok(fw)
    }

    /// Parse the header and item table from a prefix of the blob.
    ///
    /// Item payloads are checked against the length the header declares, so
    /// this accepts a buffer far shorter than the whole firmware.
    /// [`Self::item_data`] returns `None` for payloads the prefix doesn't hold.
    pub fn parse_table(data: &[u8]) -> Result<Self, ImageError> {
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

        let count = rd_u32(data, RKAF_COUNT_OFFSET);
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

        let declared_len = rd_u32(data, 0x04) as u64;
        if declared_len < table_end as u64 {
            return Err(ImageError::OutOfBounds {
                what: "RKAF declared length",
                offset: 0,
                len: declared_len,
                file_len: table_end as u64,
            });
        }

        let mut items = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let base = RKAF_ITEMS_OFFSET + i * RKAF_ITEM_LEN;
            let item = FirmwareItem {
                name: rd_str(data, base, 0x20),
                file_name: rd_str(data, base + 0x20, 0x3c),
                partition_sectors: rd_u32(data, base + 0x5c),
                data_offset: rd_u32(data, base + 0x60),
                target_sector: rd_u32(data, base + 0x64),
                padded_pages: rd_u32(data, base + 0x68),
                data_size: rd_u32(data, base + 0x6c),
            };
            // Refuse a payload that does not lie inside the blob. This is the
            // guard that turns a layout mismatch into an error rather than a
            // write to the wrong flash address.
            if item.data_size > 0 {
                let end = item.data_offset as u64 + item.data_size as u64;
                if end > declared_len {
                    return Err(ImageError::OutOfBounds {
                        what: "RKAF item payload",
                        offset: item.data_offset as u64,
                        len: item.data_size as u64,
                        file_len: declared_len,
                    });
                }
            }
            items.push(item);
        }

        Ok(Self {
            raw: data.to_vec(),
            declared_len,
            model: rd_str(data, 0x08, 0x22),
            id: rd_str(data, 0x2a, 0x1e),
            manufacturer: rd_str(data, 0x48, 0x38),
            version: rd_u32(data, RKAF_VERSION_OFFSET),
            items,
        })
    }

    /// Payload bytes for `item`, when this buffer holds them.
    pub fn item_data(&self, item: &FirmwareItem) -> Option<&[u8]> {
        let start = item.data_offset as usize;
        let end = start.checked_add(item.data_size as usize)?;
        self.raw.get(start..end)
    }

    /// Items that are plain copies to an LBA.
    pub fn writable_items(&self) -> impl Iterator<Item = &FirmwareItem> {
        self.items.iter().filter(|i| i.is_writable())
    }

    /// Firmware version as text, decoded from the packed `0xMMmm0000` word.
    pub fn version_string(&self) -> String {
        format!(
            "{}.{}",
            (self.version >> 24) & 0xff,
            (self.version >> 16) & 0xff
        )
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
        let n = 3usize;
        let table_end = RKAF_ITEMS_OFFSET + n * RKAF_ITEM_LEN;
        let p0 = b"UBOOT-PAYLOAD".to_vec();
        let p1 = b"parameter-text".to_vec();
        let o0 = table_end;
        let o1 = o0 + p0.len();
        let total = o1 + p1.len();

        let mut buf = vec![0u8; table_end];
        buf[0..4].copy_from_slice(&TAG_RKAF);
        buf[0x04..0x08].copy_from_slice(&(total as u32).to_le_bytes());
        let put = |buf: &mut Vec<u8>, at: usize, s: &str| {
            buf[at..at + s.len()].copy_from_slice(s.as_bytes());
        };
        put(&mut buf, 0x08, "RK3506");
        put(&mut buf, 0x2a, "007");
        put(&mut buf, 0x48, " RK3506");
        buf[RKAF_VERSION_OFFSET..RKAF_VERSION_OFFSET + 4]
            .copy_from_slice(&0x0801_0000u32.to_le_bytes());
        buf[RKAF_COUNT_OFFSET..RKAF_COUNT_OFFSET + 4].copy_from_slice(&(n as u32).to_le_bytes());

        #[allow(clippy::too_many_arguments)]
        let item = |buf: &mut Vec<u8>,
                    i: usize,
                    name: &str,
                    file: &str,
                    part: u32,
                    off: usize,
                    target: u32,
                    size: usize| {
            let b = RKAF_ITEMS_OFFSET + i * RKAF_ITEM_LEN;
            buf[b..b + name.len()].copy_from_slice(name.as_bytes());
            buf[b + 0x20..b + 0x20 + file.len()].copy_from_slice(file.as_bytes());
            buf[b + 0x5c..b + 0x60].copy_from_slice(&part.to_le_bytes());
            buf[b + 0x60..b + 0x64].copy_from_slice(&(off as u32).to_le_bytes());
            buf[b + 0x64..b + 0x68].copy_from_slice(&target.to_le_bytes());
            buf[b + 0x68..b + 0x6c].copy_from_slice(&(size.div_ceil(2048) as u32).to_le_bytes());
            buf[b + 0x6c..b + 0x70].copy_from_slice(&(size as u32).to_le_bytes());
        };
        item(
            &mut buf,
            0,
            "uboot",
            "uboot.img",
            0x2000,
            o0,
            0x2000,
            p0.len(),
        );
        item(
            &mut buf,
            1,
            "parameter",
            "parameter.txt",
            0x2000,
            o1,
            0,
            p1.len(),
        );
        item(
            &mut buf,
            2,
            "bootloader",
            "MiniLoaderAll.bin",
            0,
            o1,
            NO_FLASH_ADDRESS,
            p1.len(),
        );

        buf.extend_from_slice(&p0);
        buf.extend_from_slice(&p1);
        buf
    }

    #[test]
    fn parses_rkaf_items_and_payloads() {
        let blob = synth_rkaf();
        let fw = RkafFirmware::parse(&blob).expect("parse rkaf");
        assert_eq!(fw.model, "RK3506");
        assert_eq!(fw.id, "007");
        assert_eq!(fw.version_string(), "8.1");
        assert_eq!(fw.items.len(), 3);

        let uboot = &fw.items[0];
        assert_eq!(uboot.name, "uboot");
        assert_eq!(uboot.target_sector, 0x2000);
        assert_eq!(uboot.partition_sectors, 0x2000);
        assert_eq!(uboot.kind(), ItemKind::Image);
        assert!(uboot.is_writable());
        assert_eq!(fw.item_data(uboot), Some(&b"UBOOT-PAYLOAD"[..]));
    }

    /// The parameter item carries a real LBA of 0, so a naive "target != -1"
    /// test would write parameter.txt over the GPT.
    #[test]
    fn parameter_item_is_not_a_plain_write() {
        let fw = RkafFirmware::parse(&synth_rkaf()).expect("parse");
        let param = fw.items.iter().find(|i| i.name == "parameter").unwrap();
        assert_eq!(param.target_sector, 0, "parameter really does target LBA 0");
        assert_eq!(param.kind(), ItemKind::Parameter);
        assert!(!param.is_writable(), "must never be copied raw to LBA 0");
        assert!(param.kind().unsupported_reason().is_some());
    }

    #[test]
    fn bootloader_item_needs_idblock() {
        let fw = RkafFirmware::parse(&synth_rkaf()).expect("parse");
        let ldr = fw.items.iter().find(|i| i.name == "bootloader").unwrap();
        assert_eq!(ldr.kind(), ItemKind::Loader);
        assert!(!ldr.is_writable());
        assert_eq!(fw.writable_items().count(), 1, "only uboot is a plain copy");
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
        buf[0x15..0x19].copy_from_slice(b"F053");

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
        assert_eq!(h.chip_name(), "350F");
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
        assert_eq!(inner.items.len(), 3);
        assert_eq!(inner.model, "RK3506");
    }

    #[test]
    fn md5_mismatch_is_detected() {
        let mut buf = synth_rkfw();
        // Corrupt a payload byte inside the MD5-covered region.
        buf[RKFW_HEADER_LEN + 0x20] ^= 0xff;
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
