//! Export: Struktur und Ziele als JSON (remotejoe-Austausch) oder CSV (im Stil von Remote Desktop
//! Manager), auf Wunsch mit Kennwörtern; Dateien mit Kennwörtern gehören in ein verschlüsseltes ZIP (AES-256).
//!
//! Der JSON-Export enthält Ordner, Ziele, Protokolloptionen und die wirksamen Zugangsdaten je Ziel
//! (Benutzername, Domäne, nur auf Wunsch das Kennwort). Einstellungen der Ordner (Verweise auf
//! Zugangsdaten, Tunnel, SNMP) gehören nicht dazu: die Verweise gelten nur in der eigenen Schlüsseldatenbank.

use std::io::{Read, Write};

use rj_core::{Protocol, ProtocolOptions, Secret};
use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::{ImportError, SourceCredential, SourceDocument, SourceItem};

pub const FORMAT: &str = "remotejoe-export";
pub const VERSION: u64 = 1;

/// Knoten des Exportbaums.
pub enum ExportNode {
    Folder { name: String, children: Vec<ExportNode> },
    Target { name: String, protocol: Protocol, host: String, port: Option<u16>, options: ProtocolOptions, username: String, domain: Option<String>, password: Option<Secret> },
}

fn password_text(p: &Secret) -> String {
    String::from_utf8_lossy(p.as_bytes()).into_owned()
}

fn node_json(n: &ExportNode, passwords: bool) -> Value {
    match n {
        ExportNode::Folder { name, children } => json!({
            "type": "folder",
            "name": name,
            "children": children.iter().map(|c| node_json(c, passwords)).collect::<Vec<_>>(),
        }),
        ExportNode::Target { name, protocol, host, port, options, username, domain, password } => {
            let mut v = json!({
                "type": "target",
                "name": name,
                "protocol": protocol.as_str(),
                "host": host,
                "options": serde_json::to_value(options).unwrap_or(Value::Null),
            });
            if let Some(p) = port {
                v["port"] = json!(p);
            }
            if !username.is_empty() {
                v["username"] = json!(username);
            }
            if let Some(d) = domain.as_ref().filter(|d| !d.is_empty()) {
                v["domain"] = json!(d);
            }
            if passwords {
                if let Some(p) = password.as_ref().filter(|p| !p.as_bytes().is_empty()) {
                    v["password"] = json!(password_text(p));
                }
            }
            v
        }
    }
}

/// JSON-Dokument des Exports (mit Kennwörtern nur, wenn `passwords`).
pub fn to_json(nodes: &[ExportNode], passwords: bool) -> Zeroizing<String> {
    let doc = json!({
        "format": FORMAT,
        "version": VERSION,
        "passwords": passwords,
        "tree": nodes.iter().map(|n| node_json(n, passwords)).collect::<Vec<_>>(),
    });
    Zeroizing::new(serde_json::to_string_pretty(&doc).unwrap_or_default())
}

/// Feld für CSV: bei Komma, Anführungszeichen oder Zeilenumbruch in Anführungszeichen. Beginnt es mit
/// `=`, `+`, `-` oder `@` (Tabellenkalkulationen führen das als Formel aus), kommt ein `'` davor.
fn csv_field(s: &str, protect: bool) -> String {
    let guarded = if protect && s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_string() };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

fn csv_type(p: Protocol) -> &'static str {
    match p {
        Protocol::Rdp => "RDP (Microsoft Remote Desktop)",
        Protocol::Ssh => "SSH Terminal",
        Protocol::Vnc => "VNC",
        Protocol::Web => "Website",
        Protocol::Proxmox => "Proxmox",
        Protocol::Hyperv => "Hyper-V",
        Protocol::Psrp => "PowerShell",
    }
}

/// CSV im Stil von RDM (Spalten wie beim Import); Ordner stehen als Pfad mit `\` in `Group`.
pub fn to_csv(nodes: &[ExportNode], passwords: bool) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::from("ConnectionType,ConnectionSubType,SubMode,Name,Group,Description,Keywords,Expiration,Parent,Host,Port,CredentialUserName,CredentialDomain,CredentialPassword,OpenInConsole,URL,WebUrl\n"));
    fn walk(nodes: &[ExportNode], path: &str, passwords: bool, out: &mut String) {
        for n in nodes {
            match n {
                ExportNode::Folder { name, children } => {
                    let sub = if path.is_empty() { name.clone() } else { format!("{path}\\{name}") };
                    walk(children, &sub, passwords, out);
                }
                ExportNode::Target { name, protocol, host, port, options, username, domain, password } => {
                    let url = match options {
                        ProtocolOptions::Web(o) => o.url.clone().unwrap_or_else(|| rj_core::weburl::default_url(host, port.unwrap_or(443))),
                        _ => String::new(),
                    };
                    let pw = if passwords { password.as_ref().map(password_text).unwrap_or_default() } else { String::new() };
                    let cols = [
                        csv_field(csv_type(*protocol), false),
                        String::new(),
                        "0".into(),
                        csv_field(name, true),
                        csv_field(path, true),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        csv_field(host, true),
                        port.map(|p| p.to_string()).unwrap_or_default(),
                        csv_field(username, true),
                        csv_field(domain.as_deref().unwrap_or(""), true),
                        csv_field(&pw, false),
                        "Nein".into(),
                        csv_field(&url, true),
                        String::new(),
                    ];
                    out.push_str(&cols.join(","));
                    out.push('\n');
                }
            }
        }
    }
    walk(nodes, "", passwords, &mut out);
    out
}

/// Verpackt `data` als einzige Datei `name` in ein ZIP mit AES-256 (öffnen mit 7-Zip, WinZip, Keka …).
pub fn encrypted_zip(name: &str, data: &[u8], password: &str) -> Result<Vec<u8>, ImportError> {
    if password.is_empty() {
        return Err(ImportError::Format("Für das verschlüsselte ZIP fehlt das Kennwort".into()));
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .with_aes_encryption(zip::AesMode::Aes256, password);
        w.start_file(name, opts).map_err(|e| ImportError::Format(format!("ZIP: {e}")))?;
        w.write_all(data)?;
        w.finish().map_err(|e| ImportError::Format(format!("ZIP: {e}")))?;
    }
    Ok(buf.into_inner())
}

/// Ist der Text ein remotejoe-Export?
pub fn is_export(text: &str) -> bool {
    text.trim_start().starts_with('{') && text.contains(&format!("\"format\": \"{FORMAT}\"")) || text.contains(&format!("\"format\":\"{FORMAT}\""))
}

/// Liest ein ZIP mit einem Export (JSON oder CSV); verschlüsselte ZIPs brauchen `password`.
pub fn read_zip(bytes: &[u8], password: Option<&str>) -> Result<SourceDocument, ImportError> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| ImportError::Format(format!("ZIP: {e}")))?;
    if z.is_empty() {
        return Err(ImportError::Format("ZIP enthält keine Datei".into()));
    }
    let (name, encrypted) = {
        let f = z.by_index_raw(0).map_err(|e| ImportError::Format(format!("ZIP: {e}")))?;
        (f.name().map(|n| n.to_string()).unwrap_or_default(), f.encrypted())
    };
    let mut data = Vec::new();
    let limit = crate::MAX_FILE + 1;
    if encrypted {
        let pw = password.filter(|p| !p.is_empty()).ok_or(ImportError::PasswordNeeded)?;
        let mut f = z.by_index_decrypt(0, pw.as_bytes()).map_err(|_| ImportError::PasswordWrong)?;
        (&mut f).take(limit).read_to_end(&mut data).map_err(|_| ImportError::PasswordWrong)?;
    } else {
        let mut f = z.by_index(0).map_err(|e| ImportError::Format(format!("ZIP: {e}")))?;
        (&mut f).take(limit).read_to_end(&mut data)?;
    }
    if data.len() as u64 > crate::MAX_FILE {
        return Err(ImportError::Format(format!("ZIP: „{name}“ ist entpackt größer als {} MiB", crate::MAX_FILE >> 20)));
    }
    let text = crate::text(&data, &name)?;
    if name.to_ascii_lowercase().ends_with(".csv") {
        crate::csv::read_csv(text)
    } else if is_export(text) {
        read_json(text)
    } else {
        crate::rdm::read_json(text)
    }
}

/// Liest ein JSON-Dokument des remotejoe-Exports.
pub fn read_json(text: &str) -> Result<SourceDocument, ImportError> {
    let v: Value = serde_json::from_str(text).map_err(|e| ImportError::Format(format!("JSON: {e}")))?;
    if v.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(ImportError::Format("Kein remotejoe-Export".into()));
    }
    if v.get("version").and_then(Value::as_u64).unwrap_or(0) > VERSION {
        return Err(ImportError::Format("Der Export stammt aus einer neueren Fassung von remotejoe".into()));
    }
    let mut doc = SourceDocument { format: "remotejoe (Export)".into(), ..Default::default() };
    let mut counter = 0u64;
    fn walk(nodes: &[Value], parent: Option<&str>, doc: &mut SourceDocument, counter: &mut u64, depth: usize) {
        for (i, n) in nodes.iter().enumerate() {
            *counter += 1;
            let id = format!("x{counter}");
            let name = n.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            match n.get("type").and_then(Value::as_str) {
                Some("folder") => {
                    doc.items.push(SourceItem::Folder { id: id.clone(), parent: parent.map(str::to_string), name, order: i as i64 });
                    if depth < crate::MAX_DEPTH {
                        if let Some(c) = n.get("children").and_then(Value::as_array) {
                            walk(c, Some(&id), doc, counter, depth + 1);
                        }
                    }
                }
                Some("target") => {
                    let protocol = n.get("protocol").and_then(Value::as_str).and_then(|p| serde_json::from_value::<Protocol>(Value::String(p.to_string())).ok());
                    let Some(protocol) = protocol else {
                        doc.skipped.push((name, "Protokoll unbekannt".into()));
                        continue;
                    };
                    let options = n.get("options").filter(|o| !o.is_null()).and_then(|o| serde_json::from_value::<ProtocolOptions>(o.clone()).ok());
                    let mut host = n.get("host").and_then(Value::as_str).unwrap_or("").to_string();
                    if let Some(ProtocolOptions::Web(w)) = &options {
                        if let Some(u) = w.url.as_ref().filter(|u| !u.is_empty()) {
                            host = u.clone();
                        }
                    }
                    let username = n.get("username").and_then(Value::as_str).unwrap_or("").to_string();
                    let password = n.get("password").and_then(Value::as_str).map(Secret::from_str_exact);
                    let credential = (!username.is_empty() || password.is_some()).then(|| SourceCredential {
                        username,
                        domain: n.get("domain").and_then(Value::as_str).map(str::to_string),
                        password,
                    });
                    doc.items.push(SourceItem::Connection {
                        id,
                        parent: parent.map(str::to_string),
                        name,
                        order: i as i64,
                        protocol,
                        host,
                        port: n.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()),
                        credential,
                        credential_ref: None,
                        options,
                    });
                }
                _ => doc.skipped.push((name, "Unbekannter Eintragstyp".into())),
            }
        }
    }
    if let Some(tree) = v.get("tree").and_then(Value::as_array) {
        walk(tree, None, &mut doc, &mut counter, 0);
    }
    if v.get("passwords").and_then(Value::as_bool) == Some(true) {
        doc.warnings.push("Die Datei enthält Kennwörter im Klartext (ZIP entschlüsselt). Löschen Sie sie nach dem Import sicher.".into());
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rj_core::model::WebOptions;

    fn sample() -> Vec<ExportNode> {
        vec![ExportNode::Folder {
            name: "Nord".into(),
            children: vec![
                ExportNode::Folder {
                    name: "Computer".into(),
                    children: vec![ExportNode::Target { name: "rechner, 1".into(), protocol: Protocol::Rdp, host: "host1.example.test".into(), port: Some(3390), options: ProtocolOptions::None, username: "user1".into(), domain: Some("DOM".into()), password: Some(Secret::from_str_exact("geheim \"1\", ä")) }],
                },
                ExportNode::Target {
                    name: "=seite".into(),
                    protocol: Protocol::Web,
                    host: "host2.example.test".into(),
                    port: Some(8006),
                    options: ProtocolOptions::Web(WebOptions { url: Some("https://host2.example.test:8006/".into()), ignore_certificate: true, ..Default::default() }),
                    username: "root@pam".into(),
                    domain: None,
                    password: Some(Secret::from_str_exact("pw2")),
                },
            ],
        }]
    }

    #[test]
    fn json_roundtrip_keeps_structure_options_and_credentials_but_passwords_only_on_request() {
        let ohne = to_json(&sample(), false);
        assert!(!ohne.contains("geheim") && !ohne.contains("pw2"), "kein Kennwort im Export ohne Wunsch");
        assert!(ohne.contains("\"passwords\": false"));
        let doc = read_json(&ohne).unwrap();
        assert_eq!(doc.items.iter().filter(|i| matches!(i, SourceItem::Folder { .. })).count(), 2);
        let mit = to_json(&sample(), true);
        let doc = read_json(&mit).unwrap();
        let conns: Vec<_> = doc.items.iter().filter_map(|i| match i { SourceItem::Connection { name, protocol, host, port, credential, options, parent, .. } => Some((name.clone(), *protocol, host.clone(), *port, credential.clone(), options.clone(), parent.clone())), _ => None }).collect();
        assert_eq!(conns.len(), 2);
        let rdp = &conns[0];
        assert_eq!((rdp.0.as_str(), rdp.1, rdp.2.as_str(), rdp.3), ("rechner, 1", Protocol::Rdp, "host1.example.test", Some(3390)));
        let c = rdp.4.as_ref().unwrap();
        assert_eq!((c.username.as_str(), c.domain.as_deref(), c.password.as_ref().unwrap().as_str()), ("user1", Some("DOM"), Some("geheim \"1\", ä")));
        assert!(rdp.6.is_some(), "liegt im Ordner Computer");
        let web = &conns[1];
        assert_eq!(web.2, "https://host2.example.test:8006/", "Web: Adresse als Host, der Plan zerlegt sie");
        assert!(matches!(&web.5, Some(ProtocolOptions::Web(o)) if o.ignore_certificate));
        assert!(doc.warnings.iter().any(|w| w.contains("Kennwörter im Klartext")));
        assert!(read_json("{\"format\":\"anderes\"}").is_err());
        assert!(read_json(&to_json(&sample(), false).replace("\"version\": 1", "\"version\": 99")).is_err());
    }

    #[test]
    fn csv_export_matches_what_the_csv_import_reads() {
        let csv = to_csv(&sample(), true);
        assert!(csv.starts_with("ConnectionType,ConnectionSubType,SubMode,Name,Group"));
        assert!(csv.contains("\"rechner, 1\"") && csv.contains("Nord\\Computer"));
        assert!(csv.contains("'=seite"), "Formelzeichen am Anfang werden entschärft");
        assert!(csv.contains("\"geheim \"\"1\"\", ä\""));
        let doc = crate::csv::read_csv(&csv).unwrap();
        let n = doc.items.iter().filter(|i| matches!(i, SourceItem::Connection { .. })).count();
        assert_eq!(n, 2);
        assert!(!to_csv(&sample(), false).contains("geheim") && !to_csv(&sample(), false).contains("pw2"));
    }

    #[test]
    fn encrypted_zip_needs_the_right_password_and_roundtrips() {
        let json = to_json(&sample(), true);
        let zip = encrypted_zip("remotejoe-export.json", json.as_bytes(), "Zip-Kennwort ä1").unwrap();
        assert!(!zip.windows(6).any(|w| w == b"geheim"), "Inhalt im ZIP nicht lesbar");
        assert!(matches!(read_zip(&zip, None), Err(ImportError::PasswordNeeded)));
        assert!(matches!(read_zip(&zip, Some("falsch")), Err(ImportError::PasswordWrong)));
        let doc = read_zip(&zip, Some("Zip-Kennwort ä1")).unwrap();
        assert_eq!(doc.items.iter().filter(|i| matches!(i, SourceItem::Connection { .. })).count(), 2);
        assert!(encrypted_zip("x.json", b"{}", "").is_err());
    }
}
