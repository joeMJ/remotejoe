//! Vererbung von Einstellungen über den Ordnerbaum.
//!
//! Gesucht wird vom Ziel aus nach oben durch alle Ordner bis zur Wurzel; der
//! erste Treffer gewinnt. Die Funktionen hier sind rein und kennen die KDBX
//! nicht.

use uuid::Uuid;

use crate::model::*;

/// Fundort einer geerbten Einstellung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Am Ziel selbst.
    Target,
    /// An einem Ordner (ID).
    Folder(Uuid),
    /// An der Wurzel.
    Root,
}

/// Ein aufgelöster Zugangsdatensatz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCredential {
    pub name: String,
    pub reference: CredentialRef,
    pub origin: Origin,
    /// Überschriebene Felder; je Feld gilt der nächstgelegene Wert.
    pub overrides: FieldOverrides,
}

/// Ergebnis der Suche nach einem Zugangsdatensatz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialResolution {
    Found(ResolvedCredential),
    /// Kein Name festgelegt, `default` fehlt und mehrere Namen kommen in Frage:
    /// die Oberfläche bietet eine Auswahl an.
    Ambiguous(Vec<ResolvedCredential>),
    /// Nichts gefunden.
    NotFound { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Ziel {0} nicht im Index gefunden")]
pub struct UnknownTarget(pub Uuid);

/// Kette der Einstellungen vom Ziel bis zur Wurzel.
struct Chain<'a> {
    target: &'a Target,
    links: Vec<(&'a Settings, Origin)>,
}

fn chain(index: &Index, target_id: Uuid) -> Result<Chain<'_>, UnknownTarget> {
    let path = index.path_to(target_id).ok_or(UnknownTarget(target_id))?;
    let Some(Node::Target(target)) = path.last() else {
        return Err(UnknownTarget(target_id));
    };
    let mut links = vec![(&target.settings, Origin::Target)];
    for node in path.iter().rev().skip(1) {
        if let Node::Folder(f) = node {
            links.push((&f.settings, Origin::Folder(f.id)));
        }
    }
    links.push((&index.root.settings, Origin::Root));
    Ok(Chain { target, links })
}

fn merged_overrides(links: &[(&Settings, Origin)], name: &str) -> FieldOverrides {
    let mut out = FieldOverrides::default();
    for (s, _) in links {
        if let Some(o) = s.overrides.get(name) {
            if out.username.is_none() {
                out.username = o.username.clone();
            }
            if out.domain.is_none() {
                out.domain = o.domain.clone();
            }
        }
    }
    out
}

fn find_named(links: &[(&Settings, Origin)], name: &str) -> Option<ResolvedCredential> {
    links.iter().find_map(|(s, origin)| {
        s.credentials.get(name).map(|r| ResolvedCredential {
            name: name.to_string(),
            reference: r.clone(),
            origin: *origin,
            overrides: merged_overrides(links, name),
        })
    })
}

/// Sucht den Zugangsdatensatz für ein Ziel (Name des Ziels, sonst `default`).
pub fn resolve_credential(index: &Index, target_id: Uuid) -> Result<CredentialResolution, UnknownTarget> {
    let c = chain(index, target_id)?;
    let name = c.target.credential_name();
    if let Some(found) = find_named(&c.links, name) {
        return Ok(CredentialResolution::Found(found));
    }
    if c.target.credential.is_none() {
        // Alle Namen entlang der Kette, jeweils der nächstgelegene Treffer.
        let mut names: Vec<&str> = Vec::new();
        for (s, _) in &c.links {
            for n in s.credentials.keys() {
                if !names.contains(&n.as_str()) {
                    names.push(n);
                }
            }
        }
        let candidates: Vec<_> = names.iter().filter_map(|n| find_named(&c.links, n)).collect();
        match candidates.len() {
            0 => {}
            1 => return Ok(CredentialResolution::Found(candidates.into_iter().next().unwrap())),
            _ => return Ok(CredentialResolution::Ambiguous(candidates)),
        }
    }
    Ok(CredentialResolution::NotFound { name: name.to_string() })
}

/// Sucht einen ausdrücklich benannten Zugangsdatensatz (z. B. nach Auswahl).
pub fn resolve_credential_named(
    index: &Index,
    target_id: Uuid,
    name: &str,
) -> Result<Option<ResolvedCredential>, UnknownTarget> {
    let c = chain(index, target_id)?;
    Ok(find_named(&c.links, name))
}

/// Geerbter Tunnel; `None` bei `tunnel = none` oder wenn nirgends gesetzt.
pub fn resolve_tunnel(index: &Index, target_id: Uuid) -> Result<Option<(Uuid, Origin)>, UnknownTarget> {
    let c = chain(index, target_id)?;
    for (s, origin) in &c.links {
        match s.tunnel {
            Some(TunnelSetting::None) => return Ok(None),
            Some(TunnelSetting::Use(id)) => return Ok(Some((id, *origin))),
            None => {}
        }
    }
    Ok(None)
}

/// Geerbter PowerShell-Transport.
pub fn resolve_psrp_transport(index: &Index, target_id: Uuid) -> Result<Option<PsrpTransport>, UnknownTarget> {
    let c = chain(index, target_id)?;
    Ok(c.links.iter().find_map(|(s, _)| s.psrp_transport.clone()))
}

/// Geerbtes SNMP-Gerät; `None` bei abgeschaltetem SNMP.
pub fn resolve_snmp(index: &Index, target_id: Uuid) -> Result<Option<Uuid>, UnknownTarget> {
    let c = chain(index, target_id)?;
    for (s, _) in &c.links {
        match s.snmp {
            Some(SnmpSetting::None) => return Ok(None),
            Some(SnmpSetting::Device(id)) => return Ok(Some(id)),
            None => {}
        }
    }
    Ok(None)
}

/// Settings eines Knotens (oder der Wurzel) zum Ändern.
pub fn settings_mut(index: &mut Index, node_id: Uuid) -> Option<&mut Settings> {
    fn walk(f: &mut Folder, id: Uuid) -> Option<&mut Settings> {
        if f.id == id {
            return Some(&mut f.settings);
        }
        for n in &mut f.children {
            match n {
                Node::Folder(sub) => {
                    if let Some(s) = walk(sub, id) {
                        return Some(s);
                    }
                }
                Node::Target(t) if t.id == id => return Some(&mut t.settings),
                Node::Target(_) => {}
            }
        }
        None
    }
    walk(&mut index.root, node_id)
}

/// Liefert eine Kopie des Index, in der der Verweis `name` am Knoten
/// `node_id` auf einen anderen KDBX-Eintrag zeigt. Der ursprüngliche Index
/// bleibt unverändert, bis der Aufrufer die Kopie übernimmt.
pub fn with_reassigned_credential(
    index: &Index,
    node_id: Uuid,
    name: &str,
    entry: Uuid,
    title: Option<String>,
) -> Option<Index> {
    let mut copy = index.clone();
    let s = settings_mut(&mut copy, node_id)?;
    s.credentials.insert(name.to_string(), CredentialRef { entry, last_title: title });
    Some(copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        index: Index,
        folder: Uuid,
        sub: Uuid,
        target: Uuid,
        e_root: Uuid,
        e_folder: Uuid,
        e_target: Uuid,
        tunnel: Uuid,
    }

    fn cref(entry: Uuid, title: &str) -> CredentialRef {
        CredentialRef { entry, last_title: Some(title.into()) }
    }

    /// Wurzel → Ordner → Unterordner → Ziel.
    fn fixture() -> Fixture {
        let (e_root, e_folder, e_target, tunnel) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut index = Index::default();
        index.root.settings.credentials.insert("default".into(), cref(e_root, "Wurzel-Konto"));
        let mut folder = Folder::new("Standort");
        folder.settings.tunnel = Some(TunnelSetting::Use(tunnel));
        folder.settings.psrp_transport = Some(PsrpTransport::Ssh { port: Some(22) });
        let mut sub = Folder::new("Server");
        let target = Target::new("srv", Protocol::Ssh, "srv.example.test");
        let (fid, sid, tid) = (folder.id, sub.id, target.id);
        sub.children.push(Node::Target(target));
        folder.children.push(Node::Folder(sub));
        index.root.children.push(Node::Folder(folder));
        let _ = e_folder;
        let _ = e_target;
        Fixture { index, folder: fid, sub: sid, target: tid, e_root, e_folder, e_target, tunnel }
    }

    fn found(r: CredentialResolution) -> ResolvedCredential {
        match r {
            CredentialResolution::Found(f) => f,
            other => panic!("erwartet Treffer, erhalten {other:?}"),
        }
    }

    #[test]
    fn hit_in_root() {
        let f = fixture();
        let r = found(resolve_credential(&f.index, f.target).unwrap());
        assert_eq!(r.reference.entry, f.e_root);
        assert_eq!(r.origin, Origin::Root);
    }

    #[test]
    fn hit_in_parent_folder() {
        let mut f = fixture();
        settings_mut(&mut f.index, f.sub).unwrap().credentials.insert("default".into(), cref(f.e_folder, "Ordner"));
        let r = found(resolve_credential(&f.index, f.target).unwrap());
        assert_eq!(r.reference.entry, f.e_folder);
        assert_eq!(r.origin, Origin::Folder(f.sub));
    }

    #[test]
    fn hit_at_target_wins() {
        let mut f = fixture();
        settings_mut(&mut f.index, f.sub).unwrap().credentials.insert("default".into(), cref(f.e_folder, "Ordner"));
        settings_mut(&mut f.index, f.target).unwrap().credentials.insert("default".into(), cref(f.e_target, "Ziel"));
        let r = found(resolve_credential(&f.index, f.target).unwrap());
        assert_eq!(r.reference.entry, f.e_target);
        assert_eq!(r.origin, Origin::Target);
    }

    #[test]
    fn overrides_per_field_nearest_wins() {
        let mut f = fixture();
        settings_mut(&mut f.index, f.folder).unwrap().overrides.insert(
            "default".into(),
            FieldOverrides { username: Some("oben".into()), domain: Some("EXAMPLE".into()) },
        );
        settings_mut(&mut f.index, f.target)
            .unwrap()
            .overrides
            .insert("default".into(), FieldOverrides { username: Some("unten".into()), domain: None });
        let r = found(resolve_credential(&f.index, f.target).unwrap());
        assert_eq!(r.overrides.username.as_deref(), Some("unten"));
        assert_eq!(r.overrides.domain.as_deref(), Some("EXAMPLE"));
        assert_eq!(r.reference.entry, f.e_root);
    }

    #[test]
    fn named_credential() {
        let mut f = fixture();
        settings_mut(&mut f.index, f.folder).unwrap().credentials.insert("admin".into(), cref(f.e_folder, "Admin"));
        let Some(Node::Folder(folder)) = f.index.root.children.first_mut() else { panic!() };
        let Node::Folder(sub) = &mut folder.children[0] else { panic!() };
        let Node::Target(t) = &mut sub.children[0] else { panic!() };
        t.credential = Some("admin".into());
        let r = found(resolve_credential(&f.index, f.target).unwrap());
        assert_eq!(r.name, "admin");
        assert_eq!(r.reference.entry, f.e_folder);
    }

    #[test]
    fn several_names_without_default_are_ambiguous() {
        let mut f = fixture();
        f.index.root.settings.credentials.clear();
        let s = settings_mut(&mut f.index, f.folder).unwrap();
        s.credentials.insert("admin".into(), cref(f.e_folder, "Admin"));
        s.credentials.insert("user".into(), cref(f.e_target, "User"));
        match resolve_credential(&f.index, f.target).unwrap() {
            CredentialResolution::Ambiguous(c) => {
                let names: Vec<_> = c.iter().map(|c| c.name.as_str()).collect();
                assert_eq!(names, ["admin", "user"]);
            }
            other => panic!("{other:?}"),
        }
        // Nach Auswahl:
        let r = resolve_credential_named(&f.index, f.target, "user").unwrap().unwrap();
        assert_eq!(r.reference.entry, f.e_target);
    }

    #[test]
    fn single_other_name_is_taken() {
        let mut f = fixture();
        f.index.root.settings.credentials.clear();
        settings_mut(&mut f.index, f.folder).unwrap().credentials.insert("admin".into(), cref(f.e_folder, "Admin"));
        assert_eq!(found(resolve_credential(&f.index, f.target).unwrap()).name, "admin");
    }

    #[test]
    fn no_hit() {
        let mut f = fixture();
        f.index.root.settings.credentials.clear();
        assert_eq!(
            resolve_credential(&f.index, f.target).unwrap(),
            CredentialResolution::NotFound { name: "default".into() }
        );
        assert!(resolve_credential(&f.index, Uuid::new_v4()).is_err());
        // Ordner sind keine Ziele.
        assert!(resolve_credential(&f.index, f.folder).is_err());
    }

    #[test]
    fn tunnel_inherited_and_disabled() {
        let mut f = fixture();
        assert_eq!(resolve_tunnel(&f.index, f.target).unwrap(), Some((f.tunnel, Origin::Folder(f.folder))));
        settings_mut(&mut f.index, f.target).unwrap().tunnel = Some(TunnelSetting::None);
        assert_eq!(resolve_tunnel(&f.index, f.target).unwrap(), None);
    }

    #[test]
    fn transport_and_snmp_inherited() {
        let mut f = fixture();
        assert_eq!(
            resolve_psrp_transport(&f.index, f.target).unwrap(),
            Some(PsrpTransport::Ssh { port: Some(22) })
        );
        let dev = Uuid::new_v4();
        f.index.root.settings.snmp = Some(SnmpSetting::Device(dev));
        assert_eq!(resolve_snmp(&f.index, f.target).unwrap(), Some(dev));
        settings_mut(&mut f.index, f.sub).unwrap().snmp = Some(SnmpSetting::None);
        assert_eq!(resolve_snmp(&f.index, f.target).unwrap(), None);
    }

    #[test]
    fn reassign_leaves_original_untouched() {
        let f = fixture();
        let new = Uuid::new_v4();
        let copy = with_reassigned_credential(&f.index, f.index.root.id, "default", new, Some("Neu".into())).unwrap();
        assert_eq!(found(resolve_credential(&f.index, f.target).unwrap()).reference.entry, f.e_root);
        assert_eq!(found(resolve_credential(&copy, f.target).unwrap()).reference.entry, new);
    }
}
