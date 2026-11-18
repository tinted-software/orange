//! A bump allocator for `object`, which needs `alloc` even in `read_core`
//! mode. The same design as tools/qemu-boot/src/allocator.rs; nothing is freed.
//! The loader allocates only before ExitBootServices.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

const HEAP_SIZE: usize = 128 * 1024;

struct BumpAlloc {
    offset: AtomicUsize,
    heap: UnsafeCell<[u8; HEAP_SIZE]>,
}

unsafe impl Sync for BumpAlloc {}

unsafe impl GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let heap_start = self.heap.get() as usize;
        loop {
            let cur = self.offset.load(Ordering::Relaxed);
            let aligned = (heap_start + cur + layout.align() - 1) & !(layout.align() - 1);
            let new_offset = (aligned - heap_start) + layout.size();
            if new_offset > HEAP_SIZE {
                return core::ptr::null_mut();
            }
            if self
                .offset
                .compare_exchange_weak(cur, new_offset, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                return aligned as *mut u8;
            }
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOCATOR: BumpAlloc = BumpAlloc {
    offset: AtomicUsize::new(0),
    heap: UnsafeCell::new([0; HEAP_SIZE]),
};
