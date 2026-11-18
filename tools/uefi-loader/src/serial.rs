//! COM1 output. UEFI leaves the legacy UART alone, and QEMU's `-serial stdio`
//! makes it visible on the host without depending on the firmware console.

use core::arch::asm;
use core::fmt::{self, Write};

const COM1: u16 = 0x3F8;

fn outb(port: u16, value: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack)) }
}

pub struct Com1;

impl Write for Com1 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                outb(COM1, b'\r');
            }
            outb(COM1, byte);
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let _ = Com1.write_fmt(args);
}
