# Cross-compiling for the Lyra Zero W

Build a program on your machine, then install it into `rootfs.img`. The board
has no compiler, no `/usr/include` and no static libc, so nothing can be built
on it.

## The target

Measured from the board's own `busybox`, not assumed:

| Property | Value |
| --- | --- |
| Architecture | ARMv7-A, Cortex-A7 (`Tag_CPU_arch: v7`) |
| Float | hard float, VFPv4, arguments in VFP registers |
| ELF | ELF32, little endian, EABI5, flags `0x5000400` |
| C library | glibc 2.38 |
| Interpreter | `/lib/ld-linux-armhf.so.3` |

`cross.sh` passes `-march=armv7-a -mfpu=neon-vfpv4 -mfloat-abi=hard` for you.

## Build and install

1. Compile in your own source directory:
   ```sh
   /path/to/cross.sh cc -O2 hello.c -o hello
   ```
   `c++` and `make` work the same way; `make` sets `CC`, `CXX`, `AR`, `STRIP`
   and the flags.
2. Confirm it will load on the board:
   ```sh
   /path/to/cross.sh check hello
   ```
3. Run it against the board's real libraries, under emulation:
   ```sh
   /path/to/cross.sh run hello
   ```
   This needs `../ubifs.sh unpack` to have run, because it loads the target's
   own `ld.so` and libc out of the unpacked tree.
4. Install it into the image:
   ```sh
   mkdir -p ../files/usr/bin && cp hello ../files/usr/bin/
   cd .. && ./ubifs.sh apply && ./ubifs.sh pack new-rootfs.img
   ```
   `apply` makes the copied files root-owned without touching anything else.

## The glibc trap

glibc is backward compatible, not forward compatible. A binary that needs
`GLIBC_2.39` loads on the board, which has 2.38, and then dies. Nothing warns
you at build time unless your toolchain's glibc is old enough to lack the
symbol.

That is why the container is bookworm, whose armhf target is glibc 2.36: a
call to anything newer than the board fails at link time here. A newer base
works for most code and then surprises you. Both measured:

| Toolchain base | Trivial program | `pidfd_spawn`, a 2.39 symbol |
| --- | --- | --- |
| bookworm, glibc 2.36 | builds, runs on the board | link error, immediately |
| trixie, glibc 2.41 | builds, runs on the board | builds, `check` reports FAIL |

`check` compares the ELF flags, the interpreter and the highest `GLIBC_`
symbol version against the board's, and exits 1 on any mismatch. Static
linking sidesteps the whole question.

`check` and `run` answer different questions, so run both. `check` reads the
ABI out of the header and says nothing about dependencies. `run` loads the
binary for real against the image's own libraries, which is the only way to
learn that a library it needs is not in the image. A binary linked against
something absent passes `check` and fails `run` with the library named.

## Installing something that is not yours

A prebuilt Debian armhf binary often will not work. Debian's armhf baseline is
ARMv7 hard float too, so the ABI usually matches and `check` passes. What it
will not find is its shared libraries: this image has no package manager to
pull them in. Either build from source with `cross.sh`, link it statically, or
copy every library it needs into the tree too. `cross.sh run` names the first
one it cannot find, so repeat until it starts.

## Verified

- `cc` produces ELF flags `0x5000400` and interpreter
  `/lib/ld-linux-armhf.so.3`, both identical to the board's `busybox`.
- `run` executed a cross-built binary against the unpacked vendor rootfs and
  it reported the runtime glibc as 2.38, so it linked against the board's own
  libc rather than the container's.
- `check` passes a correct binary and a static one, and fails an x86-64 ELF on
  flags and interpreter, and a trixie-built `pidfd_spawn` binary on the glibc
  version.
- `check` passes a binary whose shared library is missing from the image,
  because it only reads the header. `run` on the same binary failed with
  `libnotinimage.so.1: cannot open shared object file`, which is why the two
  commands both exist.
- The full path was run end to end: compile, `files/usr/bin/hello`, `apply`,
  `pack`. The result was a valid 825-PEB UBI image, and the installed binary
  was `-rwxr-xr-x 0:0` in the tree and ran under emulation.
