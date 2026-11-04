#!/usr/bin/env bash
# Take a Lyra rootfs.img apart, change it, and put it back together.
set -euo pipefail
cd "$(dirname "$0")"

TOOLS_IMAGE=lyra-ubi-tools
VOLUME=${LYRA_ROOTFS_VOLUME:-lyra-rootfs}
TREE=/work/tree

# Geometry of the vendor rootfs.img, read out of the Luckfox Lyra SDK rather
# than guessed. Sources, so you can check them:
#   LEB, sub-page        buildroot/configs/rockchip/fs/ubifs.config
#   min I/O, PEB         buildroot/fs/ubifs/Config.in, buildroot/fs/ubi/Config.in
#   max LEB count        1024 MiB / 126976, from BR2_TARGET_ROOTFS_UBIFS_MAX_SIZE
#   compression          BR2_TARGET_ROOTFS_UBIFS_RT_LZO, the choice default
#   volume layout        buildroot/fs/ubi/ubinize.cfg
# Run `./ubifs.sh info <rootfs.img>` on your own image to confirm them.
LEB_SIZE=0x1f000
MIN_IO_SIZE=0x800
MAX_LEB_COUNT=8456
PEB_SIZE=0x20000
SUB_PAGE_SIZE=2048
COMPRESSION=lzo
VOL_NAME=rootfs

die() { echo "error: $*" >&2; exit 1; }

build_tools() {
  docker image inspect "$TOOLS_IMAGE" >/dev/null 2>&1 && return 0
  echo "Building $TOOLS_IMAGE (once)"
  docker build -t "$TOOLS_IMAGE" .
}

# Run a script inside the tools container with the work volume attached.
# $1 is the script; the rest are extra docker arguments placed before the image.
in_tools() {
  local script=$1; shift
  docker run --rm \
    -v "$VOLUME:/work" \
    -e "LEB_SIZE=$LEB_SIZE" -e "MIN_IO_SIZE=$MIN_IO_SIZE" \
    -e "MAX_LEB_COUNT=$MAX_LEB_COUNT" -e "PEB_SIZE=$PEB_SIZE" \
    -e "SUB_PAGE_SIZE=$SUB_PAGE_SIZE" -e "COMPRESSION=$COMPRESSION" \
    -e "VOL_NAME=$VOL_NAME" -e "TREE=$TREE" \
    "$@" "$TOOLS_IMAGE" bash -eu -c "$script"
}

host_file() { # absolute path to an existing file
  local p=$1
  [ -f "$p" ] || die "no such file: $p"
  python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$p"
}

cmd_info() {
  local img; img=$(host_file "${1:-}")
  build_tools
  in_tools 'ubireader_display_info /in/img' \
    -v "$img:/in/img:ro"
}

cmd_unpack() {
  local img; img=$(host_file "${1:-}")
  build_tools
  in_tools '
    rm -rf "$TREE" /work/extract
    mkdir -p /work/extract
    ubireader_extract_files -k -x -o /work/extract /in/img
    src=$(find /work/extract -mindepth 2 -maxdepth 2 -type d | head -1)
    [ -n "$src" ] || { echo "nothing extracted" >&2; exit 1; }
    mv "$src" "$TREE"
    rm -rf /work/extract
    echo "unpacked $(find "$TREE" | wc -l) entries into volume '"$VOLUME"' at tree/"
  ' -v "$img:/in/img:ro"
}

cmd_apply() {
  build_tools
  [ -d files ] || die "no files/ directory here"
  in_tools '
    [ -d "$TREE" ] || { echo "no tree yet; run unpack first" >&2; exit 1; }
    n=$(find /overlay -mindepth 1 ! -type d ! -name .gitkeep | wc -l)
    if [ "$n" -eq 0 ]; then echo "files/ is empty, nothing to apply"; exit 0; fi
    # --no-same-owner makes the copied files root-owned without touching the
    # rest of the tree. A recursive chown here would clear every setuid bit
    # in the image, which is how you get a rootfs where su no longer works.
    tar -C /overlay --exclude=.gitkeep -cf - . \
      | tar -C "$TREE" -xv --no-same-owner -f -
    echo "applied $n files from files/, owned by root"
  ' -v "$PWD/files:/overlay:ro"
}

cmd_shell() {
  build_tools
  docker run --rm -it -v "$VOLUME:/work" -w "$TREE" "$TOOLS_IMAGE" bash
}

cmd_pack() {
  local out=${1:-rootfs.img}
  local dir base
  dir=$(python3 -c 'import os,sys; print(os.path.dirname(os.path.realpath(sys.argv[1])) or ".")' "$out")
  base=$(basename "$out")
  [ -d "$dir" ] || die "no such directory: $dir"
  build_tools
  in_tools '
    [ -d "$TREE" ] || { echo "no tree yet; run unpack first" >&2; exit 1; }
    mkfs.ubifs -d "$TREE" -e "$LEB_SIZE" -c "$MAX_LEB_COUNT" -m "$MIN_IO_SIZE" \
      -x "$COMPRESSION" -F -o /work/rootfs.ubifs
    printf "[ubifs]\nmode=ubi\nvol_id=0\nvol_type=dynamic\nvol_name=%s\nvol_alignment=1\nvol_flags=autoresize\nimage=/work/rootfs.ubifs\n" \
      "$VOL_NAME" > /work/ubinize.cfg
    ubinize -o /work/rootfs.ubi -m "$MIN_IO_SIZE" -p "$PEB_SIZE" -s "$SUB_PAGE_SIZE" /work/ubinize.cfg
    cp /work/rootfs.ubi "/out/'"$base"'"
    ls -l "/out/'"$base"'"
  ' -v "$dir:/out"
  echo "wrote $dir/$base"
}

cmd_roundtrip() {
  local img; img=$(host_file "${1:-}")
  build_tools
  in_tools '
    rm -rf /work/rt; mkdir -p /work/rt/a /work/rt/b
    ubireader_extract_files -k -x -o /work/rt/a /in/img
    A=$(find /work/rt/a -mindepth 2 -maxdepth 2 -type d | head -1)
    mkfs.ubifs -d "$A" -e "$LEB_SIZE" -c "$MAX_LEB_COUNT" -m "$MIN_IO_SIZE" \
      -x "$COMPRESSION" -F -o /work/rt/rootfs.ubifs
    printf "[ubifs]\nmode=ubi\nvol_id=0\nvol_type=dynamic\nvol_name=%s\nvol_alignment=1\nvol_flags=autoresize\nimage=/work/rt/rootfs.ubifs\n" \
      "$VOL_NAME" > /work/rt/ubinize.cfg
    ubinize -o /work/rt/rootfs.ubi -m "$MIN_IO_SIZE" -p "$PEB_SIZE" -s "$SUB_PAGE_SIZE" /work/rt/ubinize.cfg
    ubireader_extract_files -k -x -o /work/rt/b /work/rt/rootfs.ubi
    B=$(find /work/rt/b -mindepth 2 -maxdepth 2 -type d | head -1)

    echo "in  $(stat -c %s /in/img) bytes"
    echo "out $(stat -c %s /work/rt/rootfs.ubi) bytes"

    # Metadata of every entry, and content of the ones that have any. diff -r
    # is no use here: it calls two identical device nodes different, because it
    # will not compare special files at all.
    meta() { ( cd "$1" && find . -print0 | sort -z \
      | xargs -0 stat -c "%n type=%F mode=%A owner=%u:%g size=%s dev=%t:%T links=%h" ); }
    content() { ( cd "$1" \
      && find . -type f -print0 | sort -z | xargs -0 sha256sum | sort -k2 \
      && find . -type l -printf "%p -> %l\\n" | sort ); }

    meta "$A" > /work/rt/a.meta; meta "$B" > /work/rt/b.meta
    content "$A" > /work/rt/a.sha; content "$B" > /work/rt/b.sha

    rc=0
    diff -u /work/rt/a.meta /work/rt/b.meta || rc=1
    diff -u /work/rt/a.sha  /work/rt/b.sha  || rc=1
    if [ $rc -eq 0 ]; then
      n=$(wc -l < /work/rt/a.meta)
      echo "round trip: $n entries identical, including modes, owners,"
      echo "link counts, device numbers, symlink targets and file contents"
    else
      echo "round trip: DIFFERENT, see above" >&2
    fi
    exit $rc
  ' -v "$img:/in/img:ro"
}

cmd_clean() { docker volume rm -f "$VOLUME" >/dev/null && echo "removed volume $VOLUME"; }

usage() {
  cat <<USAGE
Usage: ./ubifs.sh <command> [args]

  info <rootfs.img>        print the UBI and UBIFS geometry of an image
  unpack <rootfs.img>      extract it into the work volume at tree/
  shell                    root shell in the work volume, for editing the tree
  apply                    copy files/ over the tree, owned by root
  pack [out.img]           build a UBI image from the work volume
  roundtrip <rootfs.img>   unpack, repack, re-extract, compare the two trees
  clean                    delete the work volume

The tree lives in a Docker volume, not on this filesystem, because macOS
cannot store device nodes, setuid bits or root ownership. Editing it anywhere
else silently breaks the image.
USAGE
  exit 1
}

case "${1:-}" in
  info)      shift; cmd_info "$@" ;;
  unpack)    shift; cmd_unpack "$@" ;;
  apply)     shift; cmd_apply "$@" ;;
  shell)     shift; cmd_shell "$@" ;;
  pack)      shift; cmd_pack "$@" ;;
  roundtrip) shift; cmd_roundtrip "$@" ;;
  clean)     shift; cmd_clean "$@" ;;
  *) usage ;;
esac
