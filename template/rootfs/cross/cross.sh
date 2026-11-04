#!/usr/bin/env bash
# Cross-compile for the Luckfox Lyra Zero W, and check the result will run.
set -euo pipefail

SELF_DIR=$(cd "$(dirname "$0")" && pwd)
TOOLS_IMAGE=lyra-cross-tools
TRIPLE=arm-linux-gnueabihf
ROOTFS_VOLUME=${LYRA_ROOTFS_VOLUME:-lyra-rootfs}

# Measured from the board's own busybox: ELF32 ARM, EABI5, hard-float,
# Tag_CPU_arch v7, Tag_FP_arch VFPv4, Tag_ABI_VFP_args VFP registers.
# RK3506B is a Cortex-A7, which is armv7-a with NEON and VFPv4.
TARGET_FLAGS="-march=armv7-a -mfpu=neon-vfpv4 -mfloat-abi=hard"
TARGET_ELF_FLAGS=0x5000400
TARGET_GLIBC=2.38
TARGET_INTERP=/lib/ld-linux-armhf.so.3

die() { echo "error: $*" >&2; exit 1; }

build_tools() {
  docker image inspect "$TOOLS_IMAGE" >/dev/null 2>&1 && return 0
  echo "Building $TOOLS_IMAGE (once)"
  docker build -t "$TOOLS_IMAGE" "$SELF_DIR"
}

# Run in the current directory, as the current user so output is not root-owned.
in_cross() {
  local tty=""
  [ -t 0 ] && tty="-it"
  # shellcheck disable=SC2086
  docker run --rm $tty \
    --user "$(id -u):$(id -g)" \
    -v "$PWD:/src" -w /src \
    "$TOOLS_IMAGE" "$@"
}

cmd_cc() {
  [ $# -gt 0 ] || die "usage: ./cross.sh cc <gcc arguments>"
  build_tools
  # shellcheck disable=SC2086
  in_cross "$TRIPLE-gcc" $TARGET_FLAGS "$@"
}

cmd_cxx() {
  [ $# -gt 0 ] || die "usage: ./cross.sh c++ <g++ arguments>"
  build_tools
  # shellcheck disable=SC2086
  in_cross "$TRIPLE-g++" $TARGET_FLAGS "$@"
}

cmd_make() {
  build_tools
  in_cross make \
    "CC=$TRIPLE-gcc" "CXX=$TRIPLE-g++" "AR=$TRIPLE-ar" \
    "STRIP=$TRIPLE-strip" "CFLAGS=$TARGET_FLAGS -O2" \
    "CXXFLAGS=$TARGET_FLAGS -O2" "$@"
}

cmd_shell() {
  build_tools
  echo "Cross tools are ${TRIPLE}-*. Target flags: $TARGET_FLAGS"
  in_cross bash
}

cmd_check() {
  local bin=${1:-}
  [ -n "$bin" ] || die "usage: ./cross.sh check <binary>"
  [ -f "$bin" ] || die "no such file: $bin"
  build_tools
  # The target values go in as environment, not as nested quotes.
  docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/src:ro" -w /src \
    -e "WANT_FLAGS=$TARGET_ELF_FLAGS" -e "WANT_INTERP=$TARGET_INTERP" \
    -e "WANT_GLIBC=$TARGET_GLIBC" -e "BIN=$bin" \
    "$TOOLS_IMAGE" bash -euc '
    fail=0
    flags=$(readelf -h "$BIN" | sed -n "s/.*Flags:[[:space:]]*\(0x[0-9a-f]*\).*/\1/p")
    if [ "$flags" = "$WANT_FLAGS" ]; then
      echo "ok   ELF flags $flags, matching the board"
    else
      echo "FAIL ELF flags $flags, the board has $WANT_FLAGS"; fail=1
    fi
    interp=$(readelf -l "$BIN" | sed -n "s/.*program interpreter: \(.*\)\]/\1/p")
    if [ -z "$interp" ]; then
      echo "ok   static, no interpreter needed"
    elif [ "$interp" = "$WANT_INTERP" ]; then
      echo "ok   interpreter $interp"
    else
      echo "FAIL interpreter $interp, the board has $WANT_INTERP"; fail=1
    fi
    need=$(readelf -V "$BIN" 2>/dev/null | grep -oE "GLIBC_2\.[0-9]+" \
      | sort -uV | tail -1 | cut -d_ -f2)
    if [ -z "$need" ]; then
      echo "ok   needs no versioned glibc symbol"
    elif [ "$(printf "%s\n%s\n" "$need" "$WANT_GLIBC" | sort -V | head -1)" = "$need" ]; then
      echo "ok   needs glibc $need, the board has $WANT_GLIBC"
    else
      echo "FAIL needs glibc $need, the board only has $WANT_GLIBC"; fail=1
    fi
    exit $fail
  '
}

cmd_run() {
  local bin=${1:-}
  [ -n "$bin" ] || die "usage: ./cross.sh run <binary> [args]"
  [ -f "$bin" ] || die "no such file: $bin"
  shift
  build_tools
  docker volume inspect "$ROOTFS_VOLUME" >/dev/null 2>&1 \
    || die "no unpacked tree in volume $ROOTFS_VOLUME; run ../ubifs.sh unpack first"
  # The target's own libraries, under emulation, with no network. This runs
  # vendor binaries from the image, so keep it to a throwaway container.
  docker run --rm --network none \
    -v "$PWD:/src:ro" -v "$ROOTFS_VOLUME:/work:ro" -w /src \
    "$TOOLS_IMAGE" qemu-arm-static -L /work/tree "./$bin" "$@"
}

usage() {
  cat <<USAGE
Usage: ./cross.sh <command> [args]

  cc <args>        $TRIPLE-gcc with the board's ABI flags
  c++ <args>       $TRIPLE-g++ with the same
  make [args]      make with CC, CXX, AR, STRIP and CFLAGS set for the board
  check <binary>   confirm a binary will load on the board
  run <binary>     run it under emulation against the unpacked rootfs
  shell            shell with the cross tools on PATH

Compile in your own source directory, then install into the image by copying
into ../files/ and running ../ubifs.sh apply.

Target: armv7-a, hard float, VFPv4, glibc $TARGET_GLIBC.
USAGE
  exit 1
}

case "${1:-}" in
  cc)    shift; cmd_cc "$@" ;;
  c++)   shift; cmd_cxx "$@" ;;
  make)  shift; cmd_make "$@" ;;
  check) shift; cmd_check "$@" ;;
  run)   shift; cmd_run "$@" ;;
  shell) shift; cmd_shell "$@" ;;
  *) usage ;;
esac
