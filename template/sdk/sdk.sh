#!/usr/bin/env bash
# Build the Luckfox Lyra SDK in Docker, for the Lyra Zero W on SPI NAND.
set -euo pipefail
cd "$(dirname "$0")"

TOOLS_IMAGE=lyra-sdk-tools
VOLUME=${LYRA_SDK_VOLUME:-lyra-sdk}
PLATFORM=linux/amd64

# Selected non-interactively, because ./build.sh accepts a defconfig name and
# that is less fragile than feeding numbers into the lunch menu. This one is
# the Zero W on SPI NAND with Buildroot, the same as lunch options 4, 1, 0.
DEFCONFIG=luckfox_lyra_zero-w_buildroot_spinand_defconfig

die() { echo "error: $*" >&2; exit 1; }

build_tools() {
  docker image inspect "$TOOLS_IMAGE" >/dev/null 2>&1 && return 0
  echo "Building $TOOLS_IMAGE (once, several minutes under emulation)"
  docker build --platform "$PLATFORM" -t "$TOOLS_IMAGE" .
}

have_sdk() {
  docker run --rm --platform "$PLATFORM" -v "$VOLUME:/sdk" "$TOOLS_IMAGE" \
    test -f /sdk/build.sh 2>/dev/null
}

# Run a script in the SDK tree. Extra docker arguments come before the image.
in_sdk() {
  local script=$1; shift
  # A plain string, not an array: macOS ships bash 3.2, where expanding an
  # empty array under `set -u` is an unbound variable error.
  local tty=""
  [ -t 0 ] && tty="-it"
  # shellcheck disable=SC2086
  docker run --rm $tty --platform "$PLATFORM" \
    -v "$VOLUME:/sdk" -w /sdk \
    -e "DEFCONFIG=$DEFCONFIG" \
    "$@" "$TOOLS_IMAGE" bash -eu -c "$script"
}

cmd_init() {
  local tgz=${1:-}
  [ -n "$tgz" ] || die "usage: ./sdk.sh init <Luckfox_Lyra_SDK_<date>.tar.gz>"
  [ -f "$tgz" ] || die "no such file: $tgz"
  tgz=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$tgz")
  build_tools
  docker volume create "$VOLUME" >/dev/null

  in_sdk '
    sudo chown "$(id -u):$(id -g)" /sdk
    if [ -f /sdk/build.sh ]; then
      echo "SDK already extracted, skipping"
    else
      echo "Extracting $(basename /in/sdk.tar.gz), this takes a while"
      tar -C /sdk --warning=no-unknown-keyword -xf /in/sdk.tar.gz
      # Some releases wrap everything in one top-level directory.
      if [ ! -f /sdk/build.sh ]; then
        inner=$(find /sdk -maxdepth 2 -name build.sh -type f | head -1)
        [ -n "$inner" ] || { echo "no build.sh in the tarball" >&2; exit 1; }
        d=$(dirname "$inner")
        if [ "$d" != /sdk ]; then
          echo "Moving $d up to /sdk"
          ( shopt -s dotglob; mv "$d"/* /sdk/ )
        fi
      fi
    fi

    # A repo-managed tarball ships only the .repo mirror, so the working tree
    # has to be checked out from it before anything can build.
    if [ -d /sdk/.repo ] && [ ! -d /sdk/buildroot ]; then
      echo "Checking out the working tree from .repo"
      ./.repo/repo/repo sync -l
    fi

    echo
    echo "Selecting $DEFCONFIG"
    ./build.sh "$DEFCONFIG"
  ' -v "$tgz:/in/sdk.tar.gz:ro"

  echo
  echo "Ready. Next: ./sdk.sh build   (hours on a first build)"
}

cmd_shell() {
  build_tools
  have_sdk || die "no SDK in volume $VOLUME; run ./sdk.sh init <tarball> first"
  docker run --rm -it --platform "$PLATFORM" -v "$VOLUME:/sdk" -w /sdk \
    "$TOOLS_IMAGE" bash --login
}

cmd_build() {
  build_tools
  have_sdk || die "no SDK in volume $VOLUME; run ./sdk.sh init <tarball> first"
  local args="$*"
  in_sdk "./build.sh $args"
}

cmd_fetch() {
  local dest=${1:-../images}
  mkdir -p "$dest"
  dest=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$dest")
  build_tools
  in_sdk '
    [ -d /sdk/rockdev ] || { echo "nothing built yet: no rockdev/" >&2; exit 1; }
    cp -Lv /sdk/rockdev/* /out/ 2>/dev/null || true
    ls -l /out/
  ' -v "$dest:/out"
  echo "copied into $dest"
}

cmd_get() {
  local path=${1:-} dest=${2:-.}
  [ -n "$path" ] || die "usage: ./sdk.sh get <path in the SDK> [destination]"
  mkdir -p "$dest"
  dest=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$dest")
  build_tools
  in_sdk "cp -Lrv \"/sdk/$path\" /out/" -v "$dest:/out"
}

cmd_put() {
  local src=${1:-} path=${2:-}
  [ -n "$src" ] && [ -n "$path" ] || die "usage: ./sdk.sh put <local file> <path in the SDK>"
  [ -f "$src" ] || die "no such file: $src"
  src=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$src")
  build_tools
  in_sdk "cp -v /in/file \"/sdk/$path\"" -v "$src:/in/file:ro"
}

cmd_status() {
  build_tools
  if ! have_sdk; then echo "volume $VOLUME holds no SDK"; return 0; fi
  # output/defconfig and device/rockchip/.chip are the symlinks the SDK itself
  # keeps, as RK_DEFCONFIG_LINK and RK_CHIP_DIR in its build.sh.
  in_sdk '
    if [ -L output/defconfig ]; then
      echo "defconfig: $(basename "$(readlink -f output/defconfig)")"
    else
      echo "defconfig: not selected"
    fi
    if [ -L device/rockchip/.chip ]; then
      echo "chip:      $(basename "$(readlink -f device/rockchip/.chip)")"
    fi
    echo "tree size: $(du -sh /sdk 2>/dev/null | cut -f1)"
    echo
    if [ -d /sdk/rockdev ]; then echo "rockdev/:"; ls -lL /sdk/rockdev/;
    else echo "rockdev/: nothing built yet"; fi
  '
}

cmd_clean() {
  # Docker reports the volume as still in use for a moment after a --rm
  # container exits, so retry rather than fail on the race.
  local i
  for i in 1 2 3 4 5; do
    if docker volume rm -f "$VOLUME" >/dev/null 2>&1; then
      echo "removed volume $VOLUME"
      return 0
    fi
    sleep 1
  done
  docker volume rm -f "$VOLUME"
}

usage() {
  cat <<USAGE
Usage: ./sdk.sh <command> [args]

  init <sdk.tar.gz>   extract the SDK into the work volume and select the
                      Zero W SPI NAND Buildroot config
  build [target]      run ./build.sh in the SDK. With no target, build
                      everything. Useful targets:
                        rootfs            rebuild rootfs.img
                        kernel            rebuild boot.img
                        uboot             rebuild uboot.img and the loader
                        firmware          pack rockdev/
                        buildroot-config  add or remove packages
                        kernel-config     change kernel options
  fetch [dir]         copy rockdev/ out to dir (default ../images)
  get <path> [dir]    copy one path out of the SDK tree
  put <file> <path>   copy one file into the SDK tree
  shell               interactive shell in the SDK tree
  status              selected config, tree size, built images
  clean               delete the work volume and everything in it

The SDK tarball comes from Luckfox's Google Drive. This script will not
download it:
https://drive.google.com/drive/folders/1l2ixhfw53J3eZunyvHMnw2DYViH9d8Cx
USAGE
  exit 1
}

case "${1:-}" in
  init)   shift; cmd_init "$@" ;;
  build)  shift; cmd_build "$@" ;;
  fetch)  shift; cmd_fetch "$@" ;;
  get)    shift; cmd_get "$@" ;;
  put)    shift; cmd_put "$@" ;;
  shell)  shift; cmd_shell "$@" ;;
  status) shift; cmd_status "$@" ;;
  clean)  shift; cmd_clean "$@" ;;
  *) usage ;;
esac
