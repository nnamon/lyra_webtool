# Lyra Web Tool

Flash a **Luckfox Lyra Zero W** (Rockchip RK3506B) from a browser tab — no
drivers, no vendor tooling, no command line.

The flasher is Rust compiled to WebAssembly. It speaks Rockchip's **rockusb**
protocol over **WebUSB**, and ships a separate UART console over **Web Serial**.

> **Status:** the protocol layer is extensively tested and validated against
> real Rockchip binaries, but **this has not yet been run against a physical
> Lyra Zero W.** See [Hardware status](#hardware-status) before flashing
> anything you cannot recover.

## Why WebUSB and not Web Serial

A reasonable first instinct is to build this on the Web Serial API. That cannot
work. rockusb is not a UART protocol:

- The MaskROM bootstrap uses **vendor control transfers** (`bmRequestType 0x40`,
  `bRequest 0x0c`, `wIndex` `0x471`/`0x472`).
- Every command is a 31-byte **Command Block Wrapper** on a **bulk OUT**
  endpoint, with a data phase and a 13-byte status wrapper on **bulk IN**.

Web Serial exposes a byte stream over a serial port — no control transfers, no
bulk endpoints. WebUSB exposes both, so the flasher uses WebUSB. Web Serial is
still the right API for the board's *debug console*, which is what the
**Serial console** tab uses it for.

## What it does

Every rockusb opcode the vendor tool implements is available:

| Area | Operations |
|---|---|
| Identify | chip info, flash ID, flash info, capability bits, test-ready |
| Bootstrap | MaskROM `0x471`/`0x472` download from a packed loader `.bin` |
| Firmware | parse `update.img`, list items, streaming MD5 verify, flash per item |
| Storage | list media, switch media, whole-device erase |
| LBA | read / write / erase sectors, save reads to a file |
| NAND | raw sector read/write, block erase (normal and forced), bad-block scan |
| DRAM | read, write, execute |
| eFuse | read and write, both legacy (`0x1f`/`0x20`) and new (`0x23`/`0x24`) |
| Reset | reset, enter MaskROM, mass storage, power off, disconnect |
| Console | Web Serial UART terminal, 9600–3000000 baud |

Large images are **streamed**: a 400 MB `update.img` is identified from its
102-byte header, and flashed in 256 KiB chunks read straight from the `File`,
so it is never loaded into wasm memory in one piece.

## Browser support

WebUSB is required, so **Chrome or Edge on desktop**. Firefox and Safari do not
implement WebUSB and the page will say so rather than half-working.

- **Windows** — Rockchip's own driver claims the interface. Rebind it to WinUSB
  with [Zadig](https://zadig.akeo.ie/) first.
- **Linux** — you may need a udev rule so your user can claim vendor `2207`:
  ```
  SUBSYSTEM=="usb", ATTR{idVendor}=="2207", MODE="0666", TAG+="uaccess"
  ```
- **macOS** — works without setup.

## Using it

1. Put the board in **MaskROM** mode (hold `BOOT` while plugging in USB) or
   **Loader** mode.
2. Open the page and press **Connect device**; pick the Rockchip device.
3. *From MaskROM:* on the **Flash** tab, load a packed loader
   (`rk3506_spl_loader_*.bin`) and press **Download boot**. This writes nothing
   to flash — it brings the chip up so it accepts the full command set. The
   board re-enumerates, so reconnect afterwards.
4. Load an `update.img`, check the item table, and press **Flash**.

## Building

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli     # version must match app/Cargo.toml
./build.sh                          # outputs www/pkg/
cd www && python3 -m http.server 8777
```

Pushing to `main` builds and deploys to GitHub Pages automatically
(`.github/workflows/deploy.yml`). Enable Pages with **Source: GitHub Actions**
in the repository settings for the first deploy.

## Layout

```
crates/rockusb/     protocol crate — no USB dependency, runs anywhere
  protocol.rs         CBW/CSW wire format, opcode table
  device.rs           one method per rockusb command
  transport.rs        the Transport trait + an in-memory fake for tests
  crc.rs  rc4.rs      checksums and the IDBlock cipher
  loader.rs           RKBOOT loader containers
  image.rs            RKFW/RKAF update.img containers
app/                wasm crate
  webusb.rs           Transport over WebUSB
  serial.rs           UART console over Web Serial
  app.rs  dom.rs      state, event wiring, DOM helpers
www/                static site (the Pages artifact)
docs/protocol.md    reverse-engineering notes
```

The protocol crate has no browser dependency and is tested natively:

```sh
cargo test -p rockusb                              # 40 tests
cargo run -p rockusb --example inspect -- file.bin # dump a real loader/image
```

## Provenance

The wire format was recovered by disassembling Rockchip's `upgrade_tool` v2.17
(an unstripped static x86-64 ELF). [`docs/protocol.md`](docs/protocol.md) records
every constant with the function and address it came from, so each one can be
re-checked.

Two values in circulation turned out to be wrong for this binary, and both would
fail silently:

- The Rockchip CRC32 polynomial is **`0x04C10DB7`**, not the standard
  `0x04C11DB7`. Regenerating the table with it reproduces the binary's table
  exactly, and it validates a real vendor loader's stored CRC.
- The RC4 key's last byte is **`0x11`**, not the widely published `0x4d`.

## Hardware status

Honest summary of what has and has not been verified.

**Verified against real artifacts:**

- The CBW layout, opcode/direction/CDB table, bootstrap framing, CRC-16,
  Rockchip CRC32, RC4 key and `RKBOOT` layout are all decoded from the vendor
  binary, with addresses recorded.
- The `RKBOOT` parser is validated end-to-end against a genuine
  `rk356x_spl_loader_ddr1056_v1.10.111.bin`, including its stored CRC32.
- 40 unit tests, including the published CRC-16/CCITT (`0x29B1`), CRC-32
  (`0xCBF43926`) and RC4 check vectors.
- The UI is driven in a headless browser on every change.

**Not yet verified:**

- **No physical Lyra Zero W has been flashed with this.** Every USB transfer
  path is untested against real silicon.
- The inner `RKAF` item layout follows public `rkdeveloptool` headers rather
  than this binary, which contains no `RKAF` tag check (see
  [docs/protocol.md](docs/protocol.md)). The parser validates hard so a
  mismatch errors out instead of writing to a wrong address.
- IDBlock generation (`upgrade_tool ul`) is **not implemented**. Use the
  bootstrap + `update.img` path.

Start with read-only operations — **Read device info**, **Read storage list**,
a small **Read sectors** — to confirm the transport works on your board before
writing anything.

## Licence

MIT.
