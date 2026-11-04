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

# The first group is what the SDK's own preflight scripts demand:
# check-sdk.sh wants python3, rsync, gcc, g++ and findmnt from util-linux;
# check-buildroot.sh wants make >= 4.0 and unbuffer from expect.
# The rest is the Buildroot and kernel build set.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      python3 rsync gcc g++ make util-linux expect expect-dev \
      bc bison flex bzip2 cpio file gawk git gzip lz4 patch perl \
      unzip wget xz-utils zstd ca-certificates curl ssh sudo \
      libssl-dev libncurses-dev libgmp-dev libmpc-dev libc6-dev \
      gcc-multilib g++-multilib \
      device-tree-compiler u-boot-tools \
      fakeroot cmake texinfo chrpath patchelf diffstat \
      python2 python3-dev python3-pip libsqlite3-dev \
      bsdextrautils locales \
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
