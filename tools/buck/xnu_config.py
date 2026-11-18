#!/usr/bin/env python3
"""Select XNU sources from MASTER + conf/files the same way doconf+config do."""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import sys
from pathlib import Path
from typing import Iterable

COMPONENTS = (
    "osfmk",
    "bsd",
    "iokit",
    "libkern",
    "libsa",
    "pexpert",
    "security",
    "san",
)

SKIP_TOKENS = {
    "device-driver",
    "profiling-routine",
    "xnu-library",
    "bound-checks",
    "bound-checks-pending",
    "bound-checks-soft",
    "bound-checks-debug",
    "bound-checks-seed",
    "bound-checks-new-checks",
}


def simple_unifdef(text: str, macros: dict[str, str | None]) -> str:
    """Evaluate #if / #ifdef / #ifndef / #elif / #else / #endif using defined macros."""

    def defined(name: str) -> bool:
        return name in macros and macros[name] is not None

    def value(name: str) -> int:
        if not defined(name):
            return 0
        v = macros[name]
        if v is None or v == "":
            return 1
        try:
            return int(v, 0)
        except ValueError:
            return 1

    def eval_expr(expr: str) -> bool:
        expr = expr.strip()
        expr = re.sub(
            r"defined\s*\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)",
            lambda m: "1" if defined(m.group(1)) else "0",
            expr,
        )
        expr = re.sub(
            r"defined\s+([A-Za-z_][A-Za-z0-9_]*)",
            lambda m: "1" if defined(m.group(1)) else "0",
            expr,
        )

        def ident(m: re.Match) -> str:
            name = m.group(0)
            if name in {"and", "or", "not"}:
                return name
            return str(value(name))

        expr = re.sub(r"[A-Za-z_][A-Za-z0-9_]*", ident, expr)
        expr = expr.replace("&&", " and ").replace("||", " or ").replace("!", " not ")
        try:
            return bool(eval(expr, {"__builtins__": {}}, {}))
        except Exception:
            return False

    out: list[str] = []
    # stack of (parent_active, this_branch_taken, seen_true)
    stack: list[tuple[bool, bool, bool]] = []
    active = True

    for line in text.splitlines(keepends=True):
        raw = line.lstrip()
        if raw.startswith("#if "):
            parent = active
            taken = parent and eval_expr(raw[4:])
            stack.append((parent, taken, taken))
            active = taken
            continue
        if raw.startswith("#ifdef "):
            parent = active
            taken = parent and defined(raw[7:].split()[0])
            stack.append((parent, taken, taken))
            active = taken
            continue
        if raw.startswith("#ifndef "):
            parent = active
            taken = parent and not defined(raw[8:].split()[0])
            stack.append((parent, taken, taken))
            active = taken
            continue
        if raw.startswith("#elif "):
            parent, _this, seen = stack[-1]
            taken = parent and (not seen) and eval_expr(raw[6:])
            stack[-1] = (parent, taken, seen or taken)
            active = taken
            continue
        if raw.startswith("#else"):
            parent, _this, seen = stack[-1]
            taken = parent and not seen
            stack[-1] = (parent, taken, True)
            active = taken
            continue
        if raw.startswith("#endif"):
            stack.pop()
            active = stack[-1][1] if stack else True
            continue
        if active:
            out.append(line)
    return "".join(out)


def parse_master(
    srcroot: Path,
    kernel_config: str,
    cpu: str,
    soc: str,
    platform: str,
    macros: dict[str, str | None],
) -> tuple[set[str], list[str]]:
    master = (srcroot / "config" / "MASTER").read_text()
    candidates = [
        srcroot / "config" / f"MASTER.{cpu}.{soc}.{platform}",
        srcroot / "config" / f"MASTER.{cpu}.{soc}",
        srcroot / "config" / f"MASTER.{cpu}.{platform}",
        srcroot / "config" / f"MASTER.{cpu}",
    ]
    master_cpu_path = next(p for p in candidates if p.is_file())
    master_cpu = master_cpu_path.read_text()

    combined = master + "\n" + master_cpu + "\n"
    text = simple_unifdef(combined, macros)

    mappings: dict[str, list[str]] = {}
    assign_re = re.compile(r"^#\s*([A-Za-z0-9_]+)\s*=\s*\[(.*)\]")
    for line in text.splitlines():
        m = assign_re.match(line)
        if m:
            mappings[m.group(1)] = m.group(2).split()

    selected: set[str] = set()

    def expand(name: str, want: bool) -> None:
        if name in mappings:
            for child in mappings[name]:
                expand(child, want)
        else:
            if want:
                selected.add(name.lower())
            else:
                selected.discard(name.lower())

    expand(kernel_config.upper(), True)

    selected_lines: list[str] = []
    attr_re = re.compile(r"^(.*?)#.*<\s*([^>]+)\s*>\s*$")
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        m = attr_re.match(stripped)
        if m:
            body = m.group(1).rstrip()
            attrs = [a.strip() for a in m.group(2).split(",")]
            negated = False
            if attrs and attrs[0].startswith("!"):
                negated = True
                attrs[0] = attrs[0][1:]
            hit = any(a.lower() in selected for a in attrs if a)
            if (not negated and hit) or (negated and not hit):
                selected_lines.append(body)
        else:
            # no attribute list: always selected
            body = stripped.split("#", 1)[0].rstrip()
            if body:
                selected_lines.append(body)

    return selected, selected_lines


_opt_re = re.compile(
    r"^options\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:=\s*(\"(?:\\.|[^\"])*\"|\S+))?\s*$"
)


def parse_options_lines(selected_lines: list[str]) -> dict[str, str | None]:
    opts: dict[str, str | None] = {}
    for line in selected_lines:
        # Keep quoted values intact (e.g. CONFIG_NMBCLUSTERS="((1024 * 512) / MCLBYTES)").
        m = _opt_re.match(line.strip())
        if m:
            name, raw = m.group(1), m.group(2)
            if raw is None:
                opts[name] = None
            elif len(raw) >= 2 and raw[0] == '"' and raw[-1] == '"':
                opts[name] = raw[1:-1]
            else:
                opts[name] = raw
            continue
        parts = shlex.split(line, posix=True)
        if len(parts) >= 2 and parts[0] == "options":
            token = parts[1]
            if "=" in token:
                k, v = token.split("=", 1)
                opts[k] = v
            elif len(parts) >= 3 and parts[2].startswith("="):
                opts[token] = parts[2][1:] or (parts[3] if len(parts) > 3 else "")
            else:
                opts[token] = None
    return opts


def tokenize_files_line(line: str) -> list[str]:
    return line.split()


def option_enabled(
    token: str,
    attributes: set[str],
    cpp_opts: dict[str, str | None],
    devices: dict[str, int] | None = None,
) -> bool:
    low = token.lower()
    if low in attributes:
        return True
    if devices is not None and low in devices:
        return True
    up = token.upper()
    if up in cpp_opts:
        return True
    # files lists use config_dtrace; MASTER uses CONFIG_DTRACE via options
    if up.replace("-", "_") in cpp_opts:
        return True
    return False


# Mirrors SETUP/config/parser.y's PSEUDO_DEVICE grammar:
#   pseudo-device name [count] [init funcname]
_pseudo_re = re.compile(
    r"^pseudo-device\s+([A-Za-z_][A-Za-z0-9_]*)"
    r"(?:\s+(\d+))?"
    r"(?:\s+init\s+([A-Za-z_][A-Za-z0-9_]*))?"
)


def parse_pseudo_devices(selected_lines: list[str]) -> dict[str, int]:
    """Counts for selected MASTER pseudo-devices (config dtab / mkheaders)."""
    counts: dict[str, int] = {}
    for line in selected_lines:
        m = _pseudo_re.match(line.strip())
        if not m:
            continue
        name = m.group(1)
        counts[name] = int(m.group(2)) if m.group(2) else 1
    return counts


def parse_pseudo_device_inits(selected_lines: list[str]) -> list[tuple[str, int]]:
    """(init_func, count) pairs for selected pseudo-devices that declare an
    `init` routine, in MASTER order -- this is what mkioconf.c's
    pseudo_inits() emits into ioconf.c's `pseudo_inits[]` array, consumed by
    bsd_autoconf() at boot to run early pseudo-device setup (dtrace, sdt,
    fbt, systrace, lockstat, ...)."""
    inits: list[tuple[str, int]] = []
    for line in selected_lines:
        m = _pseudo_re.match(line.strip())
        if not m or not m.group(3):
            continue
        count = int(m.group(2)) if m.group(2) else 1
        inits.append((m.group(3), count))
    return inits


def parse_files_list(
    path: Path,
    attributes: set[str],
    cpp_opts: dict[str, str | None],
    devices: dict[str, int] | None = None,
) -> tuple[list[str], list[str], list[str]]:
    """Return (source_paths, option_header_stems, device_needs)."""
    if not path.is_file():
        return [], [], []
    text = simple_unifdef(
        path.read_text(),
        {**{k: (v if v is not None else "1") for k, v in cpp_opts.items()}},
    )
    sources: dict[str, bool] = {}
    option_headers: list[str] = []
    device_needs: list[str] = []

    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        toks = tokenize_files_line(line)
        if len(toks) < 2:
            continue
        fname, kind = toks[0], toks[1]
        rest = toks[2:]
        rest = [t for t in rest if t != "|" and not t.startswith("|")]
        if kind not in ("standard", "optional"):
            continue

        is_options = fname.startswith("OPTIONS/")
        reqs: list[tuple[str, bool]] = []
        if kind == "standard":
            ok = True
        else:
            invert = False
            for t in rest:
                if t == "not":
                    invert = True
                    continue
                if t in SKIP_TOKENS:
                    invert = False
                    continue
                reqs.append((t, invert))
                invert = False
            ok = True
            for token, inv in reqs:
                present = option_enabled(token, attributes, cpp_opts, devices)
                if inv:
                    present = not present
                if not present:
                    ok = False
                    break
            if not reqs:
                ok = False

        if is_options:
            stem = fname.split("/", 1)[1]
            if stem not in option_headers:
                option_headers.append(stem)
            continue

        # mkheaders: every files-list entry with f_needs generates {need}.h
        if kind == "optional" and reqs and not reqs[0][1]:
            need = reqs[0][0]
            if need not in device_needs:
                device_needs.append(need)

        if ok:
            sources[fname] = True
        elif fname not in sources:
            sources[fname] = False

    enabled = [f for f, on in sources.items() if on]
    return enabled, option_headers, device_needs


def classify(path: str) -> str:
    low = path.lower()
    if low.endswith((".cpp", ".cc", ".cxx", ".cpo")):
        return "cpp"
    if low.endswith((".s", ".S")):
        return "s"
    if low.endswith(".c"):
        return "c"
    return "other"


def resolve_src(srcroot: Path, component: str, fname: str) -> tuple[str, bool]:
    """Return (repo-relative path or generated marker, exists_in_tree)."""
    if fname.startswith("./"):
        rel = f"{component}/{fname[2:]}"
    elif fname.startswith(component + "/") or any(
        fname.startswith(p)
        for p in (
            "osfmk/",
            "bsd/",
            "iokit/",
            "libkern/",
            "libsa/",
            "pexpert/",
            "security/",
            "san/",
        )
    ):
        rel = fname
    else:
        rel = f"{component}/{fname}"
    exists = (srcroot / rel).is_file()
    return rel, exists


def _arch_for_board(board: str) -> str:
    return "x86_64" if board == "qemu-x86_64" else "arm64"


def collect(srcroot: Path, kernel_config: str, board: str = "qemu-arm64") -> dict:
    arch = _arch_for_board(board)
    if arch == "x86_64":
        cpu = "x86_64"
        soc = ""
        platform = "MacOSX"
        macros: dict[str, str | None] = {
            "PLATFORM_MacOSX": "1",
            "CPU_x86_64": "1",
            f"SYS_{kernel_config.upper()}": "1",
            "MASTER_CONFIG_OSS_HARDWARE": "1",
        }
    else:
        cpu = "arm64"
        soc = "superbird"
        platform = "MacOSX"
        macros = {
            "PLATFORM_MacOSX": "1",
            "CPU_arm64": "1",
            "SOC_CONFIG_superbird": "1",
            f"SYS_{kernel_config.upper()}": "1",
            "MASTER_CONFIG_OSS_HARDWARE": "1",
            "MASTER_CONFIG_ENABLE_EXCLAVES": None,
            "MASTER_CONFIG_ENABLE_SPTM": None,
            "MASTER_CONFIG_ENABLE_KERNEL_TAG": None,
            "SOC_IS_VIRTUALIZED": None,
        }
    attributes, selected_lines = parse_master(
        srcroot, kernel_config, cpu, soc, platform, macros
    )
    cpp_opts = parse_options_lines(selected_lines)
    devices = parse_pseudo_devices(selected_lines)
    device_inits = parse_pseudo_device_inits(selected_lines)

    components: dict = {}
    all_option_headers: list[str] = []
    all_device_needs: list[str] = []
    for comp in COMPONENTS:
        conf = srcroot / comp / "conf"
        files, hdrs, needs = parse_files_list(
            conf / "files", attributes, cpp_opts, devices
        )
        files_arch, hdrs_arch, needs_arch = parse_files_list(
            conf / f"files.{cpu}", attributes, cpp_opts, devices
        )
        all_option_headers.extend(hdrs)
        all_option_headers.extend(hdrs_arch)
        all_device_needs.extend(needs)
        all_device_needs.extend(needs_arch)
        buckets = {"c": [], "cpp": [], "s": [], "generated": [], "other": []}
        for fname in files + files_arch:
            rel, exists = resolve_src(srcroot, comp, fname)
            lang = classify(rel)
            if not exists:
                buckets["generated"].append(rel)
            elif lang in buckets:
                buckets[lang].append(rel)
            else:
                buckets["other"].append(rel)
        components[comp] = buckets

    # unique option headers, preserve order
    seen = set()
    opt_hdrs = []
    for h in all_option_headers:
        if h not in seen:
            seen.add(h)
            opt_hdrs.append(h)

    return {
        "kernel_config": kernel_config.lower(),
        "board": board,
        "machine": soc if soc else cpu,
        "arch": arch,
        "attributes": sorted(attributes),
        "cpp_options": {
            k: (v if v is not None else "") for k, v in sorted(cpp_opts.items())
        },
        "option_headers": opt_hdrs,
        "pseudo_devices": devices,
        "pseudo_device_inits": device_inits,
        "device_needs": list(dict.fromkeys(all_device_needs)),
        "components": components,
        "selected_master_lines": selected_lines,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--srcroot", type=Path, required=True)
    ap.add_argument("--kernel-config", default="DEVELOPMENT")
    ap.add_argument("--board", default="qemu")
    ap.add_argument(
        "--format", choices=("json", "lines", "stats", "mig-defines"), default="json"
    )
    ap.add_argument("--component")
    ap.add_argument("--lang")
    ap.add_argument(
        "--outdir", type=Path, help="Write manifest.json and config headers here"
    )
    args = ap.parse_args()
    data = collect(args.srcroot.resolve(), args.kernel_config, args.board)
    if args.outdir:
        args.outdir.mkdir(parents=True, exist_ok=True)
        for stale in ("types.h", "string.h"):
            p = args.outdir / stale
            if p.is_file():
                p.unlink()
        (args.outdir / "manifest.json").write_text(json.dumps(data, indent=2) + "\n")
        # Inline header generation so configure-time needs one process.
        from importlib.machinery import SourceFileLoader

        gen = Path(__file__).with_name("gen_headers.py")
        # avoid import path issues: duplicate the small writer
        enabled_attrs = set(data["attributes"])
        cpp_opts = data["cpp_options"]
        meta = []
        written: set[str] = set()
        devices = {k: int(v) for k, v in data.get("pseudo_devices", {}).items()}

        def write_count_header(stem: str, count: int) -> None:
            (args.outdir / f"{stem}.h").write_text(f"#define N{stem.upper()} {count}\n")
            if stem not in written:
                meta.append(f"#include <{stem}.h>\n")
                written.add(stem)

        for stem in data["option_headers"]:
            enabled = (
                stem.lower() in enabled_attrs
                or stem.upper() in cpp_opts
                or stem in cpp_opts
                or stem.lower() in devices
            )
            write_count_header(stem, devices.get(stem, 1 if enabled else 0))
        for name, count in devices.items():
            write_count_header(name, count)
        for stem in data.get("device_needs", []):
            if stem not in data["option_headers"] and stem not in devices:
                write_count_header(stem, devices.get(stem, 0))
        (args.outdir / "meta_features.h").write_text("".join(meta))
        lines = ["/* Generated by xnu_config.py */\n", "#pragma once\n"]
        for name, val in cpp_opts.items():
            if val == "":
                lines.append(f"#define {name} 1\n")
            else:
                lines.append(f"#define {name} {val}\n")
        for attr in sorted(enabled_attrs):
            macro = attr.upper()
            if macro not in cpp_opts:
                lines.append(f"#ifndef {macro}\n#define {macro} 1\n#endif\n")
        if args.board:
            # Board names may contain dashes (`qemu-arm64e`); C macros cannot.
            b = args.board.upper().replace("-", "_")
            lines.append(f"#ifndef {b}\n#define {b} 1\n#endif\n")
            if data["arch"] == "x86_64":
                lines.append(f"#ifndef X86_64_BOARD_CONFIG_{b}\n#define X86_64_BOARD_CONFIG_{b} 1\n#endif\n")
            else:
                lines.append(f"#ifndef ARM64_BOARD_CONFIG_{b}\n#define ARM64_BOARD_CONFIG_{b} 1\n#endif\n")
            lines.append(f"#ifndef CURRENT_MACHINE_CONFIG_LC\n#define CURRENT_MACHINE_CONFIG_LC {args.board.lower()}\n#endif\n")
        (args.outdir / "master_config.h").write_text("".join(lines))
        # Mirrors SETUP/config/mkioconf.c's pseudo_inits(): one `extern int
        # <func>(int);` per selected pseudo-device `init` routine, followed
        # by a `pseudo_inits[]` array of {count, func} entries in MASTER
        # order, NULL-terminated. bsd_autoconf() walks this array at boot to
        # run early pseudo-device setup (dtrace, sdt, fbt, systrace,
        # lockstat, lockprof, helper, profile_prvd, ...); an empty array
        # silently skips all of that (e.g. leaving dtrace_strings NULL and
        # crashing the first time dtrace_postinit's dtrace_attach touches
        # it).
        device_inits = data.get("pseudo_device_inits", [])
        ioconf_lines = ["#include <dev/busvar.h>\n\n"]
        for func, _count in device_inits:
            ioconf_lines.append(f"extern int {func}(int);\n")
        ioconf_lines.append("\nstruct pseudo_init pseudo_inits[] = {\n")
        for func, count in device_inits:
            ioconf_lines.append(f"\t{{{count},\t{func}}},\n")
        ioconf_lines.append("\t{0,\t0},\n};\n")
        (args.outdir / "ioconf.c").write_text("".join(ioconf_lines))
        # Mirror EXPORT_HDRS: libsa/string.h is exported at the osfmk include
        # root. <types.h> is *not* exported; osfmk finds it via INCFLAGS_MAKEFILE
        # (-I osfmk/libsa), *after* bsd's machine/types.h is already on the path.
        export_osfmk = args.outdir / "export" / "osfmk"
        export_osfmk.mkdir(parents=True, exist_ok=True)
        shutil.copy2(
            args.srcroot / "osfmk" / "libsa" / "string.h", export_osfmk / "string.h"
        )
        # san/memory and san/coverage export into EXPORT_HDRS/san/ (then
        # EXPORT_MI_DIR=san), included as <san/kasan.h> from -I EXPORT_HDRS/san.
        # Flatten to export/san/*.h and add -I export so the <san/...> name works.
        export_san = args.outdir / "export" / "san"
        export_san.mkdir(parents=True, exist_ok=True)
        for src, name in (
            (args.srcroot / "san" / "memory", "kasan.h"),
            (args.srcroot / "san" / "memory", "kasan-classic.h"),
            (args.srcroot / "san" / "memory", "kasan-tbi.h"),
            (args.srcroot / "san" / "memory", "ubsan_minimal.h"),
            (args.srcroot / "san" / "memory", "memintrinsics.h"),
            (args.srcroot / "san" / "coverage", "kcov.h"),
            (args.srcroot / "san" / "coverage", "kcov_data.h"),
            (args.srcroot / "san" / "coverage", "kcov_ksancov.h"),
            (args.srcroot / "san" / "coverage", "kcov_ksancov_data.h"),
            (args.srcroot / "san" / "coverage", "kcov_stksz.h"),
            (args.srcroot / "san" / "coverage", "kcov_stksz_data.h"),
        ):
            shutil.copy2(src / name, export_san / name)
        # security/Makefile EXPORT_MI_DIR=security → <security/_label.h>
        export_sec = args.outdir / "export" / "security"
        export_sec.mkdir(parents=True, exist_ok=True)
        for name in (
            "_label.h",
            "mac.h",
            "mac_data.h",
            "mac_framework.h",
            "mac_internal.h",
            "mac_mach_internal.h",
            "mac_policy.h",
        ):
            shutil.copy2(args.srcroot / "security" / name, export_sec / name)
        # libkern/libkern/Makefile EXPORT_MI_GEN_LIST = version.h
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        import stamp_version

        stamp_version.stamp(
            args.srcroot / "libkern" / "libkern" / "version.h.template",
            args.outdir / "export" / "libkern" / "version.h",
            config=args.kernel_config,
            version="99.0.0",
            objroot=str(args.outdir),
        )
        # iokit/DriverKit/Makefile EXPORT_MI_DIR=DriverKit (static headers;
        # IIG-generated *.h are written at build time into the same dir).
        export_dk = args.outdir / "export" / "DriverKit"
        export_dk.mkdir(parents=True, exist_ok=True)
        dk_src = args.srcroot / "iokit" / "DriverKit"
        for name in (
            "IOTypes.h",
            "IOReturn.h",
            "IORPC.h",
            "IOKitKeys.h",
            "IOKernelReportStructs.h",
            "IOReportTypes.h",
            "queue_implementation.h",
            "macro_help.h",
            "bounded_ptr.h",
            "bounded_array.h",
            "bounded_array_ref.h",
            "bounded_ptr_fwd.h",
            "OSBoundedArray.h",
            "OSBoundedArrayRef.h",
            "OSBoundedPtr.h",
            "OSBoundedPtrFwd.h",
            "safe_allocation.h",
        ):
            shutil.copy2(dk_src / name, export_dk / name)
        export_dk_crypto = export_dk / "crypto"
        export_dk_crypto.mkdir(parents=True, exist_ok=True)
        for name in ("md5.h", "sha1.h", "aes.h", "sha2.h"):
            shutil.copy2(dk_src / "crypto" / name, export_dk_crypto / name)
        # bsd/skywalk/*/Makefile EXPORT_MI_DIR=skywalk (flatten nested dirs)
        sk = args.srcroot / "bsd" / "skywalk"
        export_sk = args.outdir / "export" / "skywalk"
        export_sk.mkdir(parents=True, exist_ok=True)
        for src, name in (
            (sk, "os_skywalk.h"),
            (sk, "os_skywalk_private.h"),
            (sk, "os_stats_private.h"),
            (sk, "os_sysctls_private.h"),
            (sk / "channel", "os_channel.h"),
            (sk / "channel", "os_channel_private.h"),
            (sk / "channel", "os_channel_event.h"),
            (sk / "packet", "os_packet.h"),
            (sk / "packet", "os_packet_private.h"),
            (sk / "packet", "packet_common.h"),
            (sk / "nexus", "os_nexus.h"),
            (sk / "nexus", "os_nexus_private.h"),
            (sk / "nexus", "nexus_common.h"),
            (sk / "nexus", "nexus_ioctl.h"),
            (sk / "core", "skywalk_common.h"),
        ):
            shutil.copy2(src / name, export_sk / name)
    if args.format == "json":
        json.dump(data, sys.stdout, indent=2)
        sys.stdout.write("\n")
    elif args.format == "stats":
        for comp, buckets in data["components"].items():
            print(
                f"{comp}: c={len(buckets['c'])} cpp={len(buckets['cpp'])} "
                f"s={len(buckets['s'])} generated={len(buckets['generated'])}"
            )
        print(
            f"attributes={len(data['attributes'])} options={len(data['cpp_options'])} "
            f"opt_headers={len(data['option_headers'])}"
        )
    elif args.format == "mig-defines":
        # MIG only accepts -D/-U/-I, not cc -include. Skip expression values.
        cpp_opts = data["cpp_options"]
        for name, val in cpp_opts.items():
            if val == "":
                print(f"-D{name}=1")
            elif re.fullmatch(r"[0-9A-Za-z_]+", val):
                print(f"-D{name}={val}")
        for attr in data["attributes"]:
            macro = attr.upper()
            if macro not in cpp_opts:
                print(f"-D{macro}=1")
    else:
        buckets = data["components"][args.component]
        for p in buckets[args.lang]:
            print(p)
    return 0


if __name__ == "__main__":
    sys.exit(main())
