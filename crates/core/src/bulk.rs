//! Mehrere Ziele auf einmal anlegen: ein Host pro Zeile, Namensmuster mit
//! Platzhaltern, Vorschau mit Hinweisen je Zeile.
//!
//! Leerzeilen und Zeilen mit `#` am Anfang werden ignoriert. Eine Zeile darf
//! `host`, `host:port`, `[IPv6]:port` oder eine IPv6-Adresse ohne Port
//! enthalten. Auffällige Zeichen (Steuerzeichen, Zeichen ohne Breite,
//! Schreibrichtungs-Steuerzeichen) werden markiert und machen die Zeile
//! ungültig; entfernt wird nichts.

use std::collections::HashSet;
use std::net::{Ipv4Addr, Ipv6Addr};

use uuid::Uuid;

use crate::charset::{find_suspicious, mark_visible};
use crate::edit::{self, EditError};
use crate::model::*;

/// Höchstzahl der Zeilen (ohne Leer- und Kommentarzeilen) je Vorgang.
pub const MAX_ROWS: usize = 500;

/// Standard-Namensmuster.
pub const DEFAULT_PATTERN: &str = "{host}";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatternError {
    #[error("Namensmuster ist leer")]
    Empty,
    #[error("Unbekannter Platzhalter „{{{0}}}“ (erlaubt: {{host}}, {{nr}})")]
    Unknown(String),
    #[error("Namensmuster: „{{“ ohne „}}“")]
    Unclosed,
}

/// Eine Zeile der Vorschau.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Zeilennummer im Eingabefeld (ab 1).
    pub line: usize,
    /// Zeile wie eingegeben (ohne Zeilenende).
    pub raw: String,
    /// Laufende Nummer (`{nr}`, ab 1, nur Zeilen mit Inhalt).
    pub nr: usize,
    pub host: String,
    /// Port aus der Zeile; sonst `None` (Vorlage oder Standardport).
    pub port: Option<u16>,
    /// Port, der tatsächlich gilt.
    pub effective_port: u16,
    pub name: String,
    /// Fehler: die Zeile wird nicht angelegt.
    pub errors: Vec<String>,
    /// Hinweise ohne Wirkung auf das Anlegen.
    pub hints: Vec<String>,
}

impl Row {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Ersetzt die Platzhalter `{host}` und `{nr}`.
pub fn expand_pattern(pattern: &str, host: &str, nr: usize) -> Result<String, PatternError> {
    if pattern.is_empty() {
        return Err(PatternError::Empty);
    }
    let mut out = String::with_capacity(pattern.len() + host.len());
    let mut rest = pattern;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('}').ok_or(PatternError::Unclosed)?;
        match &after[..end] {
            "host" => out.push_str(host),
            "nr" => out.push_str(&nr.to_string()),
            other => return Err(PatternError::Unknown(other.to_string())),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Prüft das Namensmuster ohne Eingabezeilen.
pub fn check_pattern(pattern: &str) -> Result<(), PatternError> {
    expand_pattern(pattern, "h", 1).map(|_| ())
}

fn is_hostname(h: &str) -> bool {
    if h.len() > 253 {
        return false;
    }
    let h = h.strip_suffix('.').unwrap_or(h);
    !h.is_empty()
        && h.split('.').all(|l| {
            !l.is_empty()
                && l.chars().count() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        })
}

/// Zerlegt eine Zeile in Host und Port. Fehlertext bei ungültiger Angabe.
fn split_host_port(s: &str) -> Result<(String, Option<u16>), String> {
    let port_of = |p: &str| -> Result<u16, String> {
        match p.parse::<u32>() {
            Ok(n @ 1..=65535) => Ok(n as u16),
            _ => Err(format!("Port „{}“ ungültig (1 bis 65535)", mark_visible(p, false))),
        }
    };
    if let Some(r) = s.strip_prefix('[') {
        let (addr, tail) = r.split_once(']').ok_or("„[“ ohne „]“")?;
        addr.parse::<Ipv6Addr>().map_err(|_| format!("„{}“ ist keine IPv6-Adresse", mark_visible(addr, false)))?;
        let port = match tail {
            "" => None,
            t => Some(port_of(t.strip_prefix(':').ok_or("Nach „]“ wird „:Port“ erwartet")?)?),
        };
        return Ok((addr.to_string(), port));
    }
    if s.parse::<Ipv6Addr>().is_ok() {
        return Ok((s.to_string(), None));
    }
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) => (h, Some(port_of(p)?)),
        None => (s, None),
    };
    if host.contains(':') {
        return Err("IPv6-Adresse mit Port bitte als „[Adresse]:Port“ angeben".into());
    }
    if host.parse::<Ipv4Addr>().is_err() {
        // Nur Ziffern und Punkte: als IPv4-Adresse gemeint.
        if host.split('.').all(|p| p.chars().all(|c| c.is_ascii_digit())) {
            return Err(format!("„{host}“ ist keine gültige IPv4-Adresse"));
        }
        if !is_hostname(host) {
            return Err(format!("„{}“ ist kein gültiger Hostname", mark_visible(host, false)));
        }
    }
    Ok((host.to_string(), port))
}

/// Erstellt die Vorschau. `port_default` ist der Port der Vorlage (sonst
/// Standardport des Protokolls). Doppelte Namen werden gegen die Objekte im
/// Ordner `parent` und innerhalb der Eingabe geprüft (ohne Groß-/Kleinschreibung).
pub fn plan(index: &Index, parent: Uuid, protocol: Protocol, text: &str, pattern: &str, port_default: Option<u16>) -> Result<Vec<Row>, PatternError> {
    check_pattern(pattern)?;
    let existing: HashSet<String> = edit::folder(index, parent).map(|f| f.children.iter().map(|c| c.name().to_lowercase()).collect()).unwrap_or_default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut rows = Vec::new();
    let mut nr = 0;
    for (i, raw) in text.split('\n').enumerate() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = raw.trim_matches(|c| c == ' ' || c == '\t');
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        nr += 1;
        let mut row = Row {
            line: i + 1,
            raw: raw.to_string(),
            nr,
            host: String::new(),
            port: None,
            effective_port: port_default.unwrap_or_else(|| protocol.default_port()),
            name: String::new(),
            errors: Vec::new(),
            hints: Vec::new(),
        };
        if nr > MAX_ROWS {
            row.errors.push(format!("Mehr als {MAX_ROWS} Zeilen – bitte in mehreren Schritten anlegen"));
            rows.push(row);
            continue;
        }
        let findings = find_suspicious(trimmed, false);
        if !findings.is_empty() {
            let kinds: Vec<String> = findings.iter().map(|f| f.kind.to_string()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
            row.errors.push(format!("Auffällige Zeichen ({}): {}", kinds.join(", "), mark_visible(trimmed, false)));
            row.host = trimmed.to_string();
        } else if trimmed.contains([' ', '\t']) {
            row.errors.push("Leerzeichen im Host – ein Host pro Zeile".into());
            row.host = trimmed.to_string();
        } else {
            match split_host_port(trimmed) {
                Ok((h, p)) => {
                    row.host = h;
                    row.port = p;
                    if let Some(p) = p {
                        row.effective_port = p;
                    }
                    if !row.host.is_ascii() {
                        row.hints.push("Hostname mit Zeichen außerhalb von ASCII (internationalisierter Name)".into());
                    }
                }
                Err(e) => {
                    row.errors.push(e);
                    row.host = trimmed.to_string();
                }
            }
        }
        row.name = expand_pattern(pattern, &row.host, nr)?;
        if !find_suspicious(&row.name, false).is_empty() && row.errors.is_empty() {
            row.errors.push(format!("Name enthält auffällige Zeichen: {}", mark_visible(&row.name, false)));
        }
        let key = row.name.to_lowercase();
        if existing.contains(&key) {
            row.errors.push(format!("Name „{}“ gibt es im Ordner schon", mark_visible(&row.name, false)));
        } else if !seen.insert(key) {
            row.errors.push(format!("Name „{}“ kommt in der Eingabe mehrfach vor", mark_visible(&row.name, false)));
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Eingabetext nach dem Anlegen: nur die ungültigen Zeilen, unverändert.
pub fn remaining_text(rows: &[Row]) -> String {
    rows.iter().filter(|r| !r.is_valid()).map(|r| r.raw.as_str()).collect::<Vec<_>>().join("\n")
}

/// Legt alle gültigen Zeilen als Ziele im Ordner `parent` an (in der
/// Reihenfolge der Eingabe) und liefert ihre Kennungen.
pub fn apply(index: &mut Index, parent: Uuid, protocol: Protocol, rows: &[Row], template: Option<&Template>) -> Result<Vec<Uuid>, EditError> {
    edit::folder(index, parent).ok_or(EditError::NotAFolder)?;
    let mut ids = Vec::new();
    for r in rows.iter().filter(|r| r.is_valid()) {
        let mut t = edit::new_target(index, parent, protocol, &r.name, template);
        t.host = r.host.clone();
        if r.port.is_some() {
            t.port = r.port;
        }
        if let ProtocolOptions::Web(w) = &mut t.options {
            if w.url.as_deref().unwrap_or_default().is_empty() {
                let host = if r.host.contains(':') { format!("[{}]", r.host) } else { r.host.clone() };
                w.url = Some(match t.port {
                    Some(p) if p != 443 => format!("https://{host}:{p}/"),
                    _ => format!("https://{host}/"),
                });
            }
        }
        ids.push(edit::insert(index, parent, Node::Target(t), None)?);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_with(names: &[&str]) -> (Index, Uuid) {
        let mut idx = Index::default();
        let mut f = Folder::new("Server");
        for n in names {
            f.children.push(Node::Target(Target::new(*n, Protocol::Ssh, "192.0.2.1")));
        }
        let fid = f.id;
        idx.root.children.push(Node::Folder(f));
        (idx, fid)
    }

    #[test]
    fn pattern_placeholders() {
        assert_eq!(expand_pattern("{host}", "a.example.test", 3).unwrap(), "a.example.test");
        assert_eq!(expand_pattern("web-{nr} ({host})", "x", 12).unwrap(), "web-12 (x)");
        assert_eq!(expand_pattern("fest", "x", 1).unwrap(), "fest");
        assert_eq!(expand_pattern("{name}", "x", 1), Err(PatternError::Unknown("name".into())));
        assert_eq!(expand_pattern("{host", "x", 1), Err(PatternError::Unclosed));
        assert_eq!(expand_pattern("", "x", 1), Err(PatternError::Empty));
    }

    #[test]
    fn lines_comments_ports_and_invalid_hosts() {
        let (idx, fid) = index_with(&[]);
        let text = "# Kommentar\n\na.example.test\r\n  b.example.test:2222  \n192.0.2.7\n[2001:db8::1]:2200\n2001:db8::2\nbad host\n-x.example.test\n999.1.1.1\nc.example.test:0\n";
        let rows = plan(&idx, fid, Protocol::Ssh, text, "{nr}-{host}", None).unwrap();
        let v: Vec<_> = rows.iter().map(|r| (r.line, r.host.as_str(), r.effective_port, r.is_valid())).collect();
        assert_eq!(
            v,
            vec![
                (3, "a.example.test", 22, true),
                (4, "b.example.test", 2222, true),
                (5, "192.0.2.7", 22, true),
                (6, "2001:db8::1", 2200, true),
                (7, "2001:db8::2", 22, true),
                (8, "bad host", 22, false),
                (9, "-x.example.test", 22, false),
                (10, "999.1.1.1", 22, false),
                (11, "c.example.test:0", 22, false),
            ]
        );
        assert_eq!(rows[0].name, "1-a.example.test");
        assert_eq!(rows[4].nr, 5);
        assert_eq!(remaining_text(&rows), "bad host\n-x.example.test\n999.1.1.1\nc.example.test:0");
    }

    #[test]
    fn duplicate_names_in_folder_and_input() {
        let (idx, fid) = index_with(&["A.example.test"]);
        let rows = plan(&idx, fid, Protocol::Ssh, "a.example.test\nb.example.test\nb.example.test", "{host}", None).unwrap();
        assert!(rows[0].errors[0].contains("gibt es im Ordner schon"));
        assert!(rows[1].is_valid());
        assert!(rows[2].errors[0].contains("mehrfach"));
        // Mit {nr} sind die Namen eindeutig.
        let rows = plan(&idx, fid, Protocol::Ssh, "b.example.test\nb.example.test", "s{nr}", None).unwrap();
        assert!(rows.iter().all(Row::is_valid));
    }

    #[test]
    fn limit_of_500_rows() {
        let (idx, fid) = index_with(&[]);
        let text: String = (1..=502).map(|i| format!("h{i}.example.test\n# x\n\n")).collect();
        let rows = plan(&idx, fid, Protocol::Rdp, &text, "{host}", Some(3390)).unwrap();
        assert_eq!(rows.len(), 502);
        assert_eq!(rows.iter().filter(|r| r.is_valid()).count(), MAX_ROWS);
        assert!(rows[500].errors[0].contains("500"));
        assert_eq!(rows[0].effective_port, 3390);
        assert_eq!(remaining_text(&rows), "h501.example.test\nh502.example.test");
    }

    #[test]
    fn suspicious_characters_are_marked_not_removed() {
        let (idx, fid) = index_with(&[]);
        let text = "a\u{200B}.example.test\nb\u{202E}.example.test\nc\u{7}.example.test\nd.example.test";
        let rows = plan(&idx, fid, Protocol::Ssh, text, "{host}", None).unwrap();
        assert!(rows[0].errors[0].contains("⟦U+200B⟧") && rows[0].errors[0].contains("ohne Breite"));
        assert!(rows[1].errors[0].contains("⟦U+202E⟧"));
        assert!(rows[2].errors[0].contains("⟦U+0007⟧"));
        assert!(rows[3].is_valid());
        // Die ungültigen Zeilen bleiben byte-genau stehen.
        assert_eq!(remaining_text(&rows), "a\u{200B}.example.test\nb\u{202E}.example.test\nc\u{7}.example.test");
        // Auffällige Zeichen im Muster.
        let rows = plan(&idx, fid, Protocol::Ssh, "d.example.test", "x\u{2066}{host}", None).unwrap();
        assert!(rows[0].errors[0].contains("⟦U+2066⟧"));
    }

    #[test]
    fn apply_creates_valid_rows_with_template() {
        let (mut idx, fid) = index_with(&[]);
        let mut tpl = edit::template_from_target(&Target::new("x", Protocol::Web, ""), "Web");
        tpl.port = Some(8443);
        let rows = plan(&idx, fid, Protocol::Web, "a.example.test\nb.example.test:9443\nbad host", "{host}", tpl.port).unwrap();
        let ids = apply(&mut idx, fid, Protocol::Web, &rows, Some(&tpl)).unwrap();
        assert_eq!(ids.len(), 2);
        let f = edit::folder(&idx, fid).unwrap();
        let Node::Target(a) = &f.children[0] else { panic!() };
        assert_eq!((a.host.as_str(), a.port), ("a.example.test", Some(8443)));
        let ProtocolOptions::Web(w) = &a.options else { panic!() };
        assert_eq!(w.url.as_deref(), Some("https://a.example.test:8443/"));
        let Node::Target(b) = &f.children[1] else { panic!() };
        assert_eq!(b.port, Some(9443));
    }
}
