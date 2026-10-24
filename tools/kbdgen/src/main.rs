//! Erzeugt `crates/ui/res/tastaturbelegungen.json` aus den Belegungsdateien von Windows (nur Windows).
//!
//! Je Belegung (KLID) wird der Dateiname aus der Registry gelesen (`Keyboard Layouts\<KLID>\Layout File`), die DLL aus
//! System32 geladen und ihre Tabelle über `KbdLayerDescriptor()` ausgewertet: Spalten der Umschaltstufen
//! (`pCharModifiers`), Zeichen je virtueller Taste (`pVkToWcharTable`) und Scancodes (`pusVSCtoVK`,
//! `pVSCtoVK_E0`). Es wird keine Belegung installiert oder aktiviert (kein `LoadKeyboardLayout`).
//!
//! Aufruf: `kbdgen <Ausgabedatei>`; ohne Argument wird die Tabelle auf stdout geschrieben. Eine Übersicht je Belegung
//! geht nach stderr.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::c_void;

/// Belegungen in der Reihenfolge der Auswahl in remotejoe; die DLL dient nur der Kontrolle.
const LAYOUTS: [(&str, &str); 17] = [
    ("00000407", "KBDGR.DLL"),
    ("00000807", "KBDSG.DLL"),
    ("0000040C", "KBDFR.DLL"),
    ("0000080C", "KBDBE.DLL"),
    ("0000100C", "KBDSF.DLL"),
    ("00000809", "KBDUK.DLL"),
    ("00000409", "KBDUS.DLL"),
    ("00000410", "KBDIT.DLL"),
    ("0000040A", "KBDSP.DLL"),
    ("00000816", "KBDPO.DLL"),
    ("00000413", "KBDNE.DLL"),
    ("0000041D", "KBDSW.DLL"),
    ("0000040B", "KBDFI.DLL"),
    ("00000414", "KBDNO.DLL"),
    ("00000406", "KBDDA.DLL"),
    ("00000415", "KBDPL1.DLL"),
    ("00000405", "KBDCZ.DLL"),
];

// --- Windows-Schnittstellen (kernel32, advapi32) ---------------------------------------------------------------

const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
const RRF_RT_REG_SZ: u32 = 0x0000_0002;
const RRF_RT_REG_DWORD: u32 = 0x0000_0010;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> isize;
    fn GetProcAddress(module: isize, name: *const u8) -> *const c_void;
    fn FreeLibrary(module: isize) -> i32;
    fn GetSystemDirectoryW(buf: *mut u16, len: u32) -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, ty: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn reg_string(sub: &str, value: &str) -> Option<String> {
    let (sub, value) = (wide(sub), wide(value));
    let mut buf = vec![0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    let rc = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, sub.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
    if rc != 0 {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n]))
}

fn reg_dword(sub: &str, value: &str) -> Option<u32> {
    let (sub, value) = (wide(sub), wide(value));
    let mut v = 0u32;
    let mut len = 4u32;
    let rc = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, sub.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut v as *mut u32).cast(), &mut len) };
    (rc == 0).then_some(v)
}

fn system_dir() -> String {
    let mut buf = vec![0u16; 260];
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

// --- Strukturen aus kbd.h (64 Bit) ---------------------------------------------------------------------------

#[repr(C)]
struct Modifiers {
    p_vk_to_bit: *const c_void,
    w_max_mod_bits: u16,
    mod_number: [u8; 0],
}

#[repr(C)]
struct VkToWcharTable {
    p_vk_to_wchars: *const u8,
    n_modifications: u8,
    cb_size: u8,
}

#[repr(C)]
struct VscVk {
    vsc: u8,
    vk: u16,
}

#[repr(C)]
struct KbdTables {
    p_char_modifiers: *const Modifiers,
    p_vk_to_wchar_table: *const VkToWcharTable,
    p_dead_key: *const c_void,
    p_key_names: *const c_void,
    p_key_names_ext: *const c_void,
    p_key_names_dead: *const c_void,
    pus_vsc_to_vk: *const u16,
    b_max_vsc_to_vk: u8,
    p_vsc_to_vk_e0: *const VscVk,
    p_vsc_to_vk_e1: *const VscVk,
    f_locale_flags: u32,
    n_lg_max: u8,
    cb_lg_entry: u8,
    p_ligature: *const c_void,
    dw_type: u32,
    dw_sub_type: u32,
}

const SHFT_INVALID: u8 = 0x0F;
const WCH_NONE: u16 = 0xF000;
const WCH_DEAD: u16 = 0xF001;
const WCH_LGTR: u16 = 0xF002;
const SGCAPS: u8 = 0x02;
const VK_NONE: u8 = 0xFF;
const KBDSHIFT: u16 = 0x1;
const KBDCTRL: u16 = 0x2;
const KBDALT: u16 = 0x4;

/// Umschaltstufe; die Reihenfolge ist zugleich die Vorrangfolge (einfachste zuerst).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Level {
    Plain,
    Shift,
    AltGr,
    ShiftAltGr,
}

impl Level {
    fn from_bits(bits: u16) -> Option<Level> {
        match bits {
            0 => Some(Level::Plain),
            KBDSHIFT => Some(Level::Shift),
            b if b == KBDCTRL | KBDALT => Some(Level::AltGr),
            b if b == KBDSHIFT | KBDCTRL | KBDALT => Some(Level::ShiftAltGr),
            // Strg allein, Kana und andere Stufen werden nicht verwendet.
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Level::Plain => "",
            Level::Shift => "shift",
            Level::AltGr => "altgr",
            Level::ShiftAltGr => "shift+altgr",
        }
    }
}

/// Taste: Scancode (Set 1) und E0-Präfix.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Key {
    sc: u8,
    e0: bool,
}

impl Key {
    /// Ziffernblock (Set 1): `*`, 7–9, `-`, 4–6, `+`, 1–3, 0, `,`; mit E0 `/` und Enter.
    fn numpad(self) -> bool {
        if self.e0 {
            matches!(self.sc, 0x35 | 0x1C)
        } else {
            self.sc == 0x37 || (0x47..=0x53).contains(&self.sc)
        }
    }
}

/// Ergebnis einer Belegung.
struct Table {
    keys: BTreeMap<u32, (Level, Key)>,
    dead: BTreeSet<u32>,
}

fn usable_char(c: u16) -> bool {
    c >= 0x20 && c != 0x7F && !(0x80..=0x9F).contains(&c) && !(0xD800..=0xDFFF).contains(&c)
}

/// Wertet die Tabelle einer geladenen Belegungs-DLL aus.
unsafe fn read_tables(t: &KbdTables) -> Table {
    // Spalte → einfachste Umschaltstufe.
    let m = &*t.p_char_modifiers;
    let numbers = std::ptr::addr_of!(m.mod_number) as *const u8;
    let mut col_level: HashMap<usize, Level> = HashMap::new();
    for bits in 0..=m.w_max_mod_bits {
        let col = *numbers.add(bits as usize);
        if col == SHFT_INVALID {
            continue;
        }
        if let Some(level) = Level::from_bits(bits) {
            let e = col_level.entry(col as usize).or_insert(level);
            if level < *e {
                *e = level;
            }
        }
    }

    // Virtuelle Taste → mögliche Tasten.
    let mut vk_keys: HashMap<u8, Vec<Key>> = HashMap::new();
    for sc in 0..=t.b_max_vsc_to_vk {
        let vk = (*t.pus_vsc_to_vk.add(sc as usize) & 0xFF) as u8;
        if vk != VK_NONE && vk != 0 {
            vk_keys.entry(vk).or_default().push(Key { sc, e0: false });
        }
    }
    if !t.p_vsc_to_vk_e0.is_null() {
        let mut p = t.p_vsc_to_vk_e0;
        while (*p).vsc != 0 {
            let vk = ((*p).vk & 0xFF) as u8;
            if vk != VK_NONE && vk != 0 {
                vk_keys.entry(vk).or_default().push(Key { sc: (*p).vsc, e0: true });
            }
            p = p.add(1);
        }
    }
    // Ziffernblock bei eingeschaltetem NumLock: Windows übersetzt diese Scancodes gesondert (KBDNUMPAD).
    for (vk, sc) in [(0x60u8, 0x52u8), (0x61, 0x4F), (0x62, 0x50), (0x63, 0x51), (0x64, 0x4B), (0x65, 0x4C), (0x66, 0x4D), (0x67, 0x47), (0x68, 0x48), (0x69, 0x49), (0x6E, 0x53)] {
        vk_keys.entry(vk).or_default().push(Key { sc, e0: false });
    }

    let mut keys: BTreeMap<u32, (Level, Key)> = BTreeMap::new();
    let mut dead: BTreeSet<u32> = BTreeSet::new();
    let mut table = t.p_vk_to_wchar_table;
    while !(*table).p_vk_to_wchars.is_null() {
        let n = (*table).n_modifications as usize;
        let size = (*table).cb_size as usize;
        let mut row = (*table).p_vk_to_wchars;
        let wch = |row: *const u8, i: usize| -> u16 { std::ptr::read_unaligned(row.add(2 + 2 * i) as *const u16) };
        while *row != 0 {
            let vk = *row;
            let attrs = *row.add(1);
            if vk == VK_NONE {
                // Folgezeile ohne eigene Taste (Tottaste oder SGCAPS), gehört zur Zeile davor.
                row = row.add(size);
                continue;
            }
            let next = row.add(size);
            for col in 0..n {
                let c = wch(row, col);
                if c == WCH_DEAD {
                    // Das Zeichen der Tottaste steht in derselben Spalte der Folgezeile.
                    if *next == VK_NONE {
                        let d = wch(next, col);
                        if usable_char(d) {
                            dead.insert(d as u32);
                        }
                    }
                    continue;
                }
                if c == WCH_NONE || c == WCH_LGTR || !usable_char(c) {
                    continue;
                }
                let Some(&level) = col_level.get(&col) else { continue };
                let Some(cands) = vk_keys.get(&vk) else { continue };
                for &k in cands {
                    // Haupttastenblock vor Ziffernblock (dessen Ziffern hängen am Server vom NumLock ab), dann die
                    // einfachste Stufe.
                    let rank = |l: Level, k: Key| (k.numpad(), l, k.e0, k.sc);
                    let better = match keys.get(&(c as u32)) {
                        None => true,
                        Some(&(l0, k0)) => rank(level, k) < rank(l0, k0),
                    };
                    if better {
                        keys.insert(c as u32, (level, k));
                    }
                }
            }
            // SGCAPS: Die Folgezeile (Feststelltaste) hat die Taste 0xFF und wird oben übersprungen; es zählen nur die
            // Grundspalten dieser Zeile.
            debug_assert!(attrs & SGCAPS == 0 || *next == VK_NONE);
            row = next;
        }
        table = table.add(1);
    }
    Table { keys, dead }
}

fn load(file: &str) -> Result<Table, String> {
    let path = format!("{}\\{}", system_dir(), file);
    let w = wide(&path);
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    if module == 0 {
        return Err(format!("{file} lässt sich nicht laden"));
    }
    let res = unsafe {
        let f = GetProcAddress(module, b"KbdLayerDescriptor\0".as_ptr());
        if f.is_null() {
            Err(format!("{file}: KbdLayerDescriptor fehlt"))
        } else {
            let f: extern "system" fn() -> *const KbdTables = std::mem::transmute(f);
            let t = f();
            if t.is_null() {
                Err(format!("{file}: keine Tabelle"))
            } else {
                Ok(read_tables(&*t))
            }
        }
    };
    unsafe { FreeLibrary(module) };
    res
}

fn main() {
    let cv = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
    let product = reg_string(cv, "ProductName").unwrap_or_else(|| "Windows".into());
    let build = reg_string(cv, "CurrentBuild").unwrap_or_default();
    let ubr = reg_dword(cv, "UBR").map(|u| format!(".{u}")).unwrap_or_default();
    let source = format!("{product} {build}{ubr}");

    let mut layouts: Vec<(&str, String, Table)> = Vec::new();
    let mut failed = false;
    for (klid, expected) in LAYOUTS {
        let Some(file) = reg_string(&format!("SYSTEM\\CurrentControlSet\\Control\\Keyboard Layouts\\{klid}"), "Layout File") else {
            eprintln!("{klid}: kein Eintrag „Layout File“ in der Registry");
            failed = true;
            continue;
        };
        let file = file.to_ascii_uppercase();
        if file != expected {
            eprintln!("{klid}: Datei {file} statt erwartet {expected}");
        }
        match load(&file) {
            Ok(t) => {
                eprintln!("{klid} {file}: {} Zeichen, {} Tottasten", t.keys.len(), t.dead.len());
                layouts.push((klid, file, t));
            }
            Err(e) => {
                eprintln!("{klid}: {e}");
                failed = true;
            }
        }
    }
    layouts.sort_by(|a, b| a.0.cmp(b.0));

    let mut out = String::new();
    out.push_str(&format!("{{\"quelle\":\"{}\",\"belegungen\":[\n", source.replace('\\', "\\\\").replace('"', "\\\"")));
    for (i, (klid, file, t)) in layouts.iter().enumerate() {
        out.push_str(&format!("{{\"klid\":\"{klid}\",\"dll\":\"{file}\",\"tasten\":[\n"));
        let n = t.keys.len();
        for (j, (z, (level, key))) in t.keys.iter().enumerate() {
            out.push_str(&format!("{{\"z\":{z},\"sc\":{},\"e0\":{},\"stufe\":\"{}\"}}{}\n", key.sc, key.e0, level.name(), if j + 1 < n { "," } else { "" }));
        }
        let dead: Vec<String> = t.dead.iter().map(|d| d.to_string()).collect();
        out.push_str(&format!("],\"tot\":[{}]}}{}\n", dead.join(","), if i + 1 < layouts.len() { "," } else { "" }));
    }
    out.push_str("]}\n");

    match std::env::args().nth(1) {
        Some(path) => std::fs::write(&path, out).unwrap_or_else(|e| panic!("{path}: {e}")),
        None => print!("{out}"),
    }
    eprintln!("Quelle: {source}");
    if failed {
        std::process::exit(1);
    }
}
