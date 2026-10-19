//! Remote Desktop Manager: XML-Export (`.rdm`, `ArrayOfConnection`) und
//! JSON-Export (`{"Connections":[…]}`), tolerant gelesen.
//!
//! Ordner ergeben sich aus `Group` (Pfad mit `\`) und aus Einträgen vom Typ
//! `Group`. Kennwörter werden nur übernommen, wenn sie im Klartext exportiert
//! wurden (`Password`); verschlüsselte (`SafePassword`) werden gemeldet.

use std::collections::HashSet;

use rj_core::{Protocol, Secret};
use serde_json::Value;

use crate::xml::{self, XmlNode};
use crate::{ImportError, SourceCredential, SourceDocument, SourceItem};

/// Bekannte Felder (alles andere landet im Bericht).
const KNOWN: &[&str] = &[
    "ConnectionType", "Group", "ID", "Name", "Url", "Host", "Port", "UserName", "Username", "Domain", "Password", "SafePassword",
    "CredentialConnectionID", "Credentials", "RDP", "Terminal", "VNC", "WebBrowserUrl", "SortPriority", "Description", "Stamp",
    "CreationDateTime", "UpdateDateTime", "CreatedBy", "UpdatedBy", "OpenEmbedded", "Icon", "ImageName", "PowerShell",
];

/// Protokoll zu einem RDM-Verbindungstyp (Name oder Zahl).
fn protocol(kind: &str) -> Option<Protocol> {
    Some(match kind {
        "RDPConfigured" | "1" => Protocol::Rdp,
        "SSHShell" | "Putty" | "77" => Protocol::Ssh,
        "VNC" | "12" => Protocol::Vnc,
        "WebBrowser" | "32" => Protocol::Web,
        "PowerShell" | "PowerShellRemoteConsole" | "59" => Protocol::Psrp,
        "HyperV" | "16" => Protocol::Hyperv,
        "Proxmox" => Protocol::Proxmox,
        _ => return None,
    })
}

fn is_group(kind: &str) -> bool {
    matches!(kind, "Group" | "25")
}

fn is_credential(kind: &str) -> bool {
    matches!(kind, "Credential" | "26")
}

/// Ordnerkette aus einem Gruppenpfad anlegen; liefert die ID des letzten Ordners.
pub(crate) fn ensure_path(doc: &mut SourceDocument, seen: &mut HashSet<String>, path: &str, order: &mut i64) -> Option<String> {
    let mut parent: Option<String> = None;
    let mut acc = String::new();
    for part in path.split('\\').filter(|p| !p.is_empty()) {
        if !acc.is_empty() {
            acc.push('\\');
        }
        acc.push_str(part);
        let id = format!("pfad:{acc}");
        if seen.insert(id.clone()) {
            *order += 1;
            doc.items.push(SourceItem::Folder { id: id.clone(), parent: parent.clone(), name: part.to_string(), order: *order });
        }
        parent = Some(id);
    }
    parent
}

struct Fields<'a> {
    get: Box<dyn Fn(&str) -> Option<String> + 'a>,
    sub: Box<dyn Fn(&str, &str) -> Option<String> + 'a>,
}

fn connection(doc: &mut SourceDocument, seen: &mut HashSet<String>, f: &Fields, order: &mut i64) {
    let kind = (f.get)("ConnectionType").unwrap_or_default();
    let name = (f.get)("Name").unwrap_or_default();
    let id = (f.get)("ID").unwrap_or_else(|| format!("eintrag-{order}"));
    let group = (f.get)("Group").unwrap_or_default();
    if is_group(&kind) {
        // Gruppe: Pfad aus Group (enthält den eigenen Namen) oder Name.
        let path = if group.is_empty() { name.clone() } else { group };
        ensure_path(doc, seen, &path, order);
        return;
    }
    // Kennwort verschlüsselt exportiert: am Eintrag selbst oder in einem der Blöcke.
    let encrypted = ["", "Credentials", "RDP", "Terminal", "VNC", "Web", "PowerShell"].iter().any(|p| (f.sub)(p, "SafePassword").is_some());
    let cred = |prefix: &str| -> Option<SourceCredential> {
        let user = (f.sub)(prefix, "UserName").or_else(|| (f.sub)(prefix, "Username"))?;
        let pw = (f.sub)(prefix, "Password").map(|p| Secret::from_str_exact(&p));
        Some(SourceCredential { username: user, domain: (f.sub)(prefix, "Domain"), password: pw })
    };
    if is_credential(&kind) {
        if encrypted {
            doc.encrypted_passwords += 1;
            doc.encrypted_entries.push(("credential".into(), name.clone()));
        }
        let c = cred("Credentials").or_else(|| cred(""));
        match c {
            Some(c) => doc.items.push(SourceItem::Credential { id, name, credential: c }),
            None => doc.skipped.push((name, "Zugangsdatensatz ohne Benutzername".into())),
        }
        return;
    }
    let Some(p) = protocol(&kind) else {
        doc.skipped.push((name, format!("Verbindungstyp „{kind}“ wird nicht unterstützt")));
        return;
    };
    let host = match p {
        Protocol::Web => (f.get)("WebBrowserUrl").or_else(|| (f.get)("Url")),
        Protocol::Ssh => (f.sub)("Terminal", "Host").or_else(|| (f.get)("Url")),
        Protocol::Vnc => (f.sub)("VNC", "Host").or_else(|| (f.get)("Url")),
        _ => (f.get)("Url").or_else(|| (f.get)("Host")),
    }
    .unwrap_or_default();
    if host.is_empty() {
        doc.warnings.push(format!("„{name}“: kein Host angegeben"));
    }
    let port = (f.get)("Port")
        .or_else(|| (f.sub)("Terminal", "Port"))
        .or_else(|| (f.sub)("VNC", "Port"))
        .and_then(|p| p.parse().ok());
    if encrypted {
        doc.encrypted_passwords += 1;
        doc.encrypted_entries.push(("target".into(), name.clone()));
    }
    let parent = ensure_path(doc, seen, &group, order);
    *order += 1;
    doc.items.push(SourceItem::Connection {
        id,
        parent,
        name,
        order: *order,
        protocol: p,
        host,
        port,
        credential: cred(""),
        credential_ref: (f.get)("CredentialConnectionID"),
        options: None,
    });
}

/// XML-Export (`ArrayOfConnection`, auch `RDMExport`/`Connections` als Hülle).
pub fn read_xml(text: &str) -> Result<SourceDocument, ImportError> {
    let root = xml::parse(text)?;
    let mut doc = SourceDocument { format: "Verbindungsliste (.rdm, XML)".into(), ..Default::default() };
    let mut conns: Vec<&XmlNode> = Vec::new();
    fn collect<'a>(n: &'a XmlNode, out: &mut Vec<&'a XmlNode>) {
        for c in &n.children {
            if c.name == "Connection" {
                out.push(c);
            } else {
                collect(c, out);
            }
        }
    }
    collect(&root, &mut conns);
    if conns.is_empty() {
        return Err(ImportError::Format("keine Verbindungen (<Connection>) gefunden".into()));
    }
    let mut seen = HashSet::new();
    let mut order = 0;
    for c in conns {
        for ch in &c.children {
            if !KNOWN.contains(&ch.name.as_str()) {
                doc.note_unknown(&ch.name);
            }
        }
        let f = Fields {
            get: Box::new(|k| c.get(k).map(str::to_string)),
            sub: Box::new(|p, k| if p.is_empty() { c.get(k).map(str::to_string) } else { c.child(p).and_then(|s| s.get(k)).map(str::to_string) }),
        };
        connection(&mut doc, &mut seen, &f, &mut order);
    }
    Ok(doc)
}

fn json_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// JSON-Export (`{"Connections":[…]}` oder Feld direkt).
pub fn read_json(text: &str) -> Result<SourceDocument, ImportError> {
    let v: Value = serde_json::from_str(text).map_err(|e| ImportError::Format(format!("JSON: {e}")))?;
    let list = v.get("Connections").or_else(|| v.get("connections")).and_then(Value::as_array).or_else(|| v.as_array()).ok_or_else(|| ImportError::Format("kein Feld „Connections“".into()))?;
    let mut doc = SourceDocument { format: "Verbindungsliste (JSON)".into(), ..Default::default() };
    let mut seen = HashSet::new();
    let mut order = 0;
    for c in list {
        let Some(obj) = c.as_object() else { continue };
        for k in obj.keys() {
            if !KNOWN.contains(&k.as_str()) {
                doc.note_unknown(k);
            }
        }
        let f = Fields {
            get: Box::new(|k| obj.get(k).and_then(json_str)),
            sub: Box::new(|p, k| if p.is_empty() { obj.get(k).and_then(json_str) } else { obj.get(p).and_then(|s| s.get(k)).and_then(json_str) }),
        };
        connection(&mut doc, &mut seen, &f, &mut order);
    }
    Ok(doc)
}
