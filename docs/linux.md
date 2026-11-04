# Changing the Linux environment

Three routes change what runs on the board. They differ in how long they take
and in what they can reach.

| Route | Reaches | Cost |
| --- | --- | --- |
| Edit on the running board | Config files, scripts, startup | Seconds, no host setup |
| Rebuild `rootfs.img` offline | Everything in the root filesystem | Minutes, needs Docker |
| Rebuild from the SDK | Packages, kernel, device tree, U-Boot | Hours, needs Docker and a 2 GB download |

Routes 2 and 3 both run in Docker, so none of this needs a Linux machine.

The board runs Buildroot, so there is no package manager. `apt` and `opkg` do
not exist, and nothing can be installed at run time. That is why the third
route exists.

## What the images hold

| Image | Partition | Contents |
| --- | --- | --- |
| `uboot.img` | 4 MiB at 4 MiB | U-Boot |
| `boot.img` | 12 MiB at 8 MiB | Kernel and device tree, as a compressed FIT image |
| `rootfs.img` | 32 MiB onward, grows | UBI image holding a UBIFS root filesystem |

Kernel modules are in `rootfs.img`, under `/lib/modules`, not in `boot.img`.
The Zero W config sets `RK_ROOTFS_INSTALL_MODULES=y`, and the post-rootfs hook
`device/rockchip/common/scripts/post-modules.sh` copies the `.ko` files from
the kernel build into the root filesystem.

That splits a driver change in two. A built-in driver needs only a new
`boot.img`. A driver built as a module needs a new `boot.img` for the kernel
and a new `rootfs.img` for the `.ko` file, and the two have to match.

## Route 1: edit on the running board

UBIFS is a read-write filesystem, so edits survive a reboot. Confirm that
with `cat /proc/mounts` before relying on it; look for `ubi0:rootfs` mounted
`rw`.

1. Get a shell. Over USB the board appears as an RNDIS network adapter at
   `192.168.123.100`. Over UART, use the console on the pin header.
2. Edit what you need. `vi` and `nano` are both present.
3. Add a startup script as `/etc/init.d/S99name`, then `chmod +x` it. BusyBox
   init runs `/etc/init.d/S*` in name order, so the number sets the order.
4. Reboot to check it survives.

This route cannot add a program that is not already installed. Cross-compile
instead. `template/rootfs/cross/` does that in Docker without the SDK:

```sh
cd template/rootfs/cross
./cross.sh cc -O2 hello.c -o hello     # armv7-a, hard float, VFPv4
./cross.sh check hello                 # will it load on the board
scp hello root@192.168.123.100:/root
```

The board runs glibc 2.38, and a binary needing a newer symbol version loads
and then dies. `check` catches that, along with a wrong architecture or
interpreter. `template/rootfs/cross/README.md` has the target ABI and the
glibc rule.

If you have the SDK unpacked, its own toolchain works too, at
`prebuilts/gcc/linux-x86/arm/gcc-arm-10.3-2021.07-x86_64-arm-none-linux-gnueabihf/bin`.

## Route 2: rebuild rootfs.img offline

This changes any file in the root filesystem without building anything from
source, and it works on macOS. See `template/rootfs/README.md` for the
commands and for why the tree has to live in a Docker volume.

Use it to ship a set of config changes as a flashable image, or to prepare a
board before it ever boots. It can also drop in a `.ko` module you built
elsewhere, or a program you cross-compiled with `template/rootfs/cross/`. It
cannot add packages, and it cannot change the kernel itself.

## Route 3: rebuild from the SDK

This is the only route that adds packages, changes the kernel config, or
changes the device tree.

It follows Luckfox's own pages for the Lyra, moved into a container:
[SDK image compilation](https://wiki.luckfox.com/Luckfox-Lyra/SDK-Image-Compilation)
and [SDK Buildroot configuration](https://wiki.luckfox.com/Luckfox-Lyra/Buildroot-Configuration).
Take the Lyra pages rather than the Pico ones: the Pico Pro Max page gives the
menuconfig target as `buildrootconfig`, which this SDK does not accept, and
warns about uClibc, which this board does not use.

`template/sdk/` runs it in Docker. `template/sdk/README.md` has the setup, the
disk and time budget, and the two constraints that decide the design: the SDK
must sit on an ext4 filesystem, so the source lives in a Docker volume, and
the container must be amd64, because the SDK ships x86_64 host tools.

```sh
cd template/sdk
./sdk.sh init ~/Downloads/luckfox-lyra-241213.tar.gz
./sdk.sh build
./sdk.sh fetch
```

The tarball is a `repo` mirror, not a source tree, so `init` unpacks it and
then runs `.repo/repo/repo sync -l` to check the tree out before anything can
build.

It then selects
`device/rockchip/.chips/rk3506/luckfox_lyra_zero-w_buildroot_spinand_defconfig`,
the Zero W on SPI NAND with Buildroot, the same as `lunch` options 4, 1 and 0.
Its `parameter-lyra-spinand.txt` is byte for byte the `parameter.txt` in
`template/`, which is how we know this is the config that built the firmware
on your board. An older SDK release may not carry that defconfig, in which
case `init` lists the ones it has and stops.

Get the SDK tarball from Luckfox's Google Drive:
<https://drive.google.com/drive/folders/1l2ixhfw53J3eZunyvHMnw2DYViH9d8Cx>

The targets, which `./sdk.sh build <target>` passes straight through:

| Target | Output | Use it for |
| --- | --- | --- |
| `buildroot-config` | updates the Buildroot defconfig | Adding or removing packages |
| `rootfs` | `rockdev/rootfs.img` | Any root filesystem change |
| `kernel-config` | updates the kernel defconfig | Drivers, kernel options |
| `kernel` | `rockdev/boot.img` | Kernel or device tree changes |
| `uboot` | `rockdev/uboot.img`, `rockdev/MiniLoaderAll.bin` | Boot changes |
| `firmware` | packs `rockdev/` | Producing a full set |
| no target | everything | A first build |

`./sdk.sh fetch` copies `uboot.img`, `boot.img` and `rootfs.img` out of
`rockdev/` into `template/images/`, where `template/build.sh` expects them.
It leaves the vendor `MiniLoaderAll.bin` and `parameter.txt` alone, because
`template/README.md` builds from the vendor image and keeps the vendor
loader. `./sdk.sh fetch all` takes the SDK's versions of those too. That is
what you want after `build uboot`, which is the one target that produces a new
`MiniLoaderAll.bin`.

From there, write a single partition image with the web tool's Flash tab, or
pack a whole `update.img`.

### Where the config lives

| What | Path in the SDK |
| --- | --- |
| Board config | `device/rockchip/.chips/rk3506/luckfox_lyra_zero-w_buildroot_spinand_defconfig` |
| Partition table | `device/rockchip/.chips/rk3506/parameter-lyra-spinand.txt` |
| Buildroot packages | `buildroot/configs/rockchip_rk3506_luckfox_defconfig` |
| Root filesystem type | `buildroot/configs/rockchip/fs/ubifs.config` |
| Kernel config | `kernel-6.1/arch/arm/configs/rk3506_luckfox_defconfig` |
| Device tree | `kernel-6.1/arch/arm/boot/dts/rk3506b-luckfox-lyra-zero-w.dts` |

### Adding files without patching a package

Buildroot's overlay mechanism copies a directory tree onto the root filesystem
at the end of every build, so the files survive a clean rebuild.
`BR2_ROOTFS_OVERLAY` in the Buildroot config holds the list, and the Lyra
config already uses it for `board/rockchip/common/base`. Add your own
directory to that list rather than editing packages.

### If a package does not appear after you enable it

Buildroot skips a package whose build directory already exists. Delete it and
build again:

```sh
./sdk.sh shell
rm -rf buildroot/output/rockchip_rk3506_luckfox/build/<package>-*
exit
./sdk.sh build rootfs
```

## Changing what the board exposes: boot.img

Every driver is built into the vendor kernel, so turning on SPI, I2C, PWM or a
UART needs no kernel rebuild in principle. It needs one `status` change in the
device tree inside `boot.img`.

**This does not work yet.** A container repacked by `template/boot/` was
flashed to a board and did not boot. Before reaching for it, check whether you
need it at all: pins that no enabled node claims are already usable as GPIO,
and a userspace driver on those pins cannot break booting.

```sh
cd template/boot
./boot.sh roundtrip /path/to/boot.img    # prove pack is faithful first
./boot.sh unpack    /path/to/boot.img
./boot.sh enable-epaper-spi
./boot.sh pack      ../images/boot.img
```

25 of the 60 nodes in the shipped device tree are disabled, including both
SPI controllers, five of six UARTs, all ten PWMs, both CAN and both Ethernet.
That is the Rockchip convention, not a fault: the SoC `.dtsi` disables
everything and each board's `.dts` enables what it wires.

See [../template/boot/README.md](../template/boot/README.md), which also
covers the FIT signature question and the RM_IO pin matrix.

## WiFi on the Zero W

`wlan0` comes up `NO-CARRIER` until it associates. The onboard antenna needs
GPIO 55 driven high first.

```sh
echo 55 > /sys/class/gpio/export
echo out > /sys/class/gpio/gpio55/direction
echo 1 > /sys/class/gpio/gpio55/value
wifi-connect.sh <SSID> <password>
udhcpc -i wlan0
```

Those settings are lost on reboot. To keep them, put the same commands in
`/etc/init.d/S99wlan0` with a `sleep 5` first, make it executable, and either
edit the running board or ship the script through `template/rootfs/files/`.
