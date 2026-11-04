# Changing the root filesystem

Take `rootfs.img` apart, edit the files, build a new image, then flash it.
Everything here runs in Docker on your machine, not in the web page.

## What the image is

`rootfs.img` is a UBI image. UBI is a volume layer for raw NAND, so there is
no loop mount and no partition table. Inside it is one volume holding a UBIFS
filesystem.

| Property | Value |
| --- | --- |
| Volume | id 0, name `rootfs`, dynamic, `autoresize` |
| Logical eraseblock | 0x1f000 (126976 bytes) |
| Physical eraseblock | 0x20000 (131072 bytes) |
| Minimum I/O unit | 0x800 (2048 bytes) |
| Sub-page | 2048 bytes |
| Maximum LEB count | 8456, which caps the volume at 1 GiB |
| Compression | lzo |
| Free space fixup | on (`mkfs.ubifs -F`) |

Those match the Winbond W25N02KV on the board: 2048-byte pages, 128 KiB
blocks. `autoresize` is why the volume fills the 224 MiB left after `uboot`
and `boot`, and why `parameter.txt` marks the partition `rootfs:grow`.

Every value above comes from the Luckfox Lyra SDK, not from guesswork:

| Value | Source in the SDK |
| --- | --- |
| LEB size, sub-page | `buildroot/configs/rockchip/fs/ubifs.config` |
| Minimum I/O, PEB size | `buildroot/fs/ubifs/Config.in`, `buildroot/fs/ubi/Config.in` |
| Maximum LEB count | `BR2_TARGET_ROOTFS_UBIFS_MAX_SIZE=1024`, so 1024 MiB / 126976 |
| Compression | `BR2_TARGET_ROOTFS_UBIFS_RT_LZO`, the Kconfig choice default |
| Volume layout | `buildroot/fs/ubi/ubinize.cfg` |
| Image type | `RK_ROOTFS_UBI=y` in `luckfox_lyra_zero-w_buildroot_spinand_defconfig` |

Check them against your own file with `./ubifs.sh info rootfs.img`, which
prints the geometry the vendor actually used.

## Why the tree lives in a Docker volume

macOS cannot store what a root filesystem needs. A bind-mounted directory
drops device nodes, setuid bits and file ownership, and it drops them without
an error. Editing the tree there produces an image that flashes and then fails
to boot. So `unpack` puts the tree in a Docker volume and `shell` gives you a
root shell inside it.

## Change a file

1. Unpack the vendor image:
   `./ubifs.sh unpack /path/to/rootfs.img`
2. Edit the tree:
   `./ubifs.sh shell`
   You land in `/work/tree` as root, with `vi` and the usual tools.
3. Build a new image:
   `./ubifs.sh pack new-rootfs.img`
4. Flash it. Either write it to the `rootfs` partition with the web tool's
   Flash tab, or copy it into `../images/` and run `../build.sh` to pack a
   whole `update.img`.

## Add files from this directory instead

Anything under `files/` maps onto the root of the filesystem, which is easier
to keep in git than a Docker volume.

1. Put the file where it belongs, for example
   `files/etc/init.d/S99wlan0`.
2. `chmod +x` it if it needs to run. macOS keeps the execute bit, though not
   setuid bits or ownership.
3. `./ubifs.sh unpack /path/to/rootfs.img`
4. `./ubifs.sh apply`
5. `./ubifs.sh pack new-rootfs.img`

`apply` copies only what is in `files/` and makes those files root-owned. It
leaves the rest of the tree alone, because a recursive `chown` would clear
every setuid bit in the image and `su` would stop working.

## Check it before you flash

`roundtrip` unpacks an image, repacks it, extracts the result, and compares
the two trees:

```sh
./ubifs.sh roundtrip /path/to/rootfs.img
```

It compares the type, mode, owner, size, device numbers and link count of
every entry, the SHA-256 of every file, and the target of every symlink.
`round trip: N entries identical` means nothing is lost in the unpack and
repack, so a new image differs from the vendor's only where you changed it.

The image bytes differ every time even so, because `mkfs.ubifs` writes a fresh
UUID and timestamps. That is why the check compares trees rather than bytes,
unlike `rkimage roundtrip` one level up.

Run this on your own `rootfs.img` first. It is the only check that tells you
the geometry above matches your firmware.

A repacked image is not byte-identical to the one it came from, even with
nothing changed. `ubinize` writes a random image sequence number into the UBI
header at offset 0x18, so the files differ from byte 25 and are the same size.
`roundtrip` compares the trees, which is the thing that matters.

## What this cannot do

Buildroot has no package manager, so there is nothing to install into. New
programs have to come from somewhere else:

- Cross-compile one and copy the binary in. `cross/` does this without the
  SDK, and checks the result will actually load on the board. See
  [cross/README.md](cross/README.md). Dynamically linked programs also need
  their libraries, which this image has no way to install for them.
- Rebuild the whole root filesystem from the SDK with the package enabled. See
  `../../docs/linux.md`.

Kernel changes, device tree changes and new kernel modules are not in this
image at all. They live in `boot.img`.
