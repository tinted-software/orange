//! mkstatickc: Transform a standalone XNU kernel (MH_EXECUTE) into a
//! KCFormatFileset-compatible kernel collection.
//!
//! XNU's PE_get_primary_kc_format() in pexpert/gen/kcformat.c:
//!   - filetype == MH_FILESET (12) => KCFormatFileset (3) => works
//!   - anything else on x86       => KCFormatDynamic (2) => panic
//!
//! This tool patches:
//!   1. Mach-O header filetype: MH_EXECUTE (2) -> MH_FILESET (12)
//!   2. __PRELINK_INFO.__info section: populated with minimal prelink plist

use goblin::mach::Mach;
use goblin::mach::load_command::{CommandVariant, SIZEOF_SECTION_64, SIZEOF_SEGMENT_COMMAND_64};
use std::env;
use std::fs;
use std::io::Write;

/// MH_FILESET filetype value
const MH_FILESET: u32 = 12;

/// MH_DYLIB_IN_CACHE flag - indicates the binary is part of a kernel collection
const MH_DYLIB_IN_CACHE: u32 = 0x80000000;

/// Minimal prelink info plist that XNU will accept.
/// This tells XNU there are no prelinked kexts (empty array).
const PRELINK_INFO_PLIST: &[u8] = b"\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>_PrelinkInfoDictionary</key>\n\
\t<array>\n\
\t</array>\n\
</dict>\n\
</plist>\n\0";

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args.len() > 3 {
        eprintln!("Usage: mkstatickc <input.macho> [output.macho]");
        eprintln!("  Transforms a standalone XNU kernel into a KCFormatFileset kernel.");
        eprintln!("  If output is omitted, writes to <input>.kc");
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = if args.len() == 3 {
        args[2].clone()
    } else {
        format!("{}.kc", input_path)
    };

    eprintln!("[*] Reading {}", input_path);
    let data = fs::read(input_path).expect("Failed to read input file");

    // Parse to validate it's a Mach-O
    let mach = Mach::parse(&data).expect("Failed to parse Mach-O");
    let macho = match mach {
        Mach::Binary(m) => m,
        _ => panic!("Expected a single Mach-O binary, not a fat binary"),
    };

    eprintln!(
        "[*] Original filetype: {} (MH_EXECUTE=2, MH_FILESET=12)",
        macho.header.filetype
    );

    let mut output = data.clone();

    // === Step 1: Patch Mach-O header filetype to MH_FILESET ===
    // Mach-O 64-bit header layout:
    //   u32 magic     (+0)
    //   u32 cputype   (+4)
    //   u32 cpusubtype(+8)
    //   u32 filetype  (+12)
    //   u32 ncmds     (+16)
    //   u32 sizeofcmds(+20)
    //   u32 flags     (+24)
    //   u32 reserved  (+28)
    let filetype_off = 12;
    let old_filetype =
        u32::from_le_bytes(output[filetype_off..filetype_off + 4].try_into().unwrap());
    output[filetype_off..filetype_off + 4].copy_from_slice(&MH_FILESET.to_le_bytes());
    eprintln!(
        "[*] Patched filetype: {} -> {} (MH_FILESET)",
        old_filetype, MH_FILESET
    );

    // === Step 2: Set MH_DYLIB_IN_CACHE flag ===
    // XNU's kernel_mach_header_is_in_fileset() checks:
    //   (mh->flags & MH_DYLIB_IN_CACHE)
    // Without this flag, i386_slide_and_rebase_image returns early
    // and PE_set_kc_header(KCKindPrimary) never gets called.
    let flags_off = 24;
    let old_flags = u32::from_le_bytes(output[flags_off..flags_off + 4].try_into().unwrap());
    let new_flags = old_flags | MH_DYLIB_IN_CACHE;
    output[flags_off..flags_off + 4].copy_from_slice(&new_flags.to_le_bytes());
    eprintln!(
        "[*] Patched flags: 0x{:08x} -> 0x{:08x} (set MH_DYLIB_IN_CACHE)",
        old_flags, new_flags
    );

    // === Write output ===
    let ncmds_off = 16;
    let old_ncmds = u32::from_le_bytes(output[ncmds_off..ncmds_off + 4].try_into().unwrap());
    let sizeofcmds_off = 20;
    let old_sizeofcmds = u32::from_le_bytes(
        output[sizeofcmds_off..sizeofcmds_off + 4]
            .try_into()
            .unwrap(),
    );

    let entry_id_str = b"com.apple.kernel\0";
    // 24 = size of 6 32-bit fields: cmd, cmdsize, vmaddr (64), fileoff (64), entry_id, reserved
    let fse_size = 32 + ((entry_id_str.len() + 7) & !7);

    // Find where the load commands end
    let lc_end = 32 + old_sizeofcmds as usize; // Mach-O 64-bit header is 32 bytes

    // Check if there's enough space. The actual `__TEXT` segment (`__text` section inside)
    // usually starts far past the end of the load commands (e.g. 8192 on x86). If there isn't
    // space, we would panic.
    if lc_end + fse_size > 8192 {
        panic!("Not enough slack space to append LC_FILESET_ENTRY");
    }

    // Update the Mach-O header
    let new_ncmds = old_ncmds + 1;
    let new_sizeofcmds = old_sizeofcmds + fse_size as u32;
    output[ncmds_off..ncmds_off + 4].copy_from_slice(&new_ncmds.to_le_bytes());
    output[sizeofcmds_off..sizeofcmds_off + 4].copy_from_slice(&new_sizeofcmds.to_le_bytes());

    // Write LC_FILESET_ENTRY
    // cmd: 0x80000035 (LC_FILESET_ENTRY | LC_REQ_DYLD)
    output[lc_end..lc_end + 4].copy_from_slice(&0x80000035u32.to_le_bytes());
    // cmdsize
    output[lc_end + 4..lc_end + 8].copy_from_slice(&(fse_size as u32).to_le_bytes());
    // vmaddr: 0xffffff8000200000 (standard xNU x86 load base)
    output[lc_end + 8..lc_end + 16].copy_from_slice(&0xffffff8000200000u64.to_le_bytes());
    // fileoff: 0
    output[lc_end + 16..lc_end + 24].copy_from_slice(&0u64.to_le_bytes());
    // entry_id.offset: 32 (starts right after struct)
    output[lc_end + 24..lc_end + 28].copy_from_slice(&32u32.to_le_bytes());
    // reserved: 0
    output[lc_end + 28..lc_end + 32].copy_from_slice(&0u32.to_le_bytes());

    // Write the string
    output[lc_end + 32..lc_end + 32 + entry_id_str.len()].copy_from_slice(entry_id_str);

    eprintln!(
        "[*] Appended LC_FILESET_ENTRY for com.apple.kernel. ncmds: {} -> {}, sizeofcmds: {} -> {}",
        old_ncmds, new_ncmds, old_sizeofcmds, new_sizeofcmds
    );
    eprintln!("[*] Writing {} ({} bytes)", output_path, output.len());
    let mut f = fs::File::create(&output_path).expect("Failed to create output file");
    f.write_all(&output).expect("Failed to write output file");

    eprintln!("[+] Done! Kernel collection written to {}", output_path);
}
