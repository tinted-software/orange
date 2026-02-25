//! Mach-O kernel loader using goblin.
//!
//! Parses the XNU kernel Mach-O binary and prepares segments for loading.
//! Since UEFI firmware often reserves low memory (0-2MB), we can't directly
//! allocate at the kernel's target physical addresses during boot services.
//!
//! Strategy:
//!   1. Parse Mach-O, extract segment info and entry point
//!   2. Allocate a staging buffer in high memory
//!   3. Copy all segment file data into the staging buffer
//!   4. After ExitBootServices, copy from staging to final physical addresses

use alloc::vec::Vec;
use goblin::mach::load_command::CommandVariant;
use goblin::mach::Mach;
use uefi::boot::{self, AllocateType, MemoryType};

/// Physical address base: XNU maps vmaddr 0xffffff80_00000000 → physical 0x0.
const KERNEL_BASE: u64 = 0xffffff8000000000;

/// Information about a kernel segment to be relocated.
pub struct SegmentLoad {
    /// Target physical address (vmaddr - KERNEL_BASE)
    pub phys_addr: u64,
    /// Size in memory (vmsize)
    pub vmsize: u64,
    /// Offset into the staging buffer where file data starts
    pub staging_offset: u64,
    /// Amount of file data (filesize)
    pub filesize: u64,
}

/// Information about the loaded kernel.
pub struct KernelInfo {
    /// Physical address of _pstart (the 32-bit entry point).
    pub entry_point: u64,
    /// Lowest physical address of loaded kernel (kaddr).
    pub kaddr: u64,
    /// Total size of loaded kernel image (ksize).
    pub ksize: u64,
    /// Physical address of the Mach-O header (segment with fileoff=0).
    pub mach_header_phys: u64,
    /// Staging buffer physical address (allocated in high memory).
    pub staging_base: u64,
    /// Segments to be relocated after ExitBootServices.
    pub segments: Vec<SegmentLoad>,
}

/// Parse the XNU kernel Mach-O and prepare it for loading.
pub fn load_kernel(data: &[u8]) -> KernelInfo {
    let mach = Mach::parse(data).expect("Failed to parse Mach-O");
    let macho = match mach {
        Mach::Binary(m) => m,
        _ => panic!("Expected a single Mach-O binary, not a fat binary"),
    };

    serial_println!("    {} load commands", macho.load_commands.len());

    // Collect segments and entry point
    let mut segments: Vec<SegmentLoad> = Vec::new();
    let mut entry_point: u64 = 0;
    let mut lowest_phys: u64 = u64::MAX;
    let mut highest_phys: u64 = 0;
    let mut staging_offset: u64 = 0;
    let mut mach_header_phys: u64 = 0;

    // Also store file offsets for the copy phase
    let mut file_offsets: Vec<(u64, u64)> = Vec::new(); // (fileoff, filesize) parallel to segments

    for lc in &macho.load_commands {
        match lc.command {
            CommandVariant::Segment64(ref seg) => {
                let name = core::str::from_utf8(&seg.segname)
                    .unwrap_or("???")
                    .trim_end_matches('\0');

                serial_println!(
                    "    {:16} va=0x{:x} sz=0x{:x} fo=0x{:x} fs=0x{:x}",
                    name,
                    seg.vmaddr,
                    seg.vmsize,
                    seg.fileoff,
                    seg.filesize
                );

                if seg.vmsize == 0 {
                    continue;
                }

                let phys_addr = seg.vmaddr - KERNEL_BASE;
                // The Mach-O header is at the start of the segment with fileoff=0
                if seg.fileoff == 0 {
                    mach_header_phys = phys_addr;
                }
                let cur_offset = staging_offset;
                staging_offset += seg.filesize;

                file_offsets.push((seg.fileoff as u64, seg.filesize));

                segments.push(SegmentLoad {
                    phys_addr,
                    vmsize: seg.vmsize,
                    staging_offset: cur_offset,
                    filesize: seg.filesize,
                });

                if phys_addr < lowest_phys {
                    lowest_phys = phys_addr;
                }
                let seg_end = phys_addr + seg.vmsize;
                if seg_end > highest_phys {
                    highest_phys = seg_end;
                }
            }
            CommandVariant::Unixthread(ref _ut) => {
                let cmd_offset = lc.offset;
                let state_offset = cmd_offset + 16; // skip cmd+cmdsize+flavor+count
                if state_offset + 17 * 8 <= data.len() {
                    let rip_offset = state_offset + 16 * 8;
                    entry_point =
                        u64::from_le_bytes(data[rip_offset..rip_offset + 8].try_into().unwrap());
                    serial_println!("    LC_UNIXTHREAD rip=0x{:x}", entry_point);
                }
            }
            _ => {}
        }
    }

    if entry_point == 0 {
        panic!("No entry point found in kernel Mach-O!");
    }

    let entry_phys = entry_point - KERNEL_BASE;
    serial_println!(
        "    phys range: 0x{:x}-0x{:x} entry=0x{:x}",
        lowest_phys,
        highest_phys,
        entry_phys
    );

    // Allocate staging buffer in high memory
    let total_filedata = staging_offset;
    let staging_pages = ((total_filedata + 0xFFF) / 0x1000) as usize;
    let staging_base = boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        staging_pages,
    )
    .expect("Failed to allocate staging buffer")
    .as_ptr() as u64;

    serial_println!(
        "    staging: 0x{:x} ({} pages)",
        staging_base,
        staging_pages
    );

    // Copy segment file data into staging buffer
    for (i, seg) in segments.iter().enumerate() {
        if seg.filesize == 0 {
            continue;
        }
        let (fileoff, filesize) = file_offsets[i];
        let src_start = fileoff as usize;
        let src_end = src_start + filesize as usize;
        let dst = (staging_base + seg.staging_offset) as *mut u8;

        unsafe {
            core::ptr::copy_nonoverlapping(
                data[src_start..src_end].as_ptr(),
                dst,
                filesize as usize,
            );
        }
    }

    serial_println!("    staging copy complete");

    KernelInfo {
        entry_point: entry_phys,
        kaddr: lowest_phys,
        ksize: highest_phys - lowest_phys,
        mach_header_phys,
        staging_base,
        segments,
    }
}

/// Relocate kernel segments from staging buffer to their final physical addresses.
///
/// Must be called after ExitBootServices when we own all physical memory.
///
/// # Safety
/// The target physical address ranges must be valid memory that's not in use.
pub unsafe fn relocate_kernel(info: &KernelInfo) {
    for seg in &info.segments {
        let dst = seg.phys_addr as *mut u8;

        // Copy file data from staging
        if seg.filesize > 0 {
            let src = (info.staging_base + seg.staging_offset) as *const u8;
            core::ptr::copy_nonoverlapping(src, dst, seg.filesize as usize);
        }

        // Zero BSS (vmsize - filesize)
        if seg.vmsize > seg.filesize {
            let bss_start = dst.add(seg.filesize as usize);
            let bss_size = (seg.vmsize - seg.filesize) as usize;
            core::ptr::write_bytes(bss_start, 0, bss_size);
        }
    }
}
