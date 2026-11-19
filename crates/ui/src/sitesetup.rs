//! Ordner eines Moduls anlegen (mit Unterordnern für Netzwerk- und Serverkomponenten und Standard-Anmeldeobjekten) und der
//! Vorlagenordner `Templates` im Root, darin `Loginobjects` mit je einer Vorlage je Art (`SITE_LOGIN_KINDS`).
//! Aus einer Vorlage übernimmt ein neues Anmeldeobjekt Benutzername und Kennwort.

use rj_core::edit;
use rj_core::{CredentialRef, Folder, FolderRole, Index, Login, Node, SITE_LOGIN_KINDS};
use rj_kdbx::CredentialData;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app::App;

// Anlegen (Modul).
#[cfg(feature = "standorte")]
#[path = "modules/standorte/sitecreate.rs"]
mod sitecreate;

pub const TEMPLATES_NAME: &str = "Templates";
pub const LOGIN_TEMPLATES_NAME: &str = "Loginobjects";
pub const NETWORK_NAME: &str = "Netzwerkkomponenten";
pub const SERVERS_NAME: &str = "Serverkomponenten";

/// Vorlagenordner im Root immer ganz oben (nach jeder Änderung aufgerufen).
pub fn pin_templates(index: &mut Index) {
    let c = &mut index.root.children;
    let first = c.iter().position(|n| matches!(n, Node::Folder(f) if f.role == Some(FolderRole::Templates)));
    if let Some(i) = first.filter(|i| *i > 0) {
        let n = c.remove(i);
        c.insert(0, n);
    }
}


fn child_with_role(f: &Folder, role: FolderRole) -> Option<Uuid> {
    f.children.iter().find_map(|n| match n {
        Node::Folder(sub) if sub.role == Some(role) => Some(sub.id),
        _ => None,
    })
}

impl App {



    /// Vorlagenordner anlegen bzw. ergänzen: `Templates` (Root, oben), darin `Loginobjects` mit je einem leeren
    /// Anmeldeobjekt je Art. Vorhandenes bleibt unverändert.
    pub fn templates_folder_create(&mut self) -> Result<Value, String> {
        if self.vault.is_none() {
            return Err("Die Schlüsseldatenbank ist gesperrt".into());
        }
        let before = self.index.clone();
        let root = self.index.root.id;
        let existing = self.index.root.children.iter().find_map(|n| match n {
            Node::Folder(f) if f.role == Some(FolderRole::Templates) => Some(f.id),
            _ => None,
        });
        let tid = match existing {
            Some(id) => id,
            None => {
                let mut f = Folder::new(TEMPLATES_NAME);
                f.role = Some(FolderRole::Templates);
                edit::insert(&mut self.index, root, Node::Folder(f), Some(0)).map_err(|e| e.to_string())?
            }
        };
        let lid = match edit::folder(&self.index, tid).and_then(|t| child_with_role(t, FolderRole::LoginTemplates)) {
            Some(id) => id,
            None => {
                let mut f = Folder::new(LOGIN_TEMPLATES_NAME);
                f.role = Some(FolderRole::LoginTemplates);
                edit::insert(&mut self.index, tid, Node::Folder(f), Some(0)).map_err(|e| e.to_string())?
            }
        };
        let mut created = 0;
        for (_, label, _) in SITE_LOGIN_KINDS {
            if self.index.logins.iter().any(|l| l.folder == lid && l.name == label) {
                continue;
            }
            let title = format!("Anmeldung: {label}");
            let vault = self.vault.as_mut().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
            match vault.add_credential(CredentialData { title: title.clone(), ..Default::default() }) {
                Ok(entry) => {
                    self.index.logins.push(Login { id: Uuid::new_v4(), name: label.to_string(), folder: lid, entry: CredentialRef { entry, last_title: Some(title) } });
                    created += 1;
                }
                Err(e) => {
                    self.index = before;
                    return Err(e.to_string());
                }
            }
        }
        pin_templates(&mut self.index);
        self.commit("Vorlagenordner anlegen", before);
        Ok(json!({ "id": tid, "logins_folder": lid, "created": created }))
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use rj_kdbx::{KdfChoice, MasterKey, Vault};

    pub(crate) fn app(dir: &std::path::Path) -> App {
        let path = dir.join("t.kdbx");
        let mut v = Vault::create(&path, MasterKey::password("Prüf"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
        v.save(None).unwrap();
        let mut app = App::new(false);
        app.config_path = None;
        app.index = v.index().index.clone();
        app.vault = Some(v);
        app
    }

    pub(crate) fn cmd(app: &mut App, v: Value) -> Value {
        serde_json::from_str(&app.edit(&v.to_string())).unwrap()
    }

    #[test]
    fn templates_folder_is_created_on_top_once_with_one_login_per_kind() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        cmd(&mut app, json!({ "op": "add", "kind": "folder", "name": "A" }));
        let r = cmd(&mut app, json!({ "op": "templates_folder_create" }));
        assert_eq!(r["created"], 5, "{r}");
        let Node::Folder(t) = &app.index.root.children[0] else { panic!("kein Ordner") };
        assert_eq!((t.name.as_str(), t.role), (TEMPLATES_NAME, Some(FolderRole::Templates)));
        let lid = r["logins_folder"].as_str().unwrap();
        let names: Vec<_> = app.index.logins.iter().filter(|l| l.folder.to_string() == lid).map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["RDP", "SSH", "AOS-CX", "Legacy-Switch", "WiFi"]);
        // Zweiter Aufruf ergänzt nichts; ein neuer Ordner im Root bleibt unter Templates.
        assert_eq!(cmd(&mut app, json!({ "op": "templates_folder_create" }))["created"], 0);
        let b = cmd(&mut app, json!({ "op": "add", "kind": "folder", "name": "B" }))["id"].as_str().unwrap().to_string();
        let root = app.index.root.id;
        cmd(&mut app, json!({ "op": "move", "id": b, "parent": root, "pos": 0 }));
        assert!(matches!(&app.index.root.children[0], Node::Folder(f) if f.role == Some(FolderRole::Templates)));
    }
}
