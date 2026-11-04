# Changelog

User-visible changes to Lyra Web Tool, newest first. Each release is tagged `v<version>` with the same notes. The app shows this file as Version history.

## 0.11.0 · 2026-11-04

- `docs/linux.md` explains how to change the Linux environment on the board, by three routes: editing the running board, rebuilding `rootfs.img` offline, and rebuilding from the Luckfox SDK.
- `template/rootfs/ubifs.sh` unpacks and repacks `rootfs.img`. It is a UBI image holding a UBIFS filesystem, so no loop mount exists for it and the vendor geometry has to be reproduced exactly.
- The UBI geometry is read out of the Luckfox SDK rather than guessed, and `ubifs.sh info` prints what your own image actually uses so you can check it.
- `ubifs.sh roundtrip` compares the unpacked and repacked trees including modes, owners, link counts and device numbers. Extraction silently drops all of those unless it runs as root with the right flags, which produces an image that flashes and then fails to boot.
- Kernel modules live in `rootfs.img` under `/lib/modules`, not in `boot.img`, so a new module needs both images rebuilt.

## 0.10.0 · 2026-11-04

- `rkimage`, a command-line tool, unpacks and packs `update.img` files. It runs on your machine and is not part of the web page.
- `rkimage roundtrip` repacks what it unpacked and compares the bytes, so you can confirm the packer reproduces a vendor image before relying on it.
- `template/` packs an `update.img` from loose files, set up for the Lyra Zero W, so you can replace a partition image and flash the result.

## 0.9.1 · 2026-11-04

- Reads a reply a whole packet at a time and trims it, rather than asking for exactly the expected length. A device that answers with more would otherwise leave bytes behind and corrupt the next command.

## 0.9.0 · 2026-11-04

- Fixes the status command, which had never worked. It asked for a data phase the device does not send, so it consumed the status reply as data and always failed. Mode detection depended on it, so every board was reported as MaskROM.
- Test ready reports the progress counters a board sends during long operations.
- A failed mode probe logs the reason instead of only the verdict.

## 0.8.0 · 2026-11-04

- Mode is decided by asking the board, not by reading its USB descriptor. A Loader answers a status command and MaskROM does not, so the answer is definitive.
- Fixes a board in Loader mode being reported as MaskROM. That disabled every write control, and Download boot then sent a MaskROM-only request the Loader rejected with a transfer error.
- Connecting logs the probe result alongside what the descriptor suggested.

## 0.7.0 · 2026-11-04

- Quick flash is the first tab. Choose an `update.img`, read the checks, confirm, and it writes. It is the whole flashing path in one screen.
- The checks cover the container, the embedded loader and its CRC, the item table, the flash targets, whether each image fits its partition, the checksum, and the board's mode. Any failure disables the flash button.
- Quick flash lists exactly what it will write and where before you confirm.
- From MaskROM, Quick flash downloads the loader first, then asks you to reconnect and confirm again.
- Erase, raw sector writes, DRAM write and execute, and the whole eFuse tab are hidden unless Settings turns on "Show destructive operations". None of them is needed to flash firmware, and two of them can leave a board this tool cannot recover.

## 0.6.0 · 2026-11-04

- Device tab can treat the board as Loader or MaskROM when the detected mode is wrong. Mode comes from the USB descriptor, and a board in Loader that reads as MaskROM would otherwise leave the write controls disabled.
- Connecting logs the detected mode and the `bcdUSB` value it came from.

## 0.5.0 · 2026-11-04

- Settings holds Version history, the running version and build, Reload app, and Clear cached copy and reload.
- Settings checks whether a newer version is deployed and says so when there is one.

## 0.4.0 · 2026-11-04

- Flashing, and the LBA, erase and storage controls, stay disabled until the board reports Loader mode. In MaskROM they failed at the first command instead.
- Connecting in MaskROM says what to do next: load a loader or an `update.img`, press Download boot, then reconnect.
- After Download boot the device fields read Reconnect. The board re-enumerates with a new address, so the old connection is dead.
- The log records the board's USB layout on connect: every interface, its class, its endpoints, and which one was claimed.
- The loader summary shows the signed and RC4 flags.
- A rejected bootstrap entry names the entry, its length and the control request that failed.

## 0.3.0 · 2026-11-04

- `update.img` files load. The item table lists each image with the flash sector it targets.
- The partition table and bootloader rows are listed but never written, and say why. Your board keeps the partition table and bootloader it already has, so a flash needs a board that currently boots.
- An image whose items don't fit the container is refused rather than written to a wrong address.
- The firmware summary shows the model, id and firmware version.

## 0.2.0 · 2026-11-04

- New look, following Apple's design language, in a light and a dark appearance that follow the system.
- Parameter forms read as grouped rows, label on the left and value on the right.
- Progress bars stay hidden until a transfer starts.
- Empty fields read "Not connected" or "Not read" rather than a dash.

## 0.1.0 · 2026-11-04

- Flash a Luckfox Lyra Zero W from Chrome or Edge over WebUSB, with no driver install on macOS or Linux.
- Device tab reads chip info, flash ID, flash info and capability bits, and offers five reset modes.
- Flash tab bootstraps a board out of MaskROM, parses `update.img`, verifies its MD5 and writes each item.
- Storage tab lists and switches storage media, erases the device, a sector range or a block range, and scans for bad blocks.
- Memory tab reads and writes raw sectors, and reads, writes and executes DRAM.
- eFuse tab reads and writes OTP, behind a confirmation.
- Console tab opens the board's debug UART over Web Serial, from 9600 to 3000000 baud.
- A 400 MB image opens from its header and flashes in chunks, so it is never held in memory whole.
