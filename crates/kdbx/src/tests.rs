use super::*;
use rj_core::inherit::{resolve_credential, CredentialResolution};
use rj_core::{Folder, Node, Protocol, Target};

fn data(title: &str, user: &str, pw: &str) -> CredentialData {
    CredentialData {
        title: title.into(),
        username: user.into(),
        password: Secret::from_str_exact(pw),
        ..Default::default()
    }
}

fn new_vault(dir: &Path, storage: IndexStorage) -> (Vault, PathBuf, MasterKey) {
    let path = dir.join("tresor.kdbx");
    let key = MasterKey::password("Prüf-Kennwort ✓");
    (Vault::create(&path, key.clone(), KdfChoice::Weak, storage), path, key)
}

/// Index mit einem Ordner und einem Ziel, das auf `entry` verweist.
fn index_with(entry: Uuid, title: &str) -> (Index, Uuid) {
    let mut idx = Index::default();
    let mut f = Folder::new("Server");
    f.settings
        .credentials
        .insert("default".into(), CredentialRef { entry, last_title: Some(title.into()) });
    let t = Target::new("web01", Protocol::Ssh, "web01.example.test");
    let tid = t.id;
    f.children.push(Node::Target(t));
    idx.root.children.push(Node::Folder(f));
    (idx, tid)
}

#[test]
fn roundtrip_entries_and_index() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, path, key) = new_vault(dir.path(), IndexStorage::KdbxCustomData);
    let nfd = "Kennwo\u{0308}rt\t ";
    let id = v.add_credential(data("Server-Admin", "EXAMPLE\\admin", nfd)).unwrap();
    v.set_attachment(id, "id_ed25519", Secret::from_bytes(b"schluessel".to_vec())).unwrap();
    let (idx, tid) = index_with(id, "Server-Admin");
    v.set_index(idx.clone()).unwrap();
    assert!(v.save(None).unwrap());
    assert!(!v.save(None).unwrap(), "ohne Änderung wird nicht geschrieben");

    let v2 = Vault::open(&path, key, IndexStorage::KdbxCustomData).unwrap();
    assert_eq!(v2.index().index.root, idx.root);
    assert_eq!(v2.index().index.revision, 1);
    let c = v2.credential(id).unwrap();
    assert_eq!(c.data.password.as_bytes(), nfd.as_bytes(), "Kennwort byte-genau");
    assert_eq!(c.split_domain(), (Some("EXAMPLE"), "admin"));
    assert_eq!(c.group_path, vec![GROUP_NAME.to_string()]);
    assert_eq!(v2.attachment(id, "id_ed25519").unwrap().as_bytes(), b"schluessel");
    let CredentialResolution::Found(r) = resolve_credential(&v2.index().index, tid).unwrap() else { panic!() };
    assert_eq!(v2.resolve(&r.reference).unwrap().data.title, "Server-Admin");
}

#[test]
fn key_file_and_password() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("k.kdbx");
    let key = MasterKey::password("pw").with_key_file_bytes(b"beliebiger Inhalt".to_vec());
    let mut v = Vault::create(&path, key.clone(), KdfChoice::Aes(1000), IndexStorage::KdbxCustomData);
    v.save(None).unwrap();
    assert!(Vault::open(&path, MasterKey::password("pw"), IndexStorage::KdbxCustomData).is_err());
    Vault::open(&path, key, IndexStorage::KdbxCustomData).unwrap();
}

#[test]
fn backup_written() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, path, _) = new_vault(dir.path(), IndexStorage::KdbxCustomData);
    v.save(None).unwrap();
    v.add_credential(data("a", "b", "c")).unwrap();
    v.save(None).unwrap();
    assert!(rj_core::fsutil::backup_path(&path).exists());
}

#[test]
fn foreign_entries_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, _, _) = new_vault(dir.path(), IndexStorage::KdbxCustomData);
    let foreign = {
        let mut root = v.db.root_mut();
        let mut e = root.add_entry();
        e.set_unprotected(fields::TITLE, "Fremd");
        e.id().uuid()
    };
    assert!(matches!(v.update_credential(foreign, data("x", "y", "z")), Err(VaultError::ForeignEntry(_))));
    assert_eq!(v.credential(foreign).unwrap().data.title, "Fremd");
    assert!(matches!(
        v.update_credential(Uuid::new_v4(), data("x", "y", "z")),
        Err(VaultError::NoSuchEntry(_))
    ));
}

#[test]
fn invalid_utf8_password_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, _, _) = new_vault(dir.path(), IndexStorage::KdbxCustomData);
    let mut d = data("x", "y", "");
    d.password = Secret::from_bytes(vec![0xFF]);
    assert!(matches!(v.add_credential(d), Err(VaultError::PasswordEncoding)));
}

#[test]
fn missing_and_unassigned() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, _, _) = new_vault(dir.path(), IndexStorage::KdbxCustomData);
    let used = v.add_credential(data("Benutzt", "u", "p")).unwrap();
    let spare = v.add_credential(data("Übrig", "u", "p")).unwrap();
    let gone = Uuid::new_v4();
    let (mut idx, _) = index_with(used, "Benutzt");
    idx.root
        .settings
        .credentials
        .insert("alt".into(), CredentialRef { entry: gone, last_title: Some("Altes Konto".into()) });
    v.set_index(idx).unwrap();
    let missing = v.missing_references();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].last_title.as_deref(), Some("Altes Konto"));
    assert!(missing[0].to_string().contains("„Altes Konto“"));
    let un: Vec<_> = v.unassigned().into_iter().map(|e| e.uuid).collect();
    assert_eq!(un, vec![spare]);
}

#[test]
fn external_change_is_merged() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, path, key) = new_vault(dir.path(), IndexStorage::KdbxCustomData);
    let e1 = a.add_credential(data("Eins", "u", "p")).unwrap();
    a.set_index(index_with(e1, "Eins").0).unwrap();
    a.save(None).unwrap();

    // Ein zweiter Vorgang ändert nur Einträge.
    let mut b = Vault::open(&path, key.clone(), IndexStorage::KdbxCustomData).unwrap();
    let e2 = b.add_credential(data("Zwei", "u", "p")).unwrap();
    b.save(None).unwrap();

    // A ändert den Index und speichert: B's Eintrag bleibt erhalten.
    assert!(a.changed_on_disk().unwrap());
    let mut idx = a.index().index.clone();
    idx.root.name = "Neu".into();
    a.set_index(idx).unwrap();
    let e3 = a.add_credential(data("Drei", "u", "p")).unwrap();
    a.save(None).unwrap();

    let c = Vault::open(&path, key, IndexStorage::KdbxCustomData).unwrap();
    assert_eq!(c.index().index.root.name, "Neu");
    for e in [e1, e2, e3] {
        assert!(c.credential(e).is_some());
    }
}

#[test]
fn index_conflict_requires_choice() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, path, key) = new_vault(dir.path(), IndexStorage::KdbxCustomData);
    a.save(None).unwrap();
    let mut b = Vault::open(&path, key.clone(), IndexStorage::KdbxCustomData).unwrap();
    let mut ib = b.index().index.clone();
    ib.root.name = "Fremd".into();
    b.set_index(ib).unwrap();
    b.save(None).unwrap();

    let mut ia = a.index().index.clone();
    ia.root.name = "Eigen".into();
    a.set_index(ia).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(matches!(a.save(None), Err(VaultError::Conflict { .. })));
    assert_eq!(fs::read(&path).unwrap(), before, "bei Konflikt wird nichts geschrieben");

    a.save(Some(ConflictChoice::KeepBoth)).unwrap();
    let c = Vault::open(&path, key.clone(), IndexStorage::KdbxCustomData).unwrap();
    assert_eq!(c.index().index.root.name, "Eigen");
    assert!(index_store::read_backup(c.database(), "theirs").unwrap().contains("Fremd"));
    assert!(c.index().index.revision > 1);

    // Fremde Version laden.
    let mut d = Vault::open(&path, key.clone(), IndexStorage::KdbxCustomData).unwrap();
    let mut x = c.index().index.clone();
    x.root.name = "Dritte".into();
    let mut e = Vault::open(&path, key.clone(), IndexStorage::KdbxCustomData).unwrap();
    e.set_index(x).unwrap();
    e.save(None).unwrap();
    let mut y = d.index().index.clone();
    y.root.name = "Vierte".into();
    d.set_index(y).unwrap();
    d.save(Some(ConflictChoice::LoadTheirs)).unwrap();
    assert_eq!(d.index().index.root.name, "Dritte");
}

#[test]
fn read_only_index_is_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, path, key) = new_vault(dir.path(), IndexStorage::KdbxCustomData);
    let mut val = serde_json::to_value(Index::default()).unwrap();
    val["schema_version"] = 99.into();
    index_store::write(&mut v.db, &serde_json::to_vec(&val).unwrap()).unwrap();
    v.index_dirty = false;
    v.save(None).unwrap();
    let mut r = Vault::open(&path, key, IndexStorage::KdbxCustomData).unwrap();
    assert!(r.index().read_only);
    assert!(matches!(r.set_index(Index::default()), Err(VaultError::IndexReadOnly)));
}

#[test]
fn index_file_storage() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, path, key) = new_vault(dir.path(), IndexStorage::IndexFile);
    let id = v.add_credential(data("A", "b", "c")).unwrap();
    v.set_index(index_with(id, "A").0).unwrap();
    v.save(None).unwrap();
    assert!(index_file::path_for(&path).exists());
    assert!(v.database().meta.custom_data.is_empty());
    let r = Vault::open(&path, key, IndexStorage::IndexFile).unwrap();
    assert_eq!(r.index().index.revision, 1);
    assert_eq!(r.index().index.referenced_entries().len(), 1);
}

/// Prüfungen mit `keepassxc-cli`; ohne das Programm werden sie übersprungen.
mod keepassxc {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    const PW: &str = "Prüf-Kennwort ✓";

    fn cli() -> Option<&'static str> {
        Command::new("keepassxc-cli").arg("--version").output().ok().filter(|o| o.status.success()).map(|_| "keepassxc-cli")
    }

    fn run(args: &[&str], stdin: &str) -> std::process::Output {
        let mut child = Command::new("keepassxc-cli")
            .args(args)
            .env("QT_QPA_PLATFORM", "offscreen")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "keepassxc-cli {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        out
    }

    fn big_index(bytes: usize, entry: Uuid) -> Index {
        let (mut idx, _) = index_with(entry, "Konto");
        let Node::Folder(f) = &mut idx.root.children[0] else { unreachable!() };
        let make = |i: usize| {
            let mut t = Target::new(format!("ziel-{i:05} ä🚀"), Protocol::Rdp, format!("h{i}.example.test"));
            t.port = Some(3389);
            Node::Target(t)
        };
        let per = serde_json::to_vec_pretty(&make(0)).unwrap().len();
        for i in 0..bytes.div_ceil(per) {
            f.children.push(make(i));
        }
        idx
    }

    fn prepare(dir: &Path, idx_bytes: usize) -> (PathBuf, Uuid, Index) {
        let (mut v, path, _) = new_vault(dir, IndexStorage::KdbxCustomData);
        let id = v.add_credential(data("Konto", "benutzer", "geheim")).unwrap();
        let idx = big_index(idx_bytes, id);
        v.set_index(idx.clone()).unwrap();
        v.save(None).unwrap();
        let mut stored = idx;
        stored.revision = 1;
        (path, id, stored)
    }

    #[test]
    fn edit_and_move_keep_index() {
        if cli().is_none() {
            eprintln!("keepassxc-cli fehlt – übersprungen");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (path, id, idx) = prepare(dir.path(), 10_000);
        let p = path.to_str().unwrap();
        run(&["edit", "-q", p, "/remotejoe/Konto", "-u", "neuer-benutzer"], &format!("{PW}\n"));
        run(&["mkdir", "-q", p, "/Andere Gruppe"], &format!("{PW}\n"));
        run(&["mv", "-q", p, "/remotejoe/Konto", "/Andere Gruppe"], &format!("{PW}\n"));

        let v = Vault::open(&path, MasterKey::password(PW), IndexStorage::KdbxCustomData).unwrap();
        assert_eq!(v.index().index, idx, "Index nach Bearbeitung in KeePassXC unverändert");
        let c = v.credential(id).expect("Verweis bleibt gültig");
        assert_eq!(c.data.username, "neuer-benutzer");
        assert_eq!(c.group_path, vec!["Andere Gruppe".to_string()]);
        assert!(v.missing_references().is_empty());
    }

    #[test]
    fn index_sizes_survive_keepassxc() {
        if cli().is_none() {
            eprintln!("keepassxc-cli fehlt – übersprungen");
            return;
        }
        for size in [10_000, 500_000, 5_000_000] {
            let dir = tempfile::tempdir().unwrap();
            let t0 = std::time::Instant::now();
            let (path, _, idx) = prepare(dir.path(), size);
            eprintln!("Größe {size}: angelegt nach {:?}", t0.elapsed());
            let p = path.to_str().unwrap();
            run(&["edit", "-q", p, "/remotejoe/Konto", "-u", "x"], &format!("{PW}\n"));
            eprintln!("Größe {size}: keepassxc-cli fertig nach {:?}", t0.elapsed());
            let v = Vault::open(&path, MasterKey::password(PW), IndexStorage::KdbxCustomData)
                .unwrap_or_else(|e| panic!("Größe {size}: {e}"));
            assert_eq!(v.index().index, idx, "Größe {size}");
        }
    }

    /// Zusammenführen zweier Fassungen mit unterschiedlichem Index durch
    /// `keepassxc-cli merge`. Ergebnis wird ausgegeben, nicht erzwungen.
    #[test]
    fn merge_behaviour() {
        if cli().is_none() {
            eprintln!("keepassxc-cli fehlt – übersprungen");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (path, _, _) = prepare(dir.path(), 10_000);
        let other = dir.path().join("zweite.kdbx");
        fs::copy(&path, &other).unwrap();
        for (p, name) in [(&path, "Fassung A"), (&other, "Fassung B")] {
            std::thread::sleep(std::time::Duration::from_millis(1100));
            let mut v = Vault::open(p, MasterKey::password(PW), IndexStorage::KdbxCustomData).unwrap();
            let mut i = v.index().index.clone();
            i.root.name = name.into();
            v.set_index(i).unwrap();
            v.save(None).unwrap();
        }
        run(&["merge", "-q", "-s", path.to_str().unwrap(), other.to_str().unwrap()], &format!("{PW}\n"));
        let v = Vault::open(&path, MasterKey::password(PW), IndexStorage::KdbxCustomData);
        match v {
            Ok(v) => eprintln!("ZUSAMMENFÜHRUNG: Index lesbar, Wurzelname = {:?}", v.index().index.root.name),
            Err(e) => eprintln!("ZUSAMMENFÜHRUNG: Index nicht lesbar: {e}"),
        }
    }
}
