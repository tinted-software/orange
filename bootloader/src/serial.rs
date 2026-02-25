//! Direct serial port output for debugging.
//!
//! Writes directly to COM1 (0x3F8) which QEMU redirects to stdio
//! when using -nographic. This works both before and after ExitBootServices.

use core::fmt::{self, Write};

const COM1: u16 = 0x3F8;

/// Initialize COM1 serial port.
pub fn init() {
    unsafe {
        // Disable interrupts
        outb(COM1 + 1, 0x00);
        // Enable DLAB (set baud rate divisor)
        outb(COM1 + 3, 0x80);
        // Set divisor to 1 (115200 baud)
        outb(COM1 + 0, 0x01);
        outb(COM1 + 1, 0x00);
        // 8 bits, no parity, one stop bit
        outb(COM1 + 3, 0x03);
        // Enable FIFO, clear buffers, 14-byte threshold
        outb(COM1 + 2, 0xC7);
        // IRQs disabled, RTS/DSR set
        outb(COM1 + 4, 0x0B);
    }
}

/// Write a single byte to COM1.
fn write_byte(b: u8) {
    unsafe {
        // Wait for transmit buffer to be empty
        while (inb(COM1 + 5) & 0x20) == 0 {}
        outb(COM1, b);
    }
}

/// Write a string to serial.
pub fn write_str(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            write_byte(b'\r');
        }
        write_byte(b);
    }
}

/// Serial writer implementing fmt::Write.
pub struct SerialWriter;

impl Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_str(s);
        Ok(())
    }
}

/// Print a formatted string to serial.
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        {
            use core::fmt::Write;
            let _ = write!($crate::serial::SerialWriter, $($arg)*);
        }
    };
}

/// Print a formatted string to serial with newline.
#[macro_export]
macro_rules! serial_println {
    () => { $crate::serial::write_str("\n") };
    ($($arg:tt)*) => {
        {
            use core::fmt::Write;
            let _ = write!($crate::serial::SerialWriter, $($arg)*);
            $crate::serial::write_str("\n");
        }
    };
}

#[cfg(target_arch = "x86_64")]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack));
}

#[cfg(target_arch = "x86_64")]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") val, options(nomem, nostack));
    val
}

// Stubs for non-x86 so rust-analyzer doesn't complain
#[cfg(not(target_arch = "x86_64"))]
unsafe fn outb(_port: u16, _val: u8) {}
#[cfg(not(target_arch = "x86_64"))]
unsafe fn inb(_port: u16) -> u8 {
    0
}
