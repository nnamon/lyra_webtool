//! Build a small `update.img` for exercising `rkimage` without a vendor file.
//!
//! The layout mirrors a real Luckfox Lyra Zero W image, with the same item
//! names, partition sizes and flash sectors, but tiny payloads.
//!
//! ```sh
//! cargo run -p rockusb --example make_test_image -- /tmp/update.img
//! cargo run -p rockusb --bin rkimage -- roundtrip /tmp/update.img
//! ```

use rockusb::image::{NO_FLASH_ADDRESS, PARTITION_GROWS};
use rockusb::loader::ReleaseTime;
use rockusb::pack::{build_update_img, PackItem, PackSpec};

fn main() {
    let Some(out) = std::env::args().nth(1) else {
        eprintln!("usage: make_test_image <update.img>");
        std::process::exit(2);
    };

    let item = |name: &str, file: &str, part: u32, target: u32, len: usize, fill: u8| PackItem {
        name: name.into(),
        file_name: file.into(),
        partition_sectors: part,
        target_sector: target,
        data: vec![fill; len],
    };

    let mut chip = b"350F".to_vec();
    chip.reverse();

    let spec = PackSpec {
        model: "RK3506".into(),
        id: "007".into(),
        // The leading space is in the vendor file; keep it.
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
        chip: u32::from_le_bytes([chip[0], chip[1], chip[2], chip[3]]),
        loader: vec![0xa5; 268_736],
        items: vec![
            PackItem {
                name: "package-file".into(),
                file_name: "package-file".into(),
                partition_sectors: 0,
                target_sector: NO_FLASH_ADDRESS,
                data: b"# NAME\tPATH\npackage-file\tpackage-file\nuboot\tuboot.img\n".to_vec(),
            },
            item("parameter", "parameter.txt", 0x2000, 0, 345, 0x50),
            item(
                "bootloader",
                "MiniLoaderAll.bin",
                0,
                NO_FLASH_ADDRESS,
                268_736,
                0xa5,
            ),
            item("uboot", "uboot.img", 0x2000, 0x2000, 65_536, 0x11),
            item("boot", "boot.img", 0x6000, 0x4000, 131_072, 0x22),
            item(
                "rootfs",
                "rootfs.img",
                PARTITION_GROWS,
                0x10000,
                262_144,
                0x33,
            ),
        ],
    };

    match build_update_img(&spec) {
        Ok(img) => {
            std::fs::write(&out, &img).expect("write image");
            println!("wrote {out} ({} bytes)", img.len());
        }
        Err(e) => {
            eprintln!("pack failed: {e}");
            std::process::exit(1);
        }
    }
}
