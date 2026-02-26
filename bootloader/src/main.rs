//! XNU Bootloader for x86_64 UEFI
//!
//! Loads a Mach-O XNU kernel (kernel.development) and boots it on qemu-system-x86_64.
//! Follows the XNU boot protocol: _pstart is entered in 32-bit protected mode with
//! EAX pointing to a boot_args structure.

#![no_main]
#![no_std]

extern crate alloc;

#[macro_use]
mod serial;
mod boot_args;
mod devicetree;
mod loader;
mod trampoline;

use alloc::vec::Vec;
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode};
use uefi::{boot, CStr16};

use crate::boot_args::BootArgs;

#[cfg(not(test))]
#[panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    serial_println!("!!! PANIC !!!");
    if let Some(location) = info.location() {
        serial_println!(
            "  at {}:{}:{}: {}",
            location.file(),
            location.line(),
            location.column(),
            info.message()
        );
    } else {
        serial_println!("  {}", info.message());
    }
    loop {
        unsafe { core::arch::asm!("cli; hlt") };
    }
}

/// The kernel file to load from the EFI system partition.
const KERNEL_FILENAME: &str = "kernel.kc";

/// The ramdisk file to load from the EFI system partition (optional).
const RAMDISK_FILENAME: &str = "rootfs.dmg";

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();
    serial::init();
    serial_println!("=== XNU Bootloader ===");

    // Step 1: Load the kernel file
    serial_println!("[1] Loading kernel file...");
    let kernel_data = load_file(KERNEL_FILENAME);
    serial_println!("    Loaded {} bytes", kernel_data.len());

    // Load ramdisk (optional)
    serial_println!("[1b] Loading ramdisk...");
    let ramdisk_data = load_file_optional(RAMDISK_FILENAME);
    if let Some(ref rd) = ramdisk_data {
        serial_println!("    Loaded {} bytes", rd.len());
    } else {
        serial_println!("    No ramdisk found ({})", RAMDISK_FILENAME);
    }

    // Step 2: Parse Mach-O and stage segments
    serial_println!("[2] Parsing Mach-O...");
    let kernel_info = loader::load_kernel(&kernel_data);
    serial_println!(
        "    entry=0x{:x} kaddr=0x{:x} ksize=0x{:x}",
        kernel_info.entry_point,
        kernel_info.kaddr,
        kernel_info.ksize
    );

    // Step 3: Set up framebuffer
    serial_println!("[3] Setting up GOP...");
    let (fb_base, fb_size, width, height, stride) = setup_gop();
    serial_println!(
        "    {}x{} stride={} fb=0x{:x} size=0x{:x}",
        width,
        height,
        stride,
        fb_base,
        fb_size
    );

    // Step 4: Build device tree
    // Pre-compute ramdisk physical address: it goes right after the kernel.
    serial_println!("[4] Building device tree...");
    let rd_phys_precomputed = page_align(kernel_info.kaddr + kernel_info.ksize);
    let ramdisk_dt_info = ramdisk_data
        .as_ref()
        .map(|rd| (rd_phys_precomputed, rd.len() as u64));
    let dt_data = devicetree::build_device_tree(ramdisk_dt_info);
    serial_println!("    dt size={}", dt_data.len());

    // Capture EFI system table before exiting boot services.
    // XNU accesses it via ml_static_ptovirt which only maps 0..physfree,
    // so we must copy it into the boot data area. We also copy the
    // configuration table entries (needed for ACPI RSDP → MADT → CPU count).
    let st_ptr = uefi::table::system_table_raw().expect("Failed to get EFI system table pointer");
    let st = unsafe { &*st_ptr.as_ptr() };
    let st_size = st.header.size as usize;
    let mut st_copy = alloc::vec![0u8; st_size];
    unsafe {
        core::ptr::copy_nonoverlapping(st_ptr.as_ptr() as *const u8, st_copy.as_mut_ptr(), st_size);
    }
    let cfg_count = st.number_of_configuration_table_entries;
    let cfg_entry_size = core::mem::size_of::<uefi_raw::table::configuration::ConfigurationTable>();
    let cfg_total = cfg_count * cfg_entry_size;
    let mut cfg_copy = alloc::vec![0u8; cfg_total];
    if cfg_count > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(
                st.configuration_table as *const u8,
                cfg_copy.as_mut_ptr(),
                cfg_total,
            );
        }
    }
    serial_println!(
        "    EFI system table: {} bytes, {} config entries",
        st_size,
        cfg_count
    );

    // Step 5: Exit boot services
    serial_println!("[5] Exiting boot services...");
    let memory_map = unsafe { boot::exit_boot_services(None) };
    serial_println!("    Done.");

    // Step 6: Relocate kernel segments to final physical addresses
    serial_println!("[6] Relocating kernel...");
    unsafe {
        loader::relocate_kernel(&kernel_info);
    }
    serial_println!("    Done.");

    // Step 7: Place boot data structures right after the kernel
    //
    // XNU's Idle_PTs_init() only maps physical pages 0..physfree,
    // where physfree = kaddr + ksize. So boot_args, device tree,
    // memory map, and EFI system table MUST be within that range.
    //
    // Layout at kaddr + original_ksize:
    //   [device_tree]       (page-aligned)
    //   [memory_map]        (4 pages = 16KB)
    //   [efi_config_table]  (config table entries copy)
    //   [efi_system_table]  (system table copy, 8-byte aligned)
    //   [boot_args]         (1 page = 4096 bytes)
    //
    // Then ksize is expanded to cover everything.

    let mut cursor = page_align(kernel_info.kaddr + kernel_info.ksize);

    // 7a: Copy ramdisk into physical memory
    let ramdisk_info = if let Some(ref rd) = ramdisk_data {
        let rd_phys = cursor;
        assert_eq!(rd_phys, rd_phys_precomputed, "ramdisk address mismatch");
        let rd_pages = (rd.len() + 0xFFF) / 0x1000;
        unsafe {
            core::ptr::copy_nonoverlapping(rd.as_ptr(), rd_phys as *mut u8, rd.len());
            let remainder = rd_pages * 0x1000 - rd.len();
            if remainder > 0 {
                core::ptr::write_bytes((rd_phys as *mut u8).add(rd.len()), 0, remainder);
            }
        }
        cursor += (rd_pages * 0x1000) as u64;
        serial_println!("    ramdisk at 0x{:x}, {} bytes", rd_phys, rd.len());
        Some((rd_phys, rd.len() as u64))
    } else {
        None
    };

    // 7b: Copy device tree (already built before ExitBootServices)

    let dt_phys = cursor;
    let dt_pages = (dt_data.len() + 0xFFF) / 0x1000;
    unsafe {
        core::ptr::copy_nonoverlapping(dt_data.as_ptr(), dt_phys as *mut u8, dt_data.len());
        let remainder = dt_pages * 0x1000 - dt_data.len();
        if remainder > 0 {
            core::ptr::write_bytes((dt_phys as *mut u8).add(dt_data.len()), 0, remainder);
        }
    }
    cursor += (dt_pages * 0x1000) as u64;
    serial_println!("    dt at 0x{:x}, {} bytes", dt_phys, dt_data.len());

    // 7b: Write memory map (4 pages)
    let mmap_phys = cursor;
    let mmap_pages = 4;
    unsafe {
        core::ptr::write_bytes(mmap_phys as *mut u8, 0, mmap_pages * 0x1000);
    }
    cursor += (mmap_pages * 0x1000) as u64;

    let mmap_size = unsafe { write_memory_map(mmap_phys, &memory_map) };
    serial_println!("    mmap at 0x{:x}, {} bytes", mmap_phys, mmap_size);

    // Compute total usable memory from the memory map
    let total_mem = unsafe { count_usable_memory(&memory_map) };

    // 7c: Copy EFI configuration table entries into boot area
    let cfg_phys = cursor;
    unsafe {
        core::ptr::copy_nonoverlapping(cfg_copy.as_ptr(), cfg_phys as *mut u8, cfg_total);
        // Zero-pad to 8-byte alignment
        let aligned = (cfg_total + 7) & !7;
        if aligned > cfg_total {
            core::ptr::write_bytes((cfg_phys as *mut u8).add(cfg_total), 0, aligned - cfg_total);
        }
        cursor += aligned as u64;
    }
    serial_println!(
        "    efi cfg table at 0x{:x}, {} entries",
        cfg_phys,
        cfg_count
    );

    // 7d: Copy EFI system table into boot area, patch for XNU compatibility
    let est_phys = cursor;
    unsafe {
        core::ptr::copy_nonoverlapping(st_copy.as_ptr(), est_phys as *mut u8, st_size);
        let aligned = (st_size + 7) & !7;
        if aligned > st_size {
            core::ptr::write_bytes((est_phys as *mut u8).add(st_size), 0, aligned - st_size);
        }
        cursor += aligned as u64;

        // Patch: point ConfigurationTable to our copy in the boot area,
        // zero RuntimeServices (we don't support SetVirtualAddressMap),
        // and recompute the header CRC32.
        #[repr(C)]
        struct EfiSystemTable64 {
            signature: u64,
            revision: u32,
            header_size: u32,
            crc32: u32,
            reserved: u32,
            firmware_vendor: u64,
            firmware_revision: u32,
            _pad: u32,
            console_in_handle: u64,
            con_in: u64,
            console_out_handle: u64,
            con_out: u64,
            stderr_handle: u64,
            std_err: u64,
            runtime_services: u64,
            boot_services: u64,
            number_of_table_entries: u64,
            configuration_table: u64,
        }
        let est = &mut *(est_phys as *mut EfiSystemTable64);
        est.configuration_table = cfg_phys;
        est.runtime_services = 0;
        est.boot_services = 0;
        // Recompute CRC32 over header_size bytes with crc32 field zeroed
        est.crc32 = 0;
        let hdr_size = est.header_size as usize;
        est.crc32 = crc32(core::slice::from_raw_parts(est_phys as *const u8, hdr_size));
    }
    serial_println!("    efi sys table at 0x{:x}, {} bytes", est_phys, st_size);

    // 7e: Write boot_args (1 page)
    let ba_phys = cursor;
    cursor += 0x1000;

    // Update ksize to encompass everything we placed
    let final_ksize = cursor - kernel_info.kaddr;

    unsafe {
        let ba = &mut *(ba_phys as *mut BootArgs);
        *ba = BootArgs::zeroed();

        ba.revision = 1;
        ba.version = 2;
        ba.efi_mode = 64;

        let cmdline = if ramdisk_info.is_some() {
            &b"debug=0x14e serial=3 keepsyms=1 x2apic=0 io=0x1 rd=md0 cpus=1 -v"[..]
        } else {
            &b"debug=0x14e serial=3 keepsyms=1 x2apic=0 io=0x1 cpus=1 -v"[..]
        };
        ba.command_line[..cmdline.len()].copy_from_slice(cmdline);

        ba.kaddr = kernel_info.kaddr as u32;
        ba.ksize = final_ksize as u32;

        ba.device_tree_p = dt_phys as u32;
        ba.device_tree_length = dt_data.len() as u32;

        ba.memory_map = mmap_phys as u32;
        ba.memory_map_size = mmap_size as u32;
        ba.memory_map_descriptor_size = core::mem::size_of::<EfiMemoryRange>() as u32;
        ba.memory_map_descriptor_version = 1;

        // Video (V1 - legacy)
        ba.video_v1.v_base_addr = fb_base as u32;
        ba.video_v1.v_display = 1;
        ba.video_v1.v_row_bytes = stride * 4;
        ba.video_v1.v_width = width;
        ba.video_v1.v_height = height;
        ba.video_v1.v_depth = 32;

        // Video (V2)
        ba.video.v_display = 1;
        ba.video.v_row_bytes = stride * 4;
        ba.video.v_width = width;
        ba.video.v_height = height;
        ba.video.v_depth = 32;
        ba.video.v_base_addr = fb_base;

        ba.efi_system_table = est_phys as u32;

        ba.physical_memory_size = total_mem;
        ba.boot_mem_start = 0;
        ba.boot_mem_size = total_mem;
        ba.fsb_frequency = 100_000_000;
        ba.csr_active_config = 0x7F;

        // KC headers virtual address: virtual addr of the Mach-O header.
        // This is the __TEXT segment base (fileoff=0), NOT kaddr (which may
        // be __HIB or __NULL at a lower address).
        // With revision >= 1 and KC_hdrs_vaddr != 0, XNU calls
        // i386_slide_and_rebase_image() → PE_set_kc_header(KCKindPrimary).
        ba.kc_hdrs_vaddr = kernel_info.mach_header_phys + 0xffff_ff80_0000_0000;
    }

    serial_println!("    boot_args at 0x{:x}", ba_phys);
    serial_println!(
        "    ksize expanded: 0x{:x} -> 0x{:x}",
        kernel_info.ksize,
        final_ksize
    );
    serial_println!(
        "    physfree will be 0x{:x}",
        kernel_info.kaddr + final_ksize
    );

    // Step 8: Jump to the kernel
    serial_println!(
        "[8] Jumping to kernel at 0x{:x}...",
        kernel_info.entry_point
    );
    unsafe {
        trampoline::jump_to_kernel(kernel_info.entry_point as u32, ba_phys as u32);
    }
}

fn page_align(addr: u64) -> u64 {
    (addr + 0xFFF) & !0xFFF
}

/// Write the EFI memory map into a buffer. Returns the total bytes written.
unsafe fn write_memory_map(
    buf_phys: u64,
    memory_map: &uefi::mem::memory_map::MemoryMapOwned,
) -> usize {
    let mmap_ptr = buf_phys as *mut u8;
    let mut offset = 0usize;
    let desc_size = core::mem::size_of::<EfiMemoryRange>();

    // EFI_MEMORY_RUNTIME attribute bit. We strip this because our bootloader
    // doesn't call SetVirtualAddressMap, so VirtualStart is 0 for all runtime
    // entries. The kernel would map them all at VM_MIN_KERNEL_ADDRESS,
    // overwriting itself. Since we don't support EFI runtime services, the
    // runtime mappings are unnecessary.
    const EFI_MEMORY_RUNTIME: u64 = 0x8000_0000_0000_0000;

    for desc in memory_map.entries() {
        let range = EfiMemoryRange {
            memory_type: desc.ty.0,
            pad: 0,
            physical_start: desc.phys_start,
            virtual_start: desc.virt_start,
            number_of_pages: desc.page_count,
            attribute: desc.att.bits() & !EFI_MEMORY_RUNTIME,
        };

        core::ptr::copy_nonoverlapping(
            &range as *const EfiMemoryRange as *const u8,
            mmap_ptr.add(offset),
            desc_size,
        );
        offset += desc_size;
    }

    offset
}

/// Count total usable memory from the EFI memory map.
unsafe fn count_usable_memory(memory_map: &uefi::mem::memory_map::MemoryMapOwned) -> u64 {
    use uefi::boot::MemoryType;
    let mut total: u64 = 0;
    for desc in memory_map.entries() {
        match desc.ty {
            MemoryType::CONVENTIONAL
            | MemoryType::BOOT_SERVICES_CODE
            | MemoryType::BOOT_SERVICES_DATA
            | MemoryType::LOADER_CODE
            | MemoryType::LOADER_DATA => {
                total += desc.page_count * 0x1000;
            }
            _ => {}
        }
    }
    total
}

/// Load a file from the EFI filesystem.
fn load_file(filename: &str) -> Vec<u8> {
    let mut fs = boot::get_image_file_system(boot::image_handle()).unwrap();
    let mut root = fs.open_volume().unwrap();

    let mut name_buf = [0u16; 64];
    let name = {
        let bytes = filename.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            name_buf[i] = b as u16;
        }
        name_buf[bytes.len()] = 0;
        unsafe { CStr16::from_u16_with_nul_unchecked(&name_buf[..=bytes.len()]) }
    };

    let file_handle = root
        .open(name, FileMode::Read, FileAttribute::empty())
        .unwrap_or_else(|_| panic!("Failed to open {filename}"));

    let mut file = file_handle.into_regular_file().expect("Not a regular file");

    let mut info_buf = alloc::vec![0u8; 256];
    let info: &FileInfo = file.get_info(&mut info_buf).unwrap();
    let file_size = info.file_size() as usize;
    serial_println!("    File size: {} bytes", file_size);

    let mut data = alloc::vec![0u8; file_size];
    let bytes_read = file.read(&mut data).expect("Failed to read file");
    data.truncate(bytes_read);
    data
}

/// Load an optional file from the EFI filesystem (returns None if not found).
fn load_file_optional(filename: &str) -> Option<Vec<u8>> {
    let mut fs = boot::get_image_file_system(boot::image_handle()).unwrap();
    let mut root = fs.open_volume().unwrap();

    let mut name_buf = [0u16; 64];
    let name = {
        let bytes = filename.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            name_buf[i] = b as u16;
        }
        name_buf[bytes.len()] = 0;
        unsafe { CStr16::from_u16_with_nul_unchecked(&name_buf[..=bytes.len()]) }
    };

    let file_handle = match root.open(name, FileMode::Read, FileAttribute::empty()) {
        Ok(h) => h,
        Err(_) => return None,
    };

    let mut file = file_handle.into_regular_file().expect("Not a regular file");

    let mut info_buf = alloc::vec![0u8; 256];
    let info: &FileInfo = file.get_info(&mut info_buf).unwrap();
    let file_size = info.file_size() as usize;
    serial_println!("    File size: {} bytes", file_size);

    let mut data = alloc::vec![0u8; file_size];
    let bytes_read = file.read(&mut data).expect("Failed to read file");
    data.truncate(bytes_read);
    Some(data)
}

/// Set up the Graphics Output Protocol and return framebuffer info.
fn setup_gop() -> (u64, usize, u32, u32, u32) {
    use uefi::proto::console::gop::GraphicsOutput;

    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>().unwrap();
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle).unwrap();

    let mode = gop
        .modes()
        .find(|m| m.info().resolution() == (1024, 768))
        .or_else(|| gop.modes().last());

    if let Some(mode) = mode {
        gop.set_mode(&mode).unwrap();
    }

    let mode_info = gop.current_mode_info();
    let (width, height) = mode_info.resolution();
    let stride = mode_info.stride() as u32;
    let mut fb = gop.frame_buffer();
    let fb_base = fb.as_mut_ptr() as u64;
    let fb_size = fb.size();

    (fb_base, fb_size, width as u32, height as u32, stride)
}

/// EFI memory range descriptor matching XNU's `EfiMemoryRange`.
#[derive(Clone, Copy)]
#[repr(C)]
struct EfiMemoryRange {
    memory_type: u32,
    pad: u32,
    physical_start: u64,
    virtual_start: u64,
    number_of_pages: u64,
    attribute: u64,
}

/// Standard CRC32 (polynomial 0xEDB88320) matching XNU's crc32().
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}
