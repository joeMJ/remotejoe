//! Direkte Verweise auf KDBX-Einträge: am Knoten eingetragene Zugangsdatensätze und eigene Zugangsdaten an
//! Zugangswegen. Verweist mehr als ein solcher Verweis auf denselben Eintrag, ist er geteilt – so entsteht es z. B.
//! beim Duplizieren. Gewolltes Teilen über Anmeldeobjekte oder über das Erben vom Ordner zählt nicht.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::model::*;

/// Wo ein Eintrag direkt verwendet wird.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Use {
    /// Zugangsdatensatz `name` in den Einstellungen des Knotens (Ziel oder Ordner).
    Settings { node: Uuid, name: String },
    /// Eigene Zugangsdaten des Wegs `way` am Ziel `node`.
    Way { node: Uuid, way: Uuid },
}

impl Use {
    pub fn node(&self) -> Uuid {
        match self {
            Use::Settings { node, .. } | Use::Way { node, .. } => *node,
        }
    }
}

fn walk(f: &Folder, out: &mut BTreeMap<Uuid, Vec<Use>>) {
    settings(f.id, &f.settings, out);
    for c in &f.children {
        match c {
            Node::Folder(sub) => walk(sub, out),
            Node::Target(t) => {
                settings(t.id, &t.settings, out);
                for w in &t.ways {
                    if let Some(r) = &w.own {
                        out.entry(r.entry).or_default().push(Use::Way { node: t.id, way: w.id });
                    }
                }
            }
        }
    }
}

fn settings(node: Uuid, s: &Settings, out: &mut BTreeMap<Uuid, Vec<Use>>) {
    for (name, r) in &s.credentials {
        // Mit Anmeldeobjekt steht dessen Eintrag zusätzlich unter dem Standardnamen: das ist kein direkter Verweis.
        if s.login.is_some() && name == DEFAULT_CREDENTIAL {
            continue;
        }
        out.entry(r.entry).or_default().push(Use::Settings { node, name: name.clone() });
    }
}

/// Alle direkten Verweise je KDBX-Eintrag.
pub fn direct_uses(index: &Index) -> BTreeMap<Uuid, Vec<Use>> {
    let mut out = BTreeMap::new();
    walk(&index.root, &mut out);
    out
}

/// Einträge mit mehr als einem direkten Verweis.
pub fn shared(index: &Index) -> Vec<(Uuid, Vec<Use>)> {
    direct_uses(index).into_iter().filter(|(_, u)| u.len() > 1).collect()
}

/// Ist `entry` mehr als einmal direkt eingetragen?
pub fn is_shared(index: &Index, entry: Uuid) -> bool {
    direct_uses(index).get(&entry).is_some_and(|u| u.len() > 1)
}

/// Verweis an einer Stelle auf einen anderen Eintrag umstellen.
pub fn replace(index: &mut Index, u: &Use, reference: CredentialRef) -> bool {
    match u {
        Use::Settings { node, name } => match crate::inherit::settings_mut(index, *node) {
            Some(s) => {
                s.credentials.insert(name.clone(), reference);
                true
            }
            None => false,
        },
        Use::Way { node, way } => match crate::inherit::find_node_mut(&mut index.root, *node) {
            Some(Node::Target(t)) => match t.ways.iter_mut().find(|w| w.id == *way) {
                Some(w) => {
                    w.own = Some(reference);
                    true
                }
                None => false,
            },
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit;

    fn cred(entry: Uuid) -> Settings {
        let mut s = Settings::default();
        s.credentials.insert(DEFAULT_CREDENTIAL.into(), CredentialRef { entry, last_title: None });
        s
    }

    #[test]
    fn duplicate_shares_and_logins_do_not_count() {
        let mut idx = Index::default();
        let root = idx.root.id;
        let (e1, e2) = (Uuid::new_v4(), Uuid::new_v4());
        let mut a = Target::new("a", Protocol::Ssh, "192.0.2.1");
        a.settings = cred(e1);
        let a = edit::insert(&mut idx, root, Node::Target(a), None).unwrap();
        let b = edit::duplicate(&mut idx, a, " (Kopie)").unwrap();
        // Anmeldeobjekt: Eintrag steht unter dem Standardnamen, zählt aber nicht.
        let l = Login { id: Uuid::new_v4(), name: "L".into(), folder: root, entry: CredentialRef { entry: e2, last_title: None } };
        idx.logins.push(l.clone());
        for name in ["c", "d"] {
            let mut t = Target::new(name, Protocol::Ssh, "192.0.2.2");
            t.settings = cred(e2);
            t.settings.login = Some(l.id);
            edit::insert(&mut idx, root, Node::Target(t), None).unwrap();
        }
        let s = shared(&idx);
        assert_eq!(s.len(), 1, "{s:?}");
        assert_eq!(s[0].0, e1);
        assert_eq!(s[0].1.iter().map(Use::node).collect::<Vec<_>>(), vec![a, b]);
        assert!(is_shared(&idx, e1) && !is_shared(&idx, e2));
        let e3 = Uuid::new_v4();
        assert!(replace(&mut idx, &s[0].1[1], CredentialRef { entry: e3, last_title: None }));
        assert!(shared(&idx).is_empty());
    }
}
