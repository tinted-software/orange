//! Mach-O parsing with the `object` crate (read_core: no allocation).

use object::macho;
use object::read::macho::{MachHeader, Segment};
use object::Endianness;

/// Lists the 64-bit segments of a Mach-O image. Used to check that the
/// kernel file read from the ESP parses before anything is placed.
pub fn list_segments(data: &[u8]) -> Result<(), &'static str> {
    let header = macho::MachHeader64::<Endianness>::parse(data, 0)
        .map_err(|_| "not a 64-bit Mach-O header")?;
    let endian = header.endian().map_err(|_| "unknown endianness")?;
    let filetype = header.filetype(endian);
    let ncmds = header.ncmds(endian);
    crate::println!("  mach-o: filetype={:#x} ncmds={}", filetype, ncmds);

    let mut commands = header
        .load_commands(endian, data, 0)
        .map_err(|_| "failed to iterate load commands")?;
    while let Ok(Some(command)) = commands.next() {
        if let Ok(Some((seg, _))) = command.segment_64() {
            let name = seg.name();
            let name = core::str::from_utf8(name).unwrap_or("<invalid>").trim_end_matches('\0');
            crate::println!(
                "  segment {:<16} vm={:#018x} vmsize={:#010x} fileoff={:#010x} filesize={:#010x}",
                name,
                seg.vmaddr(endian),
                seg.vmsize(endian),
                seg.fileoff(endian),
                seg.filesize(endian),
            );
        }
    }
    Ok(())
}
