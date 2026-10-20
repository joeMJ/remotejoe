//! Ziele mit gleicher Adresse zu einem Ziel mit mehreren Zugangswegen zusammenfassen.
//!
//! Das Hauptziel bleibt stehen (Ordner, Einstellungen, Inventar); jedes weitere Ziel der Gruppe wird
//! ein Zugangsweg des Hauptziels und danach gelöscht. Ordner bleiben unberührt.

use std::collections::HashMap;

use uuid::Uuid;

use crate::edit::{self, EditError};
use crate::inherit::{self, CredentialResolution};
use crate::model::*;

/// Vergleichsschlüssel der Adresse: Rechner ohne Groß-/Kleinschreibung und ohne Schlusspunkt; bei
/// Web-Zielen der Rechner aus der URL. Leere Adressen gehören zu keiner Gruppe.
pub fn address_key(t: &Target) -> Option<String> {
    let mut host = t.host.trim().to_string();
    if t.protocol == Protocol::Web {
        if let ProtocolOptions::Web(o) = &t.options {
            if let Some((h, _)) = o.url.as_deref().and_then(crate::weburl::endpoint) {
                host = h;
            }
        }
    }
    let host = host.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn target(index: &Index, id: Uuid) -> Option<&Target> {
    match edit::node(index, id)? {
        Node::Target(t) => Some(t),
        Node::Folder(_) => None,
    }
}

/// Gruppen gleicher Adresse unter den gewählten Knoten (Ordner samt Unterordnern, einzelne Ziele).
/// Je Gruppe die Adresse und die Ziele in Baumreihenfolge; nur Gruppen mit mindestens zwei Zielen.
pub fn groups(index: &Index, scope: &[Uuid]) -> Vec<(String, Vec<Uuid>)> {
    let mut ids: Vec<Uuid> = Vec::new();
    for id in scope {
        let below = if *id == index.root.id || edit::folder(index, *id).is_some() { edit::targets_below(index, *id) } else { vec![*id] };
        for t in below {
            if !ids.contains(&t) {
                ids.push(t);
            }
        }
    }
    let mut order: Vec<String> = Vec::new();
    let mut by: HashMap<String, Vec<Uuid>> = HashMap::new();
    for id in ids {
        // Hypervisoren (Proxmox, Hyper-V) sind Verwaltungszugänge, keine Zugangswege eines Geräts.
        let Some(t) = target(index, id).filter(|t| !matches!(t.protocol, Protocol::Proxmox | Protocol::Hyperv)) else { continue };
        let Some(k) = address_key(t) else { continue };
        if !by.contains_key(&k) {
            order.push(k.clone());
        }
        by.entry(k).or_default().push(id);
    }
    order.into_iter().filter_map(|k| by.remove(&k).filter(|v| v.len() > 1).map(|v| (k, v))).collect()
}

/// Vorschlag für das Hauptziel: SSH vor PowerShell, RDP, VNC und Web; bei Gleichstand das erste.
pub fn default_keep(index: &Index, members: &[Uuid]) -> Option<Uuid> {
    let rank = |p: Protocol| match p {
        Protocol::Ssh => 0,
        Protocol::Psrp => 1,
        Protocol::Rdp => 2,
        Protocol::Vnc => 3,
        Protocol::Web => 4,
        _ => 5,
    };
    members.iter().filter_map(|id| target(index, *id)).min_by_key(|t| rank(t.protocol)).map(|t| t.id)
}

/// Ergebnis für ein zusammengefasstes Ziel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Merged {
    pub ways: usize,
    /// Neu angelegte Anmeldeobjekte (für abweichende Zugangsdaten).
    pub logins: usize,
    /// Hinweise auf Angaben, die nicht übernommen werden konnten.
    pub notes: Vec<String>,
}

fn found(r: &Result<CredentialResolution, inherit::UnknownTarget>) -> Option<&inherit::ResolvedCredential> {
    match r {
        Ok(CredentialResolution::Found(c)) => Some(c),
        _ => None,
    }
}

/// Fasst `others` in `keep` zusammen; `name` (falls nicht leer) wird der neue Name des Hauptziels.
/// Ohne Wirkung auf den Index, wenn ein Fehler auftritt (der Aufrufer arbeitet auf einer Kopie bzw.
/// stellt den Zustand wieder her).
pub fn merge(index: &mut Index, keep: Uuid, others: &[Uuid], name: &str) -> Result<Merged, EditError> {
    let base = target(index, keep).cloned().ok_or(EditError::NotFound)?;
    let mut out = Merged::default();
    let keep_cred = inherit::resolve_credential(index, keep);
    let keep_tunnel = inherit::resolve_tunnel(index, keep).ok().flatten().map(|(t, _)| t);
    let keep_folder = edit::parent_of(index, keep).unwrap_or(index.root.id);
    let mut members = Vec::new();
    for id in others.iter().filter(|o| **o != keep) {
        let t = target(index, *id).cloned().ok_or(EditError::NotFound)?;
        members.push(t);
    }
    let mut ways = Vec::new();
    let mut new_logins = Vec::new();
    let mut notes = base.meta.clone();
    for t in &members {
        let shown = crate::charset::mark_visible(&t.name, false).into_owned();
        let r = inherit::resolve_credential(index, t.id);
        let credential = match (&r, found(&r)) {
            // Gleiche Auflösung wie beim Hauptziel (auch „keine“ oder „Auswahl“): dessen Zugangsdaten.
            _ if r == keep_cred => WayCredential::Target,
            (Ok(CredentialResolution::Disabled), _) => WayCredential::None,
            (Ok(CredentialResolution::NotFound { .. }), _) => {
                out.notes.push(format!("„{shown}“: keine Zugangsdaten hinterlegt – der Weg verbindet ohne"));
                WayCredential::None
            }
            (_, Some(c)) if found(&keep_cred).is_some_and(|k| k.reference.entry == c.reference.entry && k.overrides == c.overrides) => WayCredential::Target,
            (_, Some(c)) => {
                if !c.overrides.is_empty() {
                    out.notes.push(format!("„{shown}“: überschriebene Felder der Zugangsdaten werden nicht übernommen"));
                }
                // Vorhandenes Anmeldeobjekt auf denselben Eintrag nehmen, sonst eines anlegen.
                let login = index.logins.iter().chain(new_logins.iter()).find(|l: &&Login| l.entry.entry == c.reference.entry).map(|l| l.id);
                WayCredential::Login(login.unwrap_or_else(|| {
                    let l = Login { id: Uuid::new_v4(), name: if c.name == DEFAULT_CREDENTIAL { t.name.clone() } else { format!("{} ({})", t.name, c.name) }, folder: keep_folder, entry: c.reference.clone() };
                    let id = l.id;
                    new_logins.push(l);
                    id
                }))
            }
            _ => {
                out.notes.push(format!("„{shown}“: Zugangsdaten nicht eindeutig – der Weg nutzt die des Hauptziels"));
                WayCredential::Target
            }
        };
        if inherit::resolve_tunnel(index, t.id).ok().flatten().map(|(x, _)| x) != keep_tunnel {
            out.notes.push(format!("„{shown}“: anderer Tunnel – der Weg nutzt den des Hauptziels"));
        }
        if t.settings.psrp_transport.is_some() && t.settings.psrp_transport != base.settings.psrp_transport {
            out.notes.push(format!("„{shown}“: eigener PowerShell-Transport wird nicht übernommen"));
        }
        // Web-Wege tragen ihre URL als Adresse (wie „Dienste übernehmen“); sonst bei gleicher Adresse die
        // des Hauptziels.
        let url = match &t.options {
            ProtocolOptions::Web(o) if t.protocol == Protocol::Web => o.url.clone().filter(|u| u.contains("://")),
            _ => None,
        };
        let host = match url {
            Some(u) => u,
            None if address_key(t) == address_key(&base) => String::new(),
            None => t.host.clone(),
        };
        // Der Weg trägt den Namen des bisherigen Ziels (Knopf in der Übersicht, Tab „Ziel – Weg“).
        ways.push(AccessWay { id: Uuid::new_v4(), name: t.name.clone(), protocol: t.protocol, host, port: t.port, options: t.options.clone(), credential });
        // Weitere Wege des Ziels mitnehmen; „Zugangsdaten des Ziels“ meint dort das bisherige Ziel.
        for w in &t.ways {
            let mut w = w.clone();
            if w.credential == WayCredential::Target {
                w.credential = ways.last().map(|x: &AccessWay| x.credential).unwrap_or(WayCredential::Target);
            }
            ways.push(w);
        }
        // Notizen und Beschreibung anhängen, eigene Felder ergänzen, Aufgaben und Makros vereinigen.
        let m = &t.meta;
        if !m.notes.trim().is_empty() {
            let sep = if notes.notes.trim().is_empty() { "" } else { "\n\n" };
            notes.notes = format!("{}{sep}— aus „{}“ —\n{}", notes.notes.trim_end(), t.name, m.notes.trim());
        }
        if notes.description.trim().is_empty() {
            notes.description = m.description.clone();
        }
        for (k, v) in &m.fields {
            notes.fields.entry(k.clone()).or_insert_with(|| v.clone());
        }
        for x in &m.tasks {
            if !notes.tasks.contains(x) {
                notes.tasks.push(*x);
            }
        }
        for x in &m.macros {
            if !notes.macros.contains(x) {
                notes.macros.push(*x);
            }
        }
    }
    out.ways = ways.len();
    out.logins = new_logins.len();
    index.logins.extend(new_logins);
    {
        let Some(Node::Target(k)) = inherit::find_node_mut(&mut index.root, keep) else { return Err(EditError::NotFound) };
        k.ways.extend(ways);
        k.meta = notes;
        if !name.trim().is_empty() {
            k.name = name.trim().to_string();
        }
    }
    for t in &members {
        // Ansichten und Favoriten zeigen danach auf das Hauptziel.
        for v in &mut index.views {
            if v.targets.contains(&t.id) && !v.targets.contains(&keep) {
                if let Some(p) = v.targets.iter().position(|x| *x == t.id) {
                    v.targets[p] = keep;
                }
            }
        }
        if index.favorites.contains(&t.id) && !index.favorites.contains(&keep) {
            index.favorites.push(keep);
        }
        edit::delete(index, t.id)?;
    }
    inherit::sync_logins(index);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web(name: &str, url: &str) -> Target {
        let mut t = Target::new(name, Protocol::Web, "");
        t.options = ProtocolOptions::Web(WebOptions { url: Some(url.into()), ..Default::default() });
        t
    }

    fn cred(entry: Uuid) -> Settings {
        let mut s = Settings::default();
        s.credentials.insert(DEFAULT_CREDENTIAL.to_string(), CredentialRef { entry, last_title: None });
        s
    }

    #[test]
    fn groups_by_host_and_url_host_and_merges_into_ways() {
        let mut idx = Index::default();
        let root = idx.root.id;
        let f = edit::insert(&mut idx, root, Node::Folder(Folder::new("Netz")), None).unwrap();
        let (e1, e2) = (Uuid::new_v4(), Uuid::new_v4());
        let mut ssh = Target::new("ap1", Protocol::Ssh, "192.0.2.1");
        ssh.settings = cred(e1);
        ssh.meta.notes = "Flur".into();
        let mut w = web("ap1 web", "https://192.0.2.1:4343/");
        w.settings = cred(e2);
        w.meta.notes = "Admin-Seite".into();
        let mut w2 = web("ap1 web alt", "http://192.0.2.1/");
        w2.settings = cred(e1);
        let other = Target::new("anderes", Protocol::Ssh, "192.0.2.1");
        let ids: Vec<Uuid> = [ssh, w, w2, other].into_iter().map(|t| edit::insert(&mut idx, f, Node::Target(t), None).unwrap()).collect();
        idx.favorites.push(ids[1]);
        let g = groups(&idx, &[f]);
        assert_eq!(g, vec![("192.0.2.1".to_string(), ids[..3].to_vec())]);
        assert_eq!(default_keep(&idx, &g[0].1), Some(ids[0]));
        let r = merge(&mut idx, ids[0], &ids[1..3], "AP Flur").unwrap();
        assert_eq!((r.ways, r.logins), (2, 1), "ein Anmeldeobjekt für den abweichenden Eintrag");
        let Some(Node::Target(k)) = edit::node(&idx, ids[0]) else { panic!() };
        assert_eq!(k.name, "AP Flur");
        assert_eq!(k.ways.len(), 2);
        // Die Wege tragen den Namen ihres bisherigen Ziels.
        assert_eq!((k.ways[0].name.as_str(), k.ways[1].name.as_str()), ("ap1 web", "ap1 web alt"));
        assert!(matches!(k.ways[0].credential, WayCredential::Login(l) if idx.login(l).unwrap().entry.entry == e2));
        assert!(matches!(k.ways[0].credential, WayCredential::Login(l) if idx.login(l).unwrap().name == "ap1 web"), "ohne „(default)“");
        assert_eq!(k.ways[1].credential, WayCredential::Target, "gleicher Eintrag wie das Hauptziel");
        let v = k.via(k.ways[0].id).unwrap();
        assert_eq!((v.host.as_str(), v.port), ("192.0.2.1", Some(4343)));
        assert!(k.meta.notes.contains("Flur") && k.meta.notes.contains("— aus „ap1 web“ —\nAdmin-Seite"));
        assert!(edit::node(&idx, ids[1]).is_none() && edit::node(&idx, ids[2]).is_none() && edit::node(&idx, ids[3]).is_some());
        assert_eq!(idx.favorites, vec![ids[0]]);
        assert!(groups(&idx, &[root]).is_empty());
    }

    #[test]
    fn disabled_credentials_and_case_insensitive_hosts() {
        let mut idx = Index::default();
        let root = idx.root.id;
        let a = edit::insert(&mut idx, root, Node::Target(Target::new("a", Protocol::Rdp, "Server.local.")), None).unwrap();
        let mut b = Target::new("b", Protocol::Vnc, "server.LOCAL");
        b.settings.credentials_off = true;
        let b = edit::insert(&mut idx, root, Node::Target(b), None).unwrap();
        let g = groups(&idx, &[a, b]);
        assert_eq!(g.len(), 1);
        let r = merge(&mut idx, a, &[b], "").unwrap();
        let Some(Node::Target(k)) = edit::node(&idx, a) else { panic!() };
        assert_eq!((k.name.as_str(), k.ways[0].credential, k.ways[0].host.as_str()), ("a", WayCredential::None, ""));
        assert_eq!(k.ways[0].name, "b", "Weg trägt den Namen des bisherigen Ziels");
        assert_eq!(r.logins, 0);
    }
}
