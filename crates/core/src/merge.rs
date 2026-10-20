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
    let ids = selection(index, scope);
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

/// Ziele der Auswahl (Ordner samt Unterordnern, einzelne Ziele) in Baumreihenfolge, ohne Doppelte.
pub fn selection(index: &Index, scope: &[Uuid]) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = Vec::new();
    for id in scope {
        let below = if *id == index.root.id || edit::folder(index, *id).is_some() { edit::targets_below(index, *id) } else { vec![*id] };
        for t in below {
            if !ids.contains(&t) && target(index, t).is_some() {
                ids.push(t);
            }
        }
    }
    ids
}

/// Vorschlag für das Hauptziel: Hypervisor (nur dort gibt es die Gästeliste), dann SSH vor PowerShell, RDP, VNC und
/// Web; bei Gleichstand das erste.
pub fn default_keep(index: &Index, members: &[Uuid]) -> Option<Uuid> {
    let rank = |p: Protocol| match p {
        Protocol::Hyperv | Protocol::Proxmox => -1,
        Protocol::Ssh => 0,
        Protocol::Psrp => 1,
        Protocol::Rdp => 2,
        Protocol::Vnc => 3,
        Protocol::Web => 4,
    };
    members.iter().filter_map(|id| target(index, *id)).min_by_key(|t| rank(t.protocol)).map(|t| t.id)
}

/// Ergebnis für ein zusammengefasstes Ziel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Merged {
    pub ways: usize,
    /// Wege mit eigenen Zugangsdaten (abweichender KDBX-Eintrag des bisherigen Ziels).
    pub own: usize,
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
    let mut members = Vec::new();
    for id in others.iter().filter(|o| **o != keep) {
        let t = target(index, *id).cloned().ok_or(EditError::NotFound)?;
        members.push(t);
    }
    let mut ways = Vec::new();
    let mut notes = base.meta.clone();
    for t in &members {
        let shown = crate::charset::mark_visible(&t.name, false).into_owned();
        let r = inherit::resolve_credential(index, t.id);
        let (credential, own) = match (&r, found(&r)) {
            // Gleiche Auflösung wie beim Hauptziel (auch „keine“ oder „Auswahl“): dessen Zugangsdaten.
            _ if r == keep_cred => (WayCredential::Target, None),
            (Ok(CredentialResolution::Disabled), _) => (WayCredential::None, None),
            (Ok(CredentialResolution::NotFound { .. }), _) => {
                out.notes.push(format!("„{shown}“: keine Zugangsdaten hinterlegt – der Weg verbindet ohne"));
                (WayCredential::None, None)
            }
            (_, Some(c)) if found(&keep_cred).is_some_and(|k| k.reference.entry == c.reference.entry && k.overrides == c.overrides) => (WayCredential::Target, None),
            (_, Some(c)) => {
                if !c.overrides.is_empty() {
                    out.notes.push(format!("„{shown}“: überschriebene Felder der Zugangsdaten werden nicht übernommen"));
                }
                // Steht der Eintrag schon in einem Anmeldeobjekt, dieses nehmen; sonst bekommt der Weg den
                // Eintrag als eigene Zugangsdaten.
                match index.logins.iter().find(|l| l.entry.entry == c.reference.entry) {
                    Some(l) => (WayCredential::Login(l.id), None),
                    None => {
                        out.own += 1;
                        (WayCredential::Target, Some(c.reference.clone()))
                    }
                }
            }
            _ => {
                out.notes.push(format!("„{shown}“: Zugangsdaten nicht eindeutig – der Weg nutzt die des Hauptziels"));
                (WayCredential::Target, None)
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
        // Hypervisor als Weg: dessen Verbindung (Hyper-V: RDP zum Host, Proxmox: Web-Oberfläche); die Gästeliste
        // gibt es nur am Hauptziel.
        let protocol = match t.protocol {
            Protocol::Hyperv => Protocol::Rdp,
            Protocol::Proxmox => Protocol::Web,
            p => p,
        };
        if protocol != t.protocol {
            out.notes.push(format!("„{shown}“ ist ein Hypervisor: als Weg nur die Verbindung, ohne Gästeliste – dafür als Hauptziel wählen"));
        }
        let port = match t.protocol {
            Protocol::Proxmox => Some(t.port.unwrap_or(8006)),
            _ => t.port,
        };
        // Der Weg trägt den Namen des bisherigen Ziels (Knopf in der Übersicht, Tab „Ziel – Weg“).
        ways.push(AccessWay { id: Uuid::new_v4(), name: t.name.clone(), protocol, host, port, options: t.options.clone(), credential, own: own.clone() });
        // Weitere Wege des Ziels mitnehmen; „Zugangsdaten des Ziels“ meint dort das bisherige Ziel.
        for w in &t.ways {
            let mut w = w.clone();
            if w.credential == WayCredential::Target && w.own.is_none() {
                w.credential = credential;
                w.own = own.clone();
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
        assert_eq!((r.ways, r.own), (2, 1), "eigene Zugangsdaten für den abweichenden Eintrag");
        let Some(Node::Target(k)) = edit::node(&idx, ids[0]) else { panic!() };
        assert_eq!(k.name, "AP Flur");
        assert_eq!(k.ways.len(), 2);
        // Die Wege tragen den Namen ihres bisherigen Ziels.
        assert_eq!((k.ways[0].name.as_str(), k.ways[1].name.as_str()), ("ap1 web", "ap1 web alt"));
        assert_eq!(k.ways[0].own.as_ref().map(|r| r.entry), Some(e2), "abweichender Eintrag als eigene Zugangsdaten des Wegs");
        assert!(idx.logins.is_empty(), "kein neues Anmeldeobjekt");
        assert!(idx.referenced_entries().contains(&e2), "Eintrag bleibt zugeordnet");
        assert_eq!(k.ways[1].credential, WayCredential::Target, "gleicher Eintrag wie das Hauptziel");
        let v = k.via(k.ways[0].id).unwrap();
        assert_eq!((v.host.as_str(), v.port), ("192.0.2.1", Some(4343)));
        assert!(k.meta.notes.contains("Flur") && k.meta.notes.contains("— aus „ap1 web“ —\nAdmin-Seite"));
        assert!(edit::node(&idx, ids[1]).is_none() && edit::node(&idx, ids[2]).is_none() && edit::node(&idx, ids[3]).is_some());
        assert_eq!(idx.favorites, vec![ids[0]]);
        assert!(groups(&idx, &[root]).is_empty());
    }

    #[test]
    fn selection_with_different_addresses_keeps_the_hypervisor_as_main() {
        let mut idx = Index::default();
        let root = idx.root.id;
        let hv = edit::insert(&mut idx, root, Node::Target(Target::new("210 HyperV", Protocol::Hyperv, "192.0.2.210")), None).unwrap();
        let rdp = edit::insert(&mut idx, root, Node::Target(Target::new("210 RDP", Protocol::Rdp, "192.0.2.210")), None).unwrap();
        let ilo = edit::insert(&mut idx, root, Node::Target(web("210 iLO", "https://192.0.2.192/")), None).unwrap();
        let pve = edit::insert(&mut idx, root, Node::Target(Target::new("pve", Protocol::Proxmox, "192.0.2.50")), None).unwrap();
        let sel = selection(&idx, &[rdp, ilo, hv, pve]);
        assert_eq!(sel, vec![rdp, ilo, hv, pve], "Reihenfolge der Auswahl");
        assert_eq!(default_keep(&idx, &sel), Some(hv), "Hypervisor als Hauptziel");
        let r = merge(&mut idx, hv, &[rdp, ilo, pve], "").unwrap();
        let Some(Node::Target(k)) = edit::node(&idx, hv) else { panic!() };
        assert_eq!(k.protocol, Protocol::Hyperv);
        assert_eq!((k.ways[0].protocol, k.ways[0].host.as_str()), (Protocol::Rdp, ""), "gleiche Adresse: die des Hauptziels");
        assert_eq!((k.ways[1].protocol, k.ways[1].host.as_str()), (Protocol::Web, "https://192.0.2.192/"), "iLO mit eigener Adresse");
        assert_eq!((k.ways[2].protocol, k.ways[2].host.as_str(), k.ways[2].port), (Protocol::Web, "192.0.2.50", Some(8006)), "Proxmox als Weg: Web-Oberfläche");
        assert!(r.notes.iter().any(|n| n.contains("„pve“ ist ein Hypervisor")), "{:?}", r.notes);
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
        assert_eq!(r.own, 0);
    }
}
