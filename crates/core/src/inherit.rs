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
    /// Zugangsdaten sind ausdrücklich abgeschaltet (am Ziel oder darüber).
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Ziel {0} nicht im Index gefunden")]
pub struct UnknownTarget(pub Uuid);

/// Kette der Einstellungen vom Ziel bis zur Wurzel.
struct Chain<'a> {
    target: &'a Target,
    links: Vec<(&'a Settings, Origin)>,
}

/// Einstellungen vom Knoten (Ziel, Ordner oder Wurzel) bis zur Wurzel.
fn links_for(index: &Index, node_id: Uuid) -> Option<(Option<&Target>, Vec<(&Settings, Origin)>)> {
    if node_id == index.root.id {
        return Some((None, vec![(&index.root.settings, Origin::Root)]));
    }
    let path = index.path_to(node_id)?;
    let mut links = Vec::new();
    let mut target = None;
    match path.last()? {
        Node::Target(t) => {
            target = Some(t);
            links.push((&t.settings, Origin::Target));
        }
        Node::Folder(f) => links.push((&f.settings, Origin::Folder(f.id))),
    }
    for node in path.iter().rev().skip(1) {
        if let Node::Folder(f) = node {
            links.push((&f.settings, Origin::Folder(f.id)));
        }
    }
    links.push((&index.root.settings, Origin::Root));
    Some((target, links))
}

fn chain(index: &Index, target_id: Uuid) -> Result<Chain<'_>, UnknownTarget> {
    match links_for(index, target_id) {
        Some((Some(target), links)) => Ok(Chain { target, links }),
        _ => Err(UnknownTarget(target_id)),
    }
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

/// Gemeinsame Suche für Ziele und Ordner. `own_name` ist der am Knoten selbst
/// verlangte Name (Ziel: `credential`).
fn resolve_links(links: &[(&Settings, Origin)], own_name: Option<&str>) -> CredentialResolution {
    // Abgeschaltete Zugangsdaten schneiden die Kette ab.
    let cut = links.iter().position(|(s, _)| s.credentials_off);
    let links = &links[..cut.unwrap_or(links.len())];
    let explicit = own_name.map(str::to_string).or_else(|| links.iter().find_map(|(s, _)| s.credential_name.clone()));
    let name = explicit.clone().unwrap_or_else(|| DEFAULT_CREDENTIAL.to_string());
    if let Some(found) = find_named(links, &name) {
        return CredentialResolution::Found(found);
    }
    if cut.is_some() {
        return CredentialResolution::Disabled;
    }
    if explicit.is_none() {
        // Alle Namen entlang der Kette, jeweils der nächstgelegene Treffer.
        let mut names: Vec<&str> = Vec::new();
        for (s, _) in links {
            for n in s.credentials.keys() {
                if !names.contains(&n.as_str()) {
                    names.push(n);
                }
            }
        }
        let mut candidates: Vec<_> = names.iter().filter_map(|n| find_named(links, n)).collect();
        match candidates.len() {
            0 => {}
            1 => return CredentialResolution::Found(candidates.remove(0)),
            _ => return CredentialResolution::Ambiguous(candidates),
        }
    }
    CredentialResolution::NotFound { name }
}

/// Sucht den Zugangsdatensatz für ein Ziel (Name des Ziels, sonst der
/// nächstgelegene Name eines Ordners, sonst `default`).
pub fn resolve_credential(index: &Index, target_id: Uuid) -> Result<CredentialResolution, UnknownTarget> {
    let c = chain(index, target_id)?;
    Ok(resolve_links(&c.links, c.target.credential.as_deref()))
}

/// Wirksamer Zugangsdatensatz an einem beliebigen Knoten (auch Ordner), wie
/// ihn ein Unterobjekt ohne eigene Angabe erben würde.
pub fn resolve_credential_at(index: &Index, node_id: Uuid) -> Option<CredentialResolution> {
    let (target, links) = links_for(index, node_id)?;
    Some(resolve_links(&links, target.and_then(|t| t.credential.as_deref())))
}

/// Am Knoten selbst festgelegte Art der Zugangsdaten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialMode {
    /// Nichts festgelegt: vom übergeordneten Ordner.
    Inherit,
    /// Ausdrücklich keine Zugangsdaten.
    None,
    /// Verweis auf einen KDBX-Eintrag am Knoten selbst (unter `name`).
    Entry { name: String, reference: CredentialRef },
    /// Benannter Datensatz, gesucht nach oben.
    Named(String),
    /// Anmeldeobjekt (Verweis auf `Index::logins`).
    Login(Uuid),
}

/// Alle Einstellungen im Baum (Wurzel, Ordner, Ziele) mit der Kennung ihres Knotens.
fn for_each_settings_mut(f: &mut Folder, visit: &mut dyn FnMut(Uuid, &mut Settings)) {
    visit(f.id, &mut f.settings);
    for c in &mut f.children {
        match c {
            Node::Folder(sub) => for_each_settings_mut(sub, visit),
            Node::Target(t) => visit(t.id, &mut t.settings),
        }
    }
}

fn for_each_settings(f: &Folder, visit: &mut dyn FnMut(Uuid, &Settings)) {
    visit(f.id, &f.settings);
    for c in &f.children {
        match c {
            Node::Folder(sub) => for_each_settings(sub, visit),
            Node::Target(t) => visit(t.id, &t.settings),
        }
    }
}

/// Knoten (Ordner, Ziele), die das Anmeldeobjekt `login` verwenden.
pub fn login_users(index: &Index, login: Uuid) -> Vec<Uuid> {
    let mut out = Vec::new();
    for_each_settings(&index.root, &mut |id, s| {
        if s.login == Some(login) {
            out.push(id);
        }
    });
    out
}

/// Den KDBX-Eintrag jedes Anmeldeobjekts bei allen Verwendern nachtragen (nach Änderungen am Objekt
/// oder nach dem Laden). Verweise auf nicht mehr vorhandene Objekte werden gelöst; der bisherige
/// Eintrag bleibt dann als eigener Verweis stehen.
pub fn sync_logins(index: &mut Index) {
    let logins: std::collections::HashMap<Uuid, CredentialRef> = index.logins.iter().map(|l| (l.id, l.entry.clone())).collect();
    for_each_settings_mut(&mut index.root, &mut |_, s| {
        if let Some(l) = s.login {
            match logins.get(&l) {
                Some(entry) => {
                    s.credentials.insert(DEFAULT_CREDENTIAL.to_string(), entry.clone());
                }
                None => s.login = None,
            }
        }
    });
}

/// Bisherige Verweise auf fremde KDBX-Einträge („vorhandener Datensatz“, nur unter dem
/// Standardnamen) in Anmeldeobjekte umwandeln: je Eintrag ein Objekt, abgelegt im Ordner des ersten
/// Verwenders. Eigene Einträge („Benutzername und Kennwort direkt“) bleiben, wie sie sind.
/// Liefert die Zahl der umgestellten Ordner und Ziele.
pub fn convert_entries_to_logins(index: &mut Index, is_own: &dyn Fn(Uuid) -> bool, title_of: &dyn Fn(Uuid) -> Option<String>) -> usize {
    // (Knoten, Ablageordner, Eintrag)
    let mut found: Vec<(Uuid, Uuid, CredentialRef)> = Vec::new();
    fn walk(f: &Folder, out: &mut Vec<(Uuid, Uuid, CredentialRef)>, is_own: &dyn Fn(Uuid) -> bool) {
        let take = |s: &Settings, own_name: Option<&str>| -> Option<CredentialRef> {
            if s.login.is_some() || s.credentials_off || own_name.is_some_and(|n| n != DEFAULT_CREDENTIAL) {
                return None;
            }
            s.credentials.get(DEFAULT_CREDENTIAL).filter(|r| !is_own(r.entry)).cloned()
        };
        if let Some(r) = take(&f.settings, f.settings.credential_name.as_deref()) {
            out.push((f.id, f.id, r));
        }
        for c in &f.children {
            match c {
                Node::Folder(sub) => walk(sub, out, is_own),
                Node::Target(t) => {
                    if let Some(r) = take(&t.settings, t.credential.as_deref()) {
                        out.push((t.id, f.id, r));
                    }
                }
            }
        }
    }
    walk(&index.root, &mut found, is_own);
    let mut map: std::collections::HashMap<Uuid, Uuid> = index.logins.iter().map(|l| (l.entry.entry, l.id)).collect();
    let mut assign: std::collections::HashMap<Uuid, Uuid> = std::collections::HashMap::new();
    for (node, folder, r) in &found {
        let lid = *map.entry(r.entry).or_insert_with(|| {
            let name = title_of(r.entry).or_else(|| r.last_title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| "Anmeldung".into());
            let l = Login { id: Uuid::new_v4(), name, folder: *folder, entry: r.clone() };
            let id = l.id;
            index.logins.push(l);
            id
        });
        assign.insert(*node, lid);
    }
    for_each_settings_mut(&mut index.root, &mut |id, s| {
        if let Some(l) = assign.get(&id) {
            s.login = Some(*l);
        }
    });
    found.len()
}

/// Anmeldeobjekt entfernen. Verwender behalten den Eintrag als eigenen Verweis (sie funktionieren
/// weiter); im Baum verschwindet nur das Objekt.
pub fn delete_login(index: &mut Index, login: Uuid) -> bool {
    let before = index.logins.len();
    index.logins.retain(|l| l.id != login);
    if index.logins.len() == before {
        return false;
    }
    sync_logins(index);
    true
}

/// Liest die am Knoten festgelegte Art der Zugangsdaten.
pub fn credential_mode(index: &Index, node_id: Uuid) -> Option<CredentialMode> {
    let (settings, own_name) = node_credential_parts(index, node_id)?;
    if settings.credentials_off {
        return Some(CredentialMode::None);
    }
    let name = own_name.clone().unwrap_or_else(|| DEFAULT_CREDENTIAL.to_string());
    if let Some(l) = settings.login.filter(|l| index.login(*l).is_some()) {
        return Some(CredentialMode::Login(l));
    }
    if let Some(r) = settings.credentials.get(&name) {
        return Some(CredentialMode::Entry { name, reference: r.clone() });
    }
    Some(match own_name {
        Some(n) => CredentialMode::Named(n),
        None => CredentialMode::Inherit,
    })
}

fn node_credential_parts(index: &Index, node_id: Uuid) -> Option<(&Settings, Option<String>)> {
    if node_id == index.root.id {
        return Some((&index.root.settings, index.root.settings.credential_name.clone()));
    }
    match index.path_to(node_id)?.last()? {
        Node::Target(t) => Some((&t.settings, t.credential.clone())),
        Node::Folder(f) => Some((&f.settings, f.settings.credential_name.clone())),
    }
}

/// Setzt die Art der Zugangsdaten am Knoten. Ein bisheriger eigener Verweis
/// unter dem bisherigen Namen wird entfernt (der KDBX-Eintrag bleibt).
pub fn set_credential_mode(index: &mut Index, node_id: Uuid, mode: CredentialMode) -> bool {
    let old_name = match node_credential_parts(index, node_id) {
        Some((_, n)) => n.unwrap_or_else(|| DEFAULT_CREDENTIAL.to_string()),
        None => return false,
    };
    let is_target = matches!(index.path_to(node_id).and_then(|p| p.last().copied()), Some(Node::Target(_)));
    let login_entry = match &mode {
        CredentialMode::Login(l) => match index.login(*l) {
            Some(l) => Some(l.entry.clone()),
            None => return false,
        },
        _ => None,
    };
    let Some(s) = settings_mut(index, node_id) else { return false };
    s.credentials.remove(&old_name);
    s.credentials_off = false;
    s.login = None;
    let new_name: Option<String> = match mode {
        CredentialMode::Inherit => None,
        CredentialMode::None => {
            s.credentials_off = true;
            None
        }
        CredentialMode::Entry { name, reference } => {
            s.credentials.insert(name.clone(), reference);
            (name != DEFAULT_CREDENTIAL).then_some(name)
        }
        CredentialMode::Named(n) => Some(n),
        CredentialMode::Login(l) => {
            // Unter dem Standardnamen eintragen: so finden Auflösung und ältere Fassungen den Eintrag.
            s.login = Some(l);
            s.credentials.insert(DEFAULT_CREDENTIAL.to_string(), login_entry.expect("geprüft"));
            None
        }
    };
    if is_target {
        s.credential_name = None;
        if let Some(Node::Target(t)) = find_node_mut(&mut index.root, node_id) {
            t.credential = new_name;
        }
    } else {
        s.credential_name = new_name;
    }
    true
}

/// Knoten zum Ändern (ohne Wurzel).
pub fn find_node_mut(f: &mut Folder, id: Uuid) -> Option<&mut Node> {
    for n in &mut f.children {
        if n.id() == id {
            return Some(n);
        }
        if let Node::Folder(sub) = n {
            if let Some(found) = find_node_mut(sub, id) {
                return Some(found);
            }
        }
    }
    None
}

/// Alle Namen von Zugangsdatensätzen, die am Knoten nach oben erreichbar sind.
pub fn reachable_credential_names(index: &Index, node_id: Uuid) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some((_, links)) = links_for(index, node_id) {
        for (s, _) in links.iter().skip(1) {
            for n in s.credentials.keys() {
                if !out.contains(n) {
                    out.push(n.clone());
                }
            }
        }
    }
    out
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

/// Woher die Inventar-Einstellung kommt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventorySource {
    /// Am Ziel selbst festgelegt.
    Target,
    /// Von einem Ordner geerbt.
    Folder,
    /// Nichts festgelegt: Vorgabe (Web-Ziele aus, alle anderen an).
    Default,
}

/// Darf das Inventar sich am Ziel anmelden (SSH/PowerShell)? Das Ziel selbst entscheidet zuerst, dann die
/// Ordner nach oben. Web-Ziele sind ohne eigene Festlegung immer aus – ein „an“ am Ordner schaltet sie nicht
/// ein (sonst meldete sich das Inventar mit den Zugangsdaten der Web-Seite per SSH an); ein „aus“ am Ordner gilt
/// für alle.
pub fn resolve_inventory(index: &Index, target_id: Uuid) -> Result<(bool, InventorySource), UnknownTarget> {
    inventory_from(index, target_id, false)
}

/// Wie [`resolve_inventory`], aber ohne die Festlegung am Ziel selbst: der Wert, den „geerbt“ ergäbe.
pub fn resolve_inventory_inherited(index: &Index, target_id: Uuid) -> Result<(bool, InventorySource), UnknownTarget> {
    inventory_from(index, target_id, true)
}

fn inventory_from(index: &Index, target_id: Uuid, skip_target: bool) -> Result<(bool, InventorySource), UnknownTarget> {
    let c = chain(index, target_id)?;
    let web = c.target.protocol == crate::model::Protocol::Web;
    for (i, (s, _)) in c.links.iter().enumerate().skip(usize::from(skip_target)) {
        match s.inventory {
            Some(on) if i == 0 => return Ok((on, InventorySource::Target)),
            Some(false) => return Ok((false, InventorySource::Folder)),
            Some(true) if !web => return Ok((true, InventorySource::Folder)),
            _ => {}
        }
    }
    Ok((!web, InventorySource::Default))
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

    #[test]
    fn inventory_defaults_off_for_web_and_folder_on_does_not_enable_web() {
        let mut f = fixture();
        let web = Target::new("web", Protocol::Web, "https://ap.example.test:4343");
        let wid = web.id;
        if let Some(Node::Folder(folder)) = f.index.root.children.first_mut() {
            if let Some(Node::Folder(sub)) = folder.children.first_mut() {
                sub.children.push(Node::Target(web));
            }
        }
        // Ohne Festlegung: SSH an, Web aus.
        assert_eq!(resolve_inventory(&f.index, f.target).unwrap(), (true, InventorySource::Default));
        assert_eq!(resolve_inventory(&f.index, wid).unwrap(), (false, InventorySource::Default));
        // Ordner „an“: gilt für SSH, nicht für Web.
        settings_mut(&mut f.index, f.folder).unwrap().inventory = Some(true);
        assert_eq!(resolve_inventory(&f.index, f.target).unwrap(), (true, InventorySource::Folder));
        assert_eq!(resolve_inventory(&f.index, wid).unwrap(), (false, InventorySource::Default));
        // Ordner „aus“ (näher am Ziel): gilt für alle.
        settings_mut(&mut f.index, f.sub).unwrap().inventory = Some(false);
        assert_eq!(resolve_inventory(&f.index, f.target).unwrap(), (false, InventorySource::Folder));
        assert_eq!(resolve_inventory(&f.index, wid).unwrap(), (false, InventorySource::Folder));
        // Am Ziel selbst festgelegt: gewinnt immer, auch beim Web-Ziel.
        settings_mut(&mut f.index, wid).unwrap().inventory = Some(true);
        assert_eq!(resolve_inventory(&f.index, wid).unwrap(), (true, InventorySource::Target));
        settings_mut(&mut f.index, f.target).unwrap().inventory = Some(true);
        assert_eq!(resolve_inventory(&f.index, f.target).unwrap(), (true, InventorySource::Target));
        // „Geerbt“ zeigt den Wert ohne die eigene Festlegung.
        assert_eq!(resolve_inventory_inherited(&f.index, f.target).unwrap(), (false, InventorySource::Folder));
        assert_eq!(resolve_inventory_inherited(&f.index, wid).unwrap(), (false, InventorySource::Folder));
    }

    #[test]
    fn login_object_is_used_by_folder_and_survives_deletion_as_entry() {
        let mut f = fixture();
        let entry = Uuid::new_v4();
        let login = Login { id: Uuid::new_v4(), name: "Domänenadmin".into(), folder: f.folder, entry: cref(entry, "Domänenadmin") };
        let lid = login.id;
        f.index.logins.push(login);
        assert!(set_credential_mode(&mut f.index, f.folder, CredentialMode::Login(lid)));
        assert_eq!(credential_mode(&f.index, f.folder), Some(CredentialMode::Login(lid)));
        // Das Ziel darunter erbt die Anmeldung.
        assert_eq!(found(resolve_credential(&f.index, f.target).unwrap()).reference.entry, entry);
        assert_eq!(login_users(&f.index, lid), [f.folder]);
        // Eintrag des Objekts geändert: Verwender folgen.
        let entry2 = Uuid::new_v4();
        f.index.logins[0].entry = cref(entry2, "neu");
        sync_logins(&mut f.index);
        assert_eq!(found(resolve_credential(&f.index, f.target).unwrap()).reference.entry, entry2);
        // Anderer Modus löst den Verweis.
        assert!(set_credential_mode(&mut f.index, f.folder, CredentialMode::Inherit));
        assert!(login_users(&f.index, lid).is_empty());
        // Gelöscht: Verwender behalten den Eintrag.
        assert!(set_credential_mode(&mut f.index, f.folder, CredentialMode::Login(lid)));
        assert!(delete_login(&mut f.index, lid));
        assert!(matches!(credential_mode(&f.index, f.folder), Some(CredentialMode::Entry { .. })));
        assert_eq!(found(resolve_credential(&f.index, f.target).unwrap()).reference.entry, entry2);
        // Unbekanntes Objekt wird abgelehnt.
        assert!(!set_credential_mode(&mut f.index, f.folder, CredentialMode::Login(Uuid::new_v4())));
    }

    #[test]
    fn foreign_entries_become_login_objects_once() {
        let mut f = fixture();
        let shared = Uuid::new_v4();
        let own = Uuid::new_v4();
        // Ordner und ein Ziel nutzen denselben fremden Eintrag, die Wurzel einen eigenen.
        settings_mut(&mut f.index, f.folder).unwrap().credentials.insert("default".into(), cref(shared, "Domäne"));
        settings_mut(&mut f.index, f.target).unwrap().credentials.insert("default".into(), cref(shared, "Domäne"));
        f.index.root.settings.credentials.insert("default".into(), cref(own, "eigen"));
        let n = convert_entries_to_logins(&mut f.index, &|e| e == own, &|_| Some("Domänenkonto".into()));
        assert_eq!(n, 2);
        assert_eq!(f.index.logins.len(), 1, "ein Objekt je Eintrag");
        assert_eq!(f.index.logins[0].name, "Domänenkonto");
        assert_eq!(f.index.logins[0].folder, f.folder);
        let lid = f.index.logins[0].id;
        assert_eq!(credential_mode(&f.index, f.target), Some(CredentialMode::Login(lid)));
        assert!(matches!(credential_mode(&f.index, f.index.root.id), Some(CredentialMode::Entry { .. })), "eigene Einträge bleiben");
        assert_eq!(found(resolve_credential(&f.index, f.target).unwrap()).reference.entry, shared);
        // Zweiter Lauf ändert nichts.
        assert_eq!(convert_entries_to_logins(&mut f.index, &|e| e == own, &|_| None), 0);
        assert_eq!(f.index.logins.len(), 1);
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
