//! XNU boot_args structure for x86_64 (from pexpert/pexpert/i386/boot.h)
//!
//! This is the 4096-byte boot_args structure that XNU expects to receive
//! from the bootloader. Version 2, Revision 0/1.

/// Boot line length (command line buffer size).
pub const BOOT_LINE_LENGTH: usize = 1024;

/// Video information (legacy V1 format).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BootVideoV1 {
    pub v_base_addr: u32,
    pub v_display: u32,
    pub v_row_bytes: u32,
    pub v_width: u32,
    pub v_height: u32,
    pub v_depth: u32,
}

/// Video information (V2 format with 64-bit base address).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BootVideo {
    pub v_display: u32,
    pub v_row_bytes: u32,
    pub v_width: u32,
    pub v_height: u32,
    pub v_depth: u32,
    pub v_rotate: u8,
    pub v_resv_byte: [u8; 3],
    pub v_resv: [u32; 6],
    pub v_base_addr: u64,
}

/// The main boot_args structure (4096 bytes).
/// Based on XNU's `typedef struct boot_args` from i386/boot.h.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BootArgs {
    pub revision: u16,
    pub version: u16,

    pub efi_mode: u8,
    pub debug_mode: u8,
    pub flags: u16,

    pub command_line: [u8; BOOT_LINE_LENGTH],

    pub memory_map: u32,
    pub memory_map_size: u32,
    pub memory_map_descriptor_size: u32,
    pub memory_map_descriptor_version: u32,

    pub video_v1: BootVideoV1,

    pub device_tree_p: u32,
    pub device_tree_length: u32,

    pub kaddr: u32,
    pub ksize: u32,

    pub efi_runtime_services_page_start: u32,
    pub efi_runtime_services_page_count: u32,
    pub efi_runtime_services_virtual_page_start: u64,

    pub efi_system_table: u32,
    pub kslide: u32,

    pub performance_data_start: u32,
    pub performance_data_size: u32,

    pub key_store_data_start: u32,
    pub key_store_data_size: u32,
    pub boot_mem_start: u64,
    pub boot_mem_size: u64,
    pub physical_memory_size: u64,
    pub fsb_frequency: u64,
    pub pci_config_space_base_address: u64,
    pub pci_config_space_start_bus_number: u32,
    pub pci_config_space_end_bus_number: u32,
    pub csr_active_config: u32,
    pub csr_capabilities: u32,
    pub boot_smc_plimit: u32,
    pub boot_progress_meter_start: u16,
    pub boot_progress_meter_end: u16,
    pub video: BootVideo,

    pub apfs_data_start: u32,
    pub apfs_data_size: u32,

    // Version 2, Revision 1
    pub kc_hdrs_vaddr: u64,

    pub arv_root_hash_start: u64,
    pub arv_root_hash_size: u64,

    pub arv_manifest_start: u64,
    pub arv_manifest_size: u64,

    pub bs_arv_root_hash_start: u64,
    pub bs_arv_root_hash_size: u64,

    pub bs_arv_manifest_start: u64,
    pub bs_arv_manifest_size: u64,

    // Reserved padding to fill to 4096 bytes
    pub reserved4: [u32; 692],
}

impl BootArgs {
    /// Create a zeroed boot_args structure.
    pub fn zeroed() -> Self {
        unsafe { core::mem::zeroed() }
    }
}

// Compile-time check that boot_args is exactly 4096 bytes, matching XNU's assertion.
const _: () = assert!(core::mem::size_of::<BootArgs>() == 4096);
