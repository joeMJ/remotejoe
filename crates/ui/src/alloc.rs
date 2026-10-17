//! Speicherverwaltung, die freigegebenen Speicher überschreibt.
//!
//! Kennwörter durchlaufen beim Entschlüsseln der KDBX, beim Zerlegen von
//! JSON und in Zwischenpuffern viele kurzlebige Speicherblöcke. Damit nach
//! dem Sperren keine Kopien in freigegebenem Speicher liegen bleiben, wird
//! jeder Block vor der Freigabe mit Nullen überschrieben. Gilt für allen
//! Speicher, den Rust-Code anfordert (nicht für Qt).

use std::alloc::{GlobalAlloc, Layout, System};

pub struct ZeroizingAllocator;

// SAFETY: Reicht Anforderungen unverändert an `System` weiter; überschreibt
// nur Speicher, der dem Aufrufer gehört und gleich freigegeben wird.
unsafe impl GlobalAlloc for ZeroizingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            zero(ptr, layout.size());
            System.dealloc(ptr, layout)
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Neu anlegen und kopieren, damit der alte Block überschrieben wird.
        unsafe {
            let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
            let new = System.alloc(new_layout);
            if !new.is_null() {
                std::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                zero(ptr, layout.size());
                System.dealloc(ptr, layout);
            }
            new
        }
    }
}

/// Überschreibt `len` Bytes so, dass der Übersetzer es nicht wegoptimiert.
unsafe fn zero(ptr: *mut u8, len: usize) {
    for i in 0..len {
        unsafe { std::ptr::write_volatile(ptr.add(i), 0) };
    }
}
