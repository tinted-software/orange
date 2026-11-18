# Buck2 build and QEMU testing

Buck2 is the supported build. Its platform transitions let the host tools, the
bare-metal QEMU loader and the freestanding kernel coexist in one graph:

| Target | Platform constraint | Artifacts |
| --- | --- | --- |
| Host (`x86_64-unknown-linux-gnu`) | `//platforms:host` | `config`, `iig`, `mig`, `migcom`, `mkapfs` |
| Bare metal (`aarch64-unknown-none`) | `//platforms:aarch64-none` | `qemu-boot` |
| Freestanding kernel (`aarch64-apple-darwin`) | `//platforms:aarch64-darwin` | `xnurc`, `mach_kernel`, `kernelcache` |

## Layout

| Path | Role |
| --- | --- |
| `platforms/` | Target platforms, one per toolchain |
| `toolchains/` | C++/Rust toolchains: `mold`, `ld64.mold`, `ld.lld` |
| `xnu/sources.bzl` | Kernel and subproject source lists (hand-maintained) |
| `xnu/flags.bzl` | The one definition of every compile flag (kernel, kexts, subprojects, `genassym`) |
| `xnu/kernel.bzl` | Board table, `[xnu]` config accessors, kernelcache layout and link flags |
| `xnu/codegen.bzl` | MIG/IIG job tables and the outputs `:xnu_codegen` declares |
| `xnu/platform.bzl` | Rules that pin a dependency to one platform (used by `//:qemu`) |
| `third-party/rust/` | Reindeer-vendored crates + `BUCK` |
| `rust/crypto/` | Kernel Rust: crypto primitives and the read-only APFS root reader |
| `security/*_kext/` | The sandbox and AMFI policy kexts packed into the kernelcache |
| `tools/buck/` | Host scripts the rules run (`gen_tree.py`, `kernelcache.py`, ...) |
| `tools/mkapfs/` | Packs a directory into a read-only APFS image |
| `tools/trustcache/` | Swift tool that unwraps, inspects and packs trust caches |
| `tools/qemu-boot/` | The bare-metal loader QEMU runs the kernelcache through |
| `tools/qemu.py` | QEMU launcher |

## Configuration

Build settings are keys of the `[xnu]` buckconfig section. Pass them with
`--config xnu.<key>=<value>`, or put them in `.buckconfig.local` as an `[xnu]`
section so every command picks them up.

| Key | Default | Meaning |
| --- | --- | --- |
| `board` | `qemu` | `qemu-arm64e` for anything that runs Apple userspace (arm64e, pointer authentication); also `qemu`, `superbird`, `ipad41` |
| `page_size` | `4k` | `16k` is required by Apple's arm64 userspace |
| `kernel_config` | `DEVELOPMENT` | Which `config/MASTER*` configuration to build |
| `opt_level` | `3` | `0`, `1`, `2`, `3`, `s` or `z`, for every C, C++ and Rust compile |

The commands below assume this `.buckconfig.local`:

```ini
[xnu]
board = qemu-arm64e
page_size = 16k
```

## Building

```sh
buck2 build //:kernelcache //:qemu-boot //tools/mkapfs:mkapfs
buck2 build :config :iig :mig :migcom      # host tools
buck2 build //:mach_kernel                 # the kernel Mach-O, without kexts
```

`//:kernelcache` is the MH_FILESET collection: `com.apple.kernel` plus the
sandbox and AMFI kexts, laid out by `KC_LAYOUT` in `xnu/kernel.bzl` and packed by
`tools/buck/kernelcache.py`. Print an artifact path with
`buck2 build <target> --show-full-output`.

## Running under QEMU

`//:qemu` runs `tools/qemu.py` on the kernelcache and loader. It builds both,
each for its own platform, so one command builds and boots; arguments after
`--` go to `tools/qemu.py`:

```sh
buck2 run //:qemu -- --rootfs root.apfs --bootargs 'wdt=-1 rd=disk0'
```

| Option | Meaning |
| --- | --- |
| `--rootfs FILE` | APFS image, attached read-only as a virtio-blk disk. Boot it with `rd=disk0` |
| `--bootargs STR` | Boot-args added to the loader's defaults (serial console, `amfi_allow_any_signature=1`, ...) |
| `--trustcache FILE` | `trust_cache_offsets_t` blob loaded for AMFI (see below) |
| `--memory MB` | Guest RAM (default 1024; use 2048 for a full macOS root) |
| `--cpu`, `--smp` | Default `max,pauth=on,pauth-impdef=on`, 2 cores |
| `--gdb`, `--gdb-wait` | GDB server on port 1234, optionally freezing at startup |
| anything else | Passed through to QEMU, e.g. `-no-reboot` |

The console is the terminal: type at the guest's prompt, and leave QEMU with
`Ctrl-A X`. A kernel panic resets the machine, which looks like a boot loop;
pass `-no-reboot` to stop at the first panic. Without `buck2 run`, build the
artifacts and call the launcher directly:

```sh
K=$(buck2 build //:kernelcache --show-full-output | awk '{print $2}')
B=$(buck2 build //:qemu-boot  --show-full-output | awk '{print $2}')
python3 tools/qemu.py --kernel $K --bootloader $B --rootfs root.apfs --bootargs 'rd=disk0'
```

### Without an IPSW

Everything buildable from this tree boots without Apple's binaries as far as
launching PID 1. No source-built userspace runs yet (`//userspace` builds
libSystem pieces, not a `launchd`), so the check is a kernel smoke test: boot
from a root image that has no userspace in it and confirm the kernel reaches the
`exec` of `/sbin/launchd`.

```sh
mkdir -p /tmp/sysroot/sbin /tmp/sysroot/dev
MK=$(buck2 build //tools/mkapfs:mkapfs --show-full-output | awk '{print $2}')
$MK --size 64M --label root --bsd-flags 80000 -o /tmp/root.apfs /tmp/sysroot
buck2 run //:qemu -- --rootfs /tmp/root.apfs --bootargs 'wdt=-1 rd=disk0' -no-reboot
```

It passes when the console shows the root mounting and then the missing launchd:

```
apfsro_mountroot: mounted APFS volume from root device (67108864 bytes)
load_init_program: attempting to load /sbin/launchd
panic(...): Process 1 exec of /sbin/launchd failed, errno 2 @kern_exec.c
```

That covers the loader, the MH_FILESET kernelcache and its kexts, device-tree
bring-up, the virtio-blk driver, the Rust APFS reader and `bsd/miscfs/apfsro`,
and the exec path. `mkapfs` takes `--bsd-flags 80000` (`SF_RESTRICTED`) to mark
every inode protected, as on a sealed system volume. Entry names are stored as
given and hashed after Unicode normalization, so they need not be pre-normalized.

### With an IPSW

An IPSW gives the kernel real userspace: Apple's `launchd`, `dyld`, the dyld
shared cache and the system's trust caches. The tested flow boots the macOS
IPSW (`UniversalMac_27.0.1_26A434_Restore.ipsw`) to a root shell in single-user
mode. It needs `ipsw` (blacktop/ipsw, for the AEA-encrypted DMGs) and about
100 GB of disk (the staged tree is 29 GB, the image up to 48 GB, and `ipsw`
decrypts the DMGs into `TMPDIR`).

1. Mount the system volume and the system cryptex (`ipsw` decrypts them):

   ```sh
   IPSW=~/Downloads/UniversalMac_27.0.1_26A434_Restore.ipsw
   W=~/.cache/xnu/macos
   mkdir -p $W/fsmnt $W/sysmnt ~/.cache/xnu/tmp
   TMPDIR=~/.cache/xnu/tmp ipsw mount fs  $IPSW --mount-point $W/fsmnt  --detach
   TMPDIR=~/.cache/xnu/tmp ipsw mount sys $IPSW --mount-point $W/sysmnt --detach
   ```

2. Stage one root tree from both, and add `/private/etc` from the system's
   templates (the system volume ships none of it, as it lives on the data
   volume):

   ```sh
   S=~/.cache/xnu/macos_stage
   rm -rf $S && mkdir -p $S
   rsync -a --no-acls --no-xattrs $W/fsmnt/root/  $S/
   rsync -a --no-acls --no-xattrs $W/sysmnt/root/ $S/
   chmod -R u+rwX $S
   mkdir -p $S/private/etc $S/private/var/root $S/private/tmp
   cp -a $S/System/Library/Templates/Data/private/etc/. $S/private/etc/
   ```

3. Pack it. The image is read-only and holds the whole tree:

   ```sh
   MK=$(buck2 build //tools/mkapfs:mkapfs --show-full-output | awk '{print $2}')
   $MK --size 48G --label root --bsd-flags 80000 -o ~/.cache/xnu/macos_root.apfs $S
   ```

4. Build the trust cache. AMFI only trusts binaries whose cdhash is in a static
   trust cache, and the IPSW ships one per DMG. The system volume's and the
   cryptex's both matter (the dyld shared cache is in the cryptex's). The DMG
   numbers are specific to the build; list them with
   `unzip -l $IPSW | grep trustcache`:

   ```sh
   (cd tools/trustcache && swift build -c release)
   T=tools/trustcache/.build/release/trustcache
   unzip -p $IPSW Firmware/043-70701-651.dmg.aea.trustcache > $W/sys.im4p   # cryptex
   unzip -p $IPSW Firmware/043-70867-640.dmg.aea.trustcache > $W/fs.im4p    # system volume
   $T unwrap $W/sys.im4p $W/sys.payload
   $T unwrap $W/fs.im4p  $W/fs.payload
   $T wrap $W/macos.blob $W/sys.payload $W/fs.payload
   ```

   `$T inspect` prints a module's entry count, and `$T lookup <payload> <macho>`
   tells you whether a binary (its arm64e slice, via `llvm-lipo -thin arm64e`)
   is trusted.

5. Boot, with `-s` so `launchd` starts a shell instead of the full system:

   ```sh
   buck2 run //:qemu -- --rootfs ~/.cache/xnu/macos_root.apfs --trustcache $W/macos.blob \
       --memory 2048 --bootargs 'wdt=-1 rd=disk0 -s'
   ```

   After about two minutes under QEMU's TCG emulation you get `launchd`'s
   `*** Single-user boot ***` banner and a prompt:

   ```
   localhost:/ root# id
   uid=0(root) gid=0(wheel) groups=0(wheel),1(daemon),...
   ```

   The root is read-only and no data volume is mounted. Leaving the shell
   continues the boot, which stops at the `fsck` boot task.

The iPadOS IPSW (`iPad17,*_27.0.1_24A446`) goes through the same steps (its
shared cache is in the extracted `System/Library/Caches/com.apple.dyld`), with
`unentitled_ios_sim_launch=1` in the boot-args and an `ephemeral-storage`
property in the device tree (set by `tools/qemu-boot`). `launchd` gets through
its early boot tasks, but iOS ships no shell, so there is nothing to log in to
and this flow is not complete.

### What is emulated or stubbed

Running Apple's userspace on QEMU rests on these stand-ins:

* **Sandbox and AMFI** are kexts in the kernelcache (`security/sandbox_kext`,
  `security/amfi_kext`). The sandbox implements only the one call `launchd`'s
  container setup makes (`0x43`). AMFI does trust-cache lookup and platform
  marking, not code-signature policy; the loader's default boot-args include
  `amfi_allow_any_signature=1`.
* **Keys-off processes.** Apple CPUs can turn the A pointer-authentication keys
  off per thread (`/bin/sh`, `zsh`, plugin hosts run that way). QEMU cannot, so
  `locore.s` clears `SCTLR_EL1.EnIA/EnDA` on return to EL0 of such a thread and
  sets them on every entry (`EMULATE_USER_A_KEYS_OFF`, set by a board header).
  Any CPU with PAuth can use it.
* **Hardware identity** is neutral (`QEMU,virt`), `wdt=-1` disables the
  watchdog, and the shared region is enlarged (`SHARED_REGION_SIZE_ARM64`).
* **No NVMe, SEP or data volume:** the root is one read-only virtio-blk APFS
  volume, `bsd/miscfs/apfsro`. `tmpfs` (`bsd/miscfs/tmpfs`) backs the writable
  parts of `/private/var` that boot tasks need.

## Kernel page size

`xnu.page_size=16k` defines `__ARM_16K_PG__` and updates the Mach-O segment
alignment. Apple's arm64 userspace needs it; the 4 KiB default exists for the
bare `qemu` board.

## Source lists and flags

Nothing is generated into the tree. `xnu/sources.bzl` is plain data: when you
add a source file, add it to the list of the subsystem that compiles it (the
`conf/files*` lists say which configuration a file belongs to). A new kernel
option goes in `config/MASTER`, is enabled in the per-platform `MASTER.*`
attribute lists, and gates its files in `conf/files`, as `TMPFS` does.

Compile flags are defined once, in `xnu/flags.bzl`, and every consumer calls
it: the kernel libraries (`kernel_defs.bzl`), the subprojects
(`subproject_defs.bzl`), `mach_kernel`, and the `:xnu_assym` rule that
compiles `genassym.c`. Boards are rows of `BOARDS` in `xnu/kernel.bzl`.

The optimization level is one setting for every C, C++ and Rust compile,
including host tools; `xnu.opt_level` selects it:

```sh
buck2 build --config xnu.opt_level=s //:kernelcache   # 0, 1, 2, 3, s or z
```

## Rust dependencies

Crates are vendored for Buck2 by `reindeer`. Run it from the repository root,
where `reindeer.toml` is (`third_party_dir`, `precise_srcs` and `vendor` are set
there); `third-party/rust/Cargo.toml` is the manifest:

```sh
reindeer buckify
```

`vendor/` is gitignored. `precise_srcs` lists each crate's exact files, and
reindeer cannot always find them, so a crate fails to build with a missing
file or `crate has no srcs`. Add a `third-party/rust/fixups/<crate>/fixups.toml`
with `extra_srcs`:

* edition 2015 crates (e.g. `scopeguard`): reindeer skips precise mode and its
  fallback glob honors `.gitignore`, which ignores `vendor/`;
* files selected by a `cfg` or a `#[path]` (e.g. `polyval`'s soft backends).
