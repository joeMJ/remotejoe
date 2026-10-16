//! Entfernt Terminal-Steuerfolgen aus Ausgaben, damit Muster auf dem
//! sichtbaren Text vergleichen.

/// Entfernt CSI-, OSC- und einfache ESC-Folgen sowie Rückschritte nicht.
pub fn strip(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                // CSI: Parameter bis zum Endzeichen 0x40..0x7E.
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') | Some('P') | Some('_') | Some('^') => {
                // OSC/DCS bis BEL oder ESC \
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some('(') | Some(')') => {
                chars.next();
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn strips_sequences() {
        let s = "\u{1b}[1;32mgrün\u{1b}[0m \u{1b}]0;Titel\u{7}ok\u{1b}(B ä";
        assert_eq!(super::strip(s), "grün ok ä");
    }
}
