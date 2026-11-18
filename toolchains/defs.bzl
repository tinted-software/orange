load("@prelude//cxx:cxx_toolchain_types.bzl", "LinkerType")
load("@prelude//rust:rust_toolchain.bzl", "PanicRuntime", "RustToolchainInfo")
load("@prelude//toolchains:cxx.bzl", "CxxToolsInfo")

def _custom_cxx_tools_impl(ctx) -> list[Provider]:
    linker = ctx.attrs.linker
    linker_type = LinkerType(ctx.attrs.linker_type)
    compiler_path = "clang"
    cxx_compiler_path = "clang++"
    return [
        DefaultInfo(),
        CxxToolsInfo(
            compiler = compiler_path,
            compiler_type = "clang",
            cxx_compiler = cxx_compiler_path,
            asm_compiler = compiler_path,
            asm_compiler_type = "clang",
            rc_compiler = None,
            cvtres_compiler = None,
            archiver = "ar",
            archiver_type = "darwin" if ctx.attrs.linker_type == "darwin" else "gnu",
            linker = linker,
            linker_type = linker_type,
        ),
    ]

custom_cxx_tools = rule(
    impl = _custom_cxx_tools_impl,
    attrs = {
        "linker": attrs.string(default = "clang++"),
        "linker_type": attrs.string(default = "gnu"),
        "clang": attrs.string(default = ""),
    },
)

def _sysroot_rust_toolchain_impl(ctx) -> list[Provider]:
    # Mirrors prelude//toolchains:rust.bzl's system_rust_toolchain, plus an
    # optional prebuilt sysroot (forwarded to rustc as --sysroot), for
    # targets whose std the installed toolchain doesn't ship (arm64e).
    sysroot = ctx.attrs.sysroot[DefaultInfo].default_outputs[0] if ctx.attrs.sysroot else None
    return [
        DefaultInfo(),
        RustToolchainInfo(
            compiler = RunInfo(args = ["rustc"]),
            clippy_driver = RunInfo(args = ["clippy-driver"]),
            rustdoc = RunInfo(args = ["rustdoc"]),
            default_edition = ctx.attrs.default_edition,
            panic_runtime = PanicRuntime("unwind"),
            nightly_features = True,
            rustc_binary_flags = ctx.attrs.rustc_binary_flags,
            rustc_flags = ctx.attrs.rustc_flags,
            rustc_target_triple = ctx.attrs.rustc_target_triple,
            sysroot_path = sysroot,
        ),
    ]

sysroot_rust_toolchain = rule(
    impl = _sysroot_rust_toolchain_impl,
    attrs = {
        "default_edition": attrs.option(attrs.string(), default = None),
        "rustc_binary_flags": attrs.list(attrs.arg(), default = []),
        "rustc_flags": attrs.list(attrs.arg(), default = []),
        "rustc_target_triple": attrs.string(),
        "sysroot": attrs.option(attrs.dep(providers = [DefaultInfo]), default = None),
    },
    is_toolchain_rule = True,
)
