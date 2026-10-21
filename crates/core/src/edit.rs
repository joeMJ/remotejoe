//! Änderungen am Index aus der Oberfläche: Anlegen, Verschieben,
//! Duplizieren, Löschen, Favoriten, Ansichten, Vorlagen und „Auf Unterobjekte
//! anwenden“, dazu ein Verlauf zum Rückgängigmachen.
//!
//! Alle Funktionen arbeiten nur auf dem Index; KDBX-Einträge werden hier weder
//! angelegt noch gelöscht.

use uuid::Uuid;

use crate::inherit::find_node_mut;
use crate::model::*;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("Objekt nicht gefunden")]
    NotFound,
    #[error("Ziel ist kein Ordner")]
    NotAFolder,
    #[error("Ein Ordner kann nicht in sich selbst oder einen seiner Unterordner verschoben werden")]
    Cycle,
    #[error("Die Wurzel kann nicht verändert werden")]
    Root,
    #[error("Name darf nicht leer sein")]
    EmptyName,
}

/// Ordner zum Ändern (auch die Wurzel).
pub fn folder_mut(index: &mut Index, id: Uuid) -> Option<&mut Folder> {
    if index.root.id == id {
        return Some(&mut index.root);
    }
    match find_node_mut(&mut index.root, id)? {
        Node::Folder(f) => Some(f),
        Node::Target(_) => None,
    }
}

/// Ordner zum Lesen (auch die Wurzel).
pub fn folder(index: &Index, id: Uuid) -> Option<&Folder> {
    if index.root.id == id {
        return Some(&index.root);
    }
    match index.path_to(id)?.last()? {
        Node::Folder(f) => Some(f),
        Node::Target(_) => None,
    }
}

/// Knoten zum Lesen (ohne Wurzel).
pub fn node(index: &Index, id: Uuid) -> Option<&Node> {
    index.path_to(id)?.last().copied()
}

/// Elternordner eines Knotens (bei obersten Knoten die Wurzel).
pub fn parent_of(index: &Index, id: Uuid) -> Option<Uuid> {
    let path = index.path_to(id)?;
    Some(if path.len() >= 2 { path[path.len() - 2].id() } else { index.root.id })
}

/// Ordner-ID samt Anzeigepfad, in Baumreihenfolge (für Auswahllisten).
pub fn folder_list(index: &Index) -> Vec<(Uuid, String)> {
    fn walk(f: &Folder, prefix: &str, out: &mut Vec<(Uuid, String)>) {
        for n in &f.children {
            if let Node::Folder(sub) = n {
                let p = if prefix.is_empty() { sub.name.clone() } else { format!("{prefix} / {}", sub.name) };
                out.push((sub.id, p.clone()));
                walk(sub, &p, out);
            }
        }
    }
    let mut out = vec![(index.root.id, String::new())];
    walk(&index.root, "", &mut out);
    out
}

/// Alle Ziele unterhalb eines Ordners (in Baumreihenfolge).
pub fn targets_below(index: &Index, folder_id: Uuid) -> Vec<Uuid> {
    fn walk(f: &Folder, out: &mut Vec<Uuid>) {
        for n in &f.children {
            match n {
                Node::Target(t) => out.push(t.id),
                Node::Folder(sub) => walk(sub, out),
            }
        }
    }
    let mut out = Vec::new();
    if let Some(f) = folder(index, folder_id) {
        walk(f, &mut out);
    }
    out
}

fn contains(f: &Folder, id: Uuid) -> bool {
    f.children.iter().any(|n| {
        n.id() == id
            || match n {
                Node::Folder(sub) => contains(sub, id),
                Node::Target(_) => false,
            }
    })
}

/// Fügt einen Knoten in einen Ordner ein (`pos` = Position, sonst am Ende).
pub fn insert(index: &mut Index, parent: Uuid, node: Node, pos: Option<usize>) -> Result<Uuid, EditError> {
    let id = node.id();
    let f = folder_mut(index, parent).ok_or(EditError::NotAFolder)?;
    let at = pos.unwrap_or(f.children.len()).min(f.children.len());
    f.children.insert(at, node);
    Ok(id)
}

fn take(f: &mut Folder, id: Uuid) -> Option<(Node, usize)> {
    if let Some(i) = f.children.iter().position(|n| n.id() == id) {
        return Some((f.children.remove(i), i));
    }
    for n in &mut f.children {
        if let Node::Folder(sub) = n {
            if let Some(found) = take(sub, id) {
                return Some(found);
            }
        }
    }
    None
}

/// IDs des Knotens und aller Unterknoten.
pub fn subtree(index: &Index, id: Uuid) -> Vec<Uuid> {
    let mut out = Vec::new();
    if id == index.root.id {
        for c in &index.root.children {
            ids_in(c, &mut out);
        }
        out.push(id);
    } else if let Some(n) = node(index, id) {
        ids_in(n, &mut out);
    }
    out
}

fn ids_in(node: &Node, out: &mut Vec<Uuid>) {
    out.push(node.id());
    if let Node::Folder(f) = node {
        for c in &f.children {
            ids_in(c, out);
        }
    }
}

/// Löscht einen Knoten samt Unterknoten und entfernt ihn aus Ansichten und
/// Favoriten. Zugangsdaten bleiben in der KDBX (sie erscheinen danach unter
/// „Nicht zugeordnet“).
pub fn delete(index: &mut Index, id: Uuid) -> Result<Node, EditError> {
    if id == index.root.id {
        return Err(EditError::Root);
    }
    let (node, _) = take(&mut index.root, id).ok_or(EditError::NotFound)?;
    let mut gone = Vec::new();
    ids_in(&node, &mut gone);
    for v in &mut index.views {
        v.targets.retain(|t| !gone.contains(t));
    }
    index.favorites.retain(|t| !gone.contains(t));
    Ok(node)
}

/// Verschiebt einen Knoten in einen anderen Ordner (`pos` im neuen Ordner).
pub fn move_node(index: &mut Index, id: Uuid, new_parent: Uuid, pos: Option<usize>) -> Result<(), EditError> {
    if index.logins.iter().any(|l| l.id == id) {
        return move_login(index, id, new_parent, pos);
    }
    if id == index.root.id {
        return Err(EditError::Root);
    }
    if id == new_parent {
        return Err(EditError::Cycle);
    }
    if let Some(Node::Folder(f)) = node(index, id) {
        if contains(f, new_parent) {
            return Err(EditError::Cycle);
        }
    }
    folder(index, new_parent).ok_or(EditError::NotAFolder)?;
    let old_parent = parent_of(index, id).ok_or(EditError::NotFound)?;
    let (n, old_pos) = take(&mut index.root, id).ok_or(EditError::NotFound)?;
    // Innerhalb desselben Ordners nach hinten: Position verschiebt sich um eins.
    let pos = match pos {
        Some(p) if old_parent == new_parent && p > old_pos => Some(p - 1),
        p => p,
    };
    insert(index, new_parent, n, pos)?;
    Ok(())
}

/// Stellen der Anmeldeobjekte eines Ordners in `Index::logins`, in Baumreihenfolge.
fn login_slots(index: &Index, folder: Uuid) -> Vec<usize> {
    index.logins.iter().enumerate().filter(|(_, l)| l.folder == folder).map(|(i, _)| i).collect()
}

/// Verschiebt ein Anmeldeobjekt in einen Ordner. Im Baum stehen Anmeldeobjekte nach den Unterordnern und Zielen
/// eines Ordners; `pos` zählt wie dort, also über alle Zeilen des Ordners (fehlt sie, ans Ende).
fn move_login(index: &mut Index, id: Uuid, new_parent: Uuid, pos: Option<usize>) -> Result<(), EditError> {
    let children = folder(index, new_parent).ok_or(EditError::NotAFolder)?.children.len();
    let from = index.logins.iter().position(|l| l.id == id).ok_or(EditError::NotFound)?;
    // Bisheriger Rang im selben Ordner: Beim Ziehen nach hinten zählt die eigene Zeile noch mit.
    let old_rank = (index.logins[from].folder == new_parent).then(|| login_slots(index, new_parent).iter().position(|&i| i == from)).flatten();
    let mut rank = pos.map(|p| p.saturating_sub(children));
    if let (Some(r), Some(o)) = (rank, old_rank) {
        if r > o {
            rank = Some(r - 1);
        }
    }
    let mut login = index.logins.remove(from);
    login.folder = new_parent;
    let slots = login_slots(index, new_parent);
    let at = match rank {
        Some(r) if r < slots.len() => slots[r],
        _ => slots.last().map(|&i| i + 1).unwrap_or(index.logins.len()),
    };
    index.logins.insert(at, login);
    Ok(())
}

/// Verschiebt ein Anmeldeobjekt unter den Anmeldeobjekten seines Ordners um `delta` Plätze.
fn reorder_login(index: &mut Index, id: Uuid, delta: i32) -> Result<(), EditError> {
    let from = index.logins.iter().position(|l| l.id == id).ok_or(EditError::NotFound)?;
    let slots = login_slots(index, index.logins[from].folder);
    let k = slots.iter().position(|&i| i == from).ok_or(EditError::NotFound)?;
    let j = (k as i64 + delta as i64).clamp(0, slots.len() as i64 - 1) as usize;
    let login = index.logins.remove(from);
    index.logins.insert(slots[j], login);
    Ok(())
}

/// Verschiebt einen Knoten innerhalb seines Ordners um `delta` Plätze.
pub fn reorder(index: &mut Index, id: Uuid, delta: i32) -> Result<(), EditError> {
    if index.logins.iter().any(|l| l.id == id) {
        return reorder_login(index, id, delta);
    }
    let parent = parent_of(index, id).ok_or(EditError::NotFound)?;
    let f = folder_mut(index, parent).ok_or(EditError::NotFound)?;
    let i = f.children.iter().position(|n| n.id() == id).ok_or(EditError::NotFound)?;
    let j = (i as i64 + delta as i64).clamp(0, f.children.len() as i64 - 1) as usize;
    let n = f.children.remove(i);
    f.children.insert(j, n);
    Ok(())
}

fn renew_ids(node: &mut Node) {
    match node {
        Node::Target(t) => t.id = Uuid::new_v4(),
        Node::Folder(f) => {
            f.id = Uuid::new_v4();
            for c in &mut f.children {
                renew_ids(c);
            }
        }
    }
}

/// Dupliziert einen Knoten (neue UUIDs, gleiche Verweise auf Zugangsdaten,
/// Tunnel usw.) direkt hinter dem Original.
pub fn duplicate(index: &mut Index, id: Uuid, suffix: &str) -> Result<Uuid, EditError> {
    let original = node(index, id).ok_or(EditError::NotFound)?.clone();
    let parent = parent_of(index, id).ok_or(EditError::NotFound)?;
    let pos = folder(index, parent).and_then(|f| f.children.iter().position(|n| n.id() == id)).map(|p| p + 1);
    let mut copy = original;
    renew_ids(&mut copy);
    match &mut copy {
        Node::Target(t) => t.name.push_str(suffix),
        Node::Folder(f) => f.name.push_str(suffix),
    }
    insert(index, parent, copy, pos)
}

/// Benennt einen Knoten um (Name unverändert übernommen, nicht getrimmt).
pub fn rename(index: &mut Index, id: Uuid, name: &str) -> Result<(), EditError> {
    if name.is_empty() {
        return Err(EditError::EmptyName);
    }
    match find_node_mut(&mut index.root, id).ok_or(EditError::NotFound)? {
        Node::Target(t) => t.name = name.to_string(),
        Node::Folder(f) => f.name = name.to_string(),
    }
    Ok(())
}

/// Favorit setzen oder entfernen.
pub fn set_favorite(index: &mut Index, id: Uuid, on: bool) -> Result<(), EditError> {
    node(index, id).ok_or(EditError::NotFound)?;
    index.favorites.retain(|f| *f != id);
    if on {
        index.favorites.push(id);
    }
    Ok(())
}

/// Neue Ansicht anlegen.
pub fn create_view(index: &mut Index, name: &str) -> Result<Uuid, EditError> {
    if name.is_empty() {
        return Err(EditError::EmptyName);
    }
    let id = Uuid::new_v4();
    index.views.push(View { id, name: name.to_string(), targets: Vec::new() });
    Ok(id)
}

pub fn rename_view(index: &mut Index, view: Uuid, name: &str) -> Result<(), EditError> {
    if name.is_empty() {
        return Err(EditError::EmptyName);
    }
    index.views.iter_mut().find(|v| v.id == view).ok_or(EditError::NotFound)?.name = name.to_string();
    Ok(())
}

pub fn delete_view(index: &mut Index, view: Uuid) -> Result<(), EditError> {
    let before = index.views.len();
    index.views.retain(|v| v.id != view);
    if index.views.len() == before {
        return Err(EditError::NotFound);
    }
    Ok(())
}

/// Ziele einer Ansicht hinzufügen (Ordner: alle Ziele darunter); doppelte
/// Einträge werden vermieden.
pub fn add_to_view(index: &mut Index, view: Uuid, ids: &[Uuid]) -> Result<usize, EditError> {
    let mut targets = Vec::new();
    for id in ids {
        match node(index, *id) {
            Some(Node::Target(t)) => targets.push(t.id),
            Some(Node::Folder(f)) => targets.extend(targets_below(index, f.id)),
            None => return Err(EditError::NotFound),
        }
    }
    let v = index.views.iter_mut().find(|v| v.id == view).ok_or(EditError::NotFound)?;
    let mut added = 0;
    for t in targets {
        if !v.targets.contains(&t) {
            v.targets.push(t);
            added += 1;
        }
    }
    Ok(added)
}

pub fn remove_from_view(index: &mut Index, view: Uuid, ids: &[Uuid]) -> Result<(), EditError> {
    let v = index.views.iter_mut().find(|v| v.id == view).ok_or(EditError::NotFound)?;
    v.targets.retain(|t| !ids.contains(t));
    Ok(())
}

/// Einstellung für „Auf Unterobjekte anwenden“.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyKind {
    Credentials,
    Tunnel,
    PsrpTransport,
    Snmp,
    Clipboard,
}

impl ApplyKind {
    pub fn parse(s: &str) -> Option<ApplyKind> {
        Some(match s {
            "credentials" => ApplyKind::Credentials,
            "tunnel" => ApplyKind::Tunnel,
            "psrp_transport" => ApplyKind::PsrpTransport,
            "snmp" => ApplyKind::Snmp,
            "clipboard" => ApplyKind::Clipboard,
            _ => return None,
        })
    }
}

fn clipboard_of(o: &mut ProtocolOptions) -> Option<&mut ClipboardDirection> {
    match o {
        ProtocolOptions::Rdp(r) => Some(&mut r.clipboard),
        ProtocolOptions::Vnc(v) => Some(&mut v.clipboard),
        _ => None,
    }
}

/// Entfernt die eigene Angabe, damit der Wert des Ordners gilt.
fn apply_settings(settings: &mut Settings, kind: ApplyKind) -> bool {
    match kind {
        ApplyKind::Credentials => {
            let changed = !settings.credentials.is_empty()
                || !settings.overrides.is_empty()
                || settings.credentials_off
                || settings.credential_name.is_some();
            settings.credentials.clear();
            settings.overrides.clear();
            settings.credentials_off = false;
            settings.credential_name = None;
            changed
        }
        ApplyKind::Tunnel => settings.tunnel.take().is_some(),
        ApplyKind::PsrpTransport => settings.psrp_transport.take().is_some(),
        ApplyKind::Snmp => settings.snmp.take().is_some(),
        ApplyKind::Clipboard => settings.clipboard.take().is_some(),
    }
}

/// Setzt bei einem Unterobjekt die Einstellung so, dass der Wert des Ordners
/// gilt. Liefert, ob sich etwas geändert hat.
fn apply_one(node: &mut Node, kind: ApplyKind, clip: ClipboardDirection) -> bool {
    match node {
        Node::Folder(f) => apply_settings(&mut f.settings, kind),
        Node::Target(t) => {
            let mut changed = apply_settings(&mut t.settings, kind);
            match kind {
                ApplyKind::Credentials => changed |= t.credential.take().is_some(),
                ApplyKind::Clipboard => {
                    if let Some(c) = clipboard_of(&mut t.options) {
                        changed |= *c != clip;
                        *c = clip;
                    }
                }
                _ => {}
            }
            changed
        }
    }
}

fn apply_walk(f: &mut Folder, kind: ApplyKind, clip: ClipboardDirection) -> usize {
    let mut n = 0;
    for c in &mut f.children {
        if apply_one(c, kind, clip) {
            n += 1;
        }
        if let Node::Folder(sub) = c {
            n += apply_walk(sub, kind, clip);
        }
    }
    n
}

/// Zwischenablage-Richtung, die an einem Ordner wirkt (nach oben gesucht).
pub fn folder_clipboard(index: &Index, folder_id: Uuid) -> ClipboardDirection {
    let mut chain: Vec<&Settings> = Vec::new();
    if folder_id != index.root.id {
        if let Some(path) = index.path_to(folder_id) {
            for n in path.iter().rev() {
                if let Node::Folder(f) = n {
                    chain.push(&f.settings);
                }
            }
        }
    }
    chain.push(&index.root.settings);
    chain.iter().find_map(|s| s.clipboard).unwrap_or_default()
}

/// Wie viele Unterobjekte „Auf Unterobjekte anwenden“ ändern würde.
pub fn apply_preview(index: &Index, folder_id: Uuid, kind: ApplyKind) -> Result<usize, EditError> {
    let mut copy = index.clone();
    apply_to_children(&mut copy, folder_id, kind)
}

/// Lässt alle Unterobjekte eines Ordners die Einstellung des Ordners
/// übernehmen: eigene Angaben darunter werden entfernt (Vererbung), bei der
/// Zwischenablage wird die Richtung des Ordners an jedem Ziel gesetzt.
pub fn apply_to_children(index: &mut Index, folder_id: Uuid, kind: ApplyKind) -> Result<usize, EditError> {
    let clip = folder_clipboard(index, folder_id);
    let f = folder_mut(index, folder_id).ok_or(EditError::NotAFolder)?;
    Ok(apply_walk(f, kind, clip))
}

/// Eingebaute Vorgaben für ein neues Ziel.
pub fn default_options(protocol: Protocol) -> ProtocolOptions {
    match protocol {
        Protocol::Rdp | Protocol::Hyperv => ProtocolOptions::Rdp(RdpOptions::default()),
        Protocol::Ssh => ProtocolOptions::Ssh(serde_json::from_str("{}").expect("leere Optionen sind gültig")),
        Protocol::Vnc => ProtocolOptions::Vnc(VncOptions::default()),
        Protocol::Web | Protocol::Proxmox => ProtocolOptions::Web(WebOptions::default()),
        Protocol::Psrp => ProtocolOptions::None,
    }
}

/// Standardvorlage eines Protokolls.
pub fn default_template(index: &Index, protocol: Protocol) -> Option<&Template> {
    index.templates.iter().find(|t| t.protocol == protocol && t.is_default)
}

/// Neues Ziel mit Vorbelegung aus Vorlage (oder Standardvorlage) und der
/// Zwischenablage-Vorgabe des Ordners.
pub fn new_target(index: &Index, parent: Uuid, protocol: Protocol, name: &str, template: Option<&Template>) -> Target {
    let mut t = Target::new(name, protocol, "");
    let tpl = template.or_else(|| default_template(index, protocol));
    t.options = default_options(protocol);
    if let Some(tpl) = tpl {
        t.port = tpl.port;
        if !tpl.options.is_none() {
            t.options = tpl.options.clone();
        }
        t.settings.tunnel = tpl.tunnel;
        match &tpl.credential {
            TemplateCredential::Inherit => {}
            TemplateCredential::None => t.settings.credentials_off = true,
            TemplateCredential::Named(n) => t.credential = Some(n.clone()),
        }
    }
    if tpl.is_none() {
        let clip = folder_clipboard(index, parent);
        if let Some(c) = clipboard_of(&mut t.options) {
            *c = clip;
        }
    }
    t
}

/// Vorlage aus einem Ziel. Übernommen werden Port, Optionen, Tunnel und die
/// Art der Zugangsdaten – nie ein Verweis auf einen KDBX-Eintrag.
pub fn template_from_target(t: &Target, name: &str) -> Template {
    let credential = if t.settings.credentials_off {
        TemplateCredential::None
    } else if let Some(n) = &t.credential {
        TemplateCredential::Named(n.clone())
    } else {
        TemplateCredential::Inherit
    };
    Template {
        id: Uuid::new_v4(),
        name: name.to_string(),
        protocol: t.protocol,
        is_default: false,
        port: t.port,
        options: t.options.clone(),
        tunnel: t.settings.tunnel,
        credential,
    }
}

/// Speichert eine Vorlage; ist sie Standard, verliert eine bisherige
/// Standardvorlage desselben Protokolls diese Eigenschaft.
pub fn upsert_template(index: &mut Index, tpl: Template) {
    if tpl.is_default {
        for t in &mut index.templates {
            if t.protocol == tpl.protocol && t.id != tpl.id {
                t.is_default = false;
            }
        }
    }
    match index.templates.iter_mut().find(|t| t.id == tpl.id) {
        Some(t) => *t = tpl,
        None => index.templates.push(tpl),
    }
}

/// Verlauf zum Rückgängigmachen: Zustand des Index vor jeder Aktion.
#[derive(Debug, Default, Clone)]
pub struct History {
    stack: Vec<(String, Index)>,
}

impl History {
    const MAX: usize = 50;

    /// Merkt den Zustand vor einer Aktion.
    pub fn record(&mut self, label: &str, before: &Index) {
        self.stack.push((label.to_string(), before.clone()));
        if self.stack.len() > Self::MAX {
            self.stack.remove(0);
        }
    }

    /// Bezeichnung der letzten Aktion.
    pub fn last_label(&self) -> Option<&str> {
        self.stack.last().map(|(l, _)| l.as_str())
    }

    /// Stellt den Zustand vor der letzten Aktion her. Die Revision bleibt die
    /// des aktuellen Index, damit die Konflikterkennung beim Speichern stimmt.
    pub fn undo(&mut self, current: &mut Index) -> Option<String> {
        let (label, mut before) = self.stack.pop()?;
        before.revision = current.revision;
        *current = before;
        Some(label)
    }

    pub fn clear(&mut self) {
        self.stack.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inherit::{resolve_credential, resolve_credential_at, resolve_tunnel, set_credential_mode, CredentialMode, CredentialResolution, Origin};

    fn cref() -> CredentialRef {
        CredentialRef { entry: Uuid::new_v4(), last_title: Some("Konto".into()) }
    }

    /// Wurzel → A (mit Ziel a1, Unterordner B mit Ziel b1), C.
    fn sample() -> (Index, Uuid, Uuid, Uuid, Uuid, Uuid) {
        let mut idx = Index::default();
        let mut a = Folder::new("A");
        let mut b = Folder::new("B");
        let a1 = Target::new("a1", Protocol::Rdp, "a1.example.test");
        let mut b1 = Target::new("b1", Protocol::Ssh, "b1.example.test");
        b1.settings.tunnel = Some(TunnelSetting::None);
        b1.settings.credentials.insert("default".into(), cref());
        let c = Folder::new("C");
        let ids = (a.id, b.id, a1.id, b1.id, c.id);
        b.children.push(Node::Target(b1));
        a.children.push(Node::Target(a1));
        a.children.push(Node::Folder(b));
        idx.root.children.push(Node::Folder(a));
        idx.root.children.push(Node::Folder(c));
        (idx, ids.0, ids.1, ids.2, ids.3, ids.4)
    }

    fn add_login(idx: &mut Index, name: &str, folder: Uuid) -> Uuid {
        let l = crate::model::Login { id: Uuid::new_v4(), name: name.into(), folder, entry: cref() };
        let id = l.id;
        idx.logins.push(l);
        id
    }

    fn login_names(idx: &Index, folder: Uuid) -> Vec<String> {
        idx.logins.iter().filter(|l| l.folder == folder).map(|l| l.name.clone()).collect()
    }

    #[test]
    fn logins_move_and_reorder_like_tree_rows() {
        // A hat zwei Zeilen (a1, B), danach die Anmeldeobjekte x, y, z; dazwischen eines in C.
        let (mut idx, a, _b, a1, _b1, c) = sample();
        let x = add_login(&mut idx, "x", a);
        let w = add_login(&mut idx, "w", c);
        let y = add_login(&mut idx, "y", a);
        let z = add_login(&mut idx, "z", a);
        // Nach oben und unten innerhalb des Ordners, Ränder bleiben stehen.
        reorder(&mut idx, z, -1).unwrap();
        assert_eq!(login_names(&idx, a), ["x", "z", "y"]);
        reorder(&mut idx, x, -1).unwrap();
        assert_eq!(login_names(&idx, a), ["x", "z", "y"]);
        reorder(&mut idx, x, 2).unwrap();
        assert_eq!(login_names(&idx, a), ["z", "y", "x"]);
        // Ziehen im selben Ordner: über der ersten Anmeldezeile (Zeile 2) und unter der letzten (Zeile 5).
        move_node(&mut idx, x, a, Some(2)).unwrap();
        assert_eq!(login_names(&idx, a), ["x", "z", "y"]);
        move_node(&mut idx, x, a, Some(5)).unwrap();
        assert_eq!(login_names(&idx, a), ["z", "y", "x"]);
        // Zwischen Ziele gezogen: landet als erstes Anmeldeobjekt.
        move_node(&mut idx, y, a, Some(0)).unwrap();
        assert_eq!(login_names(&idx, a), ["y", "z", "x"]);
        // In einen anderen Ordner, an eine Stelle und ans Ende.
        move_node(&mut idx, z, c, Some(0)).unwrap();
        assert_eq!(login_names(&idx, c), ["z", "w"]);
        move_node(&mut idx, y, c, None).unwrap();
        assert_eq!(login_names(&idx, c), ["z", "w", "y"]);
        assert_eq!(login_names(&idx, a), ["x"]);
        // In die Wurzel; Ziel kein Ordner.
        let root = idx.root.id;
        move_node(&mut idx, w, root, None).unwrap();
        assert_eq!(idx.login(w).unwrap().folder, root);
        assert_eq!(move_node(&mut idx, x, a1, None), Err(EditError::NotAFolder));
        assert_eq!(idx.logins.len(), 4, "nichts verloren, nichts doppelt");
    }

    #[test]
    fn move_rejects_cycles_and_keeps_order() {
        let (mut idx, a, b, a1, _b1, c) = sample();
        assert_eq!(move_node(&mut idx, a, b, None), Err(EditError::Cycle));
        assert_eq!(move_node(&mut idx, a, a, None), Err(EditError::Cycle));
        move_node(&mut idx, a1, c, None).unwrap();
        assert_eq!(parent_of(&idx, a1), Some(c));
        // Innerhalb eines Ordners nach hinten.
        let t2 = insert(&mut idx, c, Node::Target(Target::new("x", Protocol::Ssh, "x.example.test")), None).unwrap();
        move_node(&mut idx, a1, c, Some(2)).unwrap();
        let names: Vec<_> = folder(&idx, c).unwrap().children.iter().map(|n| n.id()).collect();
        assert_eq!(names, vec![t2, a1]);
        reorder(&mut idx, a1, -1).unwrap();
        assert_eq!(folder(&idx, c).unwrap().children[0].id(), a1);
    }

    #[test]
    fn duplicate_renews_ids_and_keeps_references() {
        let (mut idx, a, b, _a1, b1, _c) = sample();
        let copy = duplicate(&mut idx, b, " (Kopie)").unwrap();
        assert_ne!(copy, b);
        let Some(Node::Folder(f)) = node(&idx, copy) else { panic!() };
        assert_eq!(f.name, "B (Kopie)");
        let Node::Target(t) = &f.children[0] else { panic!() };
        assert_ne!(t.id, b1);
        let Some(Node::Target(orig)) = node(&idx, b1) else { panic!() };
        assert_eq!(t.settings, orig.settings, "gleiche Verweise");
        // Kopie steht direkt hinter dem Original.
        let kids: Vec<_> = folder(&idx, a).unwrap().children.iter().map(|n| n.id()).collect();
        assert_eq!(kids[1..], [b, copy]);
    }

    #[test]
    fn delete_cleans_views_and_favorites() {
        let (mut idx, a, _b, a1, b1, _c) = sample();
        let v = create_view(&mut idx, "Patchday").unwrap();
        assert_eq!(add_to_view(&mut idx, v, &[a]).unwrap(), 2);
        assert_eq!(add_to_view(&mut idx, v, &[a1]).unwrap(), 0, "keine Doppelten");
        set_favorite(&mut idx, b1, true).unwrap();
        rename_view(&mut idx, v, "Patchday Juni").unwrap();
        delete(&mut idx, a).unwrap();
        assert!(idx.views[0].targets.is_empty());
        assert!(idx.favorites.is_empty());
        assert_eq!(idx.views[0].name, "Patchday Juni");
        let root = idx.root.id;
        assert_eq!(delete(&mut idx, root), Err(EditError::Root));
    }

    #[test]
    fn target_in_several_views() {
        let (mut idx, _a, _b, a1, _b1, _c) = sample();
        let v1 = create_view(&mut idx, "Patchday").unwrap();
        let v2 = create_view(&mut idx, "Kunde").unwrap();
        add_to_view(&mut idx, v1, &[a1]).unwrap();
        add_to_view(&mut idx, v2, &[a1]).unwrap();
        assert!(idx.views.iter().all(|v| v.targets == vec![a1]));
        remove_from_view(&mut idx, v1, &[a1]).unwrap();
        assert!(idx.views[0].targets.is_empty() && idx.views[1].targets == vec![a1]);
    }

    #[test]
    fn apply_to_children_preview_and_undo() {
        let (mut idx, a, b, _a1, b1, _c) = sample();
        folder_mut(&mut idx, a).unwrap().settings.tunnel = Some(TunnelSetting::Use(Uuid::new_v4()));
        folder_mut(&mut idx, b).unwrap().settings.tunnel = Some(TunnelSetting::None);
        assert_eq!(apply_preview(&idx, a, ApplyKind::Tunnel).unwrap(), 2, "Ordner B und Ziel b1");
        assert!(resolve_tunnel(&idx, b1).unwrap().is_none());
        let mut h = History::default();
        let before = idx.clone();
        h.record("Auf Unterobjekte anwenden", &idx);
        assert_eq!(apply_to_children(&mut idx, a, ApplyKind::Tunnel).unwrap(), 2);
        assert_eq!(resolve_tunnel(&idx, b1).unwrap().map(|x| x.1), Some(Origin::Folder(a)));
        assert_eq!(apply_preview(&idx, a, ApplyKind::Tunnel).unwrap(), 0);
        assert_eq!(h.undo(&mut idx).as_deref(), Some("Auf Unterobjekte anwenden"));
        assert_eq!(idx, before);
        assert!(h.undo(&mut idx).is_none());
    }

    #[test]
    fn apply_credentials_and_clipboard() {
        let (mut idx, a, _b, a1, b1, _c) = sample();
        let r = cref();
        folder_mut(&mut idx, a).unwrap().settings.credentials.insert("default".into(), r.clone());
        assert_eq!(apply_to_children(&mut idx, a, ApplyKind::Credentials).unwrap(), 1);
        let CredentialResolution::Found(found) = resolve_credential(&idx, b1).unwrap() else { panic!() };
        assert_eq!(found.reference, r);
        assert_eq!(found.origin, Origin::Folder(a));
        folder_mut(&mut idx, a).unwrap().settings.clipboard = Some(ClipboardDirection::Off);
        let Some(Node::Folder(_)) = node(&idx, a) else { panic!() };
        if let Some(Node::Target(t)) = find_node_mut(&mut idx.root, a1) {
            t.options = ProtocolOptions::Rdp(RdpOptions::default());
        }
        assert_eq!(apply_to_children(&mut idx, a, ApplyKind::Clipboard).unwrap(), 1, "nur das RDP-Ziel hat eine Richtung");
        let Some(Node::Target(t)) = node(&idx, a1) else { panic!() };
        assert!(matches!(&t.options, ProtocolOptions::Rdp(o) if o.clipboard == ClipboardDirection::Off));
    }

    #[test]
    fn credential_modes_and_origin() {
        let (mut idx, a, b, a1, b1, _c) = sample();
        let root_ref = cref();
        idx.root.settings.credentials.insert("default".into(), root_ref.clone());
        let admin = cref();
        folder_mut(&mut idx, a).unwrap().settings.credentials.insert("admin".into(), admin.clone());
        // (2) geerbt
        let CredentialResolution::Found(r) = resolve_credential(&idx, a1).unwrap() else { panic!() };
        assert_eq!((r.reference, r.origin), (root_ref.clone(), Origin::Root));
        // (5) nach Name am Ordner B → gilt für b1 ohne eigene Angabe
        set_credential_mode(&mut idx, b1, CredentialMode::Inherit);
        set_credential_mode(&mut idx, b, CredentialMode::Named("admin".into()));
        let CredentialResolution::Found(r) = resolve_credential(&idx, b1).unwrap() else { panic!() };
        assert_eq!((r.name.as_str(), r.origin), ("admin", Origin::Folder(a)));
        // (1) keine am Ordner B
        set_credential_mode(&mut idx, b, CredentialMode::None);
        assert_eq!(resolve_credential(&idx, b1).unwrap(), CredentialResolution::Disabled);
        assert_eq!(crate::inherit::credential_mode(&idx, b), Some(CredentialMode::None));
        // (4) vorhandener Datensatz am Ziel hebt die Abschaltung darüber auf
        let own = cref();
        set_credential_mode(&mut idx, b1, CredentialMode::Entry { name: "default".into(), reference: own.clone() });
        let CredentialResolution::Found(r) = resolve_credential(&idx, b1).unwrap() else { panic!() };
        assert_eq!((r.reference, r.origin), (own, Origin::Target));
        // (5) am Ziel
        set_credential_mode(&mut idx, a1, CredentialMode::Named("admin".into()));
        let Some(Node::Target(t)) = node(&idx, a1) else { panic!() };
        assert_eq!(t.credential.as_deref(), Some("admin"));
        assert!(matches!(resolve_credential_at(&idx, a).unwrap(), CredentialResolution::Found(ref r) if r.origin == Origin::Root));
        // zurück auf Vererbung
        set_credential_mode(&mut idx, a1, CredentialMode::Inherit);
        let Some(Node::Target(t)) = node(&idx, a1) else { panic!() };
        assert!(t.credential.is_none() && t.settings.is_empty());
    }

    #[test]
    fn templates_never_contain_entries_or_secrets() {
        let (mut idx, a, _b, _a1, b1, _c) = sample();
        let Some(Node::Target(t)) = node(&idx, b1).cloned() else { panic!() };
        let tpl = template_from_target(&t, "Linux");
        let json = serde_json::to_string(&tpl).unwrap();
        let entry = t.settings.credentials["default"].entry.to_string();
        assert!(!json.contains(&entry), "{json}");
        assert!(!json.to_lowercase().contains("password") && !json.contains("Konto"), "{json}");
        let mut tpl = tpl;
        tpl.is_default = true;
        upsert_template(&mut idx, tpl.clone());
        let mut other = template_from_target(&t, "Linux 2");
        other.is_default = true;
        upsert_template(&mut idx, other);
        assert_eq!(idx.templates.iter().filter(|t| t.is_default).count(), 1);
        let n = new_target(&idx, a, Protocol::Ssh, "neu", Some(&tpl));
        assert_eq!(n.settings.tunnel, Some(TunnelSetting::None));
        assert!(n.settings.credentials.is_empty());
    }

    #[test]
    fn new_target_takes_folder_clipboard() {
        let (mut idx, a, _b, _a1, _b1, _c) = sample();
        folder_mut(&mut idx, a).unwrap().settings.clipboard = Some(ClipboardDirection::TargetToHost);
        let t = new_target(&idx, a, Protocol::Rdp, "neu", None);
        assert!(matches!(t.options, ProtocolOptions::Rdp(ref o) if o.clipboard == ClipboardDirection::TargetToHost));
        assert_eq!(folder_list(&idx).len(), 4);
    }
}
