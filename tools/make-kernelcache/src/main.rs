//! mkstatickc: Transform a standalone XNU kernel (`MH_EXECUTE`) into a
//! KCFormatFileset-compatible kernel collection.

use goblin::mach::Mach;
use goblin::mach::load_command::{CommandVariant, SIZEOF_SECTION_64, SIZEOF_SEGMENT_COMMAND_64};
use std::env;
use std::fs;

const MH_FILESET: u32 = 12;
const MH_DYLIB_IN_CACHE: u32 = 0x8000_0000;

const PRELINK_INFO_PLIST: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>_PrelinkInfoDictionary</key>\n\
\t<array>\n\t</array>\n\
</dict>\n\
</plist>";

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args.len() > 3 {
        eprintln!("Usage: mkstatickc <input.macho> [output.macho]");
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = if args.len() == 3 {
        args[2].clone()
    } else {
        format!("{input_path}.kc")
    };

    eprintln!("[*] Reading {input_path}");
    let data = fs::read(input_path).expect("Failed to read input file");

    let mach = Mach::parse(&data).expect("Failed to parse Mach-O");
    let Mach::Binary(macho) = mach else {
        panic!("Expected a single Mach-O binary")
    };

    let mut output = data.clone();

    patch_header(&mut output);
    patch_symbols(&macho, &mut output);
    patch_prelink_info(&macho, &mut output);

    fs::write(&output_path, &output).expect("Failed to write output file");
    eprintln!("[+] Done!");
}

fn patch_header(output: &mut [u8]) {
    // === Step 1: Patch Mach-O header filetype ===
    let filetype_off = 12;
    output[filetype_off..filetype_off + 4].copy_from_slice(&MH_FILESET.to_le_bytes());

    // === Step 2: Set MH_DYLIB_IN_CACHE flag ===
    let flags_off = 24;
    let old_flags = u32::from_le_bytes(output[flags_off..flags_off + 4].try_into().unwrap());
    let new_flags = old_flags | MH_DYLIB_IN_CACHE;
    output[flags_off..flags_off + 4].copy_from_slice(&new_flags.to_le_bytes());

    // === Step LC_FILESET_ENTRY ===
    let ncmds_off = 16;
    let old_ncmds = u32::from_le_bytes(output[ncmds_off..ncmds_off + 4].try_into().unwrap());
    let sizeofcmds_off = 20;
    let old_sizeofcmds = u32::from_le_bytes(
        output[sizeofcmds_off..sizeofcmds_off + 4]
            .try_into()
            .unwrap(),
    );
    let entry_id_str = b"com.apple.kernel\0";
    let fse_size = 32 + ((entry_id_str.len() + 7) & !7);
    let lc_end = 32 + old_sizeofcmds as usize;
    output[ncmds_off..ncmds_off + 4].copy_from_slice(&(old_ncmds + 1).to_le_bytes());
    output[sizeofcmds_off..sizeofcmds_off + 4]
        .copy_from_slice(&(old_sizeofcmds + fse_size as u32).to_le_bytes());
    output[lc_end..lc_end + 4].copy_from_slice(&0x8000_0035_u32.to_le_bytes());
    output[lc_end + 4..lc_end + 8].copy_from_slice(&(fse_size as u32).to_le_bytes());
    output[lc_end + 8..lc_end + 16].copy_from_slice(&0xffff_ff80_0020_0000_u64.to_le_bytes());
    output[lc_end + 24..lc_end + 28].copy_from_slice(&32u32.to_le_bytes());
    output[lc_end + 32..lc_end + 32 + entry_id_str.len()].copy_from_slice(entry_id_str);
}

fn patch_symbols(macho: &goblin::mach::MachO, output: &mut [u8]) {
    // === Step 3: Patch functions ===
    let find_sym_va = |target: &str| -> Option<u64> {
        let syms = macho.symbols.as_ref()?;
        for (name, nlist) in syms.iter().flatten() {
            if name == target {
                return Some(nlist.n_value);
            }
        }
        None
    };

    let find_file_offset = |va: u64| -> Option<usize> {
        for seg in &macho.segments {
            if va >= seg.vmaddr && va < seg.vmaddr + seg.vmsize {
                return Some((seg.fileoff + (va - seg.vmaddr)) as usize);
            }
        }
        None
    };

    let ret_patch_targets = [
        "_kmem_crypto_init",
        "_trust_cache_runtime_init",
        "_load_static_trust_cache",
        "_OSKextRemoveKextBootstrap",
    ];
    for target in &ret_patch_targets {
        if let Some(va) = find_sym_va(target)
            && let Some(off) = find_file_offset(va)
        {
            eprintln!("[*] Patching {target} at VA 0x{va:x} (file offset 0x{off:x}) -> ret");
            output[off] = 0xC3;
        }
    }

    // Redirect IOPanicPlatform::start -> IOPlatformExpert::start (parent class).
    // Just returning true isn't enough: the parent's start() sets gIOPlatform,
    // initializes interrupt controllers, calls configure()/registerService(), etc.
    if let (Some(panic_va), Some(parent_va)) = (
        find_sym_va("__ZN15IOPanicPlatform5startEP9IOService"),
        find_sym_va("__ZN16IOPlatformExpert5startEP9IOService"),
    ) && let Some(off) = find_file_offset(panic_va)
    {
        let rel: i32 = (parent_va.cast_signed() - (panic_va.cast_signed() + 5)) as i32;
        output[off] = 0xE9; // JMP rel32
        output[off + 1..off + 5].copy_from_slice(&rel.to_le_bytes());
        eprintln!(
            "[*] Redirecting IOPanicPlatform::start (0x{panic_va:x}) -> IOPlatformExpert::start (0x{parent_va:x})"
        );
    }

    if let (Some(src_va), Some(dst_va)) = (
        find_sym_va("_read_random"),
        find_sym_va("_read_erandom_generate"),
    ) && let Some(off) = find_file_offset(src_va)
    {
        let rel: i32 = (dst_va.cast_signed() - (src_va.cast_signed() + 5)) as i32;
        output[off] = 0xE9;
        output[off + 1..off + 5].copy_from_slice(&rel.to_le_bytes());
        eprintln!(
            "[*] Redirecting _read_random (VA 0x{src_va:x}) -> _read_erandom_generate (VA 0x{dst_va:x})"
        );
    }

    // Patch _ml_wait_max_cpus to set machine_info.max_cpus = 1,
    // max_cpus_initialized = MAX_CPUS_SET (1), and return 1.
    // Without AppleACPICPU kext, ml_set_max_cpus is never called, so
    // ml_wait_max_cpus blocks forever.
    //
    //   mov dword ptr [rip + off1], 1   ; machine_info.max_cpus = 1
    //   mov dword ptr [rip + off2], 1   ; max_cpus_initialized = MAX_CPUS_SET
    //   mov eax, 1                      ; return 1
    //   ret
    if let (Some(wait_va), Some(mi_va), Some(mci_va)) = (
        find_sym_va("_ml_wait_max_cpus"),
        find_sym_va("_machine_info"),
        find_sym_va("_max_cpus_initialized"),
    ) && let Some(off) = find_file_offset(wait_va)
    {
        let mut pos = 0usize;

        // mov dword ptr [rip+disp32], 1  -> machine_info.max_cpus (offset 8)
        let max_cpus_va = mi_va + 8;
        output[off + pos] = 0xC7;
        output[off + pos + 1] = 0x05;
        let rip_after = wait_va + (pos as u64) + 10;
        #[allow(clippy::cast_possible_wrap)]
        let disp: i32 = (max_cpus_va as i64 - rip_after as i64) as i32;
        output[off + pos + 2..off + pos + 6].copy_from_slice(&disp.to_le_bytes());
        output[off + pos + 6..off + pos + 10].copy_from_slice(&1u32.to_le_bytes());
        pos += 10;

        // mov dword ptr [rip+disp32], 1  -> max_cpus_initialized
        output[off + pos] = 0xC7;
        output[off + pos + 1] = 0x05;
        let rip_after = wait_va + (pos as u64) + 10;
        #[allow(clippy::cast_possible_wrap)]
        let disp: i32 = (mci_va as i64 - rip_after as i64) as i32;
        output[off + pos + 2..off + pos + 6].copy_from_slice(&disp.to_le_bytes());
        output[off + pos + 6..off + pos + 10].copy_from_slice(&1u32.to_le_bytes());
        pos += 10;

        // mov eax, 1
        output[off + pos] = 0xB8;
        output[off + pos + 1..off + pos + 5].copy_from_slice(&1u32.to_le_bytes());
        pos += 5;
        // ret
        output[off + pos] = 0xC3;

        eprintln!(
            "[*] Patching _ml_wait_max_cpus (0x{wait_va:x}) -> set max_cpus=1, max_cpus_initialized=1; return 1"
        );
    }
}

fn patch_prelink_info(macho: &goblin::mach::MachO, output: &mut Vec<u8>) {
    // === Step 4: Populate __PRELINK_INFO segment ===
    // Append to the end of the file and place just after the last segment in VM.
    let last_seg = macho.segments.iter().max_by_key(|s| s.vmaddr).unwrap();
    let target_va = (last_seg.vmaddr + last_seg.vmsize + 0xFFF) & !0xFFF;

    // Align file output to page boundary for clean mapping
    while !output.len().is_multiple_of(0x1000) {
        output.push(0);
    }
    let target_off = output.len() as u64;
    output.extend_from_slice(PRELINK_INFO_PLIST);
    let plist_size = PRELINK_INFO_PLIST.len() as u64;

    let mut patched = false;
    for lc in &macho.load_commands {
        if let CommandVariant::Segment64(seg) = &lc.command {
            let segname = std::str::from_utf8(&seg.segname)
                .unwrap_or("")
                .trim_matches('\0');
            if segname == "__PRELINK_INFO" {
                let seg_off = lc.offset;
                eprintln!(
                    "[*] Relocating __PRELINK_INFO to VA 0x{target_va:x}, file offset 0x{target_off:x}"
                );

                output[seg_off + 24..seg_off + 32].copy_from_slice(&target_va.to_le_bytes());
                output[seg_off + 32..seg_off + 40].copy_from_slice(&0x1000_u64.to_le_bytes());
                output[seg_off + 40..seg_off + 48].copy_from_slice(&target_off.to_le_bytes());
                output[seg_off + 48..seg_off + 56].copy_from_slice(&plist_size.to_le_bytes());

                let mut sect_off = seg_off + SIZEOF_SEGMENT_COMMAND_64;
                for _ in 0..seg.nsects {
                    let sname = std::str::from_utf8(&output[sect_off..sect_off + 16])
                        .unwrap_or("")
                        .trim_matches('\0');
                    if sname == "__info" {
                        output[sect_off + 32..sect_off + 40]
                            .copy_from_slice(&target_va.to_le_bytes());
                        output[sect_off + 40..sect_off + 48]
                            .copy_from_slice(&plist_size.to_le_bytes());
                        output[sect_off + 48..sect_off + 52]
                            .copy_from_slice(&(target_off as u32).to_le_bytes());
                        patched = true;
                        break;
                    }
                    sect_off += SIZEOF_SECTION_64;
                }
            }
        }
    }
    if !patched {
        eprintln!("[!] Warning: __PRELINK_INFO not patched");
    }
}
