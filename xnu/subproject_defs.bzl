
"""Rules for XNU kernel subprojects (iostorage, msdosfs, etc.)."""

load("//xnu:flags.bzl", "c_only", "cxx_only", "subproject_flags")
load("//xnu:kernel.bzl", "kernel_platform")
load("//xnu:sources.bzl", "SUBPROJECT_SOURCES")

def _subproject_c_flags(extra_includes = [], extra_defines = [], root = "osfmk"):
    return subproject_flags(extra_includes, extra_defines, root = root) + c_only()

def _subproject_cxx_flags(extra_includes = [], extra_defines = [], root = "osfmk"):
    return subproject_flags(extra_includes, extra_defines, root = root) + cxx_only()

def _subproject_asm_flags(extra_includes = [], extra_defines = [], root = "osfmk"):
    return subproject_flags(extra_includes, extra_defines, asm = True, root = root)

def subproject_library_args(
        name,
        subproject,
        lang = "c",
        extra_srcs = [],
        extra_includes = [],
        extra_defines = [],
        local = False,
        visibility = ["PUBLIC"],
        default_target_platform = None,
        root = "osfmk"):
    srcs = []
    base_dir = {
        "iostorage": "subprojects/IOStorageFamily",
        "msdosfs": "subprojects/msdosfs",
        "libpthread": "subprojects/libpthread",
        "compiler_rt": "subprojects/llvm-project",
        "libdispatch": "subprojects/libdispatch",
    }[subproject]

    if subproject in SUBPROJECT_SOURCES and lang in SUBPROJECT_SOURCES[subproject]:
        if subproject == "compiler_rt":
            srcs = ["//third-party/llvm:llvm-project[{}]".format(s) for s in SUBPROJECT_SOURCES[subproject][lang]]
        elif local:
            srcs = list(SUBPROJECT_SOURCES[subproject][lang])
        else:
            srcs = [base_dir + "/" + s for s in SUBPROJECT_SOURCES[subproject][lang]]
    srcs += extra_srcs

    if lang == "s":
        flags = _subproject_asm_flags(extra_includes, extra_defines, root)
        kwargs = {}
    elif lang == "cpp":
        flags = _subproject_cxx_flags(extra_includes, extra_defines, root)
        kwargs = {}
    else:
        flags = _subproject_c_flags(extra_includes, extra_defines, root)
        kwargs = {"lang_compiler_flags": {"cxx": _subproject_cxx_flags(extra_includes, extra_defines, root)}}

    ret = {
        "name": name,
        "srcs": srcs,
        "compiler_flags": flags,
        "link_style": "static",
        "link_whole": True,
        "default_target_platform": default_target_platform or kernel_platform(),
        "visibility": visibility,
    }
    ret.update(kwargs)
    return ret
