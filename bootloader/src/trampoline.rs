//! Trampoline to jump from 64-bit long mode to 32-bit protected mode
//! and enter XNU's _pstart.
//!
//! XNU's _pstart expects:
//!   - 32-bit protected mode
//!   - Flat segmentation (code/data base=0, limit=4G)
//!   - EAX = physical address of boot_args
//!   - Paging disabled
//!   - Interrupts disabled
//!
//! We must:
//!   1. Disable interrupts
//!   2. Set up a temporary GDT with 32-bit code/data segments
//!   3. Switch from 64-bit long mode to 32-bit compatibility mode
//!   4. Disable paging
//!   5. Disable long mode (clear IA32_EFER.LME)
//!   6. Load EAX with boot_args address and jump to _pstart

use core::arch::asm;

/// GDT entry for the trampoline.
#[derive(Clone, Copy)]
#[repr(C, packed)]
struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

impl GdtEntry {
    const fn null() -> Self {
        Self {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            granularity: 0,
            base_high: 0,
        }
    }

    /// Create a flat 32-bit code segment (base=0, limit=4G, 32-bit, ring 0).
    const fn code32() -> Self {
        Self {
            limit_low: 0xFFFF,
            base_low: 0,
            base_mid: 0,
            access: 0x9A,      // present, code, readable, ring 0
            granularity: 0xCF, // 4K granularity, 32-bit, limit[19:16]=0xF
            base_high: 0,
        }
    }

    /// Create a flat 32-bit data segment (base=0, limit=4G, 32-bit, ring 0).
    const fn data32() -> Self {
        Self {
            limit_low: 0xFFFF,
            base_low: 0,
            base_mid: 0,
            access: 0x92, // present, data, writable, ring 0
            granularity: 0xCF,
            base_high: 0,
        }
    }

    /// Create a 64-bit code segment (for the intermediate far jump).
    const fn code64() -> Self {
        Self {
            limit_low: 0xFFFF,
            base_low: 0,
            base_mid: 0,
            access: 0x9A,      // present, code, readable, ring 0
            granularity: 0xAF, // 4K gran, 64-bit (L=1, D=0), limit[19:16]=0xF
            base_high: 0,
        }
    }
}

/// GDTR (GDT register) format.
#[repr(C, packed)]
struct Gdtr {
    limit: u16,
    base: u64,
}

/// Our trampoline GDT, placed in static memory so its physical address is stable.
/// Layout:
///   0x00: null
///   0x08: 64-bit code (selector 0x08) - for intermediate step
///   0x10: 32-bit code (selector 0x10)
///   0x18: 32-bit data (selector 0x18)
#[repr(C, align(16))]
struct TrampolineGdt {
    entries: [GdtEntry; 4],
}

static mut TRAMPOLINE_GDT: TrampolineGdt = TrampolineGdt {
    entries: [
        GdtEntry::null(),
        GdtEntry::code64(),
        GdtEntry::code32(),
        GdtEntry::data32(),
    ],
};

static mut TRAMPOLINE_GDTR: Gdtr = Gdtr {
    limit: (4 * 8 - 1) as u16,
    base: 0,
};

/// Trampoline code that will be placed in low memory (<1MB) so it is
/// accessible after disabling paging.
///
/// This is raw machine code for the 32-bit portion of the trampoline.
/// It expects:
///   - To be running in 32-bit compatibility mode
///   - CR0.PG already cleared
///   - ECX = kernel entry point
///   - EBX = boot_args address
fn trampoline_code_32() -> [u8; 64] {
    // This is the 32-bit machine code that runs after we've dropped out of long mode.
    // At this point:
    //   - We're in 32-bit protected mode
    //   - Paging is off
    //   - EBX = boot_args physical address
    //   - ECX = _pstart physical address
    //
    // We need to:
    //   1. Clear IA32_EFER.LME
    //   2. Set EAX = boot_args
    //   3. Set EDI = boot_args (XNU also expects it in EDI)
    //   4. Jump to _pstart
    let mut code = [0u8; 64];
    let ops: &[u8] = &[
        // Disable long mode: clear IA32_EFER.LME (bit 8)
        0xB9, 0x80, 0xC0, 0x00, 0x00, // mov ecx, 0xC0000080 (IA32_EFER MSR)
        0x0F, 0x32, // rdmsr
        0x0F, 0xBA, 0xF0, 0x08, // btr eax, 8      (clear LME bit)
        0x0F, 0x30, // wrmsr
        // Set up for kernel entry:
        //   EAX = boot_args (from EBX)
        //   EDI = boot_args (XNU's start.s does: mov %eax, %edi)
        0x89, 0xD8, // mov eax, ebx
        0x89, 0xDF, // mov edi, ebx
        // Restore ECX = entry point (it was saved in ESI)
        0x89, 0xF1, // mov ecx, esi
        // Jump to kernel entry
        0xFF, 0xE1, // jmp ecx
        // Halt if we somehow return
        0xF4, // hlt
    ];
    code[..ops.len()].copy_from_slice(ops);
    code
}

/// Jump to the XNU kernel.
///
/// This function does not return. It:
/// 1. Copies a 32-bit trampoline to low memory
/// 2. Sets up a GDT with 32-bit segments
/// 3. Transitions from 64-bit long mode to 32-bit protected mode
/// 4. Disables paging
/// 5. Jumps to _pstart with EAX = boot_args
///
/// # Safety
/// Must be called after ExitBootServices with valid kernel and boot_args addresses.
pub unsafe fn jump_to_kernel(entry_point: u32, boot_args: u32) -> ! {
    // Place 32-bit trampoline code at a fixed low-memory address
    // Use 0x8000 (32K) - safe area below kernel at 0x100000
    let trampoline_addr: u64 = 0x8000;
    let code = trampoline_code_32();
    core::ptr::copy_nonoverlapping(code.as_ptr(), trampoline_addr as *mut u8, code.len());

    // Set up the GDTR to point to our trampoline GDT
    let gdt_ptr = core::ptr::addr_of!(TRAMPOLINE_GDT) as u64;
    TRAMPOLINE_GDTR.base = gdt_ptr;
    let gdtr_ptr = core::ptr::addr_of!(TRAMPOLINE_GDTR) as u64;

    // We pass everything through explicit registers to avoid cross-mode issues.
    // rdi = GDTR pointer
    // rsi = entry_point (32-bit value, zero-extended)
    // rdx = boot_args (32-bit value, zero-extended)
    // rcx = trampoline_addr
    // r8  = code32 selector (0x10) as u64 for push
    let entry_64 = entry_point as u64;
    let bootargs_64 = boot_args as u64;
    let code32_sel: u64 = 0x10;

    asm!(
        // Disable interrupts
        "cli",

        // Load new GDT
        "lgdt [rdi]",

        // Load 32-bit data segments (selector 0x18)
        "mov ax, 0x18",
        "mov ds, ax",
        "mov es, ax",
        "mov fs, ax",
        "mov gs, ax",
        "mov ss, ax",

        // Move values into registers that survive the mode switch.
        // ESI = entry_point, EBX = boot_args, EDI = trampoline_addr
        // (all already in rsi, rdx, rcx from calling convention)
        "mov ebx, edx",       // EBX = boot_args
        // ESI already has entry_point
        // ECX already has trampoline_addr

        // Far return to 32-bit compatibility mode via selector 0x10
        "push r8",              // push code32 selector
        "lea rax, [rip + 2f]",  // address of compat_entry
        "push rax",
        "retfq",                // far return → 32-bit compatibility mode

        // Now in 32-bit compatibility mode
        ".code32",
        "2:",

        // Disable paging: clear CR0.PG (bit 31)
        "mov eax, cr0",
        "and eax, 0x7FFFFFFF",
        "mov cr0, eax",

        // Jump to the 32-bit trampoline at 0x8000
        // ECX still holds trampoline_addr
        "jmp ecx",

        ".code64",

        in("rdi") gdtr_ptr,
        in("rsi") entry_64,
        in("rdx") bootargs_64,
        in("rcx") trampoline_addr,
        in("r8") code32_sel,
        options(noreturn)
    );
}
