#!/usr/bin/env python3
"""Route SPI1 to the pins a seated Raspberry Pi HAT uses, and expose spidev.

The Lyra Zero W header is pin-compatible with the Raspberry Pi 40-pin header:
all 12 power and ground pins line up. A Pi HAT seats on it directly. What does
not line up is where the SPI signals come out. The Pi puts MOSI on pin 19 and
CE0 on pin 24; the Lyra's fixed SPI1 pinmux puts MOSI on pin 26 and CS0 on
pin 12. Only the clock, on pin 23, happens to agree.

RK3506 has a crossbar, the RM_IO matrix, that can route a peripheral signal to
any header pin. This patch uses it to move SPI1 onto the three pins the HAT
actually drives, so no jumpers are needed.

  HAT signal  Pi pin  Lyra pin   RM_IO    function      mux
  CLK         23      GPIO0_B0   RM_IO8   SPI1_CLK      0x57
  DIN/MOSI    19      GPIO0_A6   RM_IO6   SPI1_MOSI     0x58
  CS          24      GPIO0_B2   RM_IO10  SPI1_CSN0     0x5a

MISO is deliberately left unrouted: the panel is write-only and the HAT does
not drive pin 21.

The mux numbers are the RM function indices from the RK3506B datasheet plus
15. That offset is not documented; it was measured against this very device
tree, where PWM0_CH0 (index 30) appears as 45, PWM1_CH0 (34) as 49, PDM_SDI0
(22) as 37, TSADC_CTRL (77) as 92 and SPDIF_TX (80) as 95. Eight functions,
one constant offset.
"""
import re
import sys

SPI_NODE = "spi@ff130000"
PCFG = 0x1D6  # pcfg-pull-none-drv-level-3, what the vendor SPI groups use

GROUP = """
			epaper-spi1-pins {
				rockchip,pins = <0x00 0x08 0x57 0x1d6 0x00 0x06 0x58 0x1d6 0x00 0x0a 0x5a 0x1d6>;
				phandle = <0x%x>;
			};
"""

CHILD = """
\t\tspidev@0 {
\t\t\tcompatible = "rohm,dh2228fv";
\t\t\treg = <0x00>;
\t\t\tspi-max-frequency = <0x3d0900>;
\t\t};
"""


def main(path):
    src = open(path).read()

    if "epaper-spi1-pins" in src:
        print("already patched, nothing to do")
        return

    used = {int(h, 16) for h in re.findall(r"phandle = <0x([0-9a-f]+)>;", src)}
    new_ph = max(used) + 1

    # Add the pin group inside the pinctrl node, next to the stock spi1 groups.
    m = re.search(r"(\n\t\t\tspi1-csn0-pins \{\n.*?\n\t\t\t\};)", src, re.S)
    if not m:
        sys.exit("error: spi1-csn0-pins not found, cannot place the new group")
    src = src[: m.end(1)] + (GROUP % new_ph).rstrip("\n") + src[m.end(1) :]

    # Point spi1 at it, enable the controller, and add spidev.
    m = re.search(r"\n\t%s \{\n(.*?)\n\t\};" % re.escape(SPI_NODE), src, re.S)
    if not m:
        sys.exit("error: %s not found" % SPI_NODE)
    body = m.group(1)
    if 'status = "disabled";' not in body:
        sys.exit("error: %s is not disabled; refusing to guess" % SPI_NODE)

    new = body.replace('status = "disabled";', 'status = "okay";')
    new = re.sub(r"pinctrl-0 = <[^>]*>;", "pinctrl-0 = <0x%x>;" % new_ph, new)
    new = new + CHILD.rstrip("\n")
    src = src[: m.start(1)] + new + src[m.end(1) :]

    open(path, "w").write(src)
    print("routed SPI1 to the HAT pins via RM_IO (phandle 0x%x)" % new_ph)
    print("enabled %s and added spidev@0 at 4 MHz" % SPI_NODE)
    print("the board will expose /dev/spidev1.0")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "board.dts")
