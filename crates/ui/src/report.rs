//! „Fehler melden …“: Text für ein GitHub-Issue zusammenstellen, interne Angaben entfernen und als Adresse für den
//! Browser aufbereiten. Gesendet wird nichts; den Bericht schickt der Anwender im Browser selbst ab.

/// Adresse zum Anlegen eines Issues im öffentlichen Repository.
pub const ISSUE_URL: &str = "https://github.com/joeMJ/remotejoe/issues/new";

/// Obergrenze der Adresse; längere Texte werden gekürzt (Browser und GitHub kappen sonst ohne Hinweis).
pub const MAX_URL: usize = 7500;

/// Interne Angaben ersetzen: IPv4-Adressen, Mailadressen, IPv6-Adressen und die übergebenen Wörter
/// (Rechner- und Benutzername, Namen und Adressen der Ziele).
pub fn sanitize(text: &str, words: &[(String, &str)]) -> String {
    let mut out = ipv4(text);
    out = mail(&out);
    out = ipv6(&out);
    // Längere Wörter zuerst, damit „srv01.example“ vor „srv01“ ersetzt wird.
    let mut w: Vec<&(String, &str)> = words.iter().filter(|(s, _)| s.chars().count() >= 3).collect();
    w.sort_by_key(|(s, _)| std::cmp::Reverse(s.len()));
    for (s, with) in w {
        out = replace_ci(&out, s, with);
    }
    out
}

fn replace_ci(text: &str, word: &str, with: &str) -> String {
    let lower = text.to_lowercase();
    let needle = word.to_lowercase();
    if needle.is_empty() || lower.len() != text.len() {
        return text.replace(word, with);
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(p) = lower[i..].find(&needle) {
        out.push_str(&text[i..i + p]);
        out.push_str(with);
        i += p + needle.len();
    }
    out.push_str(&text[i..]);
    out
}

fn ipv4(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        // Versionsnummern („Version 8.10.0.23“) sind keine Adressen und für Berichte wichtig.
        let after_version = text[..i].trim_end().to_ascii_lowercase().ends_with("version");
        if !after_version && b[i].is_ascii_digit() && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.')) {
            // Vier Zahlenblöcke 0–255 mit Punkten.
            let mut j = i;
            let mut parts = 0;
            let mut ok = true;
            loop {
                let s = j;
                while j < b.len() && b[j].is_ascii_digit() && j - s < 3 {
                    j += 1;
                }
                if j == s || text[s..j].parse::<u16>().map_or(true, |n| n > 255) {
                    ok = false;
                    break;
                }
                parts += 1;
                if parts == 4 || j >= b.len() || b[j] != b'.' || j + 1 >= b.len() || !b[j + 1].is_ascii_digit() {
                    break;
                }
                j += 1;
            }
            if ok && parts == 4 && (j >= b.len() || !b[j].is_ascii_digit()) {
                out.push_str("<IP>");
                i = j;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn mail(text: &str) -> String {
    text.split_inclusive(char::is_whitespace)
        .map(|w| {
            let core = w.trim_end();
            let tail = &w[core.len()..];
            let trimmed = core.trim_matches(|c: char| matches!(c, '<' | '>' | '(' | ')' | ',' | ';' | '"' | '\'' | '„' | '“'));
            match trimmed.split_once('@') {
                Some((a, d)) if !a.is_empty() && d.contains('.') && !d.starts_with('.') => format!("{}{tail}", core.replace(trimmed, "<Mail>")),
                _ => w.to_string(),
            }
        })
        .collect()
}

fn ipv6(text: &str) -> String {
    text.split_inclusive(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | '(' | ')' | ',' | '/'))
        .map(|w| {
            let (core, tail) = match w.char_indices().last() {
                Some((i, c)) if c.is_whitespace() || matches!(c, '[' | ']' | '(' | ')' | ',' | '/') => (&w[..i], &w[i..]),
                _ => (w, ""),
            };
            // Zonenangabe („%eth0“) gehört zur Adresse, ist aber kein Hexblock.
            let addr = core.split('%').next().unwrap_or(core);
            let hexish = !addr.is_empty() && addr.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
            let colons = addr.matches(':').count();
            // MAC-Adressen (aa:bb:cc:dd:ee:ff) bleiben; IPv6 hat „::“ oder mehr als fünf Doppelpunkte mit Hexblöcken.
            let mac = colons == 5 && addr.split(':').all(|p| p.len() == 2);
            if hexish && !mac && (addr.contains("::") && colons >= 2 || colons >= 6) {
                format!("<IPv6>{tail}")
            } else {
                w.to_string()
            }
        })
        .collect()
}

/// Prozentkodierung für die Adresszeile (alles außer unreservierten Zeichen).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Adresse für den Browser; ist der Text zu lang, wird er gekürzt (Rückgabe: Adresse, gekürzt?).
pub fn issue_url(title: &str, body: &str, label: &str) -> (String, bool) {
    let base = format!("{ISSUE_URL}?labels={}&title={}&body=", encode(label), encode(title));
    let full = format!("{base}{}", encode(body));
    if full.len() <= MAX_URL {
        return (full, false);
    }
    let note = "\n\n(… gekürzt – der vollständige Text liegt in der Zwischenablage und kann hier eingefügt werden.)";
    let mut cut: String = body.to_string();
    while !cut.is_empty() && base.len() + encode(&cut).len() + encode(note).len() > MAX_URL {
        let n = cut.chars().count();
        cut = cut.chars().take(n.saturating_sub((n / 10).max(1))).collect();
    }
    (format!("{base}{}", encode(&format!("{cut}{note}"))), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_details_are_removed() {
        let words = vec![("werkbank".to_string(), "<Rechner>"), ("john".to_string(), "<Benutzer>"), ("ap-flur-2".to_string(), "<Ziel>")];
        let t = "Fehler bei 192.0.2.1:22 und 198.51.100.4 (ap-flur-2), Mail an a.b@example.org; MAC 94:64:24:cc:9b:12, v6 fe80::1%eth0, Version 8.10.0.23, Nutzer John auf Werkbank";
        let s = sanitize(t, &words);
        assert!(!s.contains("192.168") && !s.contains("10.63") && !s.contains("example.org") && !s.contains("fe80"), "{s}");
        assert!(s.contains("<IP>:22") && s.contains("<Mail>") && s.contains("<IPv6>") && s.contains("<Ziel>") && s.contains("<Benutzer>") && s.contains("<Rechner>"), "{s}");
        assert!(s.contains("94:64:24:cc:9b:12"), "MAC bleibt: {s}");
        assert!(s.contains("Version 8.10.0.23"), "Versionsnummer bleibt: {s}");
    }

    #[test]
    fn long_texts_are_cut_and_marked() {
        let (u, cut) = issue_url("Titel", "kurz", "bug");
        assert!(!cut && u.starts_with(ISSUE_URL) && u.contains("title=Titel") && u.contains("body=kurz") && u.contains("labels=bug"));
        let long = "Zeile mit Umlauten äöü\n".repeat(2000);
        let (u, cut) = issue_url("T", &long, "bug");
        assert!(cut && u.len() <= MAX_URL, "{}", u.len());
        assert!(u.contains(&encode("gekürzt")));
    }
}
