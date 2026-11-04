# check=skip=FromPlatformFlagConstDisallowed
# The directive above is a parser directive and has to be the first line.
# The constant platform it silences is deliberate, for the reason below.
# Build environment for the Luckfox Lyra SDK.
#
# amd64 is not a preference. The SDK ships prebuilt x86_64 host tools under
# prebuilts/gcc/linux-x86/, and Buildroot checks for a compiler that can
# produce 32-bit binaries, so an arm64 container cannot run this build.
FROM --platform=linux/amd64 ubuntu:22.04

ARG UID=1000
ARG GID=1000
ARG UNAME=builder

ENV DEBIAN_FRONTEND=noninteractive
ENV LC_ALL=C.UTF-8

# Three groups, in order of authority.
#
# 1. What the SDK's own preflight scripts demand: check-sdk.sh wants python3,
#    rsync, gcc, g++ and findmnt from util-linux; check-buildroot.sh wants
#    make >= 4.0 and unbuffer from expect.
# 2. The list Luckfox publishes for this SDK, at
#    wiki.luckfox.com/Luckfox-Lyra/SDK-Image-Compilation, taken as given. A
#    dependency missing here fails a build hours in, so matching the vendor
#    beats pruning it. python-is-python3 is the one entry left out: it owns
#    /usr/bin/python as a dpkg file pointing at Python 3, which contradicts
#    the python2 symlink the same page tells you to make, below.
# 3. The rest of the Buildroot and kernel build set.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      python3 rsync gcc g++ make util-linux expect expect-dev \
      git ssh libssl-dev liblz4-tool patchelf chrpath gawk texinfo diffstat \
      binfmt-support qemu-user-static live-build bison flex fakeroot cmake \
      gcc-multilib g++-multilib unzip device-tree-compiler ncurses-dev \
      libgucharmap-2-90-dev bzip2 expat gpgv2 cpp-aarch64-linux-gnu \
      libgmp-dev libmpc-dev bc python2 curl file bsdmainutils scons \
      cpio gzip lz4 patch perl wget xz-utils zstd ca-certificates sudo \
      libncurses-dev libc6-dev u-boot-tools \
      python3-dev python3-pip libsqlite3-dev bsdextrautils locales \
 && rm -rf /var/lib/apt/lists/*

# Some Buildroot packages still call `python` and mean Python 2. This is what
# the community build recipe does, and it is what has been reported to work.
# If a package instead needs Python 3 under that name, run
# `sudo ln -sf /usr/bin/python3 /usr/bin/python` in the container.
RUN ln -sf /usr/bin/python2 /usr/bin/python

# The SDK refuses to build unless the current user owns the source tree, and
# Buildroot's configure scripts refuse to run as root, so build as a user.
RUN groupadd -g ${GID} -o ${UNAME} \
 && useradd -m -u ${UID} -g ${GID} -o -s /bin/bash ${UNAME} \
 && echo "${UNAME} ALL=(ALL:ALL) NOPASSWD: ALL" > /etc/sudoers.d/${UNAME} \
 && mkdir -p /sdk && chown ${UID}:${GID} /sdk

USER ${UNAME}
WORKDIR /sdk
CMD ["/bin/bash", "--login"]
