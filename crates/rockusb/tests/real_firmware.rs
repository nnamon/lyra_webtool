//! Parser checks against a real vendor firmware header.
//!
//! The fixture is the first 4096 bytes of the inner `RKAF` blob from
//! `Luckfox_Lyra_Zero_W_Flash_261020` (Buildroot, FIRMWARE_VER 8.1), taken at
//! the firmware offset the `RKFW` header declares. It is the header and item
//! table only, with no payloads.
//!
//! These values are the reason the `RKAF` layout is trusted at all: the item
//! targets and partition sizes agree with that image's `parameter.txt`:
//!
//! ```text
//! mtdparts=:0x00002000@0x00002000(uboot),0x00006000@0x00004000(boot),-@0x00010000(rootfs:grow)
//! ```

use rockusb::image::{ItemKind, RkafFirmware, NO_FLASH_ADDRESS, PARTITION_GROWS};

const FIXTURE: &[u8] = include_bytes!("fixtures/lyra_zero_w_rkaf_head.bin");

fn parse() -> RkafFirmware {
    RkafFirmware::parse_table(FIXTURE).expect("real RKAF header must parse")
}

#[test]
fn header_fields_decode() {
    let fw = parse();
    assert_eq!(fw.model, "RK3506");
    assert_eq!(fw.id, "007", "MACHINE_ID from parameter.txt");
    assert_eq!(
        fw.manufacturer, " RK3506",
        "leading space is in the vendor file"
    );
    assert_eq!(fw.version, 0x0801_0000);
    assert_eq!(
        fw.version_string(),
        "8.1",
        "FIRMWARE_VER from parameter.txt"
    );
    assert_eq!(fw.declared_len, 120_715_264);
    assert_eq!(fw.items.len(), 6);
}

/// The whole point of the fixture: every target must match `mtdparts`.
#[test]
fn item_targets_match_parameter_txt() {
    let fw = parse();
    let by_name = |n: &str| {
        fw.items
            .iter()
            .find(|i| i.name == n)
            .unwrap_or_else(|| panic!("no item named {n}"))
    };

    let uboot = by_name("uboot");
    assert_eq!(uboot.target_sector, 0x2000);
    assert_eq!(uboot.partition_sectors, 0x2000);
    assert_eq!(uboot.data_size, 4_194_304);

    let boot = by_name("boot");
    assert_eq!(boot.target_sector, 0x4000);
    assert_eq!(boot.partition_sectors, 0x6000);
    assert_eq!(boot.data_size, 8_109_568);

    let rootfs = by_name("rootfs");
    assert_eq!(rootfs.target_sector, 0x10000);
    assert_eq!(rootfs.partition_sectors, PARTITION_GROWS, "rootfs:grow");
    assert_eq!(rootfs.data_size, 108_134_400);
}

/// `parameter` targets LBA 0 for real. Writing it as raw bytes would put
/// `parameter.txt` over the GPT, so it must not count as a plain write.
#[test]
fn parameter_targets_lba_zero_but_is_not_writable() {
    let fw = parse();
    let param = fw.items.iter().find(|i| i.name == "parameter").unwrap();
    assert_eq!(param.target_sector, 0);
    assert_eq!(param.file_name, "parameter.txt");
    assert_eq!(param.kind(), ItemKind::Parameter);
    assert!(!param.is_writable());
}

#[test]
fn bootloader_has_no_lba_and_needs_an_idblock() {
    let fw = parse();
    let ldr = fw.items.iter().find(|i| i.name == "bootloader").unwrap();
    assert_eq!(ldr.file_name, "MiniLoaderAll.bin");
    assert_eq!(ldr.target_sector, NO_FLASH_ADDRESS);
    // Matches the standalone MiniLoaderAll.bin shipped beside update.img.
    assert_eq!(ldr.data_size, 268_736);
    assert_eq!(ldr.kind(), ItemKind::Loader);
    assert!(!ldr.is_writable());
}

#[test]
fn only_the_three_partition_images_are_plain_writes() {
    let fw = parse();
    let names: Vec<&str> = fw.writable_items().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["uboot", "boot", "rootfs"]);
}

#[test]
fn package_file_manifest_is_skipped() {
    let fw = parse();
    let pkg = fw.items.iter().find(|i| i.name == "package-file").unwrap();
    assert_eq!(pkg.kind(), ItemKind::Manifest);
    assert!(!pkg.is_writable());
}

/// Every payload sits inside the blob the header declares, and no image
/// overflows the partition it targets.
#[test]
fn payloads_and_partitions_are_consistent() {
    let fw = parse();
    for item in &fw.items {
        let end = item.data_offset as u64 + item.data_size as u64;
        assert!(
            end <= fw.declared_len,
            "{} runs past the blob: {end} > {}",
            item.name,
            fw.declared_len
        );
        assert!(
            !item.overflows_partition(),
            "{} is larger than its partition",
            item.name
        );
    }
}

/// The payload length in 2048-byte pages must agree with the byte length.
#[test]
fn padded_page_count_matches_byte_size() {
    let fw = parse();
    for item in &fw.items {
        let expected = (item.data_size as u64).div_ceil(2048) as u32;
        assert_eq!(
            item.padded_pages, expected,
            "{} pages {} vs size {}",
            item.name, item.padded_pages, item.data_size
        );
    }
}

/// A prefix this short cannot hold any payload, which is the browser's case.
#[test]
fn prefix_parse_reports_no_payload_bytes() {
    let fw = parse();
    for item in fw.writable_items() {
        assert_eq!(fw.item_data(item), None);
    }
}
