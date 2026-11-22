//! Freigegebene Puffer von `Secret` und `Secret::utf16_units` enthalten das
//! Geheimnis nicht mehr. Geprüft mit einer Speicherverwaltung, die beim
//! Freigeben nach einem Merkmal sucht (so weit von außen prüfbar).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rj_core::Secret;

const MARK: &str = "Merkmal-7f3a9c-Geheimnis";

struct Scan;
static ARMED: AtomicBool = AtomicBool::new(false);
static FOUND: AtomicUsize = AtomicUsize::new(0);

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Merkmal als UTF-16LE (nur ASCII: jedes Byte gefolgt von 0), ohne Speicher anzufordern.
fn contains_utf16(hay: &[u8], needle: &[u8]) -> bool {
    let n = needle.len() * 2;
    hay.windows(n).any(|w| w.chunks(2).zip(needle).all(|(c, b)| c[0] == *b && c[1] == 0))
}

unsafe impl GlobalAlloc for Scan {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if ARMED.load(Ordering::SeqCst) {
            let block = std::slice::from_raw_parts(p, l.size());
            if contains(block, MARK.as_bytes()) || contains_utf16(block, MARK.as_bytes()) {
                FOUND.fetch_add(1, Ordering::SeqCst);
            }
        }
        System.dealloc(p, l)
    }
}

#[global_allocator]
static A: Scan = Scan;

fn found_after(f: impl FnOnce()) -> usize {
    FOUND.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    f();
    ARMED.store(false, Ordering::SeqCst);
    FOUND.load(Ordering::SeqCst)
}

#[test]
fn freed_buffers_do_not_contain_the_secret() {
    // Gegenprobe: ein gewöhnlicher Puffer wird gefunden.
    let plain: Vec<u16> = MARK.encode_utf16().collect();
    assert_eq!(found_after(|| drop(plain)), 1);
    let s = Secret::from_str_exact(MARK);
    let units = s.utf16_units().unwrap();
    assert_eq!(units.len(), MARK.len());
    assert_eq!(found_after(|| drop(units)), 0, "UTF-16-Puffer nicht überschrieben");
    assert_eq!(found_after(|| drop(s)), 0, "Secret nicht überschrieben");
}
