"""Shared configuration for the freestanding XNU kernel build.

Board, page size and optimization level come from the `[xnu]` buckconfig
section; the kernelcache layout and link flags live here too. Compile
arguments are defined once, in `//xnu:flags.bzl`.
"""

# The kernelcache is an MH_FILESET whose base is VM_KERNEL_LINK_ADDRESS (the
# outer header lives there). `com.apple.kernel` is its first entry, linked at
# KC_KERNEL_OFFSET above it, and every kext gets a slot in a region the kernel
# link reserves for it. All offsets are from LINK_ADDR; the kernel link
# (kernel_ldflags) and tools/buck/kernelcache.py must agree, so the latter
# receives them from kc_layout_args().
LINK_ADDR = "0xfffffff007004000"
# x86_64's __HIB segment and Mach-O image base, from makedefs/MakeInc.def:
# KERNEL_MIN_ADDRESS + KERNEL_BASE_OFFSET and + 0x100000 again.  The low 32
# bits of these addresses are also the physical load addresses start.s uses, so
# the UEFI loader maps __HIB at 0x100000 and __TEXT at 0x200000.
X86_64_HIB_ADDR = "0xffffff8000100000"
X86_64_LINK_ADDR = "0xffffff8000200000"
X86_64_KERNEL_BASE = "0xffffff8000000000"
KC_LAYOUT = {
    # Outer mach header page(s); the kernel's __TEXT follows.
    "kernel_offset": 0x8000,
    # Room after the kernel's __TEXT for the kexts' __TEXT.
    "kext_text": (0x300000, 0x400000),
    "data_const": 0x400000,
    "lastdata_const": 0x600000,
    # Kexts' __DATA_CONST follow the kernel's __LASTDATA_CONST.
    "kext_data_const": (0x604000, 0x644000),
    # Kexts' __TEXT_EXEC precede the kernel's, as in a stock kernelcache.
    "kext_text_exec": (0x644000, 0x800000),
    "text_exec": 0x800000,
    # The outer __PRELINK_INFO fills the gap between the kernel's text group
    # (ending with __LAST) and its data group, so the collection has no holes.
    "kld_data": 0x1340000,
    # Kexts' __DATA, after the kernel's data group. Fixed, because a kext is
    # linked before the kernel's data end is known.
    "kext_data": (0x1700000, 0x1800000),
}

def _kc_addr(offset):
    return "0x%x" % (int(LINK_ADDR, 16) + offset)

def kc_layout_args():
    """kernelcache.py arguments that describe KC_LAYOUT.

    The base is absolute; every other value is an offset from it (a region is
    `start,end`), the same numbers kernel_ldflags() turns into addresses.
    """
    args = ["--kc-base", LINK_ADDR]
    for key, value in KC_LAYOUT.items():
        flag = "--" + key.replace("_", "-")
        if type(value) == "tuple":
            args += [flag, "0x%x,0x%x" % (value[0], value[1])]
        else:
            args += [flag, "0x%x" % value]
    return args

# Each kext links at final addresses, in its own slot of the kext_* regions.
KC_KEXT_SLOTS = 4

def _kext_slot(region, slot):
    start, end = region
    size = (end - start) // KC_KEXT_SLOTS // 0x4000 * 0x4000
    return start + slot * size

def kext_ldflags(slot):
    """Link flags for kext number `slot`: a kext (-kext) whose segments are
    placed in the kernelcache's regions, so kernelcache.py need not move code.

    The segment order is the one the regions have: kexts' __DATA_CONST sits
    below their __TEXT_EXEC, with __DATA far above both."""
    b = BOARDS[board()]
    def seg(name, region):
        return "-Wl,-segaddr," + name + "," + _kc_addr(_kext_slot(KC_LAYOUT[region], slot))
    return [
        "-target", target_triple(),
    ] + target_cpu_flags() + [
        "-nostdlib",
        "-nostdlib++",
        "-Wl,-kext",
        "--ld-path=ld64.mold",
        "-Wl,-no_fixup_chains",
        "-Wl,-segalign,0x4000",
        seg("__TEXT", "kext_text"),
        seg("__DATA_CONST", "kext_data_const"),
        seg("__TEXT_EXEC", "kext_text_exec"),
        seg("__DATA", "kext_data"),
    ]

# Per-board toolchain target, CPU, ICU source and defines.
BOARDS = {
    "qemu-arm64": {
        "triple": "arm64-apple-macos27.0.0",
        "cpu": "cortex-a53",
        "icu": "gic/gic.c",
        "defines": [
            "-DCURRENT_MACHINE_CONFIG_LC=qemu",
            "-DARM64_BOARD_CONFIG_QEMU",
            "-DQEMU",
        ],
    },
    "qemu-arm64e": {
        "triple": "arm64e-apple-macos27.0.0",
        "cpu": "generic",
        "icu": "gic/gic.c",
        "defines": [
            "-DCURRENT_MACHINE_CONFIG_LC=qemu-arm64e",
            "-DARM64_BOARD_CONFIG_QEMU_ARM64E",
            "-DQEMU",
            # arm64e triple + -mcpu=generic does not imply FeaturePAuth in the
            # integrated assembler (codegen gets it via the subtarget patch),
            # so ptrauth instructions (`paciasp`, `retab`, ...) gate on it here.
            "-mcpu=generic+pauth",
        ],
    },
    "qemu-x86_64": {
        "triple": "x86_64-apple-macos27.0.0",
        "arch": "x86_64",
        "march": "x86-64",
        "defines": [
            "-DCURRENT_MACHINE_CONFIG_LC=qemu_x86_64",
            "-DQEMU",
            "-DX86_64_BOARD_CONFIG_QEMU",
        ],
    },
    "superbird": {
        "triple": "arm64-apple-macos27.0.0",
        "cpu": "cortex-a53",
        "icu": "gic/gic.c",
        "defines": [
            "-DCURRENT_MACHINE_CONFIG_LC=superbird",
            "-DARM64_BOARD_CONFIG_SUPERBIRD",
            "-DSUPERBIRD",
        ],
    },
    "ipad41": {
        "triple": "arm64-apple-macos27.0.0",
        "cpu": "cyclone",
        "icu": "aic/aic.c",
        "defines": [
            "-DCURRENT_MACHINE_CONFIG_LC=ipad41",
            "-DARM64_BOARD_CONFIG_IPAD41",
            "-DAPPLECYCLONE",
        ],
    },
}


def board():
    """Selected board, from the `xnu.board` buckconfig key."""
    return read_config("xnu", "board", "qemu")


def arch():
    """Kernel architecture of the selected board: `arm64` or `x86_64`."""
    return BOARDS[board()].get("arch", "arm64")


def is_x86_64():
    return arch() == "x86_64"


def kernel_platform():
    """Buck platform the kernel and its libraries are built for."""
    return "//platforms:x86_64-darwin" if is_x86_64() else "//platforms:aarch64-darwin"


def kernel_link_addr():
    return X86_64_LINK_ADDR if is_x86_64() else LINK_ADDR


def target_cpu_flags():
    """`-march` on x86_64, `-mcpu` on arm64. The clang driver rejects -mcpu
    for x86 targets. x86_64 adds -mcx16: macOS's x86_64 baseline has
    cmpxchg16b, and without it 16-byte atomics become libatomic calls the
    freestanding kernel does not link."""
    b = BOARDS[board()]
    if is_x86_64():
        return ["-march=" + b.get("march", "x86-64"), "-mcx16"]
    return ["-mcpu=" + b["cpu"]]


def kernel_config():
    """Selected MASTER configuration, from the `xnu.kernel_config` key."""
    return read_config("xnu", "kernel_config", "DEVELOPMENT")

def page_size():
    """Kernel page granule, selected with --config xnu.page_size=4k|16k.

    x86_64 always uses 4 KiB pages; the arm64 16 KiB granule does not exist
    there."""
    if is_x86_64():
        return "4k"
    granule = read_config("xnu", "page_size", "4k")
    if granule not in ["4k", "16k"]:
        fail("xnu.page_size must be 4k or 16k, got %s" % granule)
    return granule


def target_triple():
    """Target triple for the selected board."""
    return BOARDS[board()]["triple"]


def opt_level():
    """Optimization level for every C, C++ and Rust compile, from the
    `xnu.opt_level` key (0, 1, 2, 3, s or z; default 3). toolchains/BUCK reads
    the same key, so one setting covers kernel, kexts and host tools."""
    level = read_config("xnu", "opt_level", "3")
    if level not in ["0", "1", "2", "3", "s", "z"]:
        fail("xnu.opt_level must be 0, 1, 2, 3, s or z, got %s" % level)
    return level

# Final Mach-O kernel link arguments, mirroring the tree's makedefs.
def kernel_ldflags():
    b = BOARDS[board()]
    if is_x86_64():
        # The upstream x86_64 layout has __HIB below the image base so the
        # booter can enter at a low physical address and keep the low 32 bits
        # of the link addresses meaningful until long mode is enabled.
        return [
            "-target", target_triple(),
        ] + target_cpu_flags() + [
            "-nostdlib",
            "-nostdlib++",
            "-static",
            "-Wl,-e,__start",
            "-Wl,-pie",
            "-Wl,-static",
            "-Wl,-image_base," + X86_64_LINK_ADDR,
            "-Wl,-segaddr,__HIB," + X86_64_HIB_ADDR,
            "-Wl,-seg_page_size,__TEXT,0x200000",
            "-Wl,-sectalign,__HIB,__bootPT,0x1000",
            "-Wl,-sectalign,__HIB,__desc,0x1000",
            "-Wl,-sectalign,__HIB,__data,0x1000",
            "-Wl,-sectalign,__HIB,__text,0x1000",
            "-Wl,-sectalign,__HIB,__const,0x1000",
            "-Wl,-sectalign,__HIB,__bss,0x1000",
            "-Wl,-sectalign,__HIB,__common,0x1000",
            "-Wl,-sectalign,__HIB,__llvm_prf_cnts,0x1000",
            "-Wl,-sectalign,__HIB,__llvm_prf_names,0x1000",
            "-Wl,-sectalign,__HIB,__llvm_prf_data,0x1000",
            "-Wl,-sectalign,__HIB,__textcoal_nt,0x1000",
            "-Wl,-sectalign,__HIB,__cstring,0x1000",
            "-Wl,-sectalign,__DATA,__percpu,0x1000",
            "-Wl,-rename_section,__DATA,__const,__DATA_CONST,__const",
            "-Wl,-segprot,__DATA_CONST,r--,r--",
            "-Wl,-rename_section,__KLD,__const,__KLDDATA,__const",
            "-Wl,-rename_section,__KLD,__cstring,__KLDDATA,__cstring",
            "-Wl,-segprot,__KLDDATA,rw-,rw-",
            "-Wl,-segprot,__KLD,r-x,r-x",
            "-Wl,-no_zero_fill_sections",
            "-Wl,-headerpad,0x1000",
            # As for arm64: -pie leaves absolute pointers in read-only data
            # (vnodeop tables, __desc) that the link resolves at its fixed
            # image base; these flags keep the link from rejecting them.
            "-Wl,-read_only_relocs,suppress",
            "-Wl,-no_fixup_chains",
            "-Wl,-u,_gIOPL031RTCBuiltinInfo",
            "-Wl,-u,__ZN11IOVirtioBlk10gMetaClassE",
            "-Wl,-u,__ZN9IOStorage10gMetaClassE",
            "-Wl,-u,__ZN20IOBlockStorageDriver10gMetaClassE",
            "-Wl,-u,__ZN7IOMedia10gMetaClassE",
            "-Wl,-u,__ZN16IOMediaBSDClient10gMetaClassE",
            "-Wl,-u,__ZN17IOPartitionScheme10gMetaClassE",
            "-Wl,-u,__ZN22IOFDiskPartitionScheme10gMetaClassE",
            "-Wl,-u,__ZN21IOGUIDPartitionScheme10gMetaClassE",
            "-Wl,-u,__ZN24IOUserBlockStorageDevice10gMetaClassE",
        ]
    return [
    "-target", target_triple(),
    ] + target_cpu_flags() + [
    "-nostdlib",
    "-nostdlib++",
    "-static",
    "-Wl,-e,__start",
    "-Wl,-pie",
    "-Wl,-export_dynamic",
    # The kernel is the kernelcache's first fileset entry: its mach header is
    # at KC_KERNEL_OFFSET above the outer header (see KC_LAYOUT).
    "-Wl,-image_base," + _kc_addr(KC_LAYOUT["kernel_offset"]),
    "-Wl,-segalign," + ("0x4000" if page_size() == "16k" else "0x1000"),
    "-Wl,-pagezero_size,0x0",
    "-Wl,-rename_section,__TEXT,__text,__TEXT_EXEC,__text",
    "-Wl,-rename_section,__DATA,__const,__DATA_CONST,__const",
    "-Wl,-rename_section,__DATA,__mod_init_func,__DATA_CONST,__mod_init_func",
    "-Wl,-rename_section,__DATA,__mod_term_func,__DATA_CONST,__mod_term_func",
    "-Wl,-read_only_relocs,suppress",
    "-Wl,-no_fixup_chains",
    "-Wl,-segprot,__TEXT,r--,r--",
    "-Wl,-segprot,__TEXT_EXEC,r-x,r-x",
    "-Wl,-segprot,__DATA_CONST,r--,r--",
    "-Wl,-segment_order,__TEXT:__DATA_CONST:__LASTDATA_CONST:__TEXT_EXEC:__KLD:__LAST:__KLDDATA:__DATA:__BOOTDATA:__LINKEDIT",
    "-Wl,-headerpad,0x1000",
    # Fixed addresses leave the reserved regions between the kernel's segment
    # groups for the kexts. The segments around them follow without a gap.
    "-Wl,-segaddr,__DATA_CONST," + _kc_addr(KC_LAYOUT["data_const"]),
    "-Wl,-segaddr,__LASTDATA_CONST," + _kc_addr(KC_LAYOUT["lastdata_const"]),
    "-Wl,-segaddr,__TEXT_EXEC," + _kc_addr(KC_LAYOUT["text_exec"]),
    "-Wl,-segaddr,__KLDDATA," + _kc_addr(KC_LAYOUT["kld_data"]),
    "-Wl,-segprot,__LASTDATA_CONST,r--,r--",
    "-Wl,-segprot,__LAST,r-x,r-x",
    "-Wl,-segprot,__KLDDATA,rw-,rw-",
    "-Wl,-u,_gIOPL031RTCBuiltinInfo",
    "-Wl,-u,__ZN10IOPL031RTC10gMetaClassE",
    "-Wl,-u,__ZN11IOVirtioBlk10gMetaClassE",
    "-Wl,-u,__ZN9IOStorage10gMetaClassE",
    "-Wl,-u,__ZN20IOBlockStorageDriver10gMetaClassE",
    "-Wl,-u,__ZN7IOMedia10gMetaClassE",
    "-Wl,-u,__ZN16IOMediaBSDClient10gMetaClassE",
    "-Wl,-u,__ZN17IOPartitionScheme10gMetaClassE",
    "-Wl,-u,__ZN22IOFDiskPartitionScheme10gMetaClassE",
    "-Wl,-u,__ZN21IOGUIDPartitionScheme10gMetaClassE",
    "-Wl,-u,__ZN24IOUserBlockStorageDevice10gMetaClassE",
    # arm64e: XNU's static kernelcache slides and signs its pointers by
    # walking the chains __TEXT,__thread_starts lists
    # (arm_slide_rebase_and_sign_image).
    ] + (["-Wl,-threaded_starts_section"] if b["triple"].startswith("arm64e") else [])
