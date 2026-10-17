//! Zuordnung von Qt-Tastencodes zu Keysyms (RDP, VNC) und Terminalbytes (SSH).

/// Umschalttasten nach Qt.
pub const SHIFT: u32 = 0x0200_0000;
pub const CTRL: u32 = 0x0400_0000;
pub const ALT: u32 = 0x0800_0000;

/// Qt-Tastencode → X11-Keysym für Sondertasten.
pub fn keysym(qt_key: u32) -> Option<u32> {
    Some(match qt_key {
        0x0100_0000 => 0xff1b,
        0x0100_0001 => 0xff09,
        0x0100_0002 => 0xfe20,
        0x0100_0003 => 0xff08,
        0x0100_0004 => 0xff0d,
        0x0100_0005 => 0xff8d,
        0x0100_0006 => 0xff63,
        0x0100_0007 => 0xffff,
        0x0100_0010 => 0xff50,
        0x0100_0011 => 0xff57,
        0x0100_0012 => 0xff51,
        0x0100_0013 => 0xff52,
        0x0100_0014 => 0xff53,
        0x0100_0015 => 0xff54,
        0x0100_0016 => 0xff55,
        0x0100_0017 => 0xff56,
        0x0100_0020 => 0xffe1,
        0x0100_0021 => 0xffe3,
        0x0100_0022 => 0xffeb,
        0x0100_0023 => 0xffe9,
        0x0100_0024 => 0xffe5,
        0x0100_0030..=0x0100_003b => 0xffbe + (qt_key - 0x0100_0030),
        0x20..=0x7e => {
            // Buchstaben als Kleinbuchstaben-Keysym (für Tastenkürzel mit Strg/Alt).
            let c = char::from_u32(qt_key)?.to_ascii_lowercase();
            c as u32
        }
        _ => return None,
    })
}

/// Bytes für das Terminal (xterm-Folgen). `text` ist der von Qt gelieferte Text.
pub fn terminal_bytes(qt_key: u32, modifiers: u32, text: &str, app_cursor: bool) -> Vec<u8> {
    let ctrl = modifiers & CTRL != 0;
    let alt = modifiers & ALT != 0;
    let csi = |s: &str| format!("\x1b[{s}").into_bytes();
    let cursor = |c: char| if app_cursor { format!("\x1bO{c}").into_bytes() } else { format!("\x1b[{c}").into_bytes() };
    let out: Vec<u8> = match qt_key {
        0x0100_0000 => vec![0x1b],
        0x0100_0001 => vec![b'\t'],
        0x0100_0002 => csi("Z"),
        0x0100_0003 => vec![0x7f],
        0x0100_0004 | 0x0100_0005 => vec![b'\r'],
        0x0100_0006 => csi("2~"),
        0x0100_0007 => csi("3~"),
        0x0100_0010 => cursor('H'),
        0x0100_0011 => cursor('F'),
        0x0100_0012 => cursor('D'),
        0x0100_0013 => cursor('A'),
        0x0100_0014 => cursor('C'),
        0x0100_0015 => cursor('B'),
        0x0100_0016 => csi("5~"),
        0x0100_0017 => csi("6~"),
        0x0100_0030..=0x0100_0033 => format!("\x1bO{}", ["P", "Q", "R", "S"][(qt_key - 0x0100_0030) as usize]).into_bytes(),
        0x0100_0034..=0x0100_003b => csi(["15~", "17~", "18~", "19~", "20~", "21~", "23~", "24~"][(qt_key - 0x0100_0034) as usize]),
        k if ctrl && (0x40..=0x5f).contains(&k) => vec![(k as u8) & 0x1f],
        k if ctrl && (0x61..=0x7a).contains(&k) => vec![(k as u8) & 0x1f],
        _ => text.as_bytes().to_vec(),
    };
    if alt && !out.is_empty() && out[0] != 0x1b {
        let mut v = vec![0x1b];
        v.extend(out);
        return v;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysyms() {
        assert_eq!(keysym(0x0100_0004), Some(0xff0d));
        assert_eq!(keysym(0x0100_003b), Some(0xffc9));
        assert_eq!(keysym('C' as u32), Some('c' as u32));
        assert_eq!(keysym(0xE4), None);
    }

    #[test]
    fn terminal() {
        assert_eq!(terminal_bytes(0x0100_0013, 0, "", false), b"\x1b[A");
        assert_eq!(terminal_bytes(0x0100_0013, 0, "", true), b"\x1bOA");
        assert_eq!(terminal_bytes('C' as u32, CTRL, "\u{3}", false), b"\x03");
        assert_eq!(terminal_bytes('X' as u32, 0, "ä", false), "ä".as_bytes());
        assert_eq!(terminal_bytes('x' as u32, ALT, "x", false), b"\x1bx");
        assert_eq!(terminal_bytes(0x0100_0003, 0, "", false), b"\x7f");
    }
}
