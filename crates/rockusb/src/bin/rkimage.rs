//! Unpack and pack Rockchip `update.img` files.
//!
//! Host-side only; this is not part of the web tool. Use it to take a vendor
//! image apart, replace a partition image, and build a new one to load into
//! the flasher.
//!
//! ```sh
//! rkimage info      update.img
//! rkimage unpack    update.img out/
//! rkimage pack      out/ new-update.img
//! rkimage roundtrip update.img      # proves pack(unpack(x)) == x
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rockusb::image::{Firmware, NO_FLASH_ADDRESS, PARTITION_GROWS};
use rockusb::loader::ReleaseTime;
use rockusb::pack::{build_update_img, PackItem, PackSpec};

/// Name of the file describing how to rebuild a container.
const CONFIG: &str = "image.conf";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        Some("info") if args.len() == 3 => info(Path::new(&args[2])),
        Some("unpack") if args.len() == 4 => unpack(Path::new(&args[2]), Path::new(&args[3])),
        Some("pack") if args.len() == 4 => pack(Path::new(&args[2]), Path::new(&args[3])),
        Some("roundtrip") if args.len() == 3 => roundtrip(Path::new(&args[2])),
        _ => {
            eprintln!(
                "usage:\n  \
                 rkimage info      <update.img>\n  \
                 rkimage unpack    <update.img> <dir>\n  \
                 rkimage pack      <dir> <update.img>\n  \
                 rkimage roundtrip <update.img>"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

type R = Result<(), String>;

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn write(path: &Path, data: &[u8]) -> R {
    std::fs::write(path, data).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn sector(v: u32) -> String {
    match v {
        NO_FLASH_ADDRESS => "-".to_string(),
        other => format!("{other:#x}"),
    }
}

fn parse_sector(s: &str) -> Result<u32, String> {
    if s == "-" {
        return Ok(NO_FLASH_ADDRESS);
    }
    let t = s.trim();
    let (digits, radix) = match t.strip_prefix("0x") {
        Some(rest) => (rest, 16),
        None => (t, 10),
    };
    u32::from_str_radix(digits, radix).map_err(|_| format!("{s:?} is not a number"))
}

fn info(path: &Path) -> R {
    let data = read(path)?;
    let fw = Firmware::parse(&data).map_err(|e| e.to_string())?;
    let h = &fw.header;
    println!("file        {}", path.display());
    println!("size        {} bytes", data.len());
    println!("chip        {}", h.chip_name());
    println!("released    {}", h.release_time);
    println!("loader      {:#x} +{}", h.boot_offset, h.boot_size);
    println!("firmware    {:#x} +{}", h.fw_offset, h.fw_size);
    match (&h.stored_md5, fw.verify_md5(&data)) {
        (Some(m), Ok(())) => println!("md5         {m} [verified]"),
        (Some(m), Err(e)) => println!("md5         {m} [FAILED: {e}]"),
        (None, _) => println!("md5         absent"),
    }
    if let Some(l) = &fw.loader {
        println!(
            "\nembedded loader: {} entries, chip {}",
            l.entries.len(),
            l.chip_name()
        );
    }
    if let Some(inner) = &fw.firmware {
        println!(
            "\n{} / {} / {}  firmware {}",
            inner.model,
            inner.id,
            inner.manufacturer,
            inner.version_string()
        );
        println!(
            "\n{:<14}{:<20}{:>12}{:>12}{:>14}",
            "NAME", "PATH", "PART(sec)", "LBA", "SIZE"
        );
        for i in &inner.items {
            let part = if i.partition_sectors == PARTITION_GROWS {
                "grow".to_string()
            } else {
                format!("{:#x}", i.partition_sectors)
            };
            println!(
                "{:<14}{:<20}{:>12}{:>12}{:>14}",
                i.name,
                i.file_name,
                part,
                sector(i.target_sector),
                i.data_size
            );
        }
    }
    Ok(())
}

fn unpack(img: &Path, dir: &Path) -> R {
    let data = read(img)?;
    let fw = Firmware::parse(&data).map_err(|e| e.to_string())?;
    let inner = fw.firmware.as_ref().ok_or("no readable item table")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let images = dir.join("images");
    std::fs::create_dir_all(&images).map_err(|e| e.to_string())?;

    // The loader sits outside the item table, so save it by name.
    let loader_name = inner
        .items
        .iter()
        .find(|i| i.name == "bootloader")
        .map(|i| i.file_name.clone())
        .unwrap_or_else(|| "MiniLoaderAll.bin".to_string());
    let boot = fw.header.boot_range();
    if fw.header.boot_size > 0 {
        write(&images.join(&loader_name), &data[boot])?;
    }

    for item in &inner.items {
        let Some(bytes) = inner.item_data(item) else {
            continue;
        };
        let name = Path::new(&item.file_name)
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&item.name));
        write(&images.join(&name), bytes)?;
    }

    let h = &fw.header;
    let mut conf = String::new();
    conf.push_str("# Rebuilt by: rkimage pack <this dir> <update.img>\n");
    conf.push_str("# Values come from the image this was unpacked from.\n\n");
    conf.push_str(&format!("model = {}\n", inner.model));
    conf.push_str(&format!("id = {}\n", inner.id));
    conf.push_str(&format!("manufacturer = {}\n", inner.manufacturer));
    conf.push_str(&format!("version = {:#010x}\n", inner.version));
    conf.push_str(&format!("fw_version = {:#010x}\n", h.version));
    conf.push_str(&format!("merge_version = {:#010x}\n", h.merge_version));
    conf.push_str(&format!(
        "release = {:04}-{:02}-{:02} {:02}:{:02}:{:02}\n",
        h.release_time.year,
        h.release_time.month,
        h.release_time.day,
        h.release_time.hour,
        h.release_time.minute,
        h.release_time.second
    ));
    conf.push_str(&format!("chip = {}\n", h.chip_name()));
    conf.push_str(&format!("loader = {loader_name}\n\n"));
    conf.push_str("# item <name> <path> <partition sectors> <flash sector>\n");
    conf.push_str("# '-' means none, or a partition that grows to fill the device.\n");
    for i in &inner.items {
        conf.push_str(&format!(
            "item {} {} {} {}\n",
            i.name,
            i.file_name,
            sector(i.partition_sectors),
            sector(i.target_sector)
        ));
    }
    write(&dir.join(CONFIG), conf.as_bytes())?;

    println!("unpacked {} items to {}", inner.items.len(), dir.display());
    println!(
        "edit images/, then: rkimage pack {} new-update.img",
        dir.display()
    );
    Ok(())
}

fn pack(dir: &Path, out: &Path) -> R {
    let text = String::from_utf8(read(&dir.join(CONFIG))?)
        .map_err(|_| format!("{CONFIG} is not UTF-8"))?;

    let mut kv: BTreeMap<String, String> = BTreeMap::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("item ") {
            let f: Vec<String> = rest.split_whitespace().map(String::from).collect();
            if f.len() != 4 {
                return Err(format!("item line needs 4 fields: {line:?}"));
            }
            rows.push(f);
        } else if let Some((k, v)) = line.split_once('=') {
            // Strip only the single separator space. The vendor's manufacturer
            // field really is " RK3506" with a leading space, and trimming it
            // changes the bytes.
            let v = v.strip_prefix(' ').unwrap_or(v).trim_end();
            kv.insert(k.trim().to_string(), v.to_string());
        }
    }
    let get = |k: &str| -> Result<String, String> {
        kv.get(k).cloned().ok_or(format!("{CONFIG} has no {k}"))
    };
    let num = |k: &str| -> Result<u32, String> { parse_sector(&get(k)?) };

    let rt = get("release")?;
    let (d, t) = rt
        .split_once(' ')
        .ok_or("release must be 'YYYY-MM-DD HH:MM:SS'")?;
    let dp: Vec<&str> = d.split('-').collect();
    let tp: Vec<&str> = t.split(':').collect();
    if dp.len() != 3 || tp.len() != 3 {
        return Err("release must be 'YYYY-MM-DD HH:MM:SS'".into());
    }
    let p = |s: &str| {
        s.parse::<u16>()
            .map_err(|_| format!("bad time field {s:?}"))
    };
    let release_time = ReleaseTime {
        year: p(dp[0])?,
        month: p(dp[1])? as u8,
        day: p(dp[2])? as u8,
        hour: p(tp[0])? as u8,
        minute: p(tp[1])? as u8,
        second: p(tp[2])? as u8,
    };

    // The chip code is the four characters stored in reverse.
    let chip_name = get("chip")?;
    let mut cb = chip_name.as_bytes().to_vec();
    if cb.len() != 4 {
        return Err(format!("chip must be 4 characters, got {chip_name:?}"));
    }
    cb.reverse();
    let chip = u32::from_le_bytes([cb[0], cb[1], cb[2], cb[3]]);

    let images = dir.join("images");
    let loader_name = get("loader")?;
    let loader = read(&images.join(&loader_name))?;

    let mut items = Vec::new();
    for f in rows {
        let path = images.join(
            Path::new(&f[1])
                .file_name()
                .ok_or(format!("bad path {:?}", f[1]))?,
        );
        items.push(PackItem {
            name: f[0].clone(),
            file_name: f[1].clone(),
            partition_sectors: parse_sector(&f[2])?,
            target_sector: parse_sector(&f[3])?,
            data: read(&path)?,
        });
    }

    let spec = PackSpec {
        model: get("model")?,
        id: get("id")?,
        manufacturer: get("manufacturer").unwrap_or_default(),
        version: num("version")?,
        fw_version: num("fw_version")?,
        merge_version: num("merge_version")?,
        release_time,
        chip,
        loader,
        items,
    };
    let img = build_update_img(&spec).map_err(|e| e.to_string())?;
    write(out, &img)?;
    println!("wrote {} ({} bytes)", out.display(), img.len());
    Ok(())
}

/// Unpack then repack, and report whether the result is byte-identical.
///
/// This is the acceptance test for the packer against a real vendor image.
fn roundtrip(img: &Path) -> R {
    let original = read(img)?;
    let dir = std::env::temp_dir().join(format!("rkimage-roundtrip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    unpack(img, &dir)?;
    let out = dir.join("repacked.img");
    pack(&dir, &out)?;
    let rebuilt = read(&out)?;

    if rebuilt == original {
        println!("\nround trip: identical ({} bytes)", original.len());
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    }
    println!("\nround trip: DIFFERENT");
    println!(
        "  original {} bytes, rebuilt {} bytes",
        original.len(),
        rebuilt.len()
    );
    let first = original
        .iter()
        .zip(rebuilt.iter())
        .position(|(a, b)| a != b);
    match first {
        Some(at) => println!("  first difference at offset {at:#x}"),
        None => println!("  one is a prefix of the other"),
    }
    println!("  kept {} for inspection", dir.display());
    Err("round trip did not reproduce the input".into())
}
