//! x86_64 UEFI bootloader for XNU.
//!
//! Stage 1 (this file): read `\mach_kernel` from the boot volume and list its
//! segments. Placement and the handoff to `pstart` are next.

#![no_std]
#![no_main]

#[macro_use]
mod log {
    #[macro_export]
    macro_rules! println {
        ($($arg:tt)*) => {
            $crate::serial::_print(format_args!("{}\n", format_args!($($arg)*)))
        };
    }
}

mod allocator;
mod efi;
mod macho;
mod serial;

use core::ffi::c_void;
use core::panic::PanicInfo;
use core::ptr::null_mut;

use efi::*;

const KERNEL_PATH: &str = "\\mach_kernel";
/// Room for the kernel file (24 MB today).
const KERNEL_BUFFER_SIZE: usize = 64 * 1024 * 1024;

type R<T> = Result<T, &'static str>;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    println!("uefi-loader: panic");
    loop {}
}

fn check(status: Status, what: &'static str) -> R<()> {
    if is_error(status) {
        println!("uefi-loader: {} failed: status {:#x}", what, status);
        Err(what)
    } else {
        Ok(())
    }
}

/// HandleProtocol for a protocol of type `T` on `handle`.
unsafe fn protocol<T>(bs: &BootServices, handle: Handle, guid: &Guid) -> R<*mut T> {
    let mut iface: *mut c_void = null_mut();
    check((bs.handle_protocol)(handle, guid, &mut iface), "HandleProtocol")?;
    Ok(iface as *mut T)
}

/// A NUL-terminated UTF-16 path (ASCII only).
fn utf16_path<const N: usize>(path: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (slot, byte) in out.iter_mut().zip(path.bytes()) {
        *slot = byte as u16;
    }
    out
}

/// Reads `\mach_kernel` from the volume the loader was started from into
/// page-aligned memory, and returns the bytes.
unsafe fn read_kernel(bs: &BootServices, image: Handle) -> R<&'static [u8]> {
    let loaded: *mut LoadedImage = protocol(bs, image, &LOADED_IMAGE_GUID)?;
    let fs: *mut SimpleFileSystem = protocol(bs, (*loaded).device_handle, &SIMPLE_FILE_SYSTEM_GUID)?;

    let mut root: *mut File = null_mut();
    check(((*fs).open_volume)(fs, &mut root), "OpenVolume")?;

    let name: [u16; 32] = utf16_path(KERNEL_PATH);
    let mut file: *mut File = null_mut();
    check(((*root).open)(root, &mut file, name.as_ptr(), FILE_MODE_READ, 0), "Open")?;

    // Read the whole file into a buffer with room to spare, and take the byte
    // count Read reports. (The file-size query is not used: GetInfo returned
    // EFI_UNSUPPORTED for the file-info GUID here, and a probe Read succeeds
    // rather than reporting the size.)
    let pages = KERNEL_BUFFER_SIZE / 4096;
    let mut buffer: u64 = 0;
    check((bs.allocate_pages)(ALLOCATE_ANY_PAGES, EFI_LOADER_DATA, pages, &mut buffer), "AllocatePages")?;
    let mut read = KERNEL_BUFFER_SIZE;
    check(((*file).read)(file, &mut read, buffer as *mut c_void), "Read")?;
    if read == KERNEL_BUFFER_SIZE {
        return Err("kernel file fills the read buffer");
    }
    println!("uefi-loader: read {} ({} bytes) at {:#x}", KERNEL_PATH, read, buffer);
    Ok(core::slice::from_raw_parts(buffer as *const u8, read))
}

fn run(image: Handle, st: *mut SystemTable) -> R<()> {
    unsafe {
        let bs = &*(*st).boot_services;
        println!("uefi-loader: started");
        let kernel = read_kernel(bs, image)?;
        macho::list_segments(kernel)?;
    }
    Ok(())
}

#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(image: Handle, st: *mut SystemTable) -> Status {
    match run(image, st) {
        Ok(()) => SUCCESS,
        Err(reason) => {
            println!("uefi-loader: error: {}", reason);
            1
        }
    }
}
