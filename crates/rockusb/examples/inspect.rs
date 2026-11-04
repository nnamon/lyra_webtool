//! Inspect a Rockchip loader or firmware file with this crate's parsers.
//!
//! Useful for checking the parsers against real vendor files:
//!
//! ```sh
//! cargo run -p rockusb --example inspect -- rk3506_spl_loader.bin
//! ```

use rockusb::loader::{EntryType, RkBoot};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: inspect <loader.bin|update.img>");
        std::process::exit(2);
    };
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    println!("file      : {path}");
    println!("size      : {} bytes", data.len());
    println!(
        "magic     : {:02x?} ({})",
        &data[..4.min(data.len())],
        String::from_utf8_lossy(&data[..4.min(data.len())])
    );
    println!();

    match RkBoot::parse(&data, true) {
        Ok(boot) => print_boot(&boot),
        Err(e) => {
            eprintln!("not a valid RKBOOT container: {e}");
            // Retry without the CRC check so the structure can still be seen.
            if let Ok(boot) = RkBoot::parse(&data, false) {
                eprintln!("(structure below parsed with the CRC check waived)\n");
                print_boot(&boot);
            }
            std::process::exit(1);
        }
    }
}

fn print_boot(boot: &RkBoot) {
    println!("== RKBOOT container ==");
    println!("tag          : {}", String::from_utf8_lossy(&boot.tag));
    println!("header len   : {}", boot.header_len);
    println!(
        "version      : {:#010x} (merge {:#010x})",
        boot.version, boot.merge_version
    );
    println!("released     : {}", boot.release_time);
    println!("chip         : {} ({:#010x})", boot.chip_name(), boot.chip);
    println!("signed       : {}", boot.signed);
    println!("rc4 disabled : {}", boot.rc4_disabled);
    println!("stored crc32 : {:#010x}  [verified]", boot.stored_crc);
    println!();
    for ty in [EntryType::Entry471, EntryType::Entry472, EntryType::Loader] {
        let entries: Vec<_> = boot.entries_of(ty).collect();
        if entries.is_empty() {
            continue;
        }
        println!("{ty:?} ({} entries):", entries.len());
        for e in entries {
            let d = boot.entry_data(e);
            let head: Vec<String> = d.iter().take(8).map(|b| format!("{b:02x}")).collect();
            println!(
                "  {:<24} off={:#08x} size={:>8}  delay={:>4}ms  data=[{}...]",
                e.name,
                e.data_offset,
                e.data_size,
                e.data_delay,
                head.join(" ")
            );
        }
        println!();
    }
}
