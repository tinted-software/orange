use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::path::{Path, PathBuf};

use clap::Parser;
use loader::{ChainedImport, LoadPlan, SegmentPlan, SymbolBinding, install_name_for_ordinal};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("{0}")]
    Loader(#[from] loader::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Msg(String),
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Parser)]
#[command(name = "dyld")]
struct Args {
    macho: PathBuf,
    #[arg(long)]
    run: bool,
    #[arg(long, short)]
    verbose: bool,
    #[arg(allow_hyphen_values = true, num_args = 0..)]
    args: Vec<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let file = std::fs::File::open(&args.macho)
        .map_err(|e| Error::Msg(format!("failed to open {}: {e}", args.macho.display())))?;
    let mmap = unsafe { memmap2::Mmap::map(&file)? };

    let slice = loader::select_arm64_slice(&mmap)?;
    let plan = loader::build_load_plan(slice)?;

    let main_path = std::fs::canonicalize(&args.macho)?;
    let main_dir = main_path.parent().unwrap_or(Path::new("."));

    let bindings = bind_imported_symbols(slice, &plan, &main_path, main_dir)?;

    if args.verbose {
        print_plan(&plan, &bindings);
    }

    if args.run {
        run_entry_trampoline(slice, &plan, &bindings, &main_path, &args.args)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Shared cache
// ---------------------------------------------------------------------------

struct SharedCacheIndex {
    paths: HashSet<String>,
    loaded: bool,
}

fn load_shared_cache_index() -> SharedCacheIndex {
    let candidates = [
        "/System/Volumes/Preboot/Cryptexes/OS/System/Library/dyld/dyld_shared_cache_arm64e",
        "/System/Library/dyld/dyld_shared_cache_arm64e",
    ];
    for path in &candidates {
        if let Ok(file) = std::fs::File::open(path)
            && let Ok(mmap) = unsafe { memmap2::Mmap::map(&file) }
            && let Ok(paths) = loader::parse_shared_cache_image_paths(&mmap)
        {
            return SharedCacheIndex {
                paths: paths.into_iter().collect(),
                loaded: true,
            };
        }
    }
    SharedCacheIndex {
        paths: HashSet::new(),
        loaded: false,
    }
}

// ---------------------------------------------------------------------------
// Dylib loading
// ---------------------------------------------------------------------------

struct LoadedDylib {
    resolved_path: String,
    handle: Option<*mut libc::c_void>,
    from_shared_cache: bool,
    exports: Option<HashSet<String>>,
}

impl Drop for LoadedDylib {
    fn drop(&mut self) {
        if let Some(h) = self.handle {
            unsafe {
                libc::dlclose(h);
            }
        }
    }
}

struct ResolveContext<'a> {
    executable_dir: &'a Path,
    loader_dir: &'a Path,
    rpaths: &'a [String],
}

fn resolve_install_name(install_name: &str, ctx: &ResolveContext<'_>) -> Option<PathBuf> {
    if let Some(suffix) = install_name.strip_prefix("@executable_path/") {
        return Some(ctx.executable_dir.join(suffix));
    }
    if let Some(suffix) = install_name.strip_prefix("@loader_path/") {
        return Some(ctx.loader_dir.join(suffix));
    }
    if let Some(suffix) = install_name.strip_prefix("@rpath/") {
        for rpath in ctx.rpaths {
            let expanded = expand_rpath(rpath, ctx);
            let candidate = PathBuf::from(&expanded).join(suffix);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        return None;
    }
    Some(PathBuf::from(install_name))
}

fn expand_rpath(raw: &str, ctx: &ResolveContext<'_>) -> String {
    if let Some(suffix) = raw.strip_prefix("@executable_path/") {
        return ctx
            .executable_dir
            .join(suffix)
            .to_string_lossy()
            .into_owned();
    }
    if let Some(suffix) = raw.strip_prefix("@loader_path/") {
        return ctx.loader_dir.join(suffix).to_string_lossy().into_owned();
    }
    if raw == "@executable_path" {
        return ctx.executable_dir.to_string_lossy().into_owned();
    }
    if raw == "@loader_path" {
        return ctx.loader_dir.to_string_lossy().into_owned();
    }
    raw.to_owned()
}

fn load_dylib_exports(path: &str) -> Option<HashSet<String>> {
    let file = std::fs::File::open(path).ok()?;
    let mmap = unsafe { memmap2::Mmap::map(&file).ok()? };
    let slice = loader::select_arm64_slice(&mmap).ok()?;
    let trie = loader::find_export_trie(slice).ok()?;
    let set = loader::parse_export_trie_symbols(trie).ok()?;
    Some(set.into_iter().collect())
}

fn load_dylibs(
    imports: &[String],
    ctx: &ResolveContext<'_>,
    shared_cache: &SharedCacheIndex,
    loaded: &mut Vec<LoadedDylib>,
    seen: &mut HashSet<String>,
    queue: &mut Vec<usize>,
) {
    for import_name in imports {
        let resolved = match resolve_install_name(import_name, ctx) {
            Some(p) => p.to_string_lossy().into_owned(),
            None => continue,
        };
        if seen.contains(&resolved) {
            continue;
        }

        let cpath = match CString::new(resolved.as_bytes()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let handle = unsafe { libc::dlopen(cpath.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL) };
        if !handle.is_null() {
            let exports = load_dylib_exports(&resolved);
            seen.insert(resolved.clone());
            loaded.push(LoadedDylib {
                resolved_path: resolved,
                handle: Some(handle),
                from_shared_cache: false,
                exports,
            });
            queue.push(loaded.len() - 1);
        } else if shared_cache.loaded && shared_cache.paths.contains(&resolved) {
            seen.insert(resolved.clone());
            loaded.push(LoadedDylib {
                resolved_path: resolved,
                handle: None,
                from_shared_cache: true,
                exports: None,
            });
            queue.push(loaded.len() - 1);
        }
    }
}

fn load_dylibs_transitive(
    plan: &LoadPlan,
    _main_path: &Path,
    main_dir: &Path,
    shared_cache: &SharedCacheIndex,
    loaded: &mut Vec<LoadedDylib>,
) {
    let mut seen = HashSet::new();
    let mut queue: Vec<usize> = Vec::new();

    let rpaths: Vec<String> = plan.rpaths.clone();
    let ctx = ResolveContext {
        executable_dir: main_dir,
        loader_dir: main_dir,
        rpaths: &rpaths,
    };
    load_dylibs(
        &plan.dylibs,
        &ctx,
        shared_cache,
        loaded,
        &mut seen,
        &mut queue,
    );

    let mut queue_idx = 0;
    while queue_idx < queue.len() {
        let dylib_idx = queue[queue_idx];
        queue_idx += 1;

        let dylib_path = loaded[dylib_idx].resolved_path.clone();
        let (dep_dylibs, dep_rpaths) = match load_image_imports(&dylib_path) {
            Some(v) => v,
            None => continue,
        };

        let dylib_dir = Path::new(&dylib_path)
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let ctx = ResolveContext {
            executable_dir: main_dir,
            loader_dir: &dylib_dir,
            rpaths: &dep_rpaths,
        };
        load_dylibs(
            &dep_dylibs,
            &ctx,
            shared_cache,
            loaded,
            &mut seen,
            &mut queue,
        );
    }
}

fn load_image_imports(path: &str) -> Option<(Vec<String>, Vec<String>)> {
    let file = std::fs::File::open(path).ok()?;
    let mmap = unsafe { memmap2::Mmap::map(&file).ok()? };
    let slice = loader::select_arm64_slice(&mmap).ok()?;
    loader::collect_image_imports(slice).ok()
}

// ---------------------------------------------------------------------------
// Symbol resolution
// ---------------------------------------------------------------------------

fn resolve_symbol(
    raw_name: &str,
    dylibs: &[LoadedDylib],
    preferred_install: Option<&str>,
    strict_preferred: bool,
) -> Option<(usize, String)> {
    let candidates: Vec<&str> = if raw_name.starts_with('_') && raw_name.len() > 1 {
        vec![raw_name, &raw_name[1..]]
    } else {
        vec![raw_name]
    };

    for cand in &candidates {
        let cname = match CString::new(*cand) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for dylib in dylibs {
            if let Some(install) = preferred_install
                && dylib.resolved_path != install
            {
                continue;
            }
            if let Some(exports) = &dylib.exports {
                let has = exports.contains(*cand)
                    || (cand.starts_with('_') && cand.len() > 1 && exports.contains(&cand[1..]));
                if !has {
                    continue;
                }
            }
            let ptr = if let Some(h) = dylib.handle {
                unsafe { libc::dlsym(h, cname.as_ptr()) }
            } else if dylib.from_shared_cache {
                unsafe { libc::dlsym(std::ptr::null_mut(), cname.as_ptr()) }
            } else {
                continue;
            };
            if !ptr.is_null() {
                return Some((ptr as usize, dylib.resolved_path.clone()));
            }
        }
    }

    if preferred_install.is_none() || strict_preferred {
        return None;
    }

    // Retry without preferred
    for cand in &candidates {
        let cname = match CString::new(*cand) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for dylib in dylibs {
            if let Some(exports) = &dylib.exports {
                let has = exports.contains(*cand)
                    || (cand.starts_with('_') && cand.len() > 1 && exports.contains(&cand[1..]));
                if !has {
                    continue;
                }
            }
            let ptr = if let Some(h) = dylib.handle {
                unsafe { libc::dlsym(h, cname.as_ptr()) }
            } else if dylib.from_shared_cache {
                unsafe { libc::dlsym(std::ptr::null_mut(), cname.as_ptr()) }
            } else {
                continue;
            };
            if !ptr.is_null() {
                return Some((ptr as usize, dylib.resolved_path.clone()));
            }
        }
    }
    None
}

fn bind_imported_symbols(
    bytes: &[u8],
    plan: &LoadPlan,
    main_path: &Path,
    main_dir: &Path,
) -> Result<Vec<SymbolBinding>> {
    let shared_cache = load_shared_cache_index();
    let mut dylibs = Vec::new();
    load_dylibs_transitive(plan, main_path, main_dir, &shared_cache, &mut dylibs);

    let mut bindings = Vec::new();
    let mut seen = HashSet::new();

    // From symtab/dysymtab undefined symbols
    let undefs = loader::iter_undefined_symbols(bytes, plan)?;
    for undef in &undefs {
        if seen.contains(undef.name) {
            continue;
        }
        let preferred = install_name_for_ordinal(plan, undef.dylib_ordinal);
        let mut binding = SymbolBinding {
            name: undef.name.to_owned(),
            source_dylib: None,
            address: None,
            weak: undef.weak,
        };
        if let Some((addr, source)) = resolve_symbol(undef.name, &dylibs, preferred, false) {
            binding.address = Some(addr);
            binding.source_dylib = Some(source);
        }
        seen.insert(undef.name.to_owned());
        bindings.push(binding);
    }

    // From chained fixup imports
    if let Some(fix_cmd) = plan.chained_fixups {
        let fixoff = fix_cmd.dataoff as usize;
        let fixsize = fix_cmd.datasize as usize;
        if fixoff + fixsize <= bytes.len() {
            let blob = &bytes[fixoff..fixoff + fixsize];
            if let Ok(hdr) = loader::parse_chained_fixups_header(blob)
                && let Ok(imports) = loader::parse_chained_imports(blob, &hdr)
            {
                seed_bindings_from_chained_imports(
                    plan,
                    &imports,
                    &dylibs,
                    &mut bindings,
                    &mut seen,
                );
            }
        }
    }

    Ok(bindings)
}

fn seed_bindings_from_chained_imports(
    plan: &LoadPlan,
    imports: &[ChainedImport],
    dylibs: &[LoadedDylib],
    bindings: &mut Vec<SymbolBinding>,
    seen: &mut HashSet<String>,
) {
    for imp in imports {
        if imp.name.is_empty() || seen.contains(&imp.name) {
            continue;
        }
        let preferred = install_name_for_ordinal(plan, imp.lib_ordinal);
        let mut binding = SymbolBinding {
            name: imp.name.clone(),
            source_dylib: None,
            address: None,
            weak: imp.weak,
        };
        if let Some((addr, source)) = resolve_symbol(&imp.name, dylibs, preferred, true) {
            binding.address = Some(addr);
            binding.source_dylib = Some(source);
        }
        seen.insert(imp.name.clone());
        bindings.push(binding);
    }
}

// ---------------------------------------------------------------------------
// Print plan
// ---------------------------------------------------------------------------

fn print_plan(plan: &LoadPlan, bindings: &[SymbolBinding]) {
    eprintln!("dyld: load plan");
    if let Some(off) = plan.entryoff {
        eprintln!("  entry file offset: 0x{off:x}");
    } else if let Some(va) = plan.entry_vmaddr {
        eprintln!("  entry vmaddr: 0x{va:x}");
    }
    eprintln!("  has dyld info: {}", plan.has_dyld_info);
    eprintln!("  has chained fixups: {}", plan.has_chained_fixups);
    eprintln!("  segments ({}):", plan.segments.len());
    for seg in &plan.segments {
        let name = seg.name_str();
        eprintln!(
            "    {name:<16} vm=0x{:012x} sz=0x{:08x} file=0x{:08x} filesz=0x{:08x} prot={:#x}/{:#x}",
            seg.vmaddr, seg.vmsize, seg.fileoff, seg.filesize, seg.initprot, seg.maxprot,
        );
    }
    eprintln!("  dylibs ({}):", plan.dylibs.len());
    for name in &plan.dylibs {
        eprintln!("    {name}");
    }
    eprintln!("  rpaths ({}):", plan.rpaths.len());
    for rpath in &plan.rpaths {
        eprintln!("    {rpath}");
    }

    let resolved = bindings.iter().filter(|b| b.address.is_some()).count();
    let unresolved = bindings.len() - resolved;
    eprintln!(
        "  symbol bindings ({} total, {resolved} resolved, {unresolved} unresolved):",
        bindings.len()
    );
    for b in bindings {
        if let Some(addr) = b.address {
            eprintln!(
                "    ok   {} -> {} @ 0x{addr:x}",
                b.name,
                b.source_dylib.as_deref().unwrap_or("?")
            );
        } else {
            let weak = if b.weak { " (weak)" } else { "" };
            eprintln!("    miss {}{weak}", b.name);
        }
    }
}

// ---------------------------------------------------------------------------
// Execution (map + jump)
// ---------------------------------------------------------------------------

fn run_entry_trampoline(
    bytes: &[u8],
    plan: &LoadPlan,
    bindings: &[SymbolBinding],
    main_path: &Path,
    extra_args: &[String],
) -> Result<()> {
    let (mapped, min_vmaddr) = map_image(bytes, plan)?;

    // Build resolve callback from bindings
    let mut sym_map: HashMap<String, usize> = HashMap::new();
    for b in bindings {
        if let Some(addr) = b.address {
            sym_map.insert(b.name.clone(), addr);
            if b.name.starts_with('_') && b.name.len() > 1 {
                let trimmed = &b.name[1..];
                sym_map.entry(trimmed.to_owned()).or_insert(addr);
            }
        }
    }
    let resolve = |name: &str, preferred: Option<&str>| -> Option<usize> {
        // Try exact, then with/without underscore
        if let Some(&a) = sym_map.get(name) {
            return Some(a);
        }
        if name.starts_with('_')
            && name.len() > 1
            && let Some(&a) = sym_map.get(&name[1..])
        {
            return Some(a);
        }
        // Also check preferred-based binding in the bindings list
        if let Some(install) = preferred {
            for b in bindings {
                if b.address.is_none() || b.name != name {
                    continue;
                }
                if let Some(src) = &b.source_dylib
                    && src == install
                {
                    return b.address;
                }
            }
        }
        None
    };

    // Apply fixups
    let mapped_slice = unsafe { std::slice::from_raw_parts_mut(mapped, plan_mapped_len(plan)?) };
    if plan.chained_fixups.is_some() {
        loader::apply_chained_fixups(bytes, plan, mapped_slice, min_vmaddr, &resolve)?;
    } else if plan.dyld_info.is_some() {
        loader::apply_dyld_info_fixups(plan, mapped_slice, min_vmaddr, bytes, &resolve)?;
    }

    // Set segment protections
    apply_segment_protections(mapped, min_vmaddr, &plan.segments)?;

    // Compute entry address
    let entry_vmaddr = if let Some(off) = plan.entryoff {
        loader::entry_vmaddr_from_offset(off, &plan.segments)
            .ok_or(loader::Error::EntryOutsideSegments)?
    } else {
        plan.entry_vmaddr.ok_or(loader::Error::MissingEntryPoint)?
    };
    let entry_off = (entry_vmaddr - min_vmaddr) as usize;
    let entry_addr = mapped as usize + entry_off;

    // Build argv
    let abs_main = main_path.to_string_lossy();
    let mut argv_strings: Vec<CString> = Vec::new();
    argv_strings.push(CString::new(abs_main.as_bytes()).unwrap());
    for arg in extra_args {
        argv_strings.push(CString::new(arg.as_bytes()).unwrap());
    }
    let mut argv_ptrs: Vec<*const u8> = argv_strings
        .iter()
        .map(|s| s.as_ptr().cast::<u8>())
        .collect();
    argv_ptrs.push(std::ptr::null());
    let argc = argv_strings.len();

    // envp
    let envp = unsafe {
        unsafe extern "C" {
            static environ: *const *const u8;
        }
        environ
    };

    // apple[]
    let apple_str = CString::new(format!("executable_path={abs_main}")).unwrap();
    let apple_ptrs: [*const u8; 2] = [apple_str.as_ptr().cast::<u8>(), std::ptr::null()];

    // Seed Darwin process state
    seed_darwin_process_state(argc, argv_ptrs.as_ptr(), envp, apple_ptrs.as_ptr());

    // Build startup stack
    let stack_size = 8 * 1024 * 1024;
    let stack_mem = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            stack_size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    if stack_mem == libc::MAP_FAILED {
        return Err(Error::Msg("failed to mmap stack".into()));
    }

    let env_count = count_null_terminated_ptrs(envp);
    let apple_count = apple_ptrs.len() - 1;
    let words_needed = 1 + (argc + 1) + (env_count + 1) + (apple_count + 1);
    let stack_bytes = words_needed * std::mem::size_of::<usize>();
    let stack_base = stack_mem as usize;
    let stack_end = stack_base + stack_size;
    let sp = (stack_end - stack_bytes) & !0xF;
    let words = unsafe { std::slice::from_raw_parts_mut(sp as *mut usize, words_needed) };

    let mut w = 0;
    words[w] = argc;
    w += 1;
    for i in 0..argc {
        words[w] = argv_ptrs[i] as usize;
        w += 1;
    }
    words[w] = 0;
    w += 1;
    for i in 0..env_count {
        words[w] = unsafe { *envp.add(i) } as usize;
        w += 1;
    }
    words[w] = 0;
    w += 1;
    for i in 0..apple_count {
        words[w] = apple_ptrs[i] as usize;
        w += 1;
    }
    words[w] = 0;

    let stack_argv = (sp + std::mem::size_of::<usize>()) as *const *const u8;
    let stack_envp = (sp + (1 + argc + 1) * std::mem::size_of::<usize>()) as *const *const u8;
    let stack_apple =
        (sp + (1 + argc + 1 + env_count + 1) * std::mem::size_of::<usize>()) as *const *const u8;

    eprintln!("dyld: jumping to entry at 0x{entry_addr:x}");

    if std::env::var("DYLD_REPL_DIRECT_CALL").is_ok() {
        type EntryFn = unsafe extern "C" fn(
            usize,
            *const *const u8,
            *const *const u8,
            *const *const u8,
        ) -> i32;
        let f: EntryFn = unsafe { std::mem::transmute(entry_addr) };
        let status = unsafe { f(argc, stack_argv, stack_envp, stack_apple) };
        std::process::exit(status);
    }

    unsafe { jump_to_entry(entry_addr, argc, stack_argv, stack_envp, stack_apple, sp) };
}

fn plan_mapped_len(plan: &LoadPlan) -> Result<usize> {
    let min = loader::min_mapped_vmaddr(&plan.segments).ok_or(loader::Error::NoMappableSegments)?;
    let mut max: u64 = 0;
    for seg in &plan.segments {
        if seg.is_pagezero() || seg.vmsize == 0 {
            continue;
        }
        max = max.max(seg.vmaddr + seg.vmsize);
    }
    let span = (max - min) as usize;
    Ok((span + 0xFFF) & !0xFFF)
}

fn map_image(bytes: &[u8], plan: &LoadPlan) -> Result<(*mut u8, u64)> {
    let min = loader::min_mapped_vmaddr(&plan.segments).ok_or(loader::Error::NoMappableSegments)?;
    let map_len = plan_mapped_len(plan)?;

    let mem = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            map_len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    if mem == libc::MAP_FAILED {
        return Err(Error::Msg("mmap failed".into()));
    }
    let base = mem.cast::<u8>();

    for seg in &plan.segments {
        if seg.is_pagezero() || seg.filesize == 0 {
            continue;
        }
        let src_off = seg.fileoff as usize;
        let copy_len = seg.filesize as usize;
        if src_off + copy_len > bytes.len() {
            return Err(loader::Error::TruncatedFile.into());
        }
        let dst_off = (seg.vmaddr - min) as usize;
        if dst_off + copy_len > map_len {
            return Err(loader::Error::InvalidSegmentLayout.into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr().add(src_off), base.add(dst_off), copy_len);
        }
    }

    Ok((base, min))
}

fn apply_segment_protections(
    base: *mut u8,
    min_vmaddr: u64,
    segments: &[SegmentPlan],
) -> Result<()> {
    let page_size = 0x4000usize; // 16K
    for seg in segments {
        if seg.is_pagezero() || seg.vmsize == 0 {
            continue;
        }
        let seg_off = (seg.vmaddr - min_vmaddr) as usize;
        let prot_len = (seg.vmsize as usize + page_size - 1) & !(page_size - 1);

        let mut prot = 0;
        if seg.initprot & 1 != 0 {
            prot |= libc::PROT_READ;
        }
        if seg.initprot & 2 != 0 {
            prot |= libc::PROT_WRITE;
        }
        if seg.initprot & 4 != 0 {
            prot |= libc::PROT_EXEC;
        }

        let addr = unsafe { base.add(seg_off) };
        let ret = unsafe { libc::mprotect(addr.cast(), prot_len, prot) };
        if ret != 0 {
            return Err(Error::Msg(format!(
                "mprotect failed for segment {}",
                seg.name_str()
            )));
        }
    }
    Ok(())
}

fn count_null_terminated_ptrs(p: *const *const u8) -> usize {
    let mut n = 0;
    unsafe {
        while !(*p.add(n)).is_null() {
            n += 1;
        }
    }
    n
}

fn seed_darwin_process_state(
    argc: usize,
    argv: *const *const u8,
    envp: *const *const u8,
    _apple: *const *const u8,
) {
    #[cfg(target_os = "macos")]
    {
        unsafe {
            unsafe extern "C" {
                fn _NSGetArgc() -> *mut libc::c_int;
                fn _NSGetArgv() -> *mut *const *const libc::c_char;
                fn _NSGetEnviron() -> *mut *const *const libc::c_char;
                fn setprogname(name: *const libc::c_char);
            }
            *_NSGetArgc() = argc as libc::c_int;
            *_NSGetArgv() = argv.cast::<*const libc::c_char>();
            *_NSGetEnviron() = envp.cast::<*const libc::c_char>();

            // Set progname
            if argc > 0 && !(*argv.cast::<*const libc::c_char>()).is_null() {
                let argv0 = *argv.cast::<*const libc::c_char>();
                let argv0_str = std::ffi::CStr::from_ptr(argv0);
                if let Some(basename) = argv0_str.to_bytes().iter().rposition(|&b| b == b'/') {
                    setprogname(argv0.add(basename + 1));
                } else {
                    setprogname(argv0);
                }
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn jump_to_entry(
    entry: usize,
    argc: usize,
    argv: *const *const u8,
    envp: *const *const u8,
    apple: *const *const u8,
    sp: usize,
) -> ! {
    let status: usize;
    unsafe {
        core::arch::asm!(
            "mov x17, sp",
            "mov x16, {entry}",
            "mov x0, {argc}",
            "mov x1, {argv}",
            "mov x2, {envp}",
            "mov x3, {apple}",
            "mov sp, {sp}",
            "adr x30, 2f",
            "pacibsp",
            "br x16",
            "2:",
            "mov sp, x17",
            entry = in(reg) entry,
            argc = in(reg) argc,
            argv = in(reg) argv,
            envp = in(reg) envp,
            apple = in(reg) apple,
            sp = in(reg) sp,
            lateout("x0") status,
            out("x1") _,
            out("x2") _,
            out("x3") _,
            out("x16") _,
            out("x17") _,
            out("x30") _,
        );
    }
    std::process::exit(status as i32);
}

#[cfg(not(target_arch = "aarch64"))]
unsafe fn jump_to_entry(
    entry: usize,
    argc: usize,
    argv: *const *const u8,
    envp: *const *const u8,
    apple: *const *const u8,
    _sp: usize,
) -> ! {
    type EntryFn =
        unsafe extern "C" fn(usize, *const *const u8, *const *const u8, *const *const u8) -> i32;
    let f: EntryFn = std::mem::transmute(entry);
    let status = f(argc, argv, envp, apple);
    std::process::exit(status);
}
