# Editing boot.img

`boot.img` is a U-Boot FIT container holding three things: the kernel, the
device tree, and a Rockchip resource blob. Changing what peripherals the board
exposes means editing the device tree, nothing else.

## Why no kernel rebuild is needed

Every driver is already built into the vendor kernel. From `info/config-6.1`
in the root filesystem:

```
CONFIG_SPI_ROCKCHIP=y      CONFIG_SPI_SPIDEV=y
CONFIG_PINCTRL_ROCKCHIP=y  CONFIG_GPIO_ROCKCHIP=y
CONFIG_GPIO_SYSFS=y        CONFIG_GPIO_CDEV=y
```

What stops SPI working is a single `status = "disabled"` in the device tree.
That is the Rockchip convention: the SoC `.dtsi` declares every peripheral
disabled and each board's `.dts` enables what it wires. This board's `.dts`
calls itself `rockchip,rk3506g-demo-display-control` and enables a DSI panel
and three touch controllers. Of 60 nodes, 25 are disabled, including both SPI
controllers, five of six UARTs, all ten PWMs, both CAN and both Ethernet.

## This does not work yet

**A `boot.img` repacked by this tool does not boot.** Tried on a Luckfox Lyra
Zero W: the board hung and had to be recovered by reflashing the vendor
image. Do not flash anything built here without a way back.

What is wrong is not yet known. The leading suspect is FIT signature
verification. The vendor container declares a signature and appears not to
carry one: `configurations/conf` has `algo`, `padding`, `key-name-hint` and
`sign-images`, but no `value` property, and the whole FIT tree is 1536 bytes,
too small to hold a 2048-bit RSA value. From that this tool concluded that
enforcement was off. That conclusion was wrong, or at least incomplete: the
vendor U-Boot contains `## Verified-boot: %d`,
`FIT: %ssigned, %sconf required` and `>sha256,rsa2048`, so it has signature
support compiled in, and the repack drops the `signature` node entirely.

Other candidates not yet ruled out: the `rollback-index` property, the
external data layout, and the device tree edit itself.

To find out which, flash a container repacked with no device tree change at
all. If that boots, the container is fine and the edit is at fault. If it
does not, the repack is at fault and the fix is to patch the device tree in
place at its original size, leaving every other byte of the container alone,
rather than rebuilding it with `mkimage`.

## For an SPI peripheral, you probably do not need this

Every pin a Raspberry Pi HAT drives is a plain GPIO in the stock device tree.
Driving SPI from userspace needs no `boot.img` change and so cannot stop the
board booting. See `../rootfs/epaper/` and its `--bitbang` mode.

## Commands

```sh
./boot.sh roundtrip /path/to/boot.img    # check pack before trusting it
./boot.sh unpack    /path/to/boot.img
./boot.sh enable-epaper-spi              # route SPI1 to the HAT pins
./boot.sh diff                           # see exactly what changed
./boot.sh pack      ../../../build/boot.img
```

Run `roundtrip` first. It unpacks and repacks unchanged, then checks the
kernel and resource hashes are identical and the device tree is unchanged as a
tree.

## Why roundtrip does not compare the device tree byte for byte

The kernel and resource blobs are copied through untouched, so their hashes
must match exactly, and they do. The device tree is decompiled to text and
recompiled, which is not byte-reproducible: `dtc` rewrites the header, so the
bytes differ from offset 28 while the tree is identical and the file is the
same 66971 bytes. Comparing the decompiled trees is the meaningful check.

## What enable-epaper-spi does

The Lyra Zero W header is pin-compatible with the Raspberry Pi 40-pin header.
All twelve power and ground pins line up, so a Pi HAT seats directly. The SPI
signals do not line up: the Pi puts MOSI on pin 19 and CE0 on pin 24, while
the Lyra's fixed SPI1 pinmux puts them on pins 26 and 12. Only the clock, on
pin 23, agrees.

RK3506 has a crossbar, the RM_IO matrix, that routes a peripheral signal to
any header pin. The patch uses it to move SPI1 onto the three pins a seated
HAT drives:

| HAT signal | Pi pin | Lyra pin | RM_IO | Function | mux |
| --- | --- | --- | --- | --- | --- |
| CLK | 23 | GPIO0_B0 | RM_IO8 | SPI1_CLK | `0x57` |
| DIN | 19 | GPIO0_A6 | RM_IO6 | SPI1_MOSI | `0x58` |
| CS | 24 | GPIO0_B2 | RM_IO10 | SPI1_CSN0 | `0x5a` |

MISO is left unrouted, because the panel is write-only and the HAT does not
drive pin 21.

### Where those mux numbers come from

They are the RM function indices from the RK3506B datasheet plus 15. The
offset is not documented. It was measured against this device tree, which
already contains matrix groups for other peripherals:

| Function | Datasheet index | mux in the DTB |
| --- | --- | --- |
| PWM0_CH0 | 30 | 45 |
| PWM1_CH0 | 34 | 49 |
| PDM_SDI0, SDI1, SDI2 | 22, 23, 24 | 37, 38, 39 |
| TSADC_CTRL | 77 | 92 |
| SPDIF_TX, SPDIF_RX | 80, 81 | 95, 96 |

Eight functions across four peripherals, one constant offset. SPI1_CLK,
SPI1_MOSI and SPI1_CSN0 are indices 72, 73 and 75, giving `0x57`, `0x58` and
`0x5a`.

## Recovery

A `boot.img` that does not boot is not fatal. The board still enters MaskROM
with `BOOT` held down, and the web tool can write the vendor `boot.img` back.
Keep the original.

## Verified

- `roundtrip` on the vendor `boot.img`: kernel and resource hashes identical,
  device tree semantically identical at 66971 bytes.
- After `enable-epaper-spi`, the packed image reads back `status = "okay"`,
  `spi-max-frequency = 4000000`, and the pin group
  `0 8 57 1d6 0 6 58 1d6 0 a 5a 1d6`.
- The kernel image inside the repacked container is byte-identical to the
  vendor's.

Tried on hardware and **failed**: a board flashed with a container repacked
here did not boot, and was recovered with the vendor image. Everything above
describes a container that parses correctly and still does not boot, which is
the gap between checking an artefact and running it.
