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
use crate::loader::KernelInfo;

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
const KERNEL_FILENAME: &str = "kernel.development";

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();
    serial::init();
    serial_println!("=== XNU Bootloader ===");

    // Step 1: Load the kernel file from the EFI filesystem
    serial_println!("[1] Loading kernel file...");
    let kernel_data = load_kernel_file();
    serial_println!("    Loaded {} bytes", kernel_data.len());

    // Step 2: Parse the Mach-O and load segments into staging buffer
    serial_println!("[2] Parsing Mach-O...");
    let kernel_info = loader::load_kernel(&kernel_data);
    serial_println!(
        "    entry=0x{:x} kaddr=0x{:x} ksize=0x{:x}",
        kernel_info.entry_point,
        kernel_info.kaddr,
        kernel_info.ksize
    );

    // Step 3: Set up the framebuffer via GOP
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

    // Step 4: Build a minimal device tree
    serial_println!("[4] Building device tree...");
    let dt_data = devicetree::build_device_tree();
    let dt_phys = allocate_and_copy(&dt_data);
    serial_println!("    dt=0x{:x} len={}", dt_phys, dt_data.len());

    // Step 5: Build boot_args structure
    serial_println!("[5] Building boot_args...");
    let boot_args_phys = build_boot_args(
        &kernel_info,
        fb_base,
        width,
        height,
        stride,
        dt_phys,
        dt_data.len() as u32,
    );
    serial_println!("    boot_args=0x{:x}", boot_args_phys);

    // Step 6: Pre-allocate buffer for the EFI memory map
    let mmap_buf_phys = {
        use uefi::boot::{AllocateType, MemoryType};
        boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, 4)
            .expect("Failed to allocate memory map buffer")
            .as_ptr() as u64
    };

    // Step 7: Exit boot services
    serial_println!("[7] Exiting boot services...");
    let memory_map = unsafe { boot::exit_boot_services(None) };
    serial_println!("    Done.");

    // Step 8: Fill memory map
    unsafe {
        fill_memory_map(boot_args_phys, mmap_buf_phys, &memory_map);
    }
    serial_println!("[8] Memory map written.");

    // Step 9: Relocate kernel
    serial_println!("[9] Relocating kernel...");
    unsafe {
        loader::relocate_kernel(&kernel_info);
    }
    serial_println!("    Done.");

    // Step 10: Jump to the kernel
    serial_println!(
        "[10] Jumping to kernel at 0x{:x}...",
        kernel_info.entry_point
    );
    unsafe {
        trampoline::jump_to_kernel(kernel_info.entry_point as u32, boot_args_phys as u32);
    }
}

/// Load kernel file from the EFI filesystem.
fn load_kernel_file() -> Vec<u8> {
    let mut fs = boot::get_image_file_system(boot::image_handle()).unwrap();
    let mut root = fs.open_volume().unwrap();

    // Convert filename to UCS-2
    let mut name_buf = [0u16; 64];
    let name = {
        let bytes = KERNEL_FILENAME.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            name_buf[i] = b as u16;
        }
        name_buf[bytes.len()] = 0;
        unsafe { CStr16::from_u16_with_nul_unchecked(&name_buf[..=bytes.len()]) }
    };

    let file_handle = root
        .open(name, FileMode::Read, FileAttribute::empty())
        .expect("Failed to open kernel file");

    let mut file = file_handle
        .into_regular_file()
        .expect("Kernel is not a regular file");

    // Get file size
    let mut info_buf = alloc::vec![0u8; 256];
    let info: &FileInfo = file.get_info(&mut info_buf).unwrap();
    let file_size = info.file_size() as usize;
    serial_println!("    File size: {} bytes", file_size);

    // Read the entire file
    let mut data = alloc::vec![0u8; file_size];
    let bytes_read = file.read(&mut data).expect("Failed to read kernel");
    data.truncate(bytes_read);
    data
}

/// Set up the Graphics Output Protocol and return framebuffer info.
fn setup_gop() -> (u64, usize, u32, u32, u32) {
    use uefi::proto::console::gop::GraphicsOutput;

    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>().unwrap();
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle).unwrap();

    // Find a good mode (prefer 1024x768 or similar)
    let mode = gop
        .modes()
        .find(|m| {
            let info = m.info();
            info.resolution() == (1024, 768)
        })
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

/// Allocate pages and copy data there, returning the physical address.
fn allocate_and_copy(data: &[u8]) -> u64 {
    use uefi::boot::{AllocateType, MemoryType};

    let pages = (data.len() + 0xFFF) / 0x1000;
    let addr = boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages)
        .expect("Failed to allocate pages for data")
        .as_ptr() as u64;

    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), addr as *mut u8, data.len());
    }

    addr
}

/// Build the boot_args structure and place it in memory.
fn build_boot_args(
    kernel_info: &KernelInfo,
    fb_base: u64,
    fb_width: u32,
    fb_height: u32,
    fb_stride: u32,
    dt_phys: u64,
    dt_length: u32,
) -> u64 {
    use uefi::boot::{AllocateType, MemoryType};

    // Allocate a page for boot_args (must be 4096 bytes)
    let ba_addr = boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, 1)
        .expect("Failed to allocate boot_args page")
        .as_ptr() as u64;

    let ba = unsafe { &mut *(ba_addr as *mut BootArgs) };
    *ba = BootArgs::zeroed();

    ba.revision = 0;
    ba.version = 2;
    ba.efi_mode = 64;
    ba.debug_mode = 0;
    ba.flags = 0;

    let cmdline = b"debug=0x14e serial=3 -v";
    ba.command_line[..cmdline.len()].copy_from_slice(cmdline);

    ba.kaddr = kernel_info.kaddr as u32;
    ba.ksize = kernel_info.ksize as u32;

    ba.device_tree_p = dt_phys as u32;
    ba.device_tree_length = dt_length;

    // Video (V1 - legacy)
    ba.video_v1.v_base_addr = fb_base as u32;
    ba.video_v1.v_display = 1; // GRAPHICS_MODE
    ba.video_v1.v_row_bytes = fb_stride * 4;
    ba.video_v1.v_width = fb_width;
    ba.video_v1.v_height = fb_height;
    ba.video_v1.v_depth = 32;

    // Video (V2)
    ba.video.v_display = 1;
    ba.video.v_row_bytes = fb_stride * 4;
    ba.video.v_width = fb_width;
    ba.video.v_height = fb_height;
    ba.video.v_depth = 32;
    ba.video.v_base_addr = fb_base;

    ba.physical_memory_size = 512 * 1024 * 1024; // Will be updated from memmap
    ba.boot_mem_start = 0;
    ba.boot_mem_size = 512 * 1024 * 1024;
    ba.fsb_frequency = 100_000_000;

    ba.efi_runtime_services_page_start = 0;
    ba.efi_runtime_services_page_count = 0;
    ba.efi_runtime_services_virtual_page_start = 0;
    ba.efi_system_table = 0;
    ba.kslide = 0;
    ba.csr_active_config = 0x7F;
    ba.csr_capabilities = 0;

    ba_addr
}

/// Fill the memory map into boot_args after ExitBootServices.
unsafe fn fill_memory_map(
    boot_args_phys: u64,
    mmap_buf_phys: u64,
    memory_map: &uefi::mem::memory_map::MemoryMapOwned,
) {
    use uefi::boot::MemoryType;

    let ba = &mut *(boot_args_phys as *mut BootArgs);
    let mmap_ptr = mmap_buf_phys as *mut u8;

    let mut offset = 0usize;
    let mut total_mem: u64 = 0;
    let desc_size = core::mem::size_of::<EfiMemoryRange>();

    for desc in memory_map.entries() {
        let range = EfiMemoryRange {
            memory_type: desc.ty.0,
            pad: 0,
            physical_start: desc.phys_start,
            virtual_start: desc.virt_start,
            number_of_pages: desc.page_count,
            attribute: desc.att.bits(),
        };

        core::ptr::copy_nonoverlapping(
            &range as *const EfiMemoryRange as *const u8,
            mmap_ptr.add(offset),
            desc_size,
        );
        offset += desc_size;

        match desc.ty {
            MemoryType::CONVENTIONAL
            | MemoryType::BOOT_SERVICES_CODE
            | MemoryType::BOOT_SERVICES_DATA
            | MemoryType::LOADER_CODE
            | MemoryType::LOADER_DATA => {
                total_mem += desc.page_count * 0x1000;
            }
            _ => {}
        }
    }

    ba.memory_map = mmap_buf_phys as u32;
    ba.memory_map_size = offset as u32;
    ba.memory_map_descriptor_size = desc_size as u32;
    ba.memory_map_descriptor_version = 1;

    if total_mem > 0 {
        ba.physical_memory_size = total_mem;
        ba.boot_mem_size = total_mem;
    }
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
