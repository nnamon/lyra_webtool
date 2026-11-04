# Lyra Web Tool

Flash a Luckfox Lyra Zero W (Rockchip RK3506B) from a browser tab. The flasher
is Rust compiled to WebAssembly. It speaks Rockchip's rockusb protocol over
WebUSB, and carries a separate UART console over Web Serial.

Live at [nnamon.github.io/lyra_webtool](https://nnamon.github.io/lyra_webtool/).
Changes are in [CHANGELOG.md](CHANGELOG.md), which Settings shows as Version
history.

Verified on a Luckfox Lyra Zero W: it flashed the Buildroot SPI NAND image and
the board booted. Read [Verification status](#verification-status) for what
that does and does not cover.

## Transport

Web Serial can't carry rockusb. The protocol needs two things Web Serial does
not expose:

- Vendor control transfers for the MaskROM bootstrap (`bmRequestType` `0x40`,
  `bRequest` `0x0c`, `wIndex` `0x471` or `0x472`).
- Raw bulk endpoints for the 31-byte command wrapper, its data phase, and the
  13-byte status wrapper.

WebUSB exposes both, so the flasher uses WebUSB. Web Serial drives the board's
debug UART in the Console tab, which is what it's for.

## Operations

Every rockusb opcode that `upgrade_tool` implements is reachable from the UI.

| Tab | Operations |
| --- | --- |
| Device | Chip info, flash ID, flash info, capability bits, test ready, five reset modes |
| Flash | MaskROM `0x471`/`0x472` bootstrap, `update.img` parsing, MD5 verify, per-item write |
| Storage | List media, switch media, whole-device erase, sector erase, block erase, bad-block scan |
| Memory | Sector read and write, DRAM read, write and execute |
| eFuse | Read and write, legacy (`0x1f`/`0x20`) and new (`0x23`/`0x24`) |
| Console | Web Serial terminal, 9600 to 3000000 baud |
| Settings | Running version and build, version history, reload and cache-clearing reload |

Large images stream. The tool identifies a 400 MB `update.img` from its
102-byte header, then flashes it in 256 KiB chunks read straight from the
`File`. It never holds the whole image in wasm memory.

## Browser support

Use Chrome or Edge on the desktop. Firefox and Safari don't implement WebUSB,
and the page says so instead of half working.

On Windows, bind the device to WinUSB with [Zadig](https://zadig.akeo.ie/)
first. Rockchip's own driver claims the interface otherwise.

On Linux, add a udev rule if you can't claim the device:

```
SUBSYSTEM=="usb", ATTR{idVendor}=="2207", MODE="0666", TAG+="uaccess"
```

macOS needs no setup.

## Flashing a board

A normal power-up does not give you a flashable device. The board boots Linux
and brings up an RNDIS network gadget, not rockusb. You have to ask for one of
the two download modes.

**Loader mode, from the running board.** U-Boot reads a magic value out of the
PMU scratch register on each boot. The device tree names it:
`syscon@ff910000` is `rockchip,rk3506-grf-pmu`, the `reboot-mode` child sits
at `offset = <0x200>`, and `mode-loader = <0x5242c301>`. So:

```sh
ssh root@192.168.123.100          # password: luckfox
devmem 0xff910200 32 0x5242c301
reboot
```

`reboot loader` does not work on the Buildroot image. Its `reboot` is BusyBox,
which takes no mode argument, so the reboot notifier never sees one and the
board comes back up normally. Writing the register is the way.

**MaskROM mode.** Hold `BOOT` while plugging in USB. From there the Flash tab
can push a packed loader (the vendor `MiniLoaderAll.bin`, or
`rk3506_spl_loader_*.bin`) with **Download boot**, which writes nothing to
flash. The board re-enumerates as a Loader, so reconnect afterwards. This path
has never been run against hardware.

Then:

1. Press **Connect** and pick the device with vendor `0x2207`.
2. Confirm the transport before writing: **Read device info**, then a
   one-sector read on the Memory tab.
3. Choose an `update.img`. Check the item table lists the targets you expect,
   at the LBAs in `parameter.txt`.
4. Press **Flash selected items**.

## After flashing

Power-cycle the board so it leaves Loader mode and boots what you wrote.

The Buildroot image brings up a USB RNDIS gadget on `usb0` at
`192.168.123.100/24`, with no DHCP server, so give your host an address on the
same subnet:

```sh
# Windows, admin shell
netsh interface ipv4 set address name="Ethernet 2" static 192.168.123.1 255.255.255.0
ssh root@192.168.123.100          # password: luckfox
```

If the address differs on your build, find the board by IPv6 link-local
instead, which needs no configuration:

```sh
ping -6 ff02::1%<interface-index>
netsh interface ipv6 show neighbors "Ethernet 2"
```

The entry that is `Reachable` with a resolved, locally administered MAC is the
board's gadget interface.

The debug UART is UART0 at 1500000 baud, which the Console tab opens over Web
Serial. It needs a 3.3 V USB-to-TTL adapter that can run at 1.5 Mbaud; many
CP210x and PL2303 cables cannot.

## Building your own images

`template/` packs an `update.img` from loose files, and `rkimage` unpacks and
packs containers:

```sh
cargo run -p rockusb --bin rkimage -- info      update.img
cargo run -p rockusb --bin rkimage -- unpack    update.img work/
cargo run -p rockusb --bin rkimage -- pack      work/ new-update.img
cargo run -p rockusb --bin rkimage -- roundtrip update.img
```

`roundtrip` repacks what it unpacked and compares the bytes, so you can check
the packer reproduces a vendor image exactly before relying on it. See
[template/README.md](template/README.md).

## Changing what runs on the board

Packing an `update.img` only rearranges partition images. To change the Linux
environment itself, read [docs/linux.md](docs/linux.md), which covers three
routes: editing the running board, rebuilding `rootfs.img` offline, and
rebuilding from the Luckfox SDK.

`rootfs.img` is a UBI image holding a UBIFS filesystem, so it has no loop
mount. `template/rootfs/ubifs.sh` unpacks and repacks it in Docker, with the
geometry read out of the SDK:

```sh
cd template/rootfs
./ubifs.sh info      /path/to/rootfs.img   # geometry the vendor used
./ubifs.sh roundtrip /path/to/rootfs.img   # check nothing is lost
./ubifs.sh unpack    /path/to/rootfs.img
./ubifs.sh shell                           # edit the tree as root
./ubifs.sh pack      new-rootfs.img
```

To cross-compile a program for the board without the SDK, use
`template/rootfs/cross/`. It targets armv7-a hard float and glibc 2.38, and
`cross.sh check` refuses a binary the board cannot load.

```sh
cd template/rootfs/cross
./cross.sh cc -O2 hello.c -o hello
./cross.sh check hello
./cross.sh run hello               # against the image's own libc, emulated
```

To add a package, change the kernel, or change the device tree, build from the
Luckfox SDK. `template/sdk/` runs that in Docker too:

```sh
cd template/sdk
./sdk.sh init ~/Downloads/luckfox-lyra-241213.tar.gz
./sdk.sh build rootfs
./sdk.sh fetch
```

That follows Luckfox's own
[SDK compilation](https://wiki.luckfox.com/Luckfox-Lyra/SDK-Image-Compilation)
and [Buildroot configuration](https://wiki.luckfox.com/Luckfox-Lyra/Buildroot-Configuration)
pages for the Lyra, in a container. `fetch` replaces `uboot.img`, `boot.img`
and `rootfs.img` and keeps the vendor loader.

Both setups keep their work in a Docker volume rather than a directory. For
the SDK that is forced: it checks the filesystem type of its own source tree
and refuses anything that is not ext4.

## Turning on a peripheral

Every driver is built into the vendor kernel, so enabling SPI, I2C, PWM or a
UART is a device tree change inside `boot.img`, not a kernel rebuild.
`template/boot/` does that in Docker, but **the image it produces does not
boot** on a Lyra Zero W and the cause is not yet known:

```sh
cd template/boot
./boot.sh roundtrip /path/to/boot.img    # prove the repack is faithful
./boot.sh unpack    /path/to/boot.img
./boot.sh enable-epaper-spi              # route SPI1 to a seated Pi HAT
./boot.sh pack      ../images/boot.img
```

`template/rootfs/epaper/` is a worked example: a Waveshare 2.7inch e-Paper
HAT demo that starts at boot. The HAT seats on the Lyra header directly, with
SPI1 routed to its pins through the RK3506 RM_IO matrix.

## Building

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli     # match the version pinned in app/Cargo.toml
./build.sh                         # writes www/pkg/
cd www && python3 -m http.server 8777
```

A push to `main` builds and deploys to GitHub Pages through
`.github/workflows/deploy.yml`. For a first deploy, set Pages to **Source:
GitHub Actions** in the repository settings.

## Repository layout

```
crates/rockusb/     protocol crate, no USB dependency
  protocol.rs         CBW/CSW wire format and the opcode table
  device.rs           one method per rockusb command
  transport.rs        the Transport trait, plus an in-memory fake for tests
  crc.rs  rc4.rs      checksums and the IDBlock cipher
  loader.rs           RKBOOT loader containers
  image.rs            RKFW and RKAF update.img containers, parsing
  pack.rs             the same containers, building
  bin/rkimage.rs      unpack, pack and round-trip CLI
app/                wasm crate
  webusb.rs           Transport over WebUSB
  serial.rs           UART console over Web Serial
  app.rs  dom.rs      state, event wiring, DOM helpers
www/                static site, the Pages artifact
template/           pack an update.img from loose files
  boot/               edit the device tree inside boot.img
  rootfs/             unpack and repack the UBI root filesystem
    cross/              cross-compile for the board, and check the ABI
    epaper/             2.7inch e-Paper HAT demo, starts at boot
  sdk/                build the Luckfox SDK in Docker
docs/protocol.md    reverse-engineering notes
docs/linux.md       changing the Linux environment on the board
```

`rockusb` has no browser dependency, so it tests natively:

```sh
cargo test -p rockusb
cargo run -p rockusb --example inspect -- rk3506_spl_loader.bin
```

`inspect` prints a loader or firmware container's header, entries and CRC
status. Use it to check a vendor file before you flash it.

## Provenance

The wire format comes from disassembling Rockchip's `upgrade_tool` v2.17, an
unstripped static x86-64 ELF. [docs/protocol.md](docs/protocol.md) records each
constant with the function and address it came from, so you can re-check any of
them.

Two values in wide circulation are wrong for this binary, and both fail
silently rather than erroring:

- Rockchip's CRC32 polynomial is `0x04C10DB7`, not the standard `0x04C11DB7`.
  Regenerating the table with it reproduces the binary's table byte for byte,
  and it validates a real vendor loader's stored CRC.
- The RC4 key's last byte is `0x11`, not the published `0x4d`.

## Verification status

Checked on hardware:

- A Luckfox Lyra Zero W was flashed with `Luckfox_Lyra_Zero_W_Flash_261020`
  over WebUSB, in Loader mode, writing `uboot`, `boot` and `rootfs`. The board
  booted, brought up its USB gadget and gave a shell.
- That covers connect, mode probe, `update.img` parsing, streaming MD5 verify
  and `WriteLBA`. It does not cover the MaskROM bootstrap, erase, DRAM or
  eFuse, none of which that run touched.

Checked against real artifacts:

- The CBW layout, opcode table, bootstrap framing, CRC-16, Rockchip CRC32, RC4
  key and `RKBOOT` layout all come from the vendor binary, with addresses
  recorded.
- The `RKBOOT` parser reads a genuine
  `rk356x_spl_loader_ddr1056_v1.10.111.bin` and Luckfox's RK3506
  `MiniLoaderAll.bin`, including both stored CRC32s.
- The `RKAF` layout is decoded from a real Luckfox Lyra Zero W `update.img`.
  Its header is committed as a test fixture, and the parsed targets match that
  image's `parameter.txt`: `uboot` at `0x2000`, `boot` at `0x4000`, `rootfs`
  at `0x10000`.
- 48 tests, covering the published CRC-16/CCITT (`0x29B1`), CRC-32
  (`0xCBF43926`) and RC4 check vectors.
- The UI runs in headless Chromium on every change.

Not checked:

- The MaskROM 471/472 bootstrap. The one board tested was in Loader mode, so
  that path has never completed against hardware.
- Erase, DRAM write and execute, and eFuse. These sit behind the Settings
  switch and no hardware run has exercised them.
- The `parameter` and `bootloader` items are skipped. `parameter` targets
  LBA 0, but on a `TYPE: GPT` device the host must build a GPT rather than copy
  `parameter.txt` there, and `bootloader` needs IDBlock generation. Neither
  transform is implemented, so a flash writes `uboot`, `boot` and `rootfs` and
  leaves the board's existing partition table and bootloader in place. That
  works on a board that currently boots. It will not revive one whose flash you
  have fully erased.
- The `RKAF` layout is confirmed against one firmware build. Another build
  could lay it out differently, so the parser still bounds-checks every item
  and refuses the image rather than writing to a wrong address.

## Licence

MIT.
