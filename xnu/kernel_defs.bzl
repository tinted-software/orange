"""Attribute builders for the freestanding XNU kernel libraries.

Buck2 exposes `cxx_library` only to BUCK files, not to `.bzl` modules, so the
helpers here return an attribute dict that the BUCK file applies:

    cxx_library(**kernel_library_args(subsys = "osfmk", ...))

Flags come from `//xnu:flags.bzl`, the one definition shared with the kexts,
the subprojects and `genassym.c`.

`compiler_flags` applies to every language in a library, so it carries only
the shared kernel flags; `lang_compiler_flags` is *additive* and supplies the
per-language parts (`-std=gnu17` for C, `-std=gnu++2c` and friends for C++).
"""

load("//xnu:flags.bzl", "GEN", "asm_flags", "c_flags", "c_only", "cxx_only")
load("//xnu:kernel.bzl", "is_x86_64", "kernel_platform")
load("//xnu:sources.bzl", "SOURCES")
load("//xnu:sources_x86_64.bzl", "SOURCES_X86_64")

def _sources(key):
    """The source list for the selected architecture."""
    return SOURCES_X86_64[key] if is_x86_64() else SOURCES[key]

def _subsystem_srcs(subsys, srcs_var, extra_vars):
    """One library's source paths, prefixed with the subsystem directory."""
    out = []
    for key in ([srcs_var] if srcs_var else []) + list(extra_vars):
        for path in _sources(key):
            # `*_cross_srcs` are already repo-root relative, because they are
            # compiled by this subsystem but owned by another one.
            out.append(path if key.endswith("_cross_srcs") else subsys + "/" + path)
    return out


def kernel_library_args(
        name,
        subsys,
        srcs_var = None,
        extra_srcs_vars = [],
        codegen_groups = [],
        extra_srcs = [],
        extra_cflags = [],
        asm = False,
        visibility = ["PUBLIC"]):
    """Return the `cxx_library` attributes for one XNU subsystem.

    `srcs_var`/`extra_srcs_vars` name keys in `//xnu:sources.bzl`;
    `codegen_groups` name keys in `CODEGEN`, selecting generated sources out
    of the shared codegen tree.
    """
    srcs = _subsystem_srcs(subsys, srcs_var, extra_srcs_vars) + list(extra_srcs)
    for group in codegen_groups:
        srcs.append(":xnu_codegen[%s]" % group)

    flags = (asm_flags if asm else c_flags)(subsys)

    kwargs = {
        "name": name,
        "srcs": srcs,
        "compiler_flags": flags + list(extra_cflags),
        "link_style": "static",
        "link_whole": True,
        "default_target_platform": kernel_platform(),
        "visibility": visibility,
    }
    if not asm:
        kwargs["lang_compiler_flags"] = {
            "c": c_only(),
            "cxx": c_only() + cxx_only(),
        }
    return kwargs
