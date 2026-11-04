//! The `RKBOOT` container: a packed loader `.bin` as `boot_merger` produces it.
//!
//! Layout is a decode of `CRKBoot::ObjectInit` @ `0x42dc20`,
//! `CRKBoot::GetEntryProperty` @ `0x42d960` and `CRKBoot::GetEntryData`
//! @ `0x42da00`, then validated field by field against a real
//! `rk356x_spl_loader_ddr1056_v1.10.111.bin`.

use crate::crc::crc32_rockchip;

/// `BOOT` tag, stored little endian as `0x544f4f42`.
pub const TAG_BOOT: [u8; 4] = *b"BOOT";
/// `LDR ` tag, stored little endian as `0x2052444c`. Note the trailing space.
pub const TAG_LDR: [u8; 4] = *b"LDR ";

/// Size of `RKBOOT_HEAD`. Real files carry `0x0066` in the size field.
pub const HEADER_LEN: usize = 102;
/// Size of one `RKBOOT_ENTRY`. Real files carry `0x39` in every entry-size field.
pub const ENTRY_LEN: usize = 57;

/// `ucSignFlag` is compared against the literal `'S'`.
const SIGN_FLAG_SIGNED: u8 = b'S';

/// Which bootstrap stage an entry belongs to.
///
/// Values are the `ENUM_RKBOOTENTRY` constants the dispatch in
/// `GetEntryProperty` compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum EntryType {
    /// Sent with control request `0x471`: DDR init.
    Entry471 = 1,
    /// Sent with control request `0x472`: the USB loader.
    Entry472 = 2,
    /// Written to flash as part of the IDBlock, not sent to MaskROM.
    Loader = 4,
}

/// Errors from parsing a loader container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoaderError {
    /// File is smaller than the fixed header.
    TooSmall { len: usize },
    /// First four bytes were neither `BOOT` nor `LDR `.
    BadTag([u8; 4]),
    /// The trailing CRC32 did not match the computed one.
    CrcMismatch { stored: u32, computed: u32 },
    /// An entry table or payload pointed outside the file.
    OutOfBounds {
        what: &'static str,
        offset: u64,
        len: u64,
    },
}

impl core::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooSmall { len } => {
                write!(
                    f,
                    "file is {len} bytes, shorter than the {HEADER_LEN}-byte header"
                )
            }
            Self::BadTag(t) => write!(
                f,
                "bad container tag {t:02x?}: expected \"BOOT\" or \"LDR \""
            ),
            Self::CrcMismatch { stored, computed } => write!(
                f,
                "loader CRC32 mismatch: file says {stored:#010x}, computed {computed:#010x}"
            ),
            Self::OutOfBounds { what, offset, len } => {
                write!(
                    f,
                    "{what} at {offset:#x} (+{len}) runs past the end of the file"
                )
            }
        }
    }
}

impl std::error::Error for LoaderError {}

/// Build date from the header's 7-byte timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl core::fmt::Display for ReleaseTime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// One entry in the container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderEntry {
    pub entry_type: EntryType,
    /// Component name, decoded from 20 UTF-16LE code units.
    pub name: String,
    /// Offset of the payload within the container.
    pub data_offset: u32,
    /// Payload length in bytes.
    pub data_size: u32,
    /// Delay the tool honours after sending this entry, in milliseconds.
    pub data_delay: u32,
}

/// A parsed loader container.
#[derive(Debug, Clone)]
pub struct RkBoot {
    raw: Vec<u8>,
    /// `BOOT` or `LDR `.
    pub tag: [u8; 4],
    pub header_len: u16,
    pub version: u32,
    pub merge_version: u32,
    pub release_time: ReleaseTime,
    /// SoC family code. Stored as ASCII digits in reverse byte order, so
    /// RK3568 appears as `0x33353638`.
    pub chip: u32,
    /// True when `ucSignFlag` is `'S'`.
    pub signed: bool,
    /// True when `ucRc4Flag` is non-zero, meaning RC4 must **not** be applied.
    ///
    /// `ObjectInit` does `cmpb $0x0, 0x2c(%rax); setne`, so a non-zero byte
    /// disables RC4. The real rk356x loader carries `0x01` here.
    pub rc4_disabled: bool,
    pub entries: Vec<LoaderEntry>,
    /// CRC32 stored in the last four bytes.
    pub stored_crc: u32,
}

fn rd_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn rd_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Decode 20 UTF-16LE code units, stopping at the first NUL.
fn decode_name(b: &[u8]) -> String {
    let (pairs, _rest) = b.as_chunks::<2>();
    let units: Vec<u16> = pairs
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

impl RkBoot {
    /// Parse a loader container.
    ///
    /// The trailing CRC32 is checked unless `verify_crc` is false, which is
    /// useful for inspecting a file that has been modified deliberately.
    pub fn parse(data: &[u8], verify_crc: bool) -> Result<Self, LoaderError> {
        if data.len() < HEADER_LEN + 4 {
            return Err(LoaderError::TooSmall { len: data.len() });
        }
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&data[0..4]);
        if tag != TAG_BOOT && tag != TAG_LDR {
            return Err(LoaderError::BadTag(tag));
        }

        // Integrity: CRC32 over everything but the trailing 4 bytes, which hold
        // the expected value. `CRC_32` uses Rockchip's table with init 0.
        let stored_crc = rd_u32(data, data.len() - 4);
        if verify_crc {
            let computed = crc32_rockchip(&data[..data.len() - 4]);
            if computed != stored_crc {
                return Err(LoaderError::CrcMismatch {
                    stored: stored_crc,
                    computed,
                });
            }
        }

        let release_time = ReleaseTime {
            year: rd_u16(data, 0x0e),
            month: data[0x10],
            day: data[0x11],
            hour: data[0x12],
            minute: data[0x13],
            second: data[0x14],
        };

        let mut boot = Self {
            raw: data.to_vec(),
            tag,
            header_len: rd_u16(data, 0x04),
            version: rd_u32(data, 0x06),
            merge_version: rd_u32(data, 0x0a),
            release_time,
            chip: rd_u32(data, 0x15),
            signed: data[0x2b] == SIGN_FLAG_SIGNED,
            rc4_disabled: data[0x2c] != 0,
            entries: Vec::new(),
            stored_crc,
        };

        // Three parallel (count, offset, size) triples, one per entry type.
        let tables = [
            (
                EntryType::Entry471,
                data[0x19],
                rd_u32(data, 0x1a),
                data[0x1e],
            ),
            (
                EntryType::Entry472,
                data[0x1f],
                rd_u32(data, 0x20),
                data[0x24],
            ),
            (
                EntryType::Loader,
                data[0x25],
                rd_u32(data, 0x26),
                data[0x2a],
            ),
        ];

        for (entry_type, count, table_offset, stride) in tables {
            let stride = if stride == 0 { ENTRY_LEN as u8 } else { stride };
            for index in 0..count {
                let at = table_offset as u64 + stride as u64 * index as u64;
                let end = at + ENTRY_LEN as u64;
                if end > data.len() as u64 {
                    return Err(LoaderError::OutOfBounds {
                        what: "entry record",
                        offset: at,
                        len: ENTRY_LEN as u64,
                    });
                }
                let e = &data[at as usize..end as usize];
                let data_offset = rd_u32(e, 0x2d);
                let data_size = rd_u32(e, 0x31);
                if data_offset as u64 + data_size as u64 > data.len() as u64 {
                    return Err(LoaderError::OutOfBounds {
                        what: "entry payload",
                        offset: data_offset as u64,
                        len: data_size as u64,
                    });
                }
                boot.entries.push(LoaderEntry {
                    entry_type,
                    name: decode_name(&e[0x05..0x2d]),
                    data_offset,
                    data_size,
                    data_delay: rd_u32(e, 0x35),
                });
            }
        }

        Ok(boot)
    }

    /// Payload bytes for an entry.
    pub fn entry_data(&self, entry: &LoaderEntry) -> &[u8] {
        let start = entry.data_offset as usize;
        &self.raw[start..start + entry.data_size as usize]
    }

    /// Entries of one type, in file order.
    pub fn entries_of(&self, entry_type: EntryType) -> impl Iterator<Item = &LoaderEntry> {
        self.entries
            .iter()
            .filter(move |e| e.entry_type == entry_type)
    }

    /// SoC family rendered as text, e.g. `"3568"`.
    ///
    /// The field holds ASCII digits in reverse byte order, so the bytes are
    /// read back big-endian to recover the printable name.
    pub fn chip_name(&self) -> String {
        let bytes = self.chip.to_be_bytes();
        let s: String = bytes
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

    /// The whole container, for callers that need the raw bytes.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but structurally valid container, with a correct CRC.
    fn synth() -> Vec<u8> {
        let payload_471 = b"DDR-INIT-BLOB".to_vec();
        let payload_472 = b"USB-LOADER-BLOB".to_vec();
        let payload_ldr = b"FLASH-LOADER".to_vec();

        let n_entries = 3usize;
        let table_off = HEADER_LEN;
        let data_off = table_off + n_entries * ENTRY_LEN;

        let mut buf = vec![0u8; data_off];
        buf[0..4].copy_from_slice(&TAG_LDR);
        buf[0x04..0x06].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        buf[0x06..0x0a].copy_from_slice(&0x0000_0101u32.to_le_bytes());
        buf[0x0a..0x0e].copy_from_slice(&0x0100_0000u32.to_le_bytes());
        buf[0x0e..0x10].copy_from_slice(&2021u16.to_le_bytes());
        buf[0x10] = 12;
        buf[0x11] = 24;
        buf[0x12] = 17;
        buf[0x13] = 46;
        buf[0x14] = 54;
        // "3568" stored in reverse byte order, as real files do.
        buf[0x15..0x19].copy_from_slice(b"8653");
        buf[0x19] = 1; // 471 count
        buf[0x1a..0x1e].copy_from_slice(&(table_off as u32).to_le_bytes());
        buf[0x1e] = ENTRY_LEN as u8;
        buf[0x1f] = 1; // 472 count
        buf[0x20..0x24].copy_from_slice(&((table_off + ENTRY_LEN) as u32).to_le_bytes());
        buf[0x24] = ENTRY_LEN as u8;
        buf[0x25] = 1; // loader count
        buf[0x26..0x2a].copy_from_slice(&((table_off + 2 * ENTRY_LEN) as u32).to_le_bytes());
        buf[0x2a] = ENTRY_LEN as u8;
        buf[0x2b] = 0; // unsigned
        buf[0x2c] = 1; // RC4 disabled

        let write_entry =
            |buf: &mut Vec<u8>, slot: usize, ty: u32, name: &str, off: usize, size: usize| {
                let base = table_off + slot * ENTRY_LEN;
                buf[base] = ENTRY_LEN as u8;
                buf[base + 1..base + 5].copy_from_slice(&ty.to_le_bytes());
                for (i, u) in name.encode_utf16().take(20).enumerate() {
                    let at = base + 0x05 + i * 2;
                    buf[at..at + 2].copy_from_slice(&u.to_le_bytes());
                }
                buf[base + 0x2d..base + 0x31].copy_from_slice(&(off as u32).to_le_bytes());
                buf[base + 0x31..base + 0x35].copy_from_slice(&(size as u32).to_le_bytes());
                buf[base + 0x35..base + 0x39].copy_from_slice(&30u32.to_le_bytes());
            };

        let o471 = data_off;
        let o472 = o471 + payload_471.len();
        let oldr = o472 + payload_472.len();
        write_entry(&mut buf, 0, 1, "ddr", o471, payload_471.len());
        write_entry(&mut buf, 1, 2, "usbplug", o472, payload_472.len());
        write_entry(&mut buf, 2, 4, "loader", oldr, payload_ldr.len());

        buf.extend_from_slice(&payload_471);
        buf.extend_from_slice(&payload_472);
        buf.extend_from_slice(&payload_ldr);

        let crc = crc32_rockchip(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        buf
    }

    #[test]
    fn parses_synthetic_container() {
        let buf = synth();
        let boot = RkBoot::parse(&buf, true).expect("parse");
        assert_eq!(boot.tag, TAG_LDR);
        assert_eq!(boot.header_len, HEADER_LEN as u16);
        assert_eq!(boot.chip_name(), "3568");
        assert!(boot.rc4_disabled);
        assert!(!boot.signed);
        assert_eq!(boot.entries.len(), 3);
        assert_eq!(boot.release_time.year, 2021);
        assert_eq!(boot.release_time.to_string(), "2021-12-24 17:46:54");
    }

    #[test]
    fn exposes_entries_by_type_with_payloads() {
        let buf = synth();
        let boot = RkBoot::parse(&buf, true).expect("parse");

        let e471: Vec<_> = boot.entries_of(EntryType::Entry471).collect();
        assert_eq!(e471.len(), 1);
        assert_eq!(e471[0].name, "ddr");
        assert_eq!(boot.entry_data(e471[0]), b"DDR-INIT-BLOB");

        let e472: Vec<_> = boot.entries_of(EntryType::Entry472).collect();
        assert_eq!(e472[0].name, "usbplug");
        assert_eq!(boot.entry_data(e472[0]), b"USB-LOADER-BLOB");

        let ldr: Vec<_> = boot.entries_of(EntryType::Loader).collect();
        assert_eq!(ldr[0].name, "loader");
        assert_eq!(boot.entry_data(ldr[0]), b"FLASH-LOADER");
    }

    #[test]
    fn rejects_bad_tag() {
        let mut buf = synth();
        buf[0] = b'X';
        assert!(matches!(
            RkBoot::parse(&buf, false),
            Err(LoaderError::BadTag(_))
        ));
    }

    #[test]
    fn detects_corrupted_payload_via_crc() {
        let mut buf = synth();
        let len = buf.len();
        buf[len - 10] ^= 0xff;
        assert!(matches!(
            RkBoot::parse(&buf, true),
            Err(LoaderError::CrcMismatch { .. })
        ));
        // The same file parses when the check is waived.
        assert!(RkBoot::parse(&buf, false).is_ok());
    }

    #[test]
    fn rejects_entry_payload_past_eof() {
        let mut buf = synth();
        // Point the first entry's payload far beyond the file.
        let base = HEADER_LEN;
        buf[base + 0x31..base + 0x35].copy_from_slice(&0xffff_0000u32.to_le_bytes());
        assert!(matches!(
            RkBoot::parse(&buf, false),
            Err(LoaderError::OutOfBounds { .. })
        ));
    }

    #[test]
    fn rejects_truncated_file() {
        assert!(matches!(
            RkBoot::parse(&[0u8; 10], false),
            Err(LoaderError::TooSmall { len: 10 })
        ));
    }
}
