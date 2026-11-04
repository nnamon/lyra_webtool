# rockusb protocol notes

Everything here was recovered from Rockchip's `upgrade_tool` v2.17 for Linux
(`upgrade_tool_v2.17-bfd48dcdba9fd8013872ca2abff19a8d.zip` from the Luckfox
wiki). The binary is a statically linked, **unstripped** x86-64 ELF, so symbol
names survive and the C++ class structure is readable.

```
sha256  8017ce9480eafecaef1771e7d46d505868cf207ec076cd86db21ccae2a86d298
```

Addresses below refer to that binary. Where a value is widely published but
differs from what this binary actually contains, that is called out — those are
the ones worth distrusting.

## Transport

`nm` finds 86 `libusb_*` symbols including `libusb_bulk_transfer` and
`libusb_control_transfer`. The only "serial" symbols are
`CRKDevice::GetSerialName` / `SetSerialName`, which handle the USB **serial
number string**, not a serial port.

The protocol therefore needs raw bulk endpoints and vendor control transfers.
The Web Serial API provides neither, which is why this project uses WebUSB.

USB vendor ID is `0x2207` for both MaskROM and Loader modes.

## Command Block Wrapper

From `CRKUsbComm::InitializeCBW(CBW*, USB_OPERATION_CODE)` at `0x4477d0`. The
function zeroes 31 bytes, stores the `USBC` signature as the little-endian word
`0x43425355`, builds a tag from four `rand()` calls, writes the opcode at
offset 15, then selects a direction and CDB length.

```
 0..4   "USBC"
 4..8   tag                        big endian
 8..12  transfer length            little endian
12      flags   0x80 = IN, 0x00 = OUT
13      LUN (always 0)
14      CDB length (6 or 10)
15      CDB[0]  opcode
16      CDB[1]  sub-opcode
17..21  CDB[2..6]  address         big endian
21      CDB[6]  reserved
22..24  CDB[7..9]  length          big endian
24..31  zero padding
```

The status wrapper is 13 bytes: `"USBS"`, tag (big endian), residue (little
endian), status byte (`0` success, `1` failure).

## Opcode table

Decoded from the jump table and bitmask tests in `InitializeCBW`. The four
targets at `0x4478ca`, `0x447938`, `0x4478af` and `0x4478fa` set
(direction, CDB length) to (IN,6), (IN,10), (OUT,6) and (OUT,10) respectively.

| Opcode | Name | Dir | CDB |
|---|---|---|---|
| `0x00` | TestUnitReady | IN | 6 |
| `0x01` | ReadFlashId | IN | 6 |
| `0x03` | TestBadBlock | IN | 10 |
| `0x04` | ReadSector | IN | 10 |
| `0x05` | WriteSector | OUT | 10 |
| `0x06` | EraseNormal | OUT | 10 |
| `0x0b` | EraseForce | OUT | 10 |
| `0x14` | ReadLBA | IN | 10 |
| `0x15` | WriteLBA | OUT | 10 |
| `0x16` | EraseSystemDisk | OUT | 6 |
| `0x17` | ReadSDram | IN | 10 |
| `0x18` | WriteSDram | OUT | 10 |
| `0x19` | ExecuteSDram | OUT | 10 |
| `0x1a` | ReadFlashInfo | IN | 6 |
| `0x1b` | ReadChipInfo | IN | 6 |
| `0x1e` | SetResetFlag | OUT | 6 |
| `0x1f` | WriteEFuse | OUT | 10 |
| `0x20` | ReadEFuse | IN | 6 |
| `0x21` | ReadSPIFlash | IN | 10 |
| `0x22` | WriteSPIFlash | OUT | 10 |
| `0x23` | WriteNewEfuse | OUT | 10 |
| `0x24` | ReadNewEfuse | IN | 10 |
| `0x25` | EraseLBA | OUT | 10 |
| `0x28` | *(unnamed)* | IN | 10 |
| `0x2a` | SwitchStorage | OUT | 6 |
| `0x2b` | GetStorageMedia | IN | 6 |
| `0xaa` | ReadCapability | IN | 6 |
| `0xff` | DeviceReset | OUT | 6 |

Opcodes `0x02`, `0x1c` and `0x1d` fall through the dispatch without either
field being set, so they are unused.

`DeviceReset` sub-opcodes: `0` reset, `1` mass storage, `2` power off,
`3` MaskROM, `4` disconnect.

## MaskROM bootstrap

`CRKUsbComm::RKU_DeviceRequest(uint, uchar*, uint)` at `0x446ca0`.

It range-checks the request with `leal -0x471(%rsi); cmpl $1; ja`, so only
`0x471` (DDR init) and `0x472` (USB loader) are accepted. Both
`libusb_control_transfer` call sites use:

| field | value |
|---|---|
| `bmRequestType` | `0x40` — host to device, vendor, device |
| `bRequest` | `0x0c` |
| `wValue` | `0` |
| `wIndex` | `0x471` or `0x472` |
| timeout | `0x1388` = 5000 ms |

A CRC-16 is appended **big endian** (`movzbl %ah` then `%al`), and the buffer is
sent in chunks of at most `0x1000` bytes.

Two length quirks, both about USB short-packet termination:

* `len & 0xfff == 0xfff` → a single zero byte is inserted *before* the CRC, so
  the CRC is computed over `len + 1` bytes.
* `len & 0xfff == 0xffe` → the payload plus CRC lands exactly on a packet
  boundary, so one extra zero byte is sent as a separate transfer afterwards.

## Checksums

### CRC-16/CCITT — `CRC_CCITT` at `0x427180`

Init `0xFFFF`, polynomial `0x1021`, MSB first, no reflection, no final XOR —
i.e. CRC-16/CCITT-FALSE. The loop conditionally XORs the polynomial twice per
bit, once on the previous CRC's bit 15 and once on the input bit; two XORs of
the same value cancel, so it reduces to the usual `msb ^ input_bit` test. An
empty buffer returns the init value.

### Rockchip CRC32 — `CRC_32` at `0x426d60`, table `gTable_Crc32` at `0x69e180`

Polynomial **`0x04C10DB7`**, MSB first, init `0`, no final inversion.

Note the third nibble. This is *not* the standard `0x04C11DB7`. Regenerating a
256-entry MSB-first table with `0x04C10DB7` reproduces the binary's table byte
for byte, and `rk356x_spl_loader_ddr1056_v1.10.111.bin` validates against it
(stored CRC `0x544175b9`).

The binary also carries an ordinary zlib table (`crc32table_le` at `0x69e620`)
for UBI and sparse images.

## RC4 — `P_RC4` at `0x426ee0`

Textbook RC4: identity state, `K[i] = key[i % 16]`, standard KSA swap and PRGA.
The key is loaded with one `movdqa` from `0x61d4c0`:

```
7c 4e 03 04 55 05 09 07 2d 2c 7b 38 17 0d 17 11
```

**The last byte is `0x11`.** This key is frequently published ending in `0x4d`;
that value does not match this binary, and a wrong final key byte changes the
entire keystream after the key schedule.

`CRKAndroidDevice::IDBRC4` at `0x436e40` applies it to three 512-byte regions of
the IDBlock — offsets `0x000`, `0x400` and `0x600`, i.e. sectors 0, 2 and 3.
Sector 1 is deliberately left in the clear.

## `RKBOOT` loader container

From `CRKBoot::ObjectInit` (`0x42dc20`), `GetEntryProperty` (`0x42d960`) and
`GetEntryData` (`0x42da00`). Header is 102 bytes; entries are 57 bytes.

```
0x00  tag "BOOT" (0x544f4f42) or "LDR " (0x2052444c)
0x04  u16  header size
0x06  u32  version
0x0a  u32  merge version      (rejected if > 0x01000000)
0x0e  u16  year
0x10  u8   month      0x11 day     0x12 hour
0x13  u8   minute     0x14 second
0x15  u32  chip code  (ASCII digits in reverse byte order: "3568" → 0x33353638)
0x19  u8   471 entry count
0x1a  u32  471 entry table offset
0x1e  u8   471 entry size
0x1f  u8   472 entry count
0x20  u32  472 entry table offset
0x24  u8   472 entry size
0x25  u8   loader entry count
0x26  u32  loader entry table offset
0x2a  u8   loader entry size
0x2b  u8   sign flag   ('S' = signed)
0x2c  u8   rc4 flag    (non-zero DISABLES RC4)
last 4 bytes: Rockchip CRC32 over everything before them
```

Each entry:

```
0x00  u8   entry size (57)
0x01  u32  type: 1 = 471, 2 = 472, 4 = loader
0x05  20 × UTF-16LE  name
0x2d  u32  data offset
0x31  u32  data size
0x35  u32  delay (ms)
```

Verified against a real `rk356x_spl_loader_ddr1056_v1.10.111.bin`: header size
`102`, entry stride `57`, and the three entry tables chain consistently at
`102 → 216 → 273`.

## `RKFW` firmware container (`update.img`)

From `CRKImage::ObjectInit` at `0x444f10`; the tag check is at `0x4451f0` and
the header is read with a fixed `fread` length of `0x66`.

```
0x00  tag "RKFW" (0x57464b52)
0x04  u16  header size
0x06  u32  version
0x0a  u32  merge version
0x0e  7 bytes  release time (same layout as RKBOOT)
0x15  u32  chip code
0x19  u32  loader offset
0x1d  u32  loader size
0x21  u32  firmware offset
0x25  u32  firmware size
```

The last 32 bytes of the file are a lowercase-hex MD5 covering
`firmware offset + firmware size` bytes from the start.

### A caveat on the inner `RKAF` table

`upgrade_tool` v2.17 contains **no `RKAF` tag comparison at all** — searching
the disassembly for `0x46414b52` finds nothing, and the item table is reached
through `CRKImage::GetData` without a magic check. So unlike everything above,
the inner item layout in this project follows the public `rkdeveloptool`
`RKANDROID_HDR` definition rather than a disassembly of this binary, and it has
not been checked against a real `update.img`.

Because a wrong offset there would mean writing a payload to the wrong flash
address, `RkafFirmware::parse` validates hard: it requires the tag, rejects an
implausible item count, and rejects any item whose payload falls outside the
container. A layout mismatch produces an error rather than a bad write.

## Command-line surface (for reference)

The tool's own usage text, recovered from `.rodata` at `0x2166ea`:

```
UpgradeFirmware:  UF <Firmware> [-noreset]
UpgradeLoader:    UL <Loader> [-noreset] [FLASH|EMMC|SPINOR|SPINAND]
DownloadImage:    DI <-p|-b|-k|-s|-r|-m|-u|-t|-re image>
EraseFlash:       EF <Loader|firmware>
CreateGPT:        GPT <Input Parameter> <Output Gpt>
WriteSector:      WS  <BeginSec> <PageSizeK> <PageSpareB> <File>
ReadLBA:          RL  <BeginSec> <SectorLen> [File]
WriteLBA:         WL  <BeginSec> [SizeSec] <File>
EraseLBA:         EL  <BeginSec> <EraseCount>
EraseBlock:       EB <CS> <BeginBlock> <BlokcLen> [--Force]
RunSystem:        RUN <uboot_addr> <trust_addr> <boot_addr> <uboot> <trust> <boot>
```

The flash-manufacturer table has exactly seven entries, at `0x21e58e`:
`SAMSUNG`, `TOSHIBA`, `HYNIX`, `INFINEON`, `MICRON`, `RENESAS`, `INTEL`.
