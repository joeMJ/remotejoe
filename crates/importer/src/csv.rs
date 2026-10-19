//! Remote Desktop Manager: CSV-Export (Kopfzeile mit `ConnectionType`, `Name`, `Group`, `Host`,
//! `Port`, `CredentialUserName`, `CredentialDomain`, `CredentialPassword`, `URL` …).
//!
//! Die Datei enthält Kennwörter im Klartext. Sie werden nur in die Schlüsseldatenbank übernommen
//! und nie angezeigt; der Bericht weist auf das sichere Löschen der Datei hin.

use std::collections::{HashMap, HashSet};

use rj_core::{Protocol, Secret};

use crate::rdm::ensure_path;
use crate::{ImportError, SourceCredential, SourceDocument, SourceItem};

/// Mehr Zeilen werden nicht gelesen (Schutz vor entarteten Dateien).
const MAX_RECORDS: usize = 200_000;

/// Spalten, die übernommen werden; alle anderen mit Inhalt landen im Bericht.
const KNOWN: &[&str] = &[
    "connectiontype", "name", "group", "host", "port", "credentialusername", "credentialdomain", "credentialpassword", "url", "weburl",
];

/// Zerlegt CSV nach RFC 4180 (Anführungszeichen, `""`, Trennzeichen und Zeilenumbrüche in Feldern).
fn records(text: &str, delim: char) -> Result<Vec<Vec<String>>, ImportError> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                started = true;
            }
            c if c == delim => {
                row.push(std::mem::take(&mut field));
                started = true;
            }
            '\r' => {}
            '\n' => {
                if started || !field.is_empty() || !row.is_empty() {
                    row.push(std::mem::take(&mut field));
                    out.push(std::mem::take(&mut row));
                    if out.len() > MAX_RECORDS {
                        return Err(ImportError::Format(format!("mehr als {MAX_RECORDS} Zeilen")));
                    }
                }
                started = false;
            }
            c => {
                field.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err(ImportError::Format("Anführungszeichen nicht geschlossen".into()));
    }
    if started || !field.is_empty() || !row.is_empty() {
        row.push(field);
        out.push(row);
    }
    Ok(out)
}

/// Trennzeichen aus der Kopfzeile: Komma, Semikolon oder Tabulator (häufigstes).
fn delimiter(text: &str) -> char {
    let head = text.lines().next().unwrap_or("");
    [',', ';', '\t'].into_iter().max_by_key(|d| head.matches(*d).count()).filter(|d| head.contains(*d)).unwrap_or(',')
}

/// Protokoll zum Typnamen, wie ihn RDM in der CSV schreibt („RDP (Microsoft Remote Desktop)“, „SSH Terminal“, „Website“).
fn protocol(kind: &str) -> Option<Protocol> {
    let k = kind.trim().to_lowercase();
    Some(if k.starts_with("rdp") {
        Protocol::Rdp
    } else if k.starts_with("ssh") {
        Protocol::Ssh
    } else if k.starts_with("vnc") {
        Protocol::Vnc
    } else if k.starts_with("website") || k.starts_with("web") {
        Protocol::Web
    } else if k.starts_with("proxmox") {
        Protocol::Proxmox
    } else if k.starts_with("hyper-v") || k.starts_with("hyperv") {
        Protocol::Hyperv
    } else if k.starts_with("powershell") {
        Protocol::Psrp
    } else {
        return None;
    })
}

/// Kennung aus dem `rdm://`-Verweis (`Session=<uuid>`), damit Einträge stabil bleiben.
fn session_id(web_url: &str) -> Option<String> {
    let rest = web_url.split("Session=").nth(1)?;
    let id: String = rest.chars().take_while(|c| c.is_ascii_hexdigit() || *c == '-').collect();
    (!id.is_empty()).then_some(id)
}

pub fn read_csv(text: &str) -> Result<SourceDocument, ImportError> {
    let recs = records(text, delimiter(text))?;
    let Some(head) = recs.first() else { return Err(ImportError::Format("Die CSV-Datei ist leer".into())) };
    let cols: HashMap<String, usize> = head.iter().enumerate().map(|(i, h)| (h.trim().to_lowercase(), i)).collect();
    if !cols.contains_key("connectiontype") || !cols.contains_key("name") {
        return Err(ImportError::Format("Keine RDM-CSV: die Spalten „ConnectionType“ und „Name“ fehlen".into()));
    }
    let mut doc = SourceDocument { format: "Verbindungsliste (CSV)".into(), ..Default::default() };
    let mut seen: HashSet<String> = HashSet::new();
    let mut order = 0i64;
    let mut passwords = 0usize;
    for (n, rec) in recs.iter().enumerate().skip(1) {
        let get = |c: &str| -> String { cols.get(c).and_then(|i| rec.get(*i)).map(|v| v.trim().to_string()).unwrap_or_default() };
        let name = get("name");
        // Unbekannte Spalten mit Inhalt melden.
        for (c, i) in &cols {
            if !KNOWN.contains(&c.as_str()) && rec.get(*i).is_some_and(|v| !v.trim().is_empty()) {
                let original = head[*i].trim().to_string();
                doc.note_unknown(&original);
            }
        }
        let kind = get("connectiontype");
        let Some(p) = protocol(&kind) else {
            doc.skipped.push((name, format!("Verbindungstyp „{kind}“ wird nicht unterstützt")));
            continue;
        };
        let host_col = get("host");
        let host = if p == Protocol::Web {
            let url = get("url");
            let web = get("weburl");
            if !url.is_empty() {
                url
            } else if web.starts_with("http") {
                web
            } else if !host_col.is_empty() {
                format!("https://{host_col}/")
            } else {
                String::new()
            }
        } else if !host_col.is_empty() {
            host_col
        } else {
            get("url")
        };
        if host.is_empty() {
            doc.warnings.push(format!("„{name}“: kein Host angegeben"));
        }
        let id = session_id(&get("weburl")).unwrap_or_else(|| format!("zeile-{n}"));
        let user = get("credentialusername");
        let password = rec.get(cols.get("credentialpassword").copied().unwrap_or(usize::MAX)).cloned().unwrap_or_default();
        let credential = if user.is_empty() && password.is_empty() {
            None
        } else {
            if !password.is_empty() {
                passwords += 1;
            }
            let domain = get("credentialdomain");
            Some(SourceCredential { username: user, domain: (!domain.is_empty()).then_some(domain), password: (!password.is_empty()).then(|| Secret::from_str_exact(&password)) })
        };
        let parent = ensure_path(&mut doc, &mut seen, &get("group"), &mut order);
        order += 1;
        doc.items.push(SourceItem::Connection { id, parent, name, order, protocol: p, host, port: get("port").parse().ok(), credential, credential_ref: None, options: None });
    }
    if passwords > 0 {
        doc.warnings.push(format!(
            "Die Datei enthält {passwords} Kennwörter im Klartext. Löschen Sie die CSV-Datei nach dem Import sicher (zum Beispiel mit „shred -u“) und legen Sie sie nicht in einem synchronisierten Ordner ab."
        ));
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "ConnectionType,ConnectionSubType,SubMode,Name,Group,Description,Keywords,Expiration,Parent,Host,Port,CredentialUserName,CredentialDomain,CredentialPassword,OpenInConsole,URL,WebUrl";

    #[test]
    fn quoting_delimiters_and_line_breaks() {
        let r = records("a,\"b,c\",\"d \"\"q\"\"\ne\"\r\n\r\nx,,y\n", ',').unwrap();
        assert_eq!(r, vec![vec!["a", "b,c", "d \"q\"\ne"], vec!["x", "", "y"]]);
        assert!(records("a,\"offen\n", ',').is_err());
        assert_eq!(delimiter("a;b;c,d"), ';');
        assert_eq!(delimiter("a\tb\tc"), '\t');
        assert_eq!(delimiter("nur"), ',');
    }

    #[test]
    fn rdm_csv_rows_become_targets_folders_and_report() {
        let text = format!(
            "{HEAD}\n\
RDP (Microsoft Remote Desktop),,0,rechner-1,Nord\\Computer,,,,,host1.example.test,3390,user1,DOM,geheim1,Nein,,rdm://open?DataSource=x&Session=aaaa-1111\n\
SSH Terminal,,0,\"switch, 1\",Nord,\"Zeile 1\nZeile 2\",,,,host2.example.test,,user2,,geheim2,Nein,,rdm://open?Session=bbbb-2222\n\
Website,,0,seite-1,Nord,,,,,host3.example.test,,user3,,geheim3,Nein,https://host3.example.test:8006/,rdm://open?Session=cccc-3333\n\
Website,,0,seite-2,,,,,,host4.example.test,,,,,Nein,,rdm://open?Session=dddd-4444\n\
Befehlszeile (Externe Anwendung),,0,programm-1,Nord,,,,,,,,,,Nein,,rdm://open?Session=eeee-5555\n"
        );
        let doc = read_csv(&text).unwrap();
        assert_eq!(doc.format, "Verbindungsliste (CSV)");
        let conns: Vec<_> = doc.items.iter().filter_map(|i| match i { SourceItem::Connection { name, protocol, host, port, parent, id, credential, .. } => Some((name.as_str(), *protocol, host.as_str(), *port, parent.clone(), id.as_str(), credential.clone())), _ => None }).collect();
        assert_eq!(conns.len(), 4);
        assert_eq!(conns[0].0, "rechner-1");
        assert_eq!((conns[0].1, conns[0].2, conns[0].3), (Protocol::Rdp, "host1.example.test", Some(3390)));
        assert_eq!(conns[0].4.as_deref(), Some("pfad:Nord\\Computer"));
        assert_eq!(conns[0].5, "aaaa-1111");
        let c = conns[0].6.as_ref().unwrap();
        assert_eq!((c.username.as_str(), c.domain.as_deref()), ("user1", Some("DOM")));
        assert_eq!(c.password.as_ref().unwrap().as_str(), Some("geheim1"));
        assert_eq!(conns[1].0, "switch, 1");
        assert_eq!(conns[1].1, Protocol::Ssh);
        assert_eq!(conns[2].2, "https://host3.example.test:8006/", "Web: Adresse aus URL");
        assert_eq!(conns[3].2, "https://host4.example.test/", "Web ohne URL: aus Host");
        assert!(conns[3].6.is_none());
        assert_eq!(doc.skipped.len(), 1);
        assert!(doc.skipped[0].1.contains("Befehlszeile"));
        assert!(doc.unknown_fields.iter().any(|(f, n)| f == "Description" && *n == 1));
        let w = doc.warnings.join("\n");
        assert!(w.contains("3 Kennwörter im Klartext") && w.contains("shred"), "{w}");
        assert!(!w.contains("geheim"), "Kennwörter stehen nie im Bericht");
    }

    #[test]
    fn foreign_csv_is_rejected_and_bom_is_handled_by_the_caller() {
        assert!(matches!(read_csv("a,b\n1,2\n"), Err(ImportError::Format(m)) if m.contains("ConnectionType")));
        assert!(read_csv("").is_err());
        let semi = "ConnectionType;Name;Host\nSSH Terminal;x;h.example.test\n";
        assert_eq!(read_csv(semi).unwrap().items.len(), 1);
    }
}
