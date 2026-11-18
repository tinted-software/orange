//! The subset of the UEFI ABI the loader uses (UEFI 2.10 layouts).
//!
//! Only the tables and protocols actually called are typed; the rest of the
//! boot services table is padded with `usize` slots so every field used here
//! sits at its spec offset. Offsets are checked against the spec's function
//! order: boot service N (1-based) lives at `24 + 8 * (N - 1)`.

use core::ffi::c_void;

pub type Handle = *mut c_void;
pub type Status = usize;

pub const SUCCESS: Status = 0;
const ERROR_BIT: usize = 1 << (usize::BITS - 1);

pub fn is_error(status: Status) -> bool {
    status & ERROR_BIT != 0
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Guid {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

pub const LOADED_IMAGE_GUID: Guid = Guid {
    d1: 0x5B1B_31A1,
    d2: 0x9562,
    d3: 0x11D2,
    d4: [0x8E, 0x3F, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B],
};

pub const SIMPLE_FILE_SYSTEM_GUID: Guid = Guid {
    d1: 0x964E_5B22,
    d2: 0x6459,
    d3: 0x11D2,
    d4: [0x8E, 0x39, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B],
};

#[repr(C)]
pub struct TableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

#[repr(C)]
pub struct SystemTable {
    pub hdr: TableHeader,
    pub firmware_vendor: *const u16,
    pub firmware_revision: u32,
    pub console_in_handle: Handle,
    pub con_in: *mut c_void,
    pub console_out_handle: Handle,
    pub con_out: *mut c_void,
    pub standard_error_handle: Handle,
    pub std_err: *mut c_void,
    pub runtime_services: *mut c_void,
    pub boot_services: *mut BootServices,
    pub number_of_table_entries: usize,
    pub configuration_table: *mut c_void,
}

#[repr(C)]
pub struct BootServices {
    pub hdr: TableHeader,
    // 1-2: RaiseTPL, RestoreTPL
    pub raise_tpl: usize,
    pub restore_tpl: usize,
    // 3: AllocatePages(Type, MemoryType, Pages, *Memory)
    pub allocate_pages:
        unsafe extern "efiapi" fn(alloc_type: u32, mem_type: u32, pages: usize, memory: *mut u64) -> Status,
    // 4-5: FreePages, GetMemoryMap
    pub free_pages: usize,
    pub get_memory_map: usize,
    // 6-7: AllocatePool, FreePool
    pub allocate_pool: usize,
    pub free_pool: usize,
    // 8-13: CreateEvent, SetTimer, WaitForEvent, SignalEvent, CloseEvent, CheckEvent
    pub events: [usize; 6],
    // 14-16: Install/Reinstall/UninstallProtocolInterface
    pub protocol_interfaces: [usize; 3],
    // 17: HandleProtocol(Handle, *Guid, **Interface)
    pub handle_protocol:
        unsafe extern "efiapi" fn(handle: Handle, guid: *const Guid, iface: *mut *mut c_void) -> Status,
    // 18-19: Reserved, RegisterProtocolNotify
    pub reserved: usize,
    pub register_protocol_notify: usize,
    // 20-26: LocateHandle, LocateDevicePath, InstallConfigurationTable,
    //        LoadImage, StartImage, Exit, UnloadImage
    pub image_services: [usize; 7],
    // 27: ExitBootServices(ImageHandle, MapKey)
    pub exit_boot_services: unsafe extern "efiapi" fn(image: Handle, map_key: usize) -> Status,
}

#[repr(C)]
pub struct LoadedImage {
    pub revision: u32,
    pub parent_handle: Handle,
    pub system_table: *mut SystemTable,
    pub device_handle: Handle,
}

#[repr(C)]
pub struct SimpleFileSystem {
    pub revision: u64,
    pub open_volume: unsafe extern "efiapi" fn(this: *mut SimpleFileSystem, root: *mut *mut File) -> Status,
}

#[repr(C)]
pub struct File {
    pub revision: u64,
    pub open: unsafe extern "efiapi" fn(
        this: *mut File,
        new_handle: *mut *mut File,
        name: *const u16,
        mode: u64,
        attributes: u64,
    ) -> Status,
    pub close: usize,
    pub delete: usize,
    pub read: unsafe extern "efiapi" fn(this: *mut File, size: *mut usize, buffer: *mut c_void) -> Status,
    pub write: usize,
    pub get_position: usize,
    pub set_position: usize,
    pub get_info: usize,
}

pub const ALLOCATE_ANY_PAGES: u32 = 0;
pub const EFI_LOADER_DATA: u32 = 2;
pub const FILE_MODE_READ: u64 = 1;

#[repr(C)]
pub struct SimpleTextOutput {
    pub reset: usize,
    pub output_string: unsafe extern "efiapi" fn(this: *mut SimpleTextOutput, string: *const u16) -> Status,
}

// Compile-time checks of the offsets the loader depends on.
const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(BootServices, allocate_pages) == 24 + 2 * 8);
    assert!(offset_of!(BootServices, handle_protocol) == 24 + 16 * 8);
    assert!(offset_of!(BootServices, exit_boot_services) == 24 + 26 * 8);
    assert!(offset_of!(SystemTable, boot_services) == 96);
    assert!(offset_of!(LoadedImage, device_handle) == 24);
    assert!(offset_of!(File, get_info) == 64);
};
