//! Zuordnung von X11-Keysyms zu RDP-Scancodes (Set 1) für Sondertasten und
//! Tasten, die mit Umschalttasten kombiniert werden. Normaler Text wird als
//! Unicode-Tastenereignis gesendet und ist von der Belegung unabhängig.

/// Erweiterte Taste (E0-Präfix).
const EXT: u32 = 0x100;

pub fn scancode(keysym: u32) -> Option<u32> {
    let sc = match keysym {
        0xff0d => 0x1C,
        0xff8d => 0x1C | EXT,
        0xff1b => 0x01,
        0xff09 => 0x0F,
        0xff08 => 0x0E,
        0xffff => 0x53 | EXT,
        0xff50 => 0x47 | EXT,
        0xff51 => 0x4B | EXT,
        0xff52 => 0x48 | EXT,
        0xff53 => 0x4D | EXT,
        0xff54 => 0x50 | EXT,
        0xff55 => 0x49 | EXT,
        0xff56 => 0x51 | EXT,
        0xff57 => 0x4F | EXT,
        0xff63 => 0x52 | EXT,
        0xff67 => 0x5D | EXT,
        0xffbe..=0xffc7 => 0x3B + (keysym - 0xffbe),
        0xffc8 => 0x57,
        0xffc9 => 0x58,
        0xffe1 => 0x2A,
        0xffe2 => 0x36,
        0xffe3 => 0x1D,
        0xffe4 => 0x1D | EXT,
        0xffe9 | 0xffe7 => 0x38,
        0xffea | 0xfe03 => 0x38 | EXT,
        0xffeb => 0x5B | EXT,
        0xffec => 0x5C | EXT,
        0xffe5 => 0x3A,
        0xff13 => 0x45,
        0xff61 => 0x37 | EXT,
        0x20 => 0x39,
        0x30 => 0x0B,
        0x31..=0x39 => 0x02 + (keysym - 0x31),
        0x41..=0x5A => return scancode(keysym + 0x20),
        0x61..=0x7A => {
            const LETTERS: [u32; 26] = [
                0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10,
                0x13, 0x1F, 0x14, 0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
            ];
            LETTERS[(keysym - 0x61) as usize]
        }
        _ => return None,
    };
    Some(sc)
}

/// Maus-Kennzeichen nach MS-RDPBCGR.
pub mod mouse {
    pub const MOVE: u16 = 0x0800;
    pub const DOWN: u16 = 0x8000;
    pub const BUTTON1: u16 = 0x1000;
    pub const BUTTON2: u16 = 0x2000;
    pub const BUTTON3: u16 = 0x4000;
    pub const WHEEL: u16 = 0x0200;
    pub const WHEEL_NEGATIVE: u16 = 0x0100;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_keys() {
        assert_eq!(scancode(0xff0d), Some(0x1C));
        assert_eq!(scancode(0xff52), Some(0x148));
        assert_eq!(scancode(b'c' as u32), Some(0x2E));
        assert_eq!(scancode(b'C' as u32), Some(0x2E));
        assert_eq!(scancode(0xffc9), Some(0x58));
        assert_eq!(scancode(0x00e4), None, "Umlaute laufen über Unicode");
    }
}
