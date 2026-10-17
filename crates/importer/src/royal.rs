//! Royal TS: `.rtsz` (ZIP mit JSON-Dokument) und `.rtsx` (XML), tolerant gelesen.
//!
//! Objekte tragen `ID`, `ParentID`, `Name`, `PositionNr`; Verbindungen
//! `URI`, `Port`, `CredentialUsername`/`CredentialPassword` oder
//! `CredentialID` (Verweis auf `RoyalCredential`).

use std::io::Read;

use rj_core::{Protocol, Secret};
use serde_json::Value;

use crate::xml::{self, XmlNode};
use crate::{ImportError, SourceCredential, SourceDocument, SourceItem};

const KNOWN: &[&str] = &[
    "Type", "ID", "ParentID", "Name", "PositionNr", "URI", "Port", "CredentialUsername", "CredentialPassword", "CredentialDomain",
    "CredentialID", "CredentialMode", "UserName", "Password", "Domain", "Description", "Modified", "Created", "ModifiedBy",
    "CreatedBy", "IsExpanded", "ColorFromParent", "Color",
];

fn protocol(kind: &str) -> Option<Protocol> {
    Some(match kind {
        "RoyalRDSConnection" => Protocol::Rdp,
        "RoyalSSHConnection" => Protocol::Ssh,
        "RoyalVNCConnection" => Protocol::Vnc,
        "RoyalWebConnection" => Protocol::Web,
        "RoyalPowerShellConnection" => Protocol::Psrp,
        "RoyalHyperVConnection" => Protocol::Hyperv,
        _ => return None,
    })
}

fn build(doc: &mut SourceDocument, kind: &str, get: &dyn Fn(&str) -> Option<String>, index: i64) {
    let id = get("ID").unwrap_or_else(|| format!("objekt-{index}"));
    let parent = get("ParentID").filter(|p| p != "00000000-0000-0000-0000-000000000000");
    let name = get("Name").unwrap_or_default();
    let order = get("PositionNr").and_then(|p| p.parse().ok()).unwrap_or(index);
    match kind {
        "RoyalFolder" => doc.items.push(SourceItem::Folder { id, parent, name, order }),
        "RoyalDocument" => {}
        "RoyalCredential" => {
            let username = get("UserName").unwrap_or_default();
            if username.is_empty() {
                doc.skipped.push((name, "Zugangsdatensatz ohne Benutzername".into()));
                return;
            }
            doc.items.push(SourceItem::Credential {
                id,
                name,
                credential: SourceCredential { username, domain: get("Domain"), password: get("Password").map(|p| Secret::from_str_exact(&p)) },
            });
        }
        k => match protocol(k) {
            Some(p) => {
                let host = get("URI").unwrap_or_default();
                if host.is_empty() {
                    doc.warnings.push(format!("„{name}“: kein Host angegeben"));
                }
                let credential = get("CredentialUsername").map(|u| SourceCredential {
                    username: u,
                    domain: get("CredentialDomain"),
                    password: get("CredentialPassword").map(|p| Secret::from_str_exact(&p)),
                });
                doc.items.push(SourceItem::Connection {
                    id,
                    parent,
                    name,
                    order,
                    protocol: p,
                    host,
                    port: get("Port").and_then(|p| p.parse().ok()).filter(|p| *p != 0),
                    credential,
                    credential_ref: get("CredentialID"),
                });
            }
            None => doc.skipped.push((name, format!("Objekttyp „{k}“ wird nicht unterstützt"))),
        },
    }
}

fn str_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// JSON-Dokument (Inhalt einer `.rtsz`).
pub fn read_json(text: &str) -> Result<SourceDocument, ImportError> {
    let v: Value = serde_json::from_str(text).map_err(|e| ImportError::Format(format!("JSON: {e}")))?;
    let list = v.get("Objects").and_then(Value::as_array).ok_or_else(|| ImportError::Format("kein Feld „Objects“".into()))?;
    let mut doc = SourceDocument { format: "Royal TS (rtsz)".into(), ..Default::default() };
    for (i, o) in list.iter().enumerate() {
        let Some(obj) = o.as_object() else { continue };
        for k in obj.keys() {
            if !KNOWN.contains(&k.as_str()) {
                doc.note_unknown(k);
            }
        }
        let kind = obj.get("Type").and_then(str_of).unwrap_or_default();
        build(&mut doc, &kind, &|k| obj.get(k).and_then(str_of), i as i64);
    }
    Ok(doc)
}

/// `.rtsz`: ZIP mit einem JSON-Dokument.
pub fn read_rtsz(bytes: &[u8]) -> Result<SourceDocument, ImportError> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| ImportError::Format(format!("ZIP: {e}")))?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| ImportError::Format(format!("ZIP: {e}")))?;
        let name = f.name().map(|n| n.to_string()).unwrap_or_default();
        if name.ends_with(".json") || name.ends_with(".rtsx") || !name.contains('.') {
            if f.size() > crate::MAX_FILE {
                return Err(ImportError::Format(format!("ZIP: „{name}“ ist entpackt größer als {} MiB", crate::MAX_FILE >> 20)));
            }
            let mut b = Vec::new();
            // Angabe im Archiv kann falsch sein: beim Entpacken erneut begrenzen.
            (&mut f).take(crate::MAX_FILE + 1).read_to_end(&mut b)?;
            if b.len() as u64 > crate::MAX_FILE {
                return Err(ImportError::Format(format!("ZIP: „{name}“ ist entpackt größer als {} MiB", crate::MAX_FILE >> 20)));
            }
            let t = crate::text(&b, &name)?;
            return if t.trim_start().starts_with('<') { read_rtsx(t) } else { read_json(t) };
        }
    }
    Err(ImportError::Format("ZIP enthält kein Dokument".into()))
}

/// `.rtsx`: XML mit Elementen `RoyalFolder`, `Royal…Connection`, `RoyalCredential`.
pub fn read_rtsx(text: &str) -> Result<SourceDocument, ImportError> {
    let root = xml::parse(text)?;
    let mut doc = SourceDocument { format: "Royal TS (rtsx)".into(), ..Default::default() };
    let mut n = 0i64;
    fn walk(node: &XmlNode, doc: &mut SourceDocument, n: &mut i64) {
        for c in &node.children {
            if c.name.starts_with("Royal") {
                *n += 1;
                for f in &c.children {
                    if !KNOWN.contains(&f.name.as_str()) && !f.name.starts_with("Royal") {
                        doc.note_unknown(&f.name);
                    }
                }
                build(doc, &c.name, &|k| c.get(k).map(str::to_string), *n);
            }
            walk(c, doc, n);
        }
    }
    walk(&root, &mut doc, &mut n);
    if doc.items.is_empty() && doc.skipped.is_empty() {
        return Err(ImportError::Format("keine Royal-TS-Objekte gefunden".into()));
    }
    Ok(doc)
}
