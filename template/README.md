# Building an update.img

Take a vendor image apart, replace a partition image, build a new one, then
flash it with the web tool. Everything here runs on your machine; none of it
is part of the web page.

Build from the **vendor image**, not the SDK. Luckfox's
`Luckfox_Lyra_Zero_W_Flash_261020` zip already contains the unpacked form of
its own `update.img`: the loader, the partition table, the manifest and the
three partition images. Keep the parts that are specific to the board and
replace only what you need. The SDK only matters once you want to *produce* a
new kernel or rootfs, which is a separate problem from packaging.

## What each file is

| File | Role | Replace it? |
| --- | --- | --- |
| `MiniLoaderAll.bin` | Loader, and the source of the on-flash IDBlock | No. The flasher cannot write an IDBlock, so keep the board's. |
| `parameter.txt` | Partition table, including the `mtdparts` line | Only if you change partition sizes |
| `package-file` | Manifest naming what goes in the container | Only if you add or drop an item |
| `uboot.img` | U-Boot, 4 MiB partition | With care |
| `boot.img` | Kernel and device tree, 12 MiB partition | Yes |
| `rootfs.img` | Root filesystem, grows to fill the device | Start here |

`image.conf` holds what the container header needs: model, id, version, chip,
and each item's partition size and flash sector.

## Build

1. Copy the vendor files into `images/`:
   `MiniLoaderAll.bin`, `uboot.img`, `boot.img`, `rootfs.img`.
2. Replace the one you want to change.
3. Run `./build.sh update.img`.
4. Load that file in the web tool's Quick flash tab.

`build.sh` prints the item table afterwards. Check the flash sectors read
`0x2000`, `0x4000` and `0x10000`, matching the `mtdparts` line in
`parameter.txt`, before you flash it.

## Start from your own image instead

The values in `image.conf` come from one Luckfox build. Two of them,
`fw_version` and `merge_version`, are the common Rockchip values rather than
ones read from that image. To be certain they match your firmware, unpack your
own:

```sh
cargo run -p rockusb --bin rkimage -- unpack /path/to/update.img work/
```

That writes `work/image.conf` and `work/images/` from the real file, so every
field is the vendor's. Edit and repack:

```sh
cargo run -p rockusb --bin rkimage -- pack work/ new-update.img
```

## Check the packer before trusting it

`roundtrip` unpacks an image and repacks it, then compares the bytes:

```sh
cargo run -p rockusb --bin rkimage -- roundtrip /path/to/update.img
```

`round trip: identical` means the packer reproduces that vendor image exactly,
so anything it builds differs from the vendor's only where you changed it.
Run this on your own `update.img` before you rely on a built image.

## Partition sizes

The `mtdparts` line in `parameter.txt` is the authority:

```
mtdparts=:0x00002000@0x00002000(uboot),0x00006000@0x00004000(boot),-@0x00010000(rootfs:grow)
```

Each entry is `size@offset(name)` in 512-byte sectors, so `uboot` is 4 MiB at
4 MiB, `boot` is 12 MiB at 8 MiB, and `rootfs` starts at 32 MiB and grows. An
image larger than its partition is refused by the flasher rather than written,
but check it here first. If you change a size, change it in both
`parameter.txt` and `image.conf`.
