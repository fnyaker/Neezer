//! A counting allocator for the tests that measure what a screen holds.
//!
//! `VmRSS` is the PROCESS's resident set, not the page's weight: the app's suite
//! runs its tests in parallel threads of one process, so a neighbouring test
//! that allocates 480 MB moves the reading between the two `VmRSS` samples, and
//! glibc's per-thread arenas make the same 228 KB of retained data read 8 404 KB
//! on a loaded two-core runner (measured on PR #236). A budget on that number
//! is a budget on the suite's scheduling, not on the page.
//!
//! This allocator counts BYTES for the thread that asks: `allocated - freed`,
//! so a screen that keeps a decoded image per row adds its bytes and a screen
//! that only builds a frame that drops adds nothing. The counters are
//! `thread_local!` with a const initialiser and no destructor (a bare `Cell`),
//! so reading them from inside the allocator never allocates and never
//! recurses. Another thread's allocations — the rest of the suite, a tokio
//! worker — are invisible to the number, which is what makes it deterministic.
//!
//! It is installed for this crate's test binary only (`main.rs`,
//! `#[cfg(test)]`), so the shipped app links the system allocator unchanged.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::thread::LocalKey;

thread_local! {
    /// Bytes handed out by [`Counting::alloc`] on this thread.
    static ALLOCATED: Cell<u64> = const { Cell::new(0) };
    /// Bytes given back by [`Counting::dealloc`] on this thread.
    static FREED: Cell<u64> = const { Cell::new(0) };
}

/// The allocator a measured test runs under: the system one, plus per-thread
/// byte counters.
pub struct Counting;

/// Adds to a thread's counter, and does nothing at all when the thread is
/// tearing down (`try_with` fails once its TLS is destroyed): an allocator must
/// never panic and must never allocate to report a byte.
fn add(slot: &'static LocalKey<Cell<u64>>, bytes: usize) {
    let bytes = bytes as u64;
    let _ = slot.try_with(|counter| counter.set(counter.get().saturating_add(bytes)));
}

// SAFETY: every method forwards the caller's pointer and layout to `System`
// unchanged and only records the layout's size afterwards; the returned pointer
// is exactly `System`'s.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            add(&ALLOCATED, layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            add(&ALLOCATED, layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        add(&FREED, layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            // The old block is gone and the new one is live: count both, so the
            // net stays `new_size - layout.size()`. A failed `realloc` keeps the
            // old block, so nothing is counted.
            add(&ALLOCATED, new_size);
            add(&FREED, layout.size());
        }
        new_ptr
    }
}

/// Bytes this thread holds right now: everything it allocated and has not freed
/// itself. Saturating, because a thread can legitimately free memory another
/// thread allocated (a channel's buffer, an `Arc` handed over); that only ever
/// pushes the number down, and a screen's own weight is what the test asserts.
pub fn retained_bytes() -> u64 {
    let allocated = ALLOCATED.with(Cell::get);
    let freed = FREED.with(Cell::get);
    allocated.saturating_sub(freed)
}

/// A reading of [`retained_bytes`], taken before and after the work a test
/// measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meter(u64);

impl Meter {
    /// The reading now, on the calling thread.
    pub fn now() -> Self {
        Self(retained_bytes())
    }

    /// Bytes the thread still holds that it did not hold at `self`.
    pub fn since(self) -> u64 {
        retained_bytes().saturating_sub(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Meter;

    /// The meter counts what the thread KEEPS, not what it touched: a buffer
    /// still held adds its bytes, and the same buffer dropped adds nothing.
    #[test]
    fn the_meter_counts_retained_bytes_and_forgets_freed_ones() {
        const SIZE: usize = 4 * 1024 * 1024;
        let before = Meter::now();
        let held: Vec<u8> = vec![1; SIZE];
        let kept = before.since();
        assert!(
            kept >= SIZE as u64,
            "a {SIZE}-byte buffer must read at least that, it read {kept}"
        );
        drop(held);
        let after_drop = before.since();
        assert_eq!(
            after_drop, 0,
            "the buffer is gone, so the thread holds nothing more than before"
        );
    }
}
