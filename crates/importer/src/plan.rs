//! Importplan: Ordnerbaum, Ziele, zusammengelegte Zugangsdaten mit Verweis
//! am nächsthöheren sinnvollen Ordner, Bericht. Erst [`apply`] schreibt.

use std::collections::{BTreeMap, HashMap};

use rj_core::charset::check_name;
use rj_core::model::{ProtocolOptions, WebOptions};
use rj_core::{CredentialRef, Folder, Index, Node, Protocol, Target};
use rj_kdbx::{CredentialData, Vault, VaultError};
use serde::Serialize;
use uuid::Uuid;

use crate::{SourceCredential, SourceDocument, SourceItem};

/// Bericht über den Import (ohne Geheimnisse).
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    pub format: String,
    pub folders: usize,
    pub targets: usize,
    /// Zugangsdaten in der Quelle (je Verbindung bzw. Datensatz).
    pub credentials_in_source: usize,
    /// Davon verschiedene (werden je ein KDBX-Eintrag).
    pub credentials_unique: usize,
    /// Verweise im Index (an Ordnern und Zielen).
    pub references: usize,
    pub skipped: Vec<(String, String)>,
    pub warnings: Vec<String>,
    pub unknown_fields: Vec<(String, usize)>,
    /// Einträge mit verschlüsselt exportiertem Kennwort (nicht übernommen).
    pub encrypted_passwords: usize,
    /// Dieselben Einträge mit Art (`target` oder `credential`) und Name.
    pub encrypted_entries: Vec<(String, String)>,
}

impl ImportReport {
    /// Lesbare Zusammenfassung.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{}: {} Ordner, {} Ziele übernommen; {} Zugangsdaten zu {} Einträgen der Schlüsseldatenbank zusammengelegt ({} Verweise).",
            self.format, self.folders, self.targets, self.credentials_in_source, self.credentials_unique, self.references
        );
        if self.encrypted_passwords > 0 {
            s.push_str(&format!(
                "\nKennwörter: {} Einträge mit verschlüsselt exportiertem Kennwort – die Kennwörter wurden NICHT übernommen und müssen in der Schlüsseldatenbank ergänzt werden (oder die Quelle mit Klartext-Kennwörtern neu exportieren).",
                self.encrypted_passwords
            ));
            for (kind, name) in &self.encrypted_entries {
                let art = if kind == "credential" { "Zugangsdatensatz" } else { "Ziel" };
                s.push_str(&format!("\n  – {art} „{name}“"));
            }
        }
        if !self.skipped.is_empty() {
            s.push_str(&format!("\nÜbersprungen: {}", self.skipped.len()));
            for (n, why) in &self.skipped {
                s.push_str(&format!("\n  – {n}: {why}"));
            }
        }
        if !self.warnings.is_empty() {
            s.push_str(&format!("\nWarnungen: {}", self.warnings.len()));
            for w in &self.warnings {
                s.push_str(&format!("\n  – {w}"));
            }
        }
        if !self.unknown_fields.is_empty() {
            let f: Vec<String> = self.unknown_fields.iter().map(|(k, n)| format!("{k} ({n}×)")).collect();
            s.push_str(&format!("\nUnbekannte Felder: {}", f.join(", ")));
        }
        s
    }
}

/// Geplanter KDBX-Eintrag.
#[derive(Debug, Clone)]
pub struct PlannedCredential {
    pub title: String,
    pub credential: SourceCredential,
}

/// Vorschau des Imports.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    /// Oberster Ordner des Imports (wird unter die Wurzel gehängt).
    pub folder: Folder,
    pub credentials: Vec<PlannedCredential>,
    /// (Knoten, Index in `credentials`): Verweis `default` am Knoten.
    pub refs: Vec<(Uuid, usize)>,
    pub report: ImportReport,
}

type Key = (String, Option<String>, Vec<u8>);

fn key(c: &SourceCredential) -> Key {
    (c.username.clone(), c.domain.clone(), c.password.as_ref().map(|p| p.as_bytes().to_vec()).unwrap_or_default())
}

struct Tmp {
    folders: HashMap<String, (Folder, Option<String>, i64)>,
}

/// Erstellt den Plan aus einem gelesenen Dokument.
pub fn plan(doc: &SourceDocument, title: &str) -> ImportPlan {
    let mut report = ImportReport {
        format: doc.format.clone(),
        skipped: doc.skipped.clone(),
        warnings: doc.warnings.clone(),
        unknown_fields: doc.unknown_fields.clone(),
        encrypted_passwords: doc.encrypted_passwords,
        encrypted_entries: doc.encrypted_entries.clone(),
        ..Default::default()
    };
    let check = |what: &str, name: &str, report: &mut ImportReport| {
        let c = check_name(name);
        if !c.is_clean() {
            let kinds: Vec<String> = c.findings.iter().map(|f| f.kind.to_string()).collect();
            report.warnings.push(format!("{what} „{}“ enthält versteckte Zeichen ({})", c.display, kinds.join(", ")));
        }
    };

    // Ordner und Ziele mit ihren Quell-IDs.
    let mut tmp = Tmp { folders: HashMap::new() };
    let mut source_creds: HashMap<String, (String, SourceCredential)> = HashMap::new();
    let mut targets: Vec<(Target, Option<String>, i64, Option<(Key, String, SourceCredential)>)> = Vec::new();
    for it in &doc.items {
        if let SourceItem::Credential { id, name, credential } = it {
            check("Zugangsdatensatz", name, &mut report);
            source_creds.insert(id.clone(), (name.clone(), credential.clone()));
        }
    }
    for it in &doc.items {
        match it {
            SourceItem::Folder { id, parent, name, order } => {
                check("Ordner", name, &mut report);
                tmp.folders.insert(id.clone(), (Folder::new(name.clone()), parent.clone(), *order));
                report.folders += 1;
            }
            SourceItem::Connection { parent, name, order, protocol, host, port, credential, credential_ref, options, .. } => {
                check("Ziel", name, &mut report);
                check("Host von", host, &mut report);
                let mut t = Target::new(name.clone(), *protocol, host.clone());
                t.port = *port;
                if *protocol == Protocol::Web {
                    // Bei Web-Zielen steht die Adresse in den Optionen; Host und Port werden daraus abgeleitet
                    // (wie beim Speichern im Eigenschaften-Tab). Ohne Schema wird https:// angenommen.
                    let url = if host.contains("://") { host.clone() } else { rj_core::weburl::default_url(host, port.unwrap_or(443)) };
                    match rj_core::weburl::endpoint(&url) {
                        Some((h, p)) => {
                            t.host = h;
                            t.port = (p != Protocol::Web.default_port()).then_some(p);
                            t.options = ProtocolOptions::Web(WebOptions { url: Some(url), ..Default::default() });
                        }
                        None => report.warnings.push(format!("„{name}“: „{}“ ist keine gültige Web-Adresse (http:// oder https://)", check_name(host).display)),
                    }
                }
                if let Some(o) = options {
                    t.options = o.clone();
                }
                let cred = match (credential, credential_ref) {
                    (Some(c), _) if !c.username.is_empty() => {
                        report.credentials_in_source += 1;
                        let title = match &c.domain {
                            Some(d) => format!("{d}\\{}", c.username),
                            None => c.username.clone(),
                        };
                        Some((key(c), title, c.clone()))
                    }
                    (_, Some(r)) => match source_creds.get(r) {
                        Some((n, c)) => {
                            report.credentials_in_source += 1;
                            Some((key(c), n.clone(), c.clone()))
                        }
                        None => {
                            report.warnings.push(format!("„{name}“: Verweis auf unbekannten Zugangsdatensatz"));
                            None
                        }
                    },
                    _ => None,
                };
                targets.push((t, parent.clone(), *order, cred));
                report.targets += 1;
            }
            SourceItem::Credential { .. } => {}
        }
    }

    // Baum aufbauen (Reihenfolge nach Quellreihenfolge bzw. Position).
    let mut top = Folder::new(title);
    let mut children: HashMap<Option<String>, Vec<(i64, Node, Option<String>)>> = HashMap::new();
    let ids: Vec<String> = tmp.folders.keys().cloned().collect();
    for id in ids {
        let (f, parent, order) = tmp.folders.remove(&id).expect("vorhanden");
        let parent = parent.filter(|p| doc.items.iter().any(|i| matches!(i, SourceItem::Folder { id, .. } if id == p)));
        children.entry(parent).or_default().push((order, Node::Folder(f), Some(id)));
    }
    let mut target_keys: HashMap<Uuid, Key> = HashMap::new();
    let mut key_data: BTreeMap<Key, (String, SourceCredential)> = BTreeMap::new();
    for (t, parent, order, cred) in targets {
        if let Some((k, title, c)) = cred {
            target_keys.insert(t.id, k.clone());
            key_data.entry(k).or_insert((title, c));
        }
        let parent = parent.filter(|p| doc.items.iter().any(|i| matches!(i, SourceItem::Folder { id, .. } if id == p)));
        children.entry(parent).or_default().push((order, Node::Target(t), None));
    }
    type Children = HashMap<Option<String>, Vec<(i64, Node, Option<String>)>>;
    fn attach(f: &mut Folder, key: Option<String>, children: &mut Children, depth: usize) {
        let mut list = children.remove(&key).unwrap_or_default();
        list.sort_by_key(|(o, _, _)| *o);
        for (_, mut n, id) in list {
            if let (Node::Folder(sub), Some(id)) = (&mut n, id) {
                if depth < crate::MAX_DEPTH {
                    attach(sub, Some(id), children, depth + 1);
                }
            }
            f.children.push(n);
        }
    }
    attach(&mut top, None, &mut children, 0);
    // Übrig sind Objekte, deren Elternverweise einen Zyklus bilden oder die zu
    // tief verschachtelt sind: oben einordnen statt still verlieren.
    let mut moved = 0;
    while let Some(k) = children.keys().next().cloned() {
        moved += children.get(&k).map_or(0, Vec::len);
        attach(&mut top, k, &mut children, 0);
    }
    if moved > 0 {
        report.warnings.push(format!(
            "{moved} Objekte mit kreisförmigem oder zu tief verschachteltem Elternverweis wurden oben eingeordnet"
        ));
    }

    // Doppelte Namen je Ordner melden.
    fn dups(f: &Folder, path: &str, report: &mut ImportReport) {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for c in &f.children {
            *seen.entry(c.name()).or_default() += 1;
            if let Node::Folder(sub) = c {
                dups(sub, &format!("{path}/{}", sub.name), report);
            }
        }
        for (n, k) in seen {
            if k > 1 {
                report.warnings.push(format!("Name „{}“ kommt in „{path}“ {k}× vor", check_name(n).display));
            }
        }
    }
    dups(&top, &top.name.clone(), &mut report);

    // Zugangsdaten zusammenlegen und verankern.
    let keys: Vec<Key> = key_data.keys().cloned().collect();
    let credentials: Vec<PlannedCredential> = keys.iter().map(|k| PlannedCredential { title: key_data[k].0.clone(), credential: key_data[k].1.clone() }).collect();
    for c in &credentials {
        let already_listed = report.encrypted_entries.iter().any(|(k, n)| k == "credential" && *n == c.title);
        if c.credential.password.is_none() && !already_listed {
            report.warnings.push(format!("Zugangsdaten „{}“ ohne Kennwort übernommen – bitte in der Schlüsseldatenbank ergänzen", check_name(&c.title).display));
        }
    }
    report.credentials_unique = credentials.len();
    let key_index: HashMap<&Key, usize> = keys.iter().enumerate().map(|(i, k)| (k, i)).collect();

    // Pfad (Ordner-IDs ab `top`) je Ziel.
    let mut paths: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    fn walk(f: &Folder, path: &mut Vec<Uuid>, out: &mut HashMap<Uuid, Vec<Uuid>>) {
        path.push(f.id);
        for c in &f.children {
            match c {
                Node::Folder(sub) => walk(sub, path, out),
                Node::Target(t) => {
                    out.insert(t.id, path.clone());
                }
            }
        }
        path.pop();
    }
    walk(&top, &mut Vec::new(), &mut paths);

    let mut placed: HashMap<Uuid, usize> = HashMap::new();
    let mut by_key: Vec<(usize, Vec<Uuid>)> = (0..keys.len()).map(|i| (i, Vec::new())).collect();
    for (t, k) in &target_keys {
        by_key[key_index[k]].1.push(*t);
    }
    by_key.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    for (ki, ts) in &by_key {
        if ts.len() < 2 {
            continue;
        }
        // Tiefster gemeinsamer Ordner.
        let mut lca: Vec<Uuid> = paths[&ts[0]].clone();
        for t in &ts[1..] {
            let p = &paths[t];
            let n = lca.iter().zip(p).take_while(|(a, b)| a == b).count();
            lca.truncate(n);
        }
        if let Some(f) = lca.last() {
            placed.entry(*f).or_insert(*ki);
        }
    }
    // Jedes Ziel prüfen: erbt es den richtigen Eintrag? Sonst Verweis am Ziel.
    for (t, k) in &target_keys {
        let want = key_index[k];
        let inherited = paths[t].iter().rev().find_map(|f| placed.get(f)).copied();
        if inherited != Some(want) {
            placed.insert(*t, want);
        }
    }
    let refs: Vec<(Uuid, usize)> = placed.into_iter().collect();
    report.references = refs.len();
    ImportPlan { folder: top, credentials, refs, report }
}

/// Übernimmt den Plan: legt KDBX-Einträge in der Gruppe `remotejoe` an und
/// liefert den neuen Index (noch nicht gespeichert).
pub fn apply(plan: &ImportPlan, vault: &mut Vault, index: &Index) -> Result<Index, VaultError> {
    let mut entries = Vec::with_capacity(plan.credentials.len());
    for c in &plan.credentials {
        let uuid = vault.add_credential(CredentialData {
            title: c.title.clone(),
            username: match &c.credential.domain {
                Some(d) => format!("{d}\\{}", c.credential.username),
                None => c.credential.username.clone(),
            },
            password: c.credential.password.clone().unwrap_or_default(),
            ..Default::default()
        })?;
        entries.push((uuid, c.title.clone()));
    }
    let mut idx = index.clone();
    let mut folder = plan.folder.clone();
    let lookup: HashMap<Uuid, usize> = plan.refs.iter().copied().collect();
    fn set(f: &mut Folder, lookup: &HashMap<Uuid, usize>, entries: &[(Uuid, String)]) {
        if let Some(i) = lookup.get(&f.id) {
            f.settings.credentials.insert("default".into(), CredentialRef { entry: entries[*i].0, last_title: Some(entries[*i].1.clone()) });
        }
        for c in &mut f.children {
            match c {
                Node::Folder(sub) => set(sub, lookup, entries),
                Node::Target(t) => {
                    if let Some(i) = lookup.get(&t.id) {
                        t.settings.credentials.insert("default".into(), CredentialRef { entry: entries[*i].0, last_title: Some(entries[*i].1.clone()) });
                    }
                }
            }
        }
    }
    set(&mut folder, &lookup, &entries);
    idx.root.children.push(Node::Folder(folder));
    Ok(idx)
}
