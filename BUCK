load("//xnu:codegen.bzl", "codegen_outs", "jobs_json")
load("//xnu:flags.bzl", "GEN", "c_flags", "genassym_flags", "genassym_source")
load("//xnu:kernel.bzl", "BOARDS", "board", "is_x86_64", "kernel_config", "kernel_platform", "page_size", "target_triple")
load("//xnu:kernel_defs.bzl", "kernel_library_args")

# The source tree the code generators read: every subsystem's headers and
# config, which MIG/IIG/genassym include from.
_TREE_SRCS = glob([
    "config/MASTER*",
    "config/version.c.template",
    "osfmk/**",
    "bsd/**",
    "iokit/**",
    "libkern/**",
    "libsa/**",
    "pexpert/**",
    "security/**",
    "san/**",
    "EXTERNAL_HEADERS/**",
    "subprojects/IOStorageFamily/*.h",
    "subprojects/IOStorageFamily/*.cpp",
    "subprojects/IOStorageFamily/BlockStorageDeviceDriverKit/*",
], exclude = [
    "**/Makefile",
    "**/*.md",
])

_CODEGEN_OUTS = codegen_outs(glob(["subprojects/IOStorageFamily/*.h"]))

# ---------------------------------------------------------------------------
# Configure-time code generation: MASTER config, MIG stubs, IIG sources,
# syscall tables, version.c. See tools/buck/gen_tree.py; the job tables are in
# //xnu:codegen.bzl.
#
# `outs` declares every produced file so the whole tree materialises intact
# on a cache hit.
# ---------------------------------------------------------------------------
genrule(
    name = "xnu_codegen",
    srcs = _TREE_SRCS + [
        "tools/buck/xnu_config.py",
        "tools/buck/stamp_version.py",
        "tools/buck/run_makesyscalls.sh",
        "tools/buck/gen_tree.py",
    ],
    outs = _CODEGEN_OUTS,
    cmd = "python3 tools/buck/gen_tree.py --outdir $OUT --srcroot . " +
          "--board " + board() + " --kernel-config " + kernel_config() +
          " --page-size " + page_size() + " --triple " + target_triple() +
          " --jobs " + jobs_json() +
          " --mig $(exe //tools/migcom:mig) --iig $(exe //tools/iig:iig)",
    default_target_platform = "//platforms:host",
    visibility = ["PUBLIC"],
)

# `assym.h` and `assym.s`: struct offsets for assembly, from compiling
# osfmk/arm64/genassym.c with the kernel's own osfmk flags (//xnu:flags.bzl)
# so the header and the sources that include it are preprocessed identically.
# genassym only needs struct offsets, which a __ptrauth qualifier does not
# change; the system clang can't parse the 4-arg form (see toolchains/BUCK,
# _NO_AUTH_NULL_SCHEMA), so it takes the 3-arg fallback here.
genrule(
    name = "xnu_assym",
    srcs = _TREE_SRCS + ["tools/buck/extract_assym.py"],
    outs = {"all": ["assym.h", "assym.s"]},
    cmd = "set -e; clang -S -fno-integrated-as -o $TMP/genassym.s " +
          " ".join(genassym_flags()) +
          " -DXNU_PTRAUTH_NO_AUTH_NULL_SCHEMA " + genassym_source() + "; " +
          "python3 tools/buck/extract_assym.py $TMP/genassym.s $OUT/assym.h $OUT/assym.s",
    default_target_platform = "//platforms:host",
    visibility = ["PUBLIC"],
)

export_file(
    name = "syscalls_master",
    src = "bsd/kern/syscalls.master",
    visibility = ["PUBLIC"],
)

filegroup(
    name = "kernel_mig_headers",
    srcs = glob([
        "osfmk/mach/**",
        "osfmk/machine/**",
        "EXTERNAL_HEADERS/**",
    ], exclude = ["**/Makefile"]),
    visibility = ["PUBLIC"],
)

# ---------------------------------------------------------------------------
# Userspace generated headers. The Apple sources expect an SDK that already
# contains the libsyscall MIG user headers (with LIBSYSCALL_INTERFACE), the
# syscall table headers, and a handful of derived availability/feature
# headers. Xcode produced these at build time; this rule reproduces them.
# ---------------------------------------------------------------------------
# Libsyscall_static's .defs inputs from Libsyscall.xcodeproj/project.pbxproj.
# ledger.defs is not an input of that target.
_LIBSYSCALL_MIG = [
    "clock",
    "clock_priv",
    "clock_reply",
    "exc",
    "host_priv",
    "host_security",
    "mach_eventlink",
    "mach_host",
    "mach_port",
    "mach_vm",
    "mach_voucher",
    "memory_entry",
    "notify",
    "processor",
    "processor_set",
    "restartable",
    "task",
    "thread_act",
    "vm_map",
]

_LIBSYSCALL_MIG_INTERNAL = [
    "mach_port",
    "mach_vm",
    "task",
    "thread_act",
    "vm_map",
]

_USERSPACE_AVAILABILITY_HEADERS = [
    "Availability.h",
    "AvailabilityInternal.h",
    "AvailabilityInternalLegacy.h",
    "AvailabilityInternalPrivate.h",
    "AvailabilityMacros.h",
    "AvailabilityProhibitedInternal.h",
    "AvailabilityVersions.h",
    "os/availability.h",
]

_USERSPACE_GENERATED_HEADERS = (
    ["include/mach/" + s + ".h" for s in _LIBSYSCALL_MIG] +
    ["include/servers/netname.h", "include/mach/clock_reply_server.h"] +
    ["include/mach/" + s + "_internal.h" for s in _LIBSYSCALL_MIG_INTERNAL] +
    ["include/" + s for s in _USERSPACE_AVAILABILITY_HEADERS] +
    [
        "include/mach/arm64/asm.h",
        "include/libc-features.h",
        "include/pthread_workqueue.h",
        "include/sys/_posix_availability.h",
        "include/sys/_symbol_aliasing.h",
        "include/sys/syscall.h",
        "include/sys/sysproto.h",
        "include/IOKit/IOKitLib.h",
        "include/IOKit/IOCFPlugIn.h",
    ]
)

# Public/install headers use MACHINE_ARCH=armv7, exactly as mach_install_mig.sh
# requires for arm64 builds. Actual stubs have distinct arm64 build headers:
# their vm_map ABI and _kernelrpc_ entry points must not leak into the SDK view.
_USERSPACE_MIG_BUILD_HEADERS = (
    ["users/" + s + ".h" for s in _LIBSYSCALL_MIG] +
    ["users/netname.h", "users/excServer.h"]
)
_USERSPACE_GENERATED_USERS = (
    ["users/" + s + "User.c" for s in _LIBSYSCALL_MIG] +
    ["users/netnameUser.c", "users/excServer.c"]
)

# make_symbol_aliasing.sh normally obtains release lists from availability.pl
# in an installed SDK. Use the vendored availability release tokens instead,
# retaining its numeric encodings and omission of tertiary iOS releases.
_USERSPACE_SYMBOL_ALIASING_CMD = r"""perl -0777 -ne '
sub release {
    my ($platform, $major, $minor, $patch) = @_;
    return if $platform eq "IPHONE" && defined $patch;
    my $name = "__${platform}_${major}_${minor}";
    $name .= "_$patch" if $patch;
    my $value = $platform eq "MAC" && ($major < 10 || ($major == 10 && $minor < 10))
        ? $major * 100 + $minor * 10
        : $major * 10000 + $minor * 100 + ($patch || 0);
    $versions{$name} = [$platform, $value];
}
while (/__(MAC|IPHONE)_(\d+)_(\d+)(?:_(\d+))?/g) { release($1, $2, $3, $4); }
while (/MAC_OS_X_VERSION_(\d+)_(\d+)(?:_(\d+))?/g) { release("MAC", $1, $2, $3); }
END {
    print "/* Generated using bsd/sys/make_symbol_aliasing.sh release gates. */\n";
    print "#ifndef _CDEFS_H_\n#error Use <sys/cdefs.h> instead.\n#endif\n\n";
    for my $name (sort keys %versions) {
        my ($platform, $value) = @{$versions{$name}};
        my $env = $platform eq "MAC" ? "MAC_OS_X" : "IPHONE_OS";
        my $alias = "__DARWIN_ALIAS_STARTING_${platform}_${name}";
        print "#if defined(__ENVIRONMENT_${env}_VERSION_MIN_REQUIRED__) && __ENVIRONMENT_${env}_VERSION_MIN_REQUIRED__ >= $value\n";
        print "#define $alias(x) x\n#else\n#define $alias(x)\n#endif\n\n";
    }
}
' $(location //userspace/AvailabilityVersions:headers)/AvailabilityInternal.h $(location //userspace/AvailabilityVersions:headers)/AvailabilityMacros.h $(location //userspace/AvailabilityVersions:headers)/AvailabilityVersions.h > $OUT/include/sys/_symbol_aliasing.h
"""

genrule(
    name = "userspace_generated",
    srcs = glob([
        "osfmk/**",
        "bsd/**",
        "EXTERNAL_HEADERS/**",
    ], exclude = ["**/Makefile"]) + [
        "//libsyscall:mig_srcs",
        "//subprojects/libpthread:workqueue_private_h",
        # liblaunch installs this as servers/bootstrap.h; libc's daemon.c and
        # utmpx-darwin.c include it by that path.
        "//userspace:liblaunch_bootstrap",
        "//userspace/IOKitUser:IOKitLibHeader",
        "//userspace/IOKitUser:IOCFPlugInHeader",
        "//userspace/libc:libc_features",
        "//userspace/AvailabilityVersions:headers",
        ":xnu_codegen",
    ],
    outs = {
        "all": _USERSPACE_GENERATED_HEADERS + _USERSPACE_MIG_BUILD_HEADERS + _USERSPACE_GENERATED_USERS,
        "headers": _USERSPACE_GENERATED_HEADERS,
        "build_headers": _USERSPACE_MIG_BUILD_HEADERS,
        "users": _USERSPACE_GENERATED_USERS,
    },
    cmd = "set -e\n" +
          "mkdir -p $OUT/include/mach/arm64 $OUT/include/servers $OUT/include/sys $OUT/include/os $OUT/include/IOKit $OUT/users\n" +
          "cp $(location //userspace:liblaunch_bootstrap) $OUT/include/servers/bootstrap.h\n" +
          "for h in " + " ".join(_USERSPACE_AVAILABILITY_HEADERS) + "; do cp $(location //userspace/AvailabilityVersions:headers)/$h $OUT/include/$h; done\n" +
          "MIG=$(exe //tools/migcom:mig)\n" +
          "BASE=\"-novouchers -I$OUT/include -IEXTERNAL_HEADERS -Iosfmk -Ibsd -Ilibsyscall/mach\"\n" +
          "for s in " + " ".join(_LIBSYSCALL_MIG) + "; do\n" +
          "  $MIG $BASE -target armv7-apple-ios -DLIBSYSCALL_INTERFACE -user /dev/null -header $OUT/include/mach/${s}.h -server /dev/null -sheader /dev/null libsyscall/mach/${s}.defs\n" +
          "  awk -f libsyscall/xcodescripts/filter_mig.awk libsyscall/mach/add_attributes_to_mig.txt $OUT/include/mach/${s}.h > $OUT/include/mach/${s}.tmp.h\n" +
          "  mv $OUT/include/mach/${s}.tmp.h $OUT/include/mach/${s}.h\n" +
          "  server=/dev/null; sheader=/dev/null\n" +
          "  if [ \"$s\" = exc ]; then server=$OUT/users/excServer.c; sheader=$OUT/users/excServer.h; fi\n" +
          "  if [ \"$s\" = clock_reply ]; then sheader=$OUT/include/mach/clock_reply_server.h; fi\n" +
          "  $MIG $BASE -target arm64-apple-macos27.0.0 -user $OUT/users/${s}User.c -header $OUT/users/${s}.h -server $server -sheader $sheader libsyscall/mach/${s}.defs\n" +
          "done\n" +
          "$MIG $BASE -target armv7-apple-ios -user /dev/null -header $OUT/include/servers/netname.h -server /dev/null -sheader /dev/null libsyscall/mach/servers/netname.defs\n" +
          "$MIG $BASE -target arm64-apple-macos27.0.0 -user $OUT/users/netnameUser.c -header $OUT/users/netname.h -server /dev/null -sheader /dev/null libsyscall/mach/servers/netname.defs\n" +
          "for s in " + " ".join(_LIBSYSCALL_MIG_INTERNAL) + "; do\n" +
          "  $MIG $BASE -target armv7-apple-ios -user /dev/null -header $OUT/include/mach/${s}_internal.h -server /dev/null -sheader /dev/null libsyscall/mach/${s}.defs\n" +
          "done\n" +
          "cp osfmk/arm64/asm.h $OUT/include/mach/arm64/asm.h\n" +
          "cp $(location //userspace/libc:libc_features)/arm64/libc-features.h $OUT/include/libc-features.h\n" +
          # Upstream installs pthread/workqueue_private.h as pthread_workqueue.h.
          "cp $(location //subprojects/libpthread:workqueue_private_h) $OUT/include/pthread_workqueue.h\n" +
          "cp $(location :xnu_codegen)/gen/sys/syscall.h $OUT/include/sys/syscall.h\n" +
          "cp $(location :xnu_codegen)/gen/sys/sysproto.h $OUT/include/sys/sysproto.h\n" +
          "cp $(location //userspace/IOKitUser:IOKitLibHeader) $OUT/include/IOKit/IOKitLib.h\n" +
          "cp $(location //userspace/IOKitUser:IOCFPlugInHeader) $OUT/include/IOKit/IOCFPlugIn.h\n" +
          _USERSPACE_SYMBOL_ALIASING_CMD +
          "mkdir -p $OUT/include/skywalk\n" +
          "cp bsd/skywalk/*.h $OUT/include/skywalk/ 2>/dev/null || true\n" +
          "for d in core channel packet nexus mem lib namespace; do cp bsd/skywalk/$d/*.h $OUT/include/skywalk/ 2>/dev/null || true; done\n" +
          "sh bsd/sys/make_posix_availability.sh $OUT/include/sys/_posix_availability.h\n",
    default_target_platform = "//platforms:host",
    visibility = ["PUBLIC"],
)

# XNU installs its bsd/sys and osfmk/machine headers into
# System.framework/PrivateHeaders, which is the layout <System/sys/fsctl.h>
# (libc/gen/sync_volume_np.c), <System/sys/csr.h> and
# <System/machine/cpu_capabilities.h> (libc/libdarwin/variant.c) resolve
# through. libsystem's create_system_framework.sh only links the dylib and the
# Versions skeleton; the header tree comes from these projects.
genrule(
    name = "system_framework_headers",
    srcs = glob(["bsd/sys/**"], exclude = ["**/Makefile"]) + glob(["osfmk/machine/**"]) + ["osfmk/mach/dyld_pager.h", "//libsyscall:thread_self_restrict_h", "//userspace/libc:atexit_header"],
    out = "system_framework_headers",
    cmd = "set -e\nmkdir -p $OUT/System\n" +
          "cp -RL bsd/sys $OUT/System/sys\n" +
          "cp -RL osfmk/machine $OUT/System/machine\n" +
          "mkdir -p $OUT/System/mach\n" +
          "cp osfmk/mach/dyld_pager.h $OUT/System/mach/dyld_pager.h\n" +
          # libc installs its internal atexit.h as System/atexit.h.
          "cp $(location //userspace/libc:atexit_header) $OUT/System/atexit.h\n" +
          # <os/thread_self_restrict.h>: the port TPRO/rwx stand-in (libsyscall/os).
          "mkdir -p $OUT/os\n" +
          "cp $(location //libsyscall:thread_self_restrict_h) $OUT/os/thread_self_restrict.h\n",
    default_target_platform = "//platforms:aarch64-darwin",
    visibility = ["PUBLIC"],
)

alias(
    name = "libsyscall_syscalls",
    actual = "//libsyscall:libsyscall_syscalls",
    visibility = ["PUBLIC"],
)

alias(
    name = "libsyscalls",
    actual = "//libsyscall:libsyscalls",
    visibility = ["PUBLIC"],
)

# ---------------------------------------------------------------------------
# Kernel libraries. All compiled for //platforms:aarch64-darwin (freestanding).
# Source lists come from xnu/sources.bzl, generated off the meson.build lists.
# ---------------------------------------------------------------------------
cxx_library(**kernel_library_args(
    name = "xnu_osfmk",
    subsys = "osfmk",
    srcs_var = "osfmk_c_sources",
    codegen_groups = ["mig_c", "osfmk_gen"],
    # meson adds these to `osfmk_cflags` only; this target alone is affected.
    extra_cflags = [
        "-Wno-implicit-function-declaration",
        "-Wno-int-conversion",
    ],
))

cxx_library(**kernel_library_args(
    name = "xnu_osfmk_vm",
    subsys = "osfmk",
    srcs_var = "osfmk_vm_sources",
    extra_cflags = ["-DVM_UNSAFE_TYPES"],
))

cxx_library(**kernel_library_args(
    name = "xnu_osfmk_asm",
    subsys = "osfmk",
    srcs_var = "osfmk_s_sources",
    asm = True,
))

cxx_library(**kernel_library_args(
    name = "xnu_bsd",
    subsys = "bsd",
    srcs_var = "bsd_c_sources",
    extra_srcs_vars = ["bsd_cpp_sources", "bsd_cross_srcs"],
    codegen_groups = ["bsd_gen"],
))

cxx_library(**kernel_library_args(
    name = "xnu_bsd_vm",
    subsys = "bsd",
    srcs_var = "bsd_vm_sources",
    extra_cflags = ["-DVM_UNSAFE_TYPES"],
))

cxx_library(**kernel_library_args(
    name = "xnu_bsd_mman",
    subsys = "bsd",
    srcs_var = "bsd_mman_sources",
    extra_cflags = ["-DVM_UNSAFE_TYPES"],
))

cxx_library(**kernel_library_args(
    name = "xnu_bsd_asm",
    subsys = "bsd",
    srcs_var = "bsd_s_sources",
    asm = True,
))

cxx_library(**kernel_library_args(
    name = "xnu_libkern",
    subsys = "libkern",
    srcs_var = "libkern_c_sources",
    extra_srcs_vars = ["libkern_cpp_sources"],
))

cxx_library(**kernel_library_args(
    name = "xnu_iokit",
    subsys = "iokit",
    srcs_var = "iokit_c_sources",
    extra_srcs_vars = ["iokit_cpp_sources"],
))

cxx_library(**kernel_library_args(
    name = "xnu_iokit_asm",
    subsys = "iokit",
    srcs_var = "iokit_s_sources",
    asm = True,
))

cxx_library(**kernel_library_args(
    name = "xnu_iokit_driverkit",
    subsys = "iokit",
    codegen_groups = ["iig_driverkit_cpp"],
    extra_cflags = [
        "-DDRIVER_PRIVATE",
        "-DIOKIT_KERNEL_PRIVATE",
        "-DDRIVERKIT_PRIVATE=1",
    ],
))

cxx_library(**kernel_library_args(
    name = "xnu_libsa",
    subsys = "libsa",
    srcs_var = "libsa_cpp_sources",
))

cxx_library(**kernel_library_args(
    name = "xnu_pexpert",
    subsys = "pexpert",
    srcs_var = "pexpert_c_sources",
    extra_srcs = (["pexpert/" + BOARDS[board()]["icu"]]
                  if BOARDS[board()].get("icu") else []),
))

cxx_library(**kernel_library_args(
    name = "xnu_security",
    subsys = "security",
    srcs_var = "security_c_sources",
))

cxx_library(**kernel_library_args(
    name = "xnu_san",
    subsys = "san",
))

cxx_library(**kernel_library_args(
    name = "xnu_lastkernelconstructor",
    subsys = "libsa",
    extra_srcs = ["libsa/lastkernelconstructor.c"],
))

# ---------------------------------------------------------------------------
# Kernel Subprojects (subprojects/*)
# ---------------------------------------------------------------------------
load("//xnu:subproject_defs.bzl", "subproject_library_args")

cxx_library(**subproject_library_args(
    name = "xnu_iostorage",
    subproject = "iostorage",
    lang = "cpp",
    extra_srcs = [":xnu_codegen[iig_iostorage_cpp]"],
    extra_includes = [
        "-Isubprojects/IOStorageFamily",
        "-I$(location :xnu_codegen)/gen/export/IOKit/storage",
        "-I$(location :xnu_codegen)/gen/export/DriverKit",
    ],
    extra_defines = [
        "-DDRIVER_PRIVATE",
        "-DIOKIT_KERNEL_PRIVATE",
        "-DDRIVERKIT_PRIVATE=1",
        "-DIOMATCHDEBUG=1",
        "-DIOALLOCDEBUG=1",
        "-DPRIVATE_WIFI_ONLY=1",
        "-DIOASSERT=1",
    ],
))

alias(
    name = "libsystem_kernel",
    actual = "//libsyscall:libsystem_kernel",
    visibility = ["PUBLIC"],
)

alias(
    name = "libsystem_kernel_glue",
    actual = "//libsyscall:libsystem_kernel_glue",
    visibility = ["PUBLIC"],
)

alias(
    name = "libsystem_kernel_static",
    actual = "//libsyscall:libsystem_kernel_static",
    visibility = ["PUBLIC"],
)

alias(
    name = "libsystem_pthread",
    actual = "//subprojects/libpthread:libsystem_pthread",
    visibility = ["PUBLIC"],
)

alias(
    name = "libsystem_pthread_dyld",
    actual = "//subprojects/libpthread:libsystem_pthread_dyld",
    visibility = ["PUBLIC"],
)

alias(
    name = "libsystem_pthread_objects",
    actual = "//subprojects/libpthread:libsystem_pthread_objects",
    visibility = ["PUBLIC"],
)

# ---------------------------------------------------------------------------
# Final Kernel Link (mach_kernel) & Kernelcache
# ---------------------------------------------------------------------------
load("//xnu:kernel.bzl", "kc_layout_args", "kernel_ldflags", "kext_ldflags")

KERNEL_LIBS = [
    ":xnu_osfmk",
    ":xnu_osfmk_vm",
    ":xnu_osfmk_asm",
    ":xnu_bsd",
    ":xnu_bsd_vm",
    ":xnu_bsd_mman",
    ":xnu_bsd_asm",
    ":xnu_libkern",
    ":xnu_iokit",
    ":xnu_iokit_asm",
    ":xnu_iokit_driverkit",
    ":xnu_libsa",
    ":xnu_pexpert",
    ":xnu_security",
    ":xnu_san",
    ":xnu_iostorage",
    "//subprojects/msdosfs:xnu_msdosfs",
    "//subprojects/libpthread:xnu_libpthread",
    "//third-party/llvm:xnu_compiler_rt",
    "//subprojects/libdispatch:xnu_libdispatch",
    ":xnu_lastkernelconstructor",
]

cxx_binary(
    name = "mach_kernel",
    srcs = ["libsa/lastkerneldataconst.c"],
    compiler_flags = c_flags("libsa"),
    linker_flags = kernel_ldflags(),
    link_style = "static",
    deps = KERNEL_LIBS + ["//rust:xnu-rs"],
    default_target_platform = kernel_platform(),
    visibility = ["PUBLIC"],
)

# Kexts of the kernelcache. Each is compiled with the kernel's flags for its
# subsystem and linked, by mold, at its slot of the regions the kernel reserves
# (kext_ldflags); tools/buck/kernelcache.py links it into the collection.
cxx_library(**kernel_library_args(
    name = "sandbox_kext_objects",
    subsys = "security",
    extra_srcs = ["security/sandbox_kext/sandbox_kext.c"],
))

cxx_binary(
    name = "sandbox_kext",
    deps = [":sandbox_kext_objects"],
    linker_flags = kext_ldflags(0),
    link_style = "static",
    default_target_platform = kernel_platform(),
    visibility = ["PUBLIC"],
)

# AppleMobileFileIntegrity: static trust cache and platform marking (slot 1).
cxx_library(**kernel_library_args(
    name = "amfi_kext_objects",
    subsys = "security",
    extra_srcs = ["security/amfi_kext/amfi_kext.c"],
))

cxx_binary(
    name = "amfi_kext",
    deps = [":amfi_kext_objects"],
    linker_flags = kext_ldflags(1),
    link_style = "static",
    default_target_platform = kernel_platform(),
    visibility = ["PUBLIC"],
)

# x86_64 boots a plain MH_EXECUTE kernel: the loader hands it no kexts, and
# libsa/bootstrap.cpp's non-prelinked path is the upstream booter one. The
# target keeps the same name so //:qemu and scripts do not branch. Buck2's BUCK
# dialect has no top-level `if`, so the file lists are chosen in variables.
_KERNELCACHE_SRCS = ([":mach_kernel"] if is_x86_64() else [
    ":mach_kernel",
    ":sandbox_kext",
    ":amfi_kext",
    "tools/buck/kernelcache.py",
])
_KERNELCACHE_CMD = (
    "cp $(location :mach_kernel) $OUT" if is_x86_64() else
    "python3 tools/buck/kernelcache.py $(location :mach_kernel) -o $OUT " +
    "--kext com.apple.security.sandbox=$(location :sandbox_kext) " +
    "--kext com.apple.driver.AppleMobileFileIntegrity=$(location :amfi_kext) " +
    " ".join(kc_layout_args())
)

genrule(
    name = "kernelcache",
    srcs = _KERNELCACHE_SRCS,
    out = "kernelcache",
    cmd = _KERNELCACHE_CMD,
    default_target_platform = kernel_platform(),
    visibility = ["PUBLIC"],
)

# ---------------------------------------------------------------------------
# Top-level Convenience Aliases
# ---------------------------------------------------------------------------
alias(
    name = "config",
    actual = "//SETUP/config:config",
    default_target_platform = "//platforms:host",
    visibility = ["PUBLIC"],
)

alias(
    name = "migcom",
    actual = "//tools/migcom:migcom",
    default_target_platform = "//platforms:host",
    visibility = ["PUBLIC"],
)

alias(
    name = "mig",
    actual = "//tools/migcom:mig",
    default_target_platform = "//platforms:host",
    visibility = ["PUBLIC"],
)

alias(
    name = "iig",
    actual = "//tools/iig:iig",
    default_target_platform = "//platforms:host",
    visibility = ["PUBLIC"],
)

alias(
    name = "qemu-boot",
    actual = "//tools/qemu-boot:qemu-boot.elf",
    default_target_platform = "//platforms:aarch64-none",
    visibility = ["PUBLIC"],
)

alias(
    name = "rust",
    actual = "//rust:xnu-rs",
    default_target_platform = "//platforms:aarch64-darwin",
    visibility = ["PUBLIC"],
)

# ---------------------------------------------------------------------------
# QEMU runner: `buck2 run //:qemu -- --rootfs root.apfs --bootargs rd=disk0`
#
# Runs tools/qemu.py (a host tool) on the kernelcache and the loader, which are
# built for other platforms; the wrappers below transition them there, so one
# `buck2 run` builds everything it runs. Further arguments go to tools/qemu.py.
# ---------------------------------------------------------------------------
load("//xnu:platform.bzl", "bare_metal_artifact", "darwin_artifact")

darwin_artifact(
    name = "kernelcache_for_qemu",
    actual = ":kernelcache",
)

bare_metal_artifact(
    name = "qemu_boot_for_qemu",
    actual = ":qemu-boot",
)

python_bootstrap_binary(
    name = "qemu_py",
    main = "tools/qemu.py",
    default_target_platform = "//platforms:host",
)

command_alias(
    name = "qemu",
    exe = ":qemu_py",
    args = [
        "--kernel",
        "$(location :kernelcache_for_qemu)",
        "--bootloader",
        "$(location :qemu_boot_for_qemu)",
    ],
    default_target_platform = "//platforms:host",
    visibility = ["PUBLIC"],
)

alias(
    name = "libcxx_default_assertion_handler",
    actual = "//third-party/llvm:libcxx_default_assertion_handler",
    visibility = ["PUBLIC"],
)

alias(
    name = "libcxx_functional_src",
    actual = "//third-party/llvm:libcxx_functional_src",
    visibility = ["PUBLIC"],
)

alias(
    name = "libdispatch_objects",
    actual = "//subprojects/libdispatch:libdispatch_objects",
    visibility = ["PUBLIC"],
)
