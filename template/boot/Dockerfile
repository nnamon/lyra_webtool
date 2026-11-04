# Tools for taking boot.img apart and putting it back together.
#
# boot.img is a U-Boot FIT container holding three images: the kernel, the
# device tree, and a Rockchip "resource" blob. dtc edits the device tree,
# mkimage rebuilds the container and recomputes the sha256 hashes it carries.
FROM debian:bookworm
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      device-tree-compiler u-boot-tools python3 xxd file \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /work
