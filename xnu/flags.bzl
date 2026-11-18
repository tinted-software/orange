"""The one definition of the kernel's compile arguments.

Everything that compiles XNU code for the target -- the kernel libraries
(`kernel_defs.bzl`), the subproject libraries (`subproject_defs.bzl`), the
kexts and `genassym.c` (BUCK) -- takes its flags from here, so a source file
and the `assym` header paired with it are preprocessed identically, and a
change to a define, an include root or the optimization level is made once.

Include order is load-bearing: for example `<string.h>` must resolve to
`gen/export/osfmk/string.h` for osfmk sources, and `gen/` must precede the
source tree so `master_config.h` wins.

Board, page size and optimization level come from the `[xnu]` buckconfig
section (see `kernel.bzl`).
"""

load(
    "//xnu:kernel.bzl",
    "BOARDS",
    "LINK_ADDR",
    "X86_64_LINK_ADDR",
    "board",
    "is_x86_64",
    "opt_level",
    "page_size",
    "target_cpu_flags",
)

# The generated tree (`:xnu_codegen`) and the `assym` header pair (`:xnu_assym`).
GEN = "$(location //:xnu_codegen)"
ASSYM = "$(location //:xnu_assym)"

# Subsystems with their own library flags.
SUBSYSTEMS = ["osfmk", "bsd", "iokit", "libkern", "libsa", "pexpert", "security", "san"]

# MIG output subdirectories whose roots are added to every subsystem.
MIG_SUBDIRS = ["mach", "device", "UserNotification", "gssd", "lockd", "kextd"]

_SUBSYS = {
    "osfmk": [
        "-DMACH_KERNEL_PRIVATE",
        "-DMACH_KERNEL",
        "-Iosfmk/libsa",
    ],
    "bsd": [
        "-DDRIVER_PRIVATE",
        "-D_KERNEL_BUILD",
        "-DKERNEL_BUILD",
        "-DMACH_KERNEL",
        "-DBSD_BUILD",
        "-DBSD_KERNEL_PRIVATE",
    ],
    "iokit": [
        "-DDRIVER_PRIVATE",
        "-DIOKIT_KERNEL_PRIVATE",
        "-DDRIVERKIT_PRIVATE=1",
        "-DIOMATCHDEBUG=1",
        "-DIOALLOCDEBUG=1",
        "-DPRIVATE_WIFI_ONLY=1",
        "-DIOASSERT=1",
    ],
    "libkern": [
        "-DLIBKERN_KERNEL_PRIVATE",
        "-DOSALLOCDEBUG=1",
    ],
    "libsa": ["-DLIBSA_KERNEL_PRIVATE"],
    "pexpert": ["-DPEXPERT_KERNEL_PRIVATE"],
    "security": ["-DBSD_KERNEL_PRIVATE"],
    "san": [],
}

# Code generation, language and diagnostics flags. The optimization level is
# added in `_common()`.
_COMMON_HEAD = [
    "-ffreestanding",
    "-fno-builtin",
    "-fno-common",
    "-fno-stack-protector",
    "-fno-strict-aliasing",
    "-fno-exceptions",
    "-momit-leaf-frame-pointer",
    "-nostdlibinc",
    "-ftrivial-auto-var-init=zero",
    "-fmerge-all-constants",
]

_COMMON_TAIL = [
    "-DLP64",
    "-DPAGE_SIZE_FIXED",
    "-DAPPLE",
    "-DKERNEL",
    "-DKERNEL_PRIVATE",
    "-DXNU_KERNEL_PRIVATE",
    "-DPRIVATE",
    "-DOS_FIREHOSE_SPI=1",
    "-D__MACHO__=1",
    "-DXNU_KERN_EVENT_DATA_IS_VLA",
    "-DXNU_PLATFORM_MacOSX",
    "-DXNU_TARGET_OS_OSX",
    "-D__API__=v4",
    "-Wno-unknown-warning-option",
    "-Wno-unused-command-line-argument",
    "-Wno-nonportable-include-path",
]

_ARM64_TAIL = [
    "-Darm64",
    "-DARM64",
    "-D__ARM64__",
    "-UARM_LARGE_MEMORY",
    "-UARM_LARGE_MEMORY_KERNONLY",
    "-fno-aarch64-jump-table-hardening",
]

# From makedefs/MakeInc.def CFLAGS_X86_64. -mkernel is what gives the kernel
# its -mno-red-zone and kernel code model on Apple's clang.
_X86_64_TAIL = [
    "-Dx86_64",
    "-DX86_64",
    "-D__X86_64__",
    "-DKERNEL_BASE_OFFSET=0x100000",
    "-mkernel",
    "-msoft-float",
    # XNU's C++ kalloc-type validator falls back to a member-walking
    # approximation when Apple's __builtin_xnu_type_summary is absent. On
    # x86_64 it conservatively flags plain structs with trailing padding
    # (e.g. IOPMCalendarStruct) as possibly holding pointers; the kernel's
    # own OSData calls are still audited, just not by this static assert.
    "-DXNU_KT_ALLOW_IMPRECISE_SUMMARY=1",
]

# C++-only arguments.
_CXX_ONLY = [
    "-std=gnu++2c",
    "-fno-rtti",
    "-fno-c++-static-destructors",
    "-fsized-deallocation",
    "-Wno-enum-enum-conversion",
]

# Assembly-only arguments. Buck2 already preprocesses `.s` files, so
# `-x assembler-with-cpp` is not repeated.
_ASM_ONLY = ["-D__ASSEMBLER__", "-DASSEMBLER"]

# IOStorageFamily's headers are published into the codegen tree under this
# root; iokit uses them as <IOKit/storage/...>.
_IOSTORAGE_INC = ["/gen/export/IOKit/storage"]


def _target():
    b = BOARDS[board()]
    return ["-target", b["triple"]] + target_cpu_flags()


def _common(gen):
    """Target-wide arguments: target, codegen, defines, board, page size."""
    b = BOARDS[board()]
    args = _target() + _COMMON_HEAD + ["-O" + opt_level()] + _COMMON_TAIL
    args += _X86_64_TAIL if is_x86_64() else _ARM64_TAIL
    args += ["-include", gen + "/gen/master_config.h"]
    args += ["-DVM_KERNEL_LINK_ADDRESS=" + (X86_64_LINK_ADDR if is_x86_64() else LINK_ADDR)]
    args += b["defines"]
    # The page size is a buckconfig choice. Every TU (C and asm) must agree,
    # or e.g. start.s fills a 4k-sized bootstrap_pagetables with 16k tables.
    if not is_x86_64() and page_size() == "16k":
        args += ["-D__ARM_16K_PG__=1"]
    return args


_ROOTS = ["osfmk", "bsd", "iokit", "libkern", "libsa", "pexpert", "security"]

def _root_flags(gen, subsys):
    """The source-tree roots, with the subsystem's own root first.

    As in makedefs' INCFLAGS (INCFLAGS_GEN), a component's own tree is searched
    before the others: bsd's <machine/endian.h> must find bsd/i386/endian.h,
    not osfmk/i386/endian.h. The generated export/osfmk tree follows osfmk.
    """
    out = []
    for root in [subsys] + [r for r in _ROOTS if r != subsys]:
        out.append("-I" + root)
        if root == "osfmk":
            out.append("-I" + gen + "/gen/export/osfmk")
    return out

def _includes(gen, assym, subsys = "osfmk"):
    """Include roots, interleaved in the order that preserves header shadowing.

    `assym` is the directory holding `assym.h`/`assym.s`, or None for the one
    compile that produces them (`genassym.c`).
    """
    out = [
        "-I.",
        "-I" + gen,
        "-I" + gen + "/gen",
    ] + _root_flags(gen, subsys) + [
        "-I" + gen + "/gen/export",
        "-IEXTERNAL_HEADERS",
        "-I" + gen + "/gen",
    ]
    if assym:
        out.append("-I" + assym)
    out.append("-I" + gen + "/mig")
    out += ["-I" + gen + "/mig/" + s for s in MIG_SUBDIRS]
    out += [
        "-I" + gen + "/iig",
        "-I" + gen + "/gen/export",
        "-I" + gen + "/gen/export/DriverKit",
    ]
    return out


def _mig_ks(gen):
    """Headers of the kernel-server MIG stubs, which osfmk puts first."""
    return ["-I" + gen + "/mig/ks"] + ["-I" + gen + "/mig/ks/" + s for s in MIG_SUBDIRS]


def _meta_features(gen):
    return ["-include", gen + "/gen/meta_features.h"]


def c_only():
    """C-only arguments."""
    return ["-std=gnu17"]


def cxx_only():
    """C++-only arguments."""
    return list(_CXX_ONLY)


def c_flags(subsys, gen = GEN, assym = ASSYM):
    """Flags shared by every language of one kernel subsystem's library."""
    args = _common(gen)
    if subsys == "osfmk":
        args = _mig_ks(gen) + args
    args += _includes(gen, assym, subsys)
    if subsys == "iokit":
        args += ["-I" + gen + p for p in _IOSTORAGE_INC]
    args += _meta_features(gen)
    return args + _SUBSYS[subsys]


def asm_flags(subsys, gen = GEN, assym = ASSYM):
    """Assembly flags for one kernel subsystem."""
    args = _mig_ks(gen) + _common(gen) + _includes(gen, assym, subsys) + _ASM_ONLY
    args += _meta_features(gen)
    return args + _SUBSYS[subsys]


def genassym_source():
    """The genassym translation unit for the selected architecture."""
    return "osfmk/i386/genassym.c" if is_x86_64() else "osfmk/arm64/genassym.c"


def genassym_flags(gen = GEN):
    """Flags for compiling osfmk/arm64/genassym.c, which writes `assym.h`.

    The osfmk set minus the `assym` include root, which does not exist yet.
    """
    return c_flags("osfmk", gen, None)


def subproject_flags(extra_includes = [], extra_defines = [], asm = False, gen = GEN, assym = ASSYM, root = "osfmk"):
    """Flags for a subproject library (IOStorageFamily, msdosfs, ...).

    The same target, codegen and include setup as the kernel, without a
    subsystem's own defines; `extra_*` come from the subproject. `root` is the
    source-tree component the subproject belongs to (msdosfs is a bsd kext), so
    its own headers resolve as they do for that component.
    """
    args = _common(gen) + _meta_features(gen) + list(extra_includes)
    args += _includes(gen, assym, root)
    args += _ASM_ONLY if asm else []
    return args + list(extra_defines)
