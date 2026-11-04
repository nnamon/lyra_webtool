# Building from the Luckfox SDK

Build `rootfs.img`, `boot.img` and `uboot.img` from source in Docker, then
flash them with the web tool. This is the only route that adds packages,
changes the kernel, or changes the device tree.

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
   ./sdk.sh init ~/Downloads/Luckfox_Lyra_SDK_250815.tar.gz
   ```
   This selects `luckfox_lyra_zero-w_buildroot_spinand_defconfig`, which is
   the Zero W on SPI NAND with Buildroot. It is the same choice as `lunch`
   options 4, 1 and 0, made by name because that is less fragile than feeding
   numbers into a menu.
2. Build everything once:
   ```sh
   ./sdk.sh build
   ```
3. Collect the images:
   ```sh
   ./sdk.sh fetch
   ```
   They land in `../images/`, where `../build.sh` expects them.
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
| Anything else | `./sdk.sh shell` |

`buildroot-config` and `kernel-config` open menuconfig and save the result
back into the defconfig, so a later clean build keeps your choice.

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

Not verified: a full `./sdk.sh build`. That needs the SDK tarball, which is
not public enough to download unattended, and hours of emulated compilation.
Expect to debug a missing dependency or two on the first run, and add it to
the `Dockerfile`.
