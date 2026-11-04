# Changelog

User-visible changes to Lyra Web Tool, newest first. Each release is tagged `v<version>` with the same notes. The app shows this file as Version history.

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
