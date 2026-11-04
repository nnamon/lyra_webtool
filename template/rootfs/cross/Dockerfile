# Cross toolchain for the Luckfox Lyra Zero W userspace.
#
# bookworm is chosen for its glibc, so that mistakes fail here and not on the
# board. The board runs glibc 2.38. Bookworm's armhf cross target is 2.36, so
# nothing it links can need a symbol the board lacks: a call to a post-2.38
# function fails at link time, on this machine, in a second.
#
# A newer base is not wrong, it is just quieter. Trixie's 2.41 compiles the
# same trivial program fine, but it will also link pidfd_spawn, which is
# GLIBC_2.39, and that binary loads on the board only to die. Measured both
# ways. `./cross.sh check` catches it either way, but only if you run it.
FROM debian:bookworm

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      crossbuild-essential-armhf pkg-config make cmake file \
      qemu-user-static binutils-arm-linux-gnueabihf \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
