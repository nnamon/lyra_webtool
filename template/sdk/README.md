# Building from the Luckfox SDK

Build `rootfs.img`, `boot.img` and `uboot.img` from source in Docker, then
flash them with the web tool. This is the only route that adds packages,
changes the kernel, or changes the device tree.

This follows Luckfox's own instructions for the Lyra, moved into a container:

- [SDK image compilation](https://wiki.luckfox.com/Luckfox-Lyra/SDK-Image-Compilation)
- [SDK Buildroot configuration](https://wiki.luckfox.com/Luckfox-Lyra/Buildroot-Configuration)

Use the Lyra pages, not the Pico ones. The Pico Pro Max page gives the
menuconfig target as `buildrootconfig`; on this SDK it is `buildroot-config`,
and the other spelling is not a valid target. The Pico page also warns that
its Buildroot is uClibc, so third-party packages must not need glibc. That
does not apply here. The Lyra toolchain is
`arm-none-linux-gnueabihf`, and `gnueabihf` without `uclibc` in the triplet
is glibc.

## What you need first

- Docker, with about 60 GB free in its disk image. The SDK source is around
  15 GB and a full build adds more.
- The SDK tarball, `Luckfox_Lyra_SDK_<date>.tar.gz`, from Luckfox's Google
  Drive:
  <https://drive.google.com/drive/folders/1l2ixhfw53J3eZunyvHMnw2DYViH9d8Cx>
  `sdk.sh` will not download it for you.
- Hours. A first full build is long, and longer still on an Apple Silicon Mac,
  where every compiler process runs emulated.

The image is amd64 and cannot be anything else. The SDK ships prebuilt x86_64
host tools in `prebuilts/gcc/linux-x86/`, and Buildroot checks for a host
compiler that can produce 32-bit binaries. On an Apple Silicon Mac, turn on
"Use Rosetta for x86_64/amd64 emulation" in Docker Desktop's settings, under
General. Without it the build runs under QEMU and takes far longer.

## Build

1. Extract the SDK and select the board:
   ```sh
   ./sdk.sh init ~/Downloads/luckfox-lyra-241213.tar.gz
   ```
   The tarball holds a `repo` mirror rather than a source tree, so `init`
   unpacks it and then runs `.repo/repo/repo sync -l` to check the tree out,
   which is the order the vendor page gives. Nothing exists to build until
   that finishes.

   It then selects `luckfox_lyra_zero-w_buildroot_spinand_defconfig`, the Zero
   W on SPI NAND with Buildroot. That is the same choice as `lunch` options 4,
   1 and 0, made by name because it is less fragile than feeding numbers into
   a menu. If your SDK release does not carry that defconfig, `init` stops and
   lists the rk3506 defconfigs it does have, so set `DEFCONFIG` at the top of
   `sdk.sh` to one of those.
2. Build everything once:
   ```sh
   ./sdk.sh build
   ```
3. Collect the images:
   ```sh
   ./sdk.sh fetch
   ```
   They land in `../images/`, where `../build.sh` expects them.

   `fetch` copies `uboot.img`, `boot.img` and `rootfs.img` only. `rockdev/`
   also holds an SDK-built `MiniLoaderAll.bin` and `parameter.txt`, and
   `../README.md` builds from the vendor image and keeps the vendor loader, so
   taking all of `rockdev/` would discard it. `./sdk.sh fetch all` copies
   everything if that is what you want.
4. Pack and flash:
   ```sh
   cd .. && ./build.sh update.img
   ```

After the first build, build only what you changed. `./sdk.sh build rootfs`
takes minutes rather than hours.

## Change something

| Goal | Commands |
| --- | --- |
| Add or remove a package | `./sdk.sh build buildroot-config`, then `./sdk.sh build rootfs` |
| Change a kernel option | `./sdk.sh build kernel-config`, then `./sdk.sh build kernel` |
| Edit the device tree | `./sdk.sh get kernel-6.1/arch/arm/boot/dts/rk3506b-luckfox-lyra-zero-w.dts`, edit, `./sdk.sh put rk3506b-luckfox-lyra-zero-w.dts kernel-6.1/arch/arm/boot/dts/`, then `./sdk.sh build kernel` |
| Enable SPI, I2C, PWM or a UART | the same, and note the board shares `rk3506-luckfox-lyra-ultra.dtsi` with the Ultra and Pi. Build it here rather than repacking `boot.img` by hand: `../boot/` produced an image that does not boot |
| Anything else | `./sdk.sh shell` |

`buildroot-config` and `kernel-config` open menuconfig and save the result
back into the defconfig, so a later clean build keeps your choice.

`buildroot-config` is the name the vendor Buildroot page gives. `kernel-config`
is not on a vendor page; it matches the hyphenated pattern and a Lyra user
reports it working, so check `./build.sh help` in your own tree if it is
rejected.

Kernel modules install into `rootfs.img` under `/lib/modules`, not into
`boot.img`. A driver built as a module therefore needs `./sdk.sh build kernel`
and then `./sdk.sh build rootfs`, in that order.

### Adding files without patching a package

Buildroot copies the directories listed in `BR2_ROOTFS_OVERLAY` onto the root
filesystem at the end of every build. The Lyra config already uses it for
`board/rockchip/common/base`. Add your own directory to that list rather than
editing packages, and your files survive a clean rebuild.

For a handful of files, `../rootfs/` is quicker: it edits a built
`rootfs.img` without building anything from source.

### If a package does not appear after you enable it

Buildroot skips a package whose build directory already exists. Delete it:

```sh
./sdk.sh shell
rm -rf buildroot/output/rockchip_rk3506_luckfox/build/<package>-*
exit
./sdk.sh build rootfs
```

## Where the dependency list comes from

The `Dockerfile` installs the apt list Luckfox publishes on the SDK
compilation page, plus what the SDK's own preflight scripts check for. One
vendor entry is left out: `python-is-python3` takes ownership of
`/usr/bin/python` as a dpkg file pointing at Python 3, which contradicts the
`ln -sf /usr/bin/python2 /usr/bin/python` the same page tells you to run. The
`Dockerfile` makes that symlink instead.

## Why the tree lives in a Docker volume

The SDK refuses to build anywhere else. `check-sdk.sh` runs
`findmnt -o FSTYPE` on its own directory and exits unless the answer is
`ext4`, `f2fs` or `btrfs`. Measured on Docker Desktop for Mac:

| Location | Reported type | SDK check |
| --- | --- | --- |
| Named Docker volume | `ext4` | passes |
| Directory bind-mounted from macOS | `virtiofs` | fails |
| The container's own filesystem | `overlay` | fails |

So the source cannot sit in your home directory, and it cannot sit in the
image either. `sdk.sh` keeps it in the volume `lyra-sdk` and moves files in
and out with `get`, `put` and `fetch`. `./sdk.sh clean` deletes it.

The same script also runs as a non-root user that owns the tree, because
`check-sdk.sh` requires the owner to match and Buildroot's configure scripts
refuse to run as root.

## The network warning is expected

Every build prints this and waits five seconds:

```
Your network is not able to access sources.buildroot.net!
```

Ignore it. `check-network.sh` sends an HTTP `HEAD` to the bare hostname, which
the mirror answers with 403, and then falls back to `ping`, which Docker's VM
blocks. Downloads themselves work: a `GET` of
`https://sources.buildroot.net/bc/bc-1.07.1.tar.gz` returns 200 from inside
this container. The Zero W config leaves `RK_NETWORK_CHECK` unset, so the
check only warns.

## Verified so far

Built and checked on an Apple Silicon Mac with Docker Desktop:

- The image builds, and `check-sdk.sh` and `check-buildroot.sh` from the SDK
  both pass inside it against a named volume.
- `gcc -m32` and `g++ -m32` both compile, which is Buildroot's own host
  requirement.
- Every tool the SDK's preflight scripts look for is present: `python3`,
  `python2`, `rsync`, `unbuffer`, `bc`, `bison`, `flex`, `dtc`, `mkimage`,
  `cpio`, `fakeroot`, `lz4`, `zstd`, `git`, `wget`, `patchelf`, `chrpath`,
  `makeinfo`, and `make` 4.3.
- `init` handles a `repo`-only tarball, which is the shape the vendor page
  describes. Tested against a stub mirror whose `repo sync -l` writes the
  working tree: `init` unpacks, syncs, selects the defconfig, and `status`
  then reads back `luckfox_lyra_zero-w_buildroot_spinand_defconfig` and chip
  `rk3506`. Tested against a mirror without that defconfig too, where `init`
  lists the alternatives and exits 1.
- `fetch` replaces the three partition images and leaves a vendor
  `MiniLoaderAll.bin` and `parameter.txt` in place. `fetch all` overwrites
  them. Both checked by marking the files and reading them back.

Not verified: a full `./sdk.sh build`. That needs the SDK tarball, which is
not public enough to download unattended, and hours of emulated compilation.
The dependency list is the vendor's own, so a missing package is less likely
than it was, but if one turns up, add it to the `Dockerfile`.
