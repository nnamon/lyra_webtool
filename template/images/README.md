# images

Drop the partition images here. The quickest source is Luckfox's own zip,
`Luckfox_Lyra_Zero_W_Flash_261020`, which ships every one of them loose:

| File | What it is | Safe to replace |
| --- | --- | --- |
| `MiniLoaderAll.bin` | Loader, also written as the IDBlock | No. Keep the vendor's. |
| `uboot.img` | U-Boot, 4 MiB partition | Yes, with care |
| `boot.img` | Kernel and device tree, 12 MiB partition | Yes |
| `rootfs.img` | Root filesystem, grows to fill the device | Yes, start here |

`package-file` and `parameter.txt` are copied in from the directory above by
`build.sh`, so don't edit the copies here.

This directory is not tracked by git, because these files are large and
vendor-supplied.
