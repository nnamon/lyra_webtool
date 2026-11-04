# Changelog

User-visible changes to Lyra Web Tool, newest first. Each release is tagged `v<version>` with the same notes. The app shows this file as Version history.

## 0.16.0 · 2026-11-05

- Split the firmware tooling out to [lyra-firmware](https://github.com/nnamon/lyra-firmware). This repository flashes a board from a browser; building and editing the images it writes is a different job, and keeping both here buried the one instruction that matters.
- Moved: `template/sdk/`, `template/rootfs/` with its cross toolchain and e-Paper demo, `template/build.sh`, `image.conf`, and `docs/linux.md`.
- `rkimage` stays, because it is the same container parser the flasher uses, exercised from a shell. `lyra-firmware`'s `build.sh` finds it on `PATH`, in a sibling checkout, or via `LYRA_WEBTOOL`.

## 0.15.1 · 2026-11-05

- Corrected the provenance of `kernel-config`. The SDK README said it was not on a vendor page and rested on a forum report; Luckfox's own Lyra kernel configuration page gives it, as the Buildroot page gives `buildroot-config`. Both are now cited.
- Noted the Buildroot caching behaviour the vendor FAQ describes: dependency checks are cached after the first build and not rechecked, which is why a newly enabled package needs its build directory deleted.

## 0.15.0 · 2026-11-05

- Removed `template/boot/`. Repacking the `boot.img` FIT by hand produced an image that does not boot, and keeping a tool that fails is worse than not having one. Changing the device tree is the SDK's job: `template/sdk/sdk.sh build kernel` packs the FIT the way the vendor's was.
- The e-Paper demo no longer depends on any firmware change. It drives all seven HAT pins as plain GPIO, all of which the stock device tree leaves unclaimed, so the vendor `boot.img` is used unchanged.
- `docs/linux.md` now says to check whether a pin is already free before changing anything, and points at `/sys/kernel/debug/pinctrl/` for reading what a running board has claimed.

## 0.14.3 · 2026-11-05

- The route for a device tree change is now the SDK, which packs and signs the FIT the way the vendor's was, rather than repacking `boot.img` by hand. Luckfox document the build on their SDK image compilation page and the files on their device tree page.
- Named the shared include the docs were missing: the Zero W builds `rk3506b-luckfox-lyra-zero-w.dts`, which pulls in `rk3506-luckfox-lyra-ultra.dtsi`, shared with the Ultra, Ultra W and Pi. The `-sd` variant of that `.dts` is for SD boot, not SPI NAND.

## 0.14.2 · 2026-11-05

- A `boot.img` repacked by `template/boot/` does not boot. Flashed to a Luckfox Lyra Zero W, the board hung and needed the vendor image reflashed to recover. The tooling and its documentation now say so at the top rather than describing a route that fails.
- The claim that FIT signature verification was inactive was an inference from the absence of a `value` property, presented as a finding. The vendor U-Boot contains `## Verified-boot`, `FIT: %ssigned, %sconf required` and `>sha256,rsa2048`, so it has signature support compiled in, and the repack drops the `signature` node. Recorded as the leading suspect, not a conclusion.
- The e-Paper demo now defaults to `--bitbang`. All seven pins a seated HAT drives are plain GPIOs in the stock device tree, so the demo needs no `boot.img` change and cannot stop the board booting. A 5808-byte frame against a three-second refresh makes the speed cost invisible.
- `docs/linux.md` and the template READMEs no longer present the device tree route as working.

## 0.14.1 · 2026-11-05

- Corrected the flashing instructions. A normal power-up gives you Linux and an RNDIS gadget, not a rockusb device, so the old "for Loader mode, power up normally" was wrong and left you with nothing to connect to.
- Documented how to actually reach Loader mode: write `0x5242c301` to `0xff910200` and reboot. The device tree names the register, as `reboot-mode` at `offset = <0x200>` under `syscon@ff910000`, which is `rockchip,rk3506-grf-pmu`.
- Noted that `reboot loader` does not work on this image. Its `reboot` is BusyBox, which takes no mode argument, so the reboot notifier never sees one.

## 0.14.0 · 2026-11-05

- `template/boot/` takes `boot.img` apart, edits the device tree and puts it back. Enabling a peripheral on this board needs no kernel rebuild: `CONFIG_SPI_ROCKCHIP`, `CONFIG_SPI_SPIDEV`, `CONFIG_GPIO_SYSFS` and `CONFIG_GPIO_CDEV` are all built in already, and the only thing stopping SPI is `status = "disabled"` in the device tree.
- `boot.sh roundtrip` unpacks and repacks a container unchanged and checks it. The kernel and resource hashes must be identical; the device tree is compared as a tree, because `dtc` rewrites the header and a recompile is not byte-reproducible.
- The FIT declares a signature but carries no value, so repacking with regenerated sha256 hashes is viable. Recorded with the evidence in `template/boot/README.md`.
- `boot.sh enable-epaper-spi` routes SPI1 onto the pins a seated Raspberry Pi HAT drives, using the RK3506 RM_IO crossbar. The Lyra Zero W header is Pi pin-compatible, all twelve power and ground pins line up, but the SPI signals are not in the Pi positions. The RM mux numbers are the datasheet function indices plus 15, an undocumented offset measured against eight functions in the shipped device tree.
- `template/rootfs/epaper/` is a 2.7inch e-Paper HAT demo that starts at boot from `/etc/init.d/S99epaper`. It supports both panel revisions, which use different controllers and opposite BUSY polarity, and falls back from one to the other if the panel does not answer.
- The demo exits within seconds when no panel is attached, so a board with no HAT boots normally. `--bitbang` drives the same pins from userspace for an unpatched `boot.img`.

## 0.13.0 · 2026-11-05

- `template/rootfs/cross/` cross-compiles for the board in Docker, without the SDK. `cross.sh cc`, `c++` and `make` pass the target ABI flags; `check` confirms a binary will load on the board; `run` executes it under emulation against the image's own libraries.
- The target ABI is measured from the board's own `busybox`, not assumed: ARMv7-A, hard float, VFPv4, ELF flags `0x5000400`, glibc 2.38, interpreter `/lib/ld-linux-armhf.so.3`.
- glibc is backward compatible only. A binary needing `GLIBC_2.39` loads on the board and then dies, and nothing warns you at build time unless the toolchain's glibc is old enough to lack the symbol. The container is Debian bookworm, whose armhf target is glibc 2.36, so such a call fails at link time instead.
- `check` reads the header and says nothing about dependencies; `run` loads the binary for real and names a missing library. Both are needed and the README says why.
- `ubifs.sh` is now verified against a real vendor `rootfs.img`: roundtrip reports 7263 entries identical, and the repacked image is the same size as the original.
- A repacked image is not byte-identical to its source, because `ubinize` writes a random image sequence number at offset `0x18`. Documented, so a `cmp` difference at byte 25 is not mistaken for data loss.

## 0.12.1 · 2026-11-05

- `sdk.sh init` now checks the working tree out of `.repo` before it looks for `build.sh`. The Luckfox Lyra tarball ships a `repo` mirror and no source tree, which is the layout the vendor page documents, and 0.12.0 gave up with "no build.sh in the tarball" before it ever ran `repo sync`.
- `sdk.sh fetch` copies `uboot.img`, `boot.img` and `rootfs.img` only, and leaves the vendor `MiniLoaderAll.bin` and `parameter.txt` in `template/images/`. It used to copy all of `rockdev/`, overwriting the vendor loader that `template/README.md` tells you to keep. `sdk.sh fetch all` restores the old behaviour deliberately.
- `sdk.sh init` lists the rk3506 defconfigs an SDK release actually carries when the Zero W one is missing, instead of failing with the SDK's own error.
- The `Dockerfile` installs the apt list Luckfox publishes for this SDK, rather than one derived from the preflight scripts. `python-is-python3` is the one entry left out, because it conflicts with the `python2` symlink the same page prescribes.
- The SDK docs now cite Luckfox's Lyra pages, and note that the Pico Pro Max page does not transfer: its menuconfig target is `buildrootconfig`, this SDK wants `buildroot-config`, and its uClibc warning does not apply to the Lyra's glibc toolchain.

## 0.12.0 · 2026-11-04

- `template/sdk/` builds the Luckfox Lyra SDK in Docker, so adding a package or changing the kernel or device tree needs no Linux machine. `sdk.sh init` extracts the SDK and selects the Zero W SPI NAND config by name; `sdk.sh build <target>`, `fetch`, `get`, `put` and `shell` do the rest.
- The SDK source has to live in a Docker volume. Its own `check-sdk.sh` reads the filesystem type of its source tree and accepts only ext4, f2fs or btrfs. A directory bind-mounted from macOS reports `virtiofs` and is refused.
- The container is amd64 and cannot be anything else, because the SDK ships prebuilt x86_64 host tools and Buildroot needs a host compiler that can produce 32-bit binaries.
- `sdk.sh fetch` writes the built images into `template/images/`, where `template/build.sh` already expects them.
- The SDK's own preflight scripts, `check-sdk.sh` and `check-buildroot.sh`, both pass inside the container. A full build is not yet verified; it needs the SDK tarball and hours of emulated compilation.

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
