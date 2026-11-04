# 2.7inch e-Paper HAT demo

A demo that draws on a Waveshare 2.7inch e-Paper HAT and starts at boot, so
the board shows something with nothing plugged in but the panel.

## Use bit-bang, not hardware SPI

The demo defaults to `--bitbang`, which clocks the panel from userspace GPIO.
That is not a fallback, it is the recommended mode: every pin the HAT drives
is a plain GPIO in the stock device tree, so it needs no `boot.img` change
and cannot stop the board booting.

The hardware SPI route in `../../boot/` produced a `boot.img` that does not
boot on real hardware. Until that is understood, use bit-bang. The panel is
write-only, a frame is 5808 bytes and a full refresh takes three seconds, so
clocking it by hand costs nothing you can see.

## Wiring

None. The HAT seats on the Lyra header directly. All twelve Raspberry Pi
power and ground pins line up.

| HAT | Pi pin | Lyra pin | Linux GPIO | Driven by |
| --- | --- | --- | --- | --- |
| VCC | 1 | 3V3_OUT | - | - |
| GND | 6 | GND | - | - |
| RST | 11 | GPIO0_A3 | 3 | sysfs GPIO |
| PWR | 12 | GPIO0_B6 | 14 | sysfs GPIO, held high |
| BUSY | 18 | GPIO0_B4 | 12 | sysfs GPIO, input |
| DIN | 19 | GPIO0_A6 | 6 | SPI1_MOSI |
| DC | 22 | GPIO0_B3 | 11 | sysfs GPIO |
| CLK | 23 | GPIO0_B0 | 8 | SPI1_CLK |
| CS | 24 | GPIO0_B2 | 10 | SPI1_CSN0 |

## Build and install

```sh
../cross/cross.sh cc -O2 -Wall -Wextra -o epaper-demo epaper-demo.c
../cross/cross.sh check epaper-demo

mkdir -p ../files/usr/bin ../files/etc/init.d ../files/etc/default
cp epaper-demo    ../files/usr/bin/
cp S99epaper      ../files/etc/init.d/
cp epaper.default ../files/etc/default/epaper
cd .. && ./ubifs.sh apply && ./ubifs.sh pack rootfs.img
```

Then patch `boot.img` as `../../boot/README.md` describes, and flash both.

## Two incompatible panel revisions

The 2.7inch HAT exists in two revisions with different controllers, and the
differences are not cosmetic:

| | V1 | V2 |
| --- | --- | --- |
| Controller | UC8151 class | SSD1675 class |
| Programmed with | power, booster, PLL, five waveform LUTs | RAM windows and an update sequence |
| Refresh | `0x13` data, `0x12` refresh | `0x24` data, `0x22`/`0x20` update |
| BUSY polarity | **busy when low** | **busy when high** |

The demo defaults to V2 and falls back to V1 if the panel does not answer
within 15 seconds, so you do not have to know which one you have. `--v1`
skips the first attempt.

## Options

Set `ARGS` in `/etc/default/epaper`:

```
--v1        UC8151-class panel
--v2        SSD1675-class panel, the default
--bitbang   drive SPI from GPIO instead of /dev/spidev1.0
--once      draw one frame and exit
```

`--bitbang` is the escape hatch. It clocks the same three pins by hand from
userspace and needs no device tree change, so it works on an unpatched
`boot.img`. The panel is write-only and a full refresh takes seconds, so the
speed cost does not show.

## It will not hang your boot

If no panel answers, the demo gives up and exits within a few seconds. A
board with no HAT attached boots normally, and the init script is `S99` so it
runs after everything else.

## Refresh rate

The loop is slow deliberately. A full refresh takes seconds and wears the
panel, so the demo draws a handful of frames and then waits a minute.

## Verified

- Cross-compiles clean with `-Wall -Wextra`, and `cross.sh check` passes:
  ELF flags `0x5000400`, interpreter `/lib/ld-linux-armhf.so.3`, needs glibc
  2.34 against the board's 2.38.
- Run under emulation with no GPIO and no spidev present, it reports
  `gpio 3: cannot set direction to out` and exits in about a second rather
  than blocking.
- Installed into the real vendor root filesystem and packed: the binary is
  `-rwxr-xr-x 0:0` at `/usr/bin/epaper-demo`, the init script is executable at
  `/etc/init.d/S99epaper`, and the result is a valid 825-PEB UBI image.

Not verified on hardware: nothing here has met a real panel. The command
sequences are Waveshare's own, and the pin mapping is derived from the
datasheet and the board pinout, but no frame has been drawn on glass.

The hardware SPI variant is worse than unverified: the `boot.img` it needs
was flashed and did not boot. All seven HAT pins were confirmed free as plain
GPIO in the stock device tree, which is why `--bitbang` is the default.
