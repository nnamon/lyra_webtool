#!/usr/bin/env bash
# Take boot.img apart, edit the device tree, and put it back together.
#
# boot.img is a U-Boot FIT holding the kernel, the device tree and a Rockchip
# resource blob. Only the device tree needs changing to turn on a peripheral:
# every driver on this board is already built into the kernel.
set -euo pipefail
cd "$(dirname "$0")"

TOOLS_IMAGE=lyra-boot-tools
VOLUME=${LYRA_BOOT_VOLUME:-lyra-boot}
SELF=$PWD

die() { echo "error: $*" >&2; exit 1; }

build_tools() {
  docker image inspect "$TOOLS_IMAGE" >/dev/null 2>&1 && return 0
  echo "Building $TOOLS_IMAGE (once)"
  docker build -q -t "$TOOLS_IMAGE" . >/dev/null
}

abspath() { python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$1"; }

in_work() {
  local script=$1; shift
  local tty=""
  [ -t 0 ] && tty="-it"
  # shellcheck disable=SC2086
  docker run --rm $tty -v "$VOLUME:/work" -w /work "$@" \
    "$TOOLS_IMAGE" bash -eu -c "$script"
}

have_work() {
  docker run --rm -v "$VOLUME:/work" "$TOOLS_IMAGE" \
    test -f /work/board.dts 2>/dev/null
}

cmd_unpack() {
  local img=${1:-}
  [ -n "$img" ] || die "usage: ./boot.sh unpack <boot.img>"
  [ -f "$img" ] || die "no such file: $img"
  img=$(abspath "$img")
  build_tools
  docker volume create "$VOLUME" >/dev/null
  in_work '
    find /work -mindepth 1 -delete
    cp /in/boot.img /work/
    cp /in/boot.its.template /work/boot.its
    dumpimage -T flat_dt -p 0 -o fdt.dtb      boot.img >/dev/null
    dumpimage -T flat_dt -p 1 -o kernel.bin   boot.img >/dev/null
    dumpimage -T flat_dt -p 2 -o resource.bin boot.img >/dev/null
    dtc -I dtb -O dts fdt.dtb -o board.dts 2>/dev/null
    cp board.dts board.dts.orig
    ls -la fdt.dtb kernel.bin resource.bin board.dts
  ' -v "$img:/in/boot.img:ro" -v "$SELF/boot.its.template:/in/boot.its.template:ro"
  echo "Unpacked into volume $VOLUME. Next: ./boot.sh enable-epaper-spi"
}

cmd_pack() {
  local out=${1:-boot-new.img}
  mkdir -p "$(dirname "$out")"
  local dir base
  dir=$(abspath "$(dirname "$out")"); base=$(basename "$out")
  build_tools
  have_work || die "nothing unpacked; run ./boot.sh unpack <boot.img> first"
  in_work '
    dtc -I dts -O dtb board.dts -o fdt.dtb 2>/dev/null
    # -E keeps the image data outside the tree and -p places the first blob at
    # 0x800, the same layout the vendor container uses. mkimage recomputes
    # every sha256 as it writes, and those hashes are what U-Boot checks.
    mkimage -E -p 0x800 -f boot.its "/out/$OUT" >/dev/null
    ls -la "/out/$OUT"
  ' -e "OUT=$base" -v "$dir:/out"
  echo "wrote $dir/$base"
}

cmd_enable_spi1() {
  build_tools
  have_work || die "nothing unpacked; run ./boot.sh unpack <boot.img> first"
  in_work 'python3 /in/patch-epaper-spi.py /work/board.dts' \
    -v "$SELF/patch-epaper-spi.py:/in/patch-epaper-spi.py:ro"
}

cmd_roundtrip() {
  local img=${1:-}
  [ -n "$img" ] || die "usage: ./boot.sh roundtrip <boot.img>"
  cmd_unpack "$img" >/dev/null
  cmd_pack /tmp/lyra-boot-rt.img >/dev/null
  build_tools
  img=$(abspath "$img")
  # The kernel and resource blobs are copied through untouched, so their
  # hashes must match exactly. The device tree is decompiled and recompiled,
  # which is not byte-reproducible: dtc rewrites the header. Compare that one
  # as a tree instead, which is what the kernel actually reads.
  in_work '
    fail=0
    for n in 1 2; do
      a=$(dumpimage -l /in/a.img | grep "Hash value" | sed -n "$((n+1))p")
      b=$(dumpimage -l /in/b.img | grep "Hash value" | sed -n "$((n+1))p")
      nm=$([ $n = 1 ] && echo kernel || echo resource)
      if [ "$a" = "$b" ]; then
        echo "ok   $nm hash identical"
      else
        echo "FAIL $nm hash changed"; fail=1
      fi
    done
    dumpimage -T flat_dt -p 0 -o rt-a.dtb /in/a.img >/dev/null
    dumpimage -T flat_dt -p 0 -o rt-b.dtb /in/b.img >/dev/null
    dtc -I dtb -O dts rt-a.dtb -o rt-a.dts 2>/dev/null
    dtc -I dtb -O dts rt-b.dtb -o rt-b.dts 2>/dev/null
    if diff -q rt-a.dts rt-b.dts >/dev/null; then
      echo "ok   device tree semantically identical ($(stat -c%s rt-a.dtb) bytes both)"
    else
      echo "FAIL device tree changed:"; diff rt-a.dts rt-b.dts | head -20; fail=1
    fi
    rm -f rt-a.dtb rt-b.dtb rt-a.dts rt-b.dts
    echo
    if [ "$fail" = 0 ]; then
      echo "round trip clean: pack reproduces this boot.img"
    else
      echo "round trip FAILED, do not flash anything this built" >&2; exit 1
    fi
  ' -v "$img:/in/a.img:ro" -v "/tmp/lyra-boot-rt.img:/in/b.img:ro"
  rm -f /tmp/lyra-boot-rt.img
}

cmd_diff() {
  build_tools
  have_work || die "nothing unpacked"
  in_work 'diff -u board.dts.orig board.dts || true'
}

cmd_shell() {
  build_tools
  docker run --rm -it -v "$VOLUME:/work" -w /work "$TOOLS_IMAGE" bash
}

cmd_clean() {
  local i
  for i in 1 2 3 4 5; do
    docker volume rm -f "$VOLUME" >/dev/null 2>&1 && { echo "removed $VOLUME"; return 0; }
    sleep 1
  done
  docker volume rm -f "$VOLUME"
}

usage() {
  cat <<USAGE
Usage: ./boot.sh <command> [args]

  unpack <boot.img>    split the FIT and decompile the device tree
  enable-epaper-spi          route SPI1 to the seated HAT pins, enable it, add spidev
  diff                 show what changed in board.dts
  pack [out.img]       recompile and rebuild the FIT (default boot-new.img)
  roundtrip <boot.img> unpack and repack unchanged, compare every hash
  shell                shell in the work volume
  clean                delete the work volume

Run roundtrip on your own boot.img before trusting pack with a real edit.
USAGE
  exit 1
}

case "${1:-}" in
  unpack)      shift; cmd_unpack "$@" ;;
  enable-epaper-spi) shift; cmd_enable_spi1 "$@" ;;
  pack)        shift; cmd_pack "$@" ;;
  roundtrip)   shift; cmd_roundtrip "$@" ;;
  diff)        shift; cmd_diff "$@" ;;
  shell)       shift; cmd_shell "$@" ;;
  clean)       shift; cmd_clean "$@" ;;
  *) usage ;;
esac
