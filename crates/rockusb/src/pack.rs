//! Build an `update.img` from loose files.
//!
//! This is the inverse of [`crate::image`], and the layout rules come from the
//! same place: a real Luckfox Lyra Zero W `update.img`. Payloads are aligned to
//! [`RKAF_PAYLOAD_ALIGN`], the first starts at that boundary rather than
//! immediately after the table, and the length the header declares is the end
//! of the last payload with no trailing padding.
//!
//! Packing is host-side. Nothing here runs in the browser.

use crate::image::{
    RKAF_ITEMS_OFFSET, RKAF_ITEM_LEN, RKAF_MAX_ITEMS, RKAF_PAYLOAD_ALIGN, RKFW_HEADER_LEN,
    TAG_RKAF, TAG_RKFW,
};
use crate::loader::ReleaseTime;

/// One file to place in the container.
#[derive(Debug, Clone)]
pub struct PackItem {
    /// Logical name, as the `package-file` manifest spells it.
    pub name: String,
    /// Path as the manifest spells it, stored in the item record.
    pub file_name: String,
    /// Partition size in sectors, or `0xffff_ffff` for a growing partition.
    pub partition_sectors: u32,
    /// Flash address in sectors, or `0xffff_ffff` for no fixed address.
    pub target_sector: u32,
    /// File contents.
    pub data: Vec<u8>,
}

/// Everything needed to build a container.
#[derive(Debug, Clone)]
pub struct PackSpec {
    pub model: String,
    pub id: String,
    pub manufacturer: String,
    /// Packed firmware version, e.g. `0x0801_0000` for 8.1.
    pub version: u32,
    /// Outer `RKFW` version field.
    pub fw_version: u32,
    pub merge_version: u32,
    pub release_time: ReleaseTime,
    /// SoC code, stored reversed, e.g. `350F` becomes `F053` on disk.
    pub chip: u32,
    /// The loader placed between the header and the firmware.
    pub loader: Vec<u8>,
    pub items: Vec<PackItem>,
}

/// Why a container could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    NoItems,
    TooManyItems(usize),
    /// A name or path does not fit its fixed-width field.
    FieldTooLong {
        field: &'static str,
        value: String,
        max: usize,
    },
}

impl core::fmt::Display for PackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoItems => write!(f, "nothing to pack"),
            Self::TooManyItems(n) => {
                write!(
                    f,
                    "{n} items, more than the {RKAF_MAX_ITEMS} a container holds"
                )
            }
            Self::FieldTooLong { field, value, max } => {
                write!(f, "{field} {value:?} is longer than {max} bytes")
            }
        }
    }
}

impl std::error::Error for PackError {}

fn put_str(
    buf: &mut [u8],
    at: usize,
    s: &str,
    max: usize,
    field: &'static str,
) -> Result<(), PackError> {
    // Leave room for the terminating NUL, as the vendor files do.
    if s.len() >= max {
        return Err(PackError::FieldTooLong {
            field,
            value: s.to_string(),
            max: max - 1,
        });
    }
    buf[at..at + s.len()].copy_from_slice(s.as_bytes());
    Ok(())
}

fn align_up(n: usize, to: usize) -> usize {
    n.div_ceil(to) * to
}

/// Build the inner `RKAF` blob.
pub fn build_rkaf(spec: &PackSpec) -> Result<Vec<u8>, PackError> {
    if spec.items.is_empty() {
        return Err(PackError::NoItems);
    }
    if spec.items.len() > RKAF_MAX_ITEMS {
        return Err(PackError::TooManyItems(spec.items.len()));
    }

    // Lay the payloads out first so the item records can name their offsets.
    let table_end = RKAF_ITEMS_OFFSET + spec.items.len() * RKAF_ITEM_LEN;
    let mut offsets = Vec::with_capacity(spec.items.len());
    let mut cursor = align_up(table_end.max(1), RKAF_PAYLOAD_ALIGN);
    for item in &spec.items {
        offsets.push(cursor);
        cursor = align_up(cursor + item.data.len(), RKAF_PAYLOAD_ALIGN);
    }
    // The declared length stops at the end of the last payload, unpadded.
    let last = offsets[offsets.len() - 1] + spec.items[spec.items.len() - 1].data.len();

    let mut buf = vec![0u8; table_end];
    buf[0..4].copy_from_slice(&TAG_RKAF);
    buf[0x04..0x08].copy_from_slice(&(last as u32).to_le_bytes());
    put_str(&mut buf, 0x08, &spec.model, 0x22, "model")?;
    put_str(&mut buf, 0x2a, &spec.id, 0x1e, "id")?;
    put_str(&mut buf, 0x48, &spec.manufacturer, 0x38, "manufacturer")?;
    buf[0x84..0x88].copy_from_slice(&spec.version.to_le_bytes());
    buf[0x88..0x8c].copy_from_slice(&(spec.items.len() as u32).to_le_bytes());

    for (i, item) in spec.items.iter().enumerate() {
        let b = RKAF_ITEMS_OFFSET + i * RKAF_ITEM_LEN;
        put_str(&mut buf, b, &item.name, 0x20, "item name")?;
        put_str(&mut buf, b + 0x20, &item.file_name, 0x3c, "item path")?;
        buf[b + 0x5c..b + 0x60].copy_from_slice(&item.partition_sectors.to_le_bytes());
        buf[b + 0x60..b + 0x64].copy_from_slice(&(offsets[i] as u32).to_le_bytes());
        buf[b + 0x64..b + 0x68].copy_from_slice(&item.target_sector.to_le_bytes());
        let pages = item.data.len().div_ceil(RKAF_PAYLOAD_ALIGN) as u32;
        buf[b + 0x68..b + 0x6c].copy_from_slice(&pages.to_le_bytes());
        buf[b + 0x6c..b + 0x70].copy_from_slice(&(item.data.len() as u32).to_le_bytes());
    }

    buf.resize(last, 0);
    for (i, item) in spec.items.iter().enumerate() {
        let at = offsets[i];
        buf[at..at + item.data.len()].copy_from_slice(&item.data);
    }
    Ok(buf)
}

/// Build a whole `update.img`: `RKFW` header, loader, firmware, trailing MD5.
pub fn build_update_img(spec: &PackSpec) -> Result<Vec<u8>, PackError> {
    let rkaf = build_rkaf(spec)?;

    let boot_offset = RKFW_HEADER_LEN;
    let fw_offset = boot_offset + spec.loader.len();

    let mut out = vec![0u8; RKFW_HEADER_LEN];
    out[0..4].copy_from_slice(&TAG_RKFW);
    out[0x04..0x06].copy_from_slice(&(RKFW_HEADER_LEN as u16).to_le_bytes());
    out[0x06..0x0a].copy_from_slice(&spec.fw_version.to_le_bytes());
    out[0x0a..0x0e].copy_from_slice(&spec.merge_version.to_le_bytes());
    out[0x0e..0x10].copy_from_slice(&spec.release_time.year.to_le_bytes());
    out[0x10] = spec.release_time.month;
    out[0x11] = spec.release_time.day;
    out[0x12] = spec.release_time.hour;
    out[0x13] = spec.release_time.minute;
    out[0x14] = spec.release_time.second;
    out[0x15..0x19].copy_from_slice(&spec.chip.to_le_bytes());
    out[0x19..0x1d].copy_from_slice(&(boot_offset as u32).to_le_bytes());
    out[0x1d..0x21].copy_from_slice(&(spec.loader.len() as u32).to_le_bytes());
    out[0x21..0x25].copy_from_slice(&(fw_offset as u32).to_le_bytes());
    out[0x25..0x29].copy_from_slice(&(rkaf.len() as u32).to_le_bytes());

    out.extend_from_slice(&spec.loader);
    out.extend_from_slice(&rkaf);

    // The MD5 covers everything up to the end of the firmware.
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(&out);
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    out.extend_from_slice(hex.as_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{Firmware, RkafFirmware, NO_FLASH_ADDRESS};

    fn spec() -> PackSpec {
        PackSpec {
            model: "RK3506".into(),
            id: "007".into(),
            manufacturer: " RK3506".into(),
            version: 0x0801_0000,
            fw_version: 0x0000_0101,
            merge_version: 0x0100_0000,
            release_time: ReleaseTime {
                year: 2026,
                month: 10,
                day: 19,
                hour: 10,
                minute: 51,
                second: 18,
            },
            chip: u32::from_le_bytes(*b"F053"),
            loader: vec![0xa5; 4096],
            items: vec![
                PackItem {
                    name: "package-file".into(),
                    file_name: "package-file".into(),
                    partition_sectors: 0,
                    target_sector: NO_FLASH_ADDRESS,
                    data: b"# NAME\tPATH\n".to_vec(),
                },
                PackItem {
                    name: "uboot".into(),
                    file_name: "uboot.img".into(),
                    partition_sectors: 0x2000,
                    target_sector: 0x2000,
                    data: vec![0x11; 5000],
                },
                PackItem {
                    name: "rootfs".into(),
                    file_name: "rootfs.img".into(),
                    partition_sectors: 0xffff_ffff,
                    target_sector: 0x10000,
                    data: vec![0x22; 70000],
                },
            ],
        }
    }

    /// What is packed must parse back with the same targets and payloads.
    #[test]
    fn round_trips_through_the_parser() {
        let img = build_update_img(&spec()).expect("pack");
        let fw = Firmware::parse(&img).expect("parse packed image");
        fw.verify_md5(&img).expect("md5 must verify");

        let inner = fw.firmware.expect("inner firmware");
        assert_eq!(inner.model, "RK3506");
        assert_eq!(inner.id, "007");
        assert_eq!(inner.version_string(), "8.1");
        assert_eq!(inner.items.len(), 3);

        let uboot = &inner.items[1];
        assert_eq!(uboot.name, "uboot");
        assert_eq!(uboot.target_sector, 0x2000);
        assert_eq!(uboot.data_size, 5000);
        assert_eq!(inner.item_data(uboot), Some(&vec![0x11u8; 5000][..]));

        let rootfs = &inner.items[2];
        assert_eq!(rootfs.target_sector, 0x10000);
        assert_eq!(inner.item_data(rootfs), Some(&vec![0x22u8; 70000][..]));
    }

    /// Payload placement must match what the vendor image does.
    #[test]
    fn payloads_are_aligned_and_the_length_is_exact() {
        let s = spec();
        let rkaf = build_rkaf(&s).expect("pack");
        let fw = RkafFirmware::parse(&rkaf).expect("parse");

        for item in &fw.items {
            assert_eq!(
                item.data_offset as usize % RKAF_PAYLOAD_ALIGN,
                0,
                "{} is not aligned",
                item.name
            );
        }
        assert_eq!(fw.items[0].data_offset as usize, RKAF_PAYLOAD_ALIGN);

        let last = fw.items.last().unwrap();
        assert_eq!(
            fw.declared_len,
            last.data_offset as u64 + last.data_size as u64,
            "declared length stops at the last payload, unpadded"
        );
        assert_eq!(rkaf.len() as u64, fw.declared_len);
    }

    #[test]
    fn page_count_matches_the_byte_size() {
        let rkaf = build_rkaf(&spec()).expect("pack");
        let fw = RkafFirmware::parse(&rkaf).expect("parse");
        for item in &fw.items {
            assert_eq!(item.padded_pages, (item.data_size).div_ceil(2048));
        }
    }

    #[test]
    fn rejects_an_overlong_name() {
        let mut s = spec();
        s.items[0].name = "x".repeat(40);
        assert!(matches!(
            build_rkaf(&s),
            Err(PackError::FieldTooLong {
                field: "item name",
                ..
            })
        ));
    }

    #[test]
    fn rejects_an_empty_spec() {
        let mut s = spec();
        s.items.clear();
        assert_eq!(build_rkaf(&s), Err(PackError::NoItems));
    }
}
