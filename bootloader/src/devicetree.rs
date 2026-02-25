//! Minimal Apple Device Tree builder.
//!
//! XNU expects a flattened Open Firmware device tree. The format is a
//! recursive structure of nodes, each containing properties.
//!
//! Structure per node:
//!   - u32: number of properties
//!   - u32: number of children
//!   - For each property:
//!     - [u8; 32]: property name (null-padded)
//!     - u32: value length (with flags in upper bits)
//!     - [u8; aligned_length]: value data (aligned to 4 bytes)
//!   - For each child: recursive node structure

use alloc::vec::Vec;

/// Build a minimal device tree for XNU.
///
/// XNU requires at least:
/// - A root node with "compatible" = "ACPI"
/// - "model" property
/// - "chosen" child with "boot-args" and other props
/// - "efi" child with firmware-related info
/// - "memory-map" (can be empty but node must exist)
pub fn build_device_tree() -> Vec<u8> {
    let mut dt = Vec::with_capacity(4096);

    // Root node: "/" (unnamed, but has properties)
    // properties for root: compatible, model, #address-cells, #size-cells
    // children: chosen, efi, memory-map, options
    let root_props: &[(&str, &[u8])] = &[
        ("compatible", b"ACPI\0"),
        ("model", b"ACPI\0"),
        ("#address-cells", &1u32.to_le_bytes()),
        ("#size-cells", &1u32.to_le_bytes()),
        ("name", b"device-tree\0"),
        ("clock-frequency", &100_000_000u32.to_le_bytes()),
    ];

    // 64 bytes of non-zero "random" seed data
    // XNU requires this at /chosen/random-seed, panics without it.
    let random_seed: [u8; 64] = [
        0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE, 0x13, 0x37, 0x42, 0x69, 0x55, 0xAA, 0xF0,
        0x0D, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54,
        0x32, 0x10, 0xA5, 0x5A, 0x3C, 0xC3, 0x96, 0x69, 0xF0, 0x0F, 0x11, 0x22, 0x33, 0x44, 0x55,
        0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x0A, 0x0B, 0x0C, 0x0D,
        0x0E, 0x0F, 0x1A, 0x2B,
    ];

    let chosen_props: &[(&str, &[u8])] = &[
        ("name", b"chosen\0"),
        ("random-seed", &random_seed),
        ("boot-uuid", b"\0"),
        ("boot-kernelcache-adler32", &0u32.to_le_bytes()),
    ];

    let options_props: &[(&str, &[u8])] = &[("name", b"options\0")];

    let efi_props: &[(&str, &[u8])] = &[
        ("name", b"efi\0"),
        ("firmware-vendor", b"Q\0E\0M\0U\0\0\0"), // "QEMU" in UCS-2
        ("firmware-abi", b"EFI64\0"),
    ];

    let efi_runtime_props: &[(&str, &[u8])] = &[("name", b"runtime-services\0")];

    let efi_platform_props: &[(&str, &[u8])] = &[
        ("name", b"platform\0"),
        (
            "system-id",
            &[
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
                0x0F, 0x10,
            ],
        ),
        ("FSBFrequency", &100_000_000u64.to_le_bytes()),
    ];

    // Root node: 6 properties, 3 children (chosen, options, efi)
    write_u32(&mut dt, root_props.len() as u32); // nproperties
    write_u32(&mut dt, 3); // nchildren: chosen, options, efi

    // Write root properties
    for (name, value) in root_props {
        write_property(&mut dt, name, value);
    }

    // Child: chosen (0 children)
    write_u32(&mut dt, chosen_props.len() as u32);
    write_u32(&mut dt, 0);
    for (name, value) in chosen_props {
        write_property(&mut dt, name, value);
    }

    // Child: options (0 children)
    write_u32(&mut dt, options_props.len() as u32);
    write_u32(&mut dt, 0);
    for (name, value) in options_props {
        write_property(&mut dt, name, value);
    }

    // Child: efi (2 children: runtime-services, platform)
    write_u32(&mut dt, efi_props.len() as u32);
    write_u32(&mut dt, 2); // 2 children: runtime-services, platform

    for (name, value) in efi_props {
        write_property(&mut dt, name, value);
    }

    // efi/runtime-services (0 children)
    write_u32(&mut dt, efi_runtime_props.len() as u32);
    write_u32(&mut dt, 0);
    for (name, value) in efi_runtime_props {
        write_property(&mut dt, name, value);
    }

    // efi/platform (0 children)
    write_u32(&mut dt, efi_platform_props.len() as u32);
    write_u32(&mut dt, 0);
    for (name, value) in efi_platform_props {
        write_property(&mut dt, name, value);
    }

    dt
}

/// Write a property to the device tree buffer.
///
/// Property format:
///   - [u8; 32]: name (null-padded)
///   - u32: length (value length | flags)
///   - [u8; align4(length)]: value data
fn write_property(buf: &mut Vec<u8>, name: &str, value: &[u8]) {
    // Write 32-byte null-padded name
    let mut name_bytes = [0u8; 32];
    let name_b = name.as_bytes();
    let copy_len = core::cmp::min(name_b.len(), 31);
    name_bytes[..copy_len].copy_from_slice(&name_b[..copy_len]);
    buf.extend_from_slice(&name_bytes);

    // Write length (value length, upper bits can have flags but we use 0)
    let len = value.len() as u32;
    write_u32(buf, len & 0x7FFFFFFF); // Mask off flag bit

    // Write value data, padded to 4-byte alignment
    buf.extend_from_slice(value);
    let padding = (4 - (value.len() % 4)) % 4;
    for _ in 0..padding {
        buf.push(0);
    }
}

/// Write a little-endian u32 to the buffer.
fn write_u32(buf: &mut Vec<u8>, val: u32) {
    buf.extend_from_slice(&val.to_le_bytes());
}
