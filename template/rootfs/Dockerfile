# Tools for taking a Lyra rootfs.img apart and putting it back together.
# UBI is a raw-NAND volume format, so there is no loop mount for it. These
# tools read and write the image instead.
FROM debian:bookworm

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      mtd-utils python3 python3-pip file \
 && rm -rf /var/lib/apt/lists/*

# 0.8.16 is the version whose -k and -x flags this was tested against.
RUN pip3 install --break-system-packages --no-cache-dir ubi_reader==0.8.16

WORKDIR /rootfs
