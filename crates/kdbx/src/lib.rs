//! KDBX4-Tresor: Zugangsdaten-Einträge und Ablage des Index.
//!
//! Geheimnisse liegen als gewöhnliche KeePass-Einträge in der Datei. Neue
//! Einträge legt das Programm in der Gruppe `remotejoe` an; geändert werden
//! nur Einträge innerhalb dieser Gruppe. Der Index liegt verschlüsselt als
//! Datenbank-`CustomData` (siehe [`index_store`]) oder als eigene Datei
//! neben der KDBX (siehe [`index_file`]).

pub mod index_file;
pub mod index_store;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use keepass::config::{DatabaseConfig, KdfConfig};
use keepass::db::{fields, EntryId, GroupId, Value};
use keepass::{Database, DatabaseKey};
use rj_core::config::IndexStorage;
use rj_core::fsutil::{atomic_write_locked, WriteLock};
use rj_core::schema::LoadedIndex;
use rj_core::{CredentialRef, Index, Secret};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Name der Gruppe, in der das Programm Einträge anlegt.
pub const GROUP_NAME: &str = "remotejoe";

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("Datei: {0}")]
    Io(#[from] std::io::Error),
    #[error("KDBX konnte nicht geöffnet werden: {0}")]
    Open(#[from] keepass::db::DatabaseOpenError),
    #[error("KDBX konnte nicht gespeichert werden: {0}")]
    Save(#[from] keepass::db::DatabaseSaveError),
    #[error("Kennwort ist kein gültiges UTF-8; KDBX-Kennwörter werden als UTF-8 verarbeitet")]
    PasswordEncoding,
    #[error("Eintrag {0} nicht gefunden")]
    NoSuchEntry(Uuid),
    #[error("Eintrag {0} liegt außerhalb der Gruppe „remotejoe“ und wird nicht verändert")]
    ForeignEntry(Uuid),
    #[error("Index: {0}")]
    Index(#[from] index_store::IndexStoreError),
    #[error("Indexdatei: {0}")]
    IndexFile(#[from] index_file::IndexFileError),
    #[error("Index ist schreibgeschützt (neuere Schema-Version)")]
    IndexReadOnly,
    #[error(
        "Die Datei wurde von einem anderen Programm verändert und beide Fassungen des Index \
         unterscheiden sich (eigene Revision {mine}, fremde Revision {theirs}). Bitte wählen: \
         eigene Version behalten, fremde Version laden oder beide als Sicherung ablegen."
    )]
    Conflict { mine: u64, theirs: u64 },
}

/// Hauptschlüssel: Kennwort und/oder Schlüsseldatei.
#[derive(Clone, Default)]
pub struct MasterKey {
    pub password: Option<Secret>,
    pub key_file: Option<Secret>,
}

impl MasterKey {
    pub fn password(pw: &str) -> Self {
        MasterKey { password: Some(Secret::from_str_exact(pw)), key_file: None }
    }

    pub fn with_key_file_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.key_file = Some(Secret::from_bytes(bytes));
        self
    }

    pub fn with_key_file(self, path: &Path) -> std::io::Result<Self> {
        Ok(self.with_key_file_bytes(fs::read(path)?))
    }

    fn to_db_key(&self) -> Result<DatabaseKey, VaultError> {
        let mut k = DatabaseKey::new();
        if let Some(pw) = &self.password {
            k = k.with_password(pw.as_str().ok_or(VaultError::PasswordEncoding)?);
        }
        if let Some(kf) = &self.key_file {
            k = k.with_keyfile(&mut kf.as_bytes())?;
        }
        Ok(k)
    }

    /// Eingangsmaterial für abgeleitete Schlüssel (Indexdatei):
    /// SHA-256(Kennwort) ‖ SHA-256(Schlüsseldatei).
    pub(crate) fn derivation_material(&self) -> Secret {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&Sha256::digest(self.password.as_ref().map(|s| s.as_bytes()).unwrap_or(&[])));
        out.extend_from_slice(&Sha256::digest(self.key_file.as_ref().map(|s| s.as_bytes()).unwrap_or(&[])));
        Secret::from_bytes(out)
    }
}

/// Schlüsselableitung für neue Dateien.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KdfChoice {
    /// Argon2id, 64 MiB, 10 Durchläufe, 2 Fäden.
    Argon2idDefault,
    /// AES-KDF mit gegebener Rundenzahl.
    Aes(u64),
    /// Sehr schwache Ableitung, nur für Tests.
    Weak,
}

impl KdfChoice {
    fn config(self) -> KdfConfig {
        match self {
            KdfChoice::Argon2idDefault => KdfConfig::Argon2id {
                iterations: 10,
                memory: 64 * 1024 * 1024,
                parallelism: 2,
                version: argon2::Version::Version13,
            },
            KdfChoice::Aes(rounds) => KdfConfig::Aes { rounds },
            KdfChoice::Weak => KdfConfig::Argon2id {
                iterations: 1,
                memory: 1024 * 1024,
                parallelism: 1,
                version: argon2::Version::Version13,
            },
        }
    }
}

/// Inhalt eines Zugangsdaten-Eintrags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CredentialData {
    pub title: String,
    pub username: String,
    pub password: Secret,
    pub url: Option<String>,
    pub notes: Option<String>,
    /// TOTP-Feld wie in KeePassXC (`otp`).
    pub otp: Option<Secret>,
}

/// Ein gelesener Eintrag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub uuid: Uuid,
    pub data: CredentialData,
    pub attachments: Vec<String>,
    /// Pfad der Gruppen von der Wurzel (ohne Wurzel).
    pub group_path: Vec<String>,
}

impl Credential {
    /// Domäne und Benutzername; `DOMÄNE\benutzer` wird zerlegt.
    pub fn split_domain(&self) -> (Option<&str>, &str) {
        match self.data.username.split_once('\\') {
            Some((d, u)) => (Some(d), u),
            None => (None, &self.data.username),
        }
    }
}

/// Kurzbeschreibung eines Eintrags für Listen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySummary {
    pub uuid: Uuid,
    pub title: String,
    pub username: String,
    pub group_path: Vec<String>,
}

/// Meldung „Zugangsdaten fehlen“.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Zugangsdaten fehlen: {}", match .last_title { Some(t) => format!("Eintrag „{t}“ ist in der KDBX nicht mehr vorhanden"), None => format!("Eintrag {} ist in der KDBX nicht vorhanden", .uuid) })]
pub struct MissingCredential {
    pub uuid: Uuid,
    pub last_title: Option<String>,
}

/// Wahl bei einem Konflikt zwischen eigener und fremder Fassung des Index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    KeepMine,
    LoadTheirs,
    KeepBoth,
}

/// Änderungen an Einträgen, die beim Zusammenführen erneut angewandt werden.
#[derive(Clone)]
enum EntryOp {
    Create { uuid: Uuid, data: CredentialData },
    Update { uuid: Uuid, data: CredentialData },
    Attach { uuid: Uuid, name: String, data: Secret },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    sha256: [u8; 32],
}

fn fingerprint(bytes: &[u8]) -> Fingerprint {
    Fingerprint { len: bytes.len() as u64, sha256: Sha256::digest(bytes).into() }
}

/// Geöffnete KDBX mit Index.
pub struct Vault {
    path: PathBuf,
    key: MasterKey,
    db: Database,
    storage: IndexStorage,
    disk: Option<Fingerprint>,
    index: LoadedIndex,
    /// Prüfsumme des Index beim Laden (Grundlage für Konflikterkennung).
    base_index_sha: Option<[u8; 32]>,
    index_dirty: bool,
    ops: Vec<EntryOp>,
    /// Abgeleiteter Schlüssel der Indexdatei (nur bei `IndexStorage::IndexFile`).
    index_file_key: Option<index_file::FileKey>,
}

impl Vault {
    /// Legt eine neue, leere KDBX an (noch nicht gespeichert).
    pub fn create(path: &Path, key: MasterKey, kdf: KdfChoice, storage: IndexStorage) -> Vault {
        let mut config = DatabaseConfig::default();
        config.kdf_config = kdf.config();
        let mut db = Database::with_config(config);
        db.root_mut().name = "Datenbank".into();
        db.meta.generator = Some("remotejoe".into());
        Vault {
            path: path.to_path_buf(),
            key,
            db,
            storage,
            disk: None,
            index: LoadedIndex { index: Index::default(), original_version: rj_core::SCHEMA_VERSION, read_only: false, notice: None },
            base_index_sha: None,
            index_dirty: true,
            ops: Vec::new(),
            index_file_key: None,
        }
    }

    /// Öffnet eine vorhandene KDBX.
    pub fn open(path: &Path, key: MasterKey, storage: IndexStorage) -> Result<Vault, VaultError> {
        let bytes = fs::read(path)?;
        let db = Database::parse(&bytes, key.to_db_key()?)?;
        let mut v = Vault {
            path: path.to_path_buf(),
            key,
            db,
            storage,
            disk: Some(fingerprint(&bytes)),
            index: LoadedIndex { index: Index::default(), original_version: rj_core::SCHEMA_VERSION, read_only: false, notice: None },
            base_index_sha: None,
            index_dirty: false,
            ops: Vec::new(),
            index_file_key: None,
        };
        let (loaded, fk) = Self::load_index(storage, &v.path, &v.key, &v.db)?;
        v.index_file_key = fk;
        match loaded {
            Some((loaded, sha)) => {
                v.index = loaded;
                v.base_index_sha = Some(sha);
            }
            None => v.index_dirty = false,
        }
        Ok(v)
    }

    #[allow(clippy::type_complexity)]
    fn load_index(
        storage: IndexStorage,
        path: &Path,
        key: &MasterKey,
        db: &Database,
    ) -> Result<(Option<(LoadedIndex, [u8; 32])>, Option<index_file::FileKey>), VaultError> {
        match storage {
            IndexStorage::KdbxCustomData => Ok((index_store::read(db)?, None)),
            IndexStorage::IndexFile => match index_file::read(&index_file::path_for(path), key)? {
                Some(r) => Ok((Some((r.loaded, r.sha)), r.key)),
                None => Ok((None, None)),
            },
        }
    }

    /// Argon2id-Parameter der Indexdatei: die der KDBX, wenn diese Argon2
    /// verwendet, sonst die Vorgabe für neue Dateien.
    fn index_file_params(&self) -> index_file::Argon2Params {
        match &self.db.config.kdf_config {
            KdfConfig::Argon2 { iterations, memory, parallelism, .. }
            | KdfConfig::Argon2id { iterations, memory, parallelism, .. } => index_file::Argon2Params {
                memory_kib: u32::try_from(memory / 1024).unwrap_or(u32::MAX),
                iterations: u32::try_from(*iterations).unwrap_or(u32::MAX),
                parallelism: *parallelism,
            },
            _ => index_file::Argon2Params::DEFAULT,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn storage(&self) -> IndexStorage {
        self.storage
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    pub fn index(&self) -> &LoadedIndex {
        &self.index
    }

    /// Übernimmt einen geänderten Index (gespeichert wird erst mit [`Vault::save`]).
    pub fn set_index(&mut self, index: Index) -> Result<(), VaultError> {
        if self.index.read_only {
            return Err(VaultError::IndexReadOnly);
        }
        if index != self.index.index {
            self.index.index = index;
            self.index_dirty = true;
        }
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.index_dirty || !self.ops.is_empty() || self.disk.is_none()
    }

    /// Wurde die Datei seit dem Laden oder Speichern von außen verändert?
    pub fn changed_on_disk(&self) -> Result<bool, VaultError> {
        match (&self.disk, fs::read(&self.path)) {
            (Some(fp), Ok(bytes)) => Ok(fingerprint(&bytes) != *fp),
            (None, Ok(_)) => Ok(true),
            (_, Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(self.disk.is_some()),
            (_, Err(e)) => Err(e.into()),
        }
    }

    fn group_id(db: &Database) -> Option<GroupId> {
        db.root().groups().find(|g| g.name == GROUP_NAME).map(|g| g.id())
    }

    fn ensure_group(db: &mut Database) -> GroupId {
        if let Some(id) = Self::group_id(db) {
            return id;
        }
        let mut root = db.root_mut();
        let mut g = root.add_group();
        g.name = GROUP_NAME.into();
        g.id()
    }

    fn group_path(db: &Database, id: EntryId) -> Vec<String> {
        let Some(e) = db.entry(id) else { return Vec::new() };
        let mut path = Vec::new();
        let mut gid = Some(e.parent().id());
        while let Some(group) = gid.and_then(|id| db.group(id)) {
            let parent = group.parent().map(|p| p.id());
            if parent.is_none() {
                break; // Wurzel
            }
            path.push(group.name.clone());
            gid = parent;
        }
        path.reverse();
        path
    }

    fn in_own_group(db: &Database, id: EntryId) -> bool {
        Self::group_path(db, id).first().map(String::as_str) == Some(GROUP_NAME)
    }

    fn in_recycle_bin(db: &Database, id: EntryId) -> bool {
        let Some(bin) = db.recycle_bin().map(|b| b.id()) else { return false };
        let Some(e) = db.entry(id) else { return false };
        let mut gid = Some(e.parent().id());
        while let Some(group) = gid.and_then(|id| db.group(id)) {
            if group.id() == bin {
                return true;
            }
            gid = group.parent().map(|p| p.id());
        }
        false
    }

    /// Alle Einträge außerhalb des Papierkorbs.
    pub fn entries(&self) -> Vec<EntrySummary> {
        let mut out: Vec<_> = self
            .db
            .iter_all_entries()
            .filter(|e| !Self::in_recycle_bin(&self.db, e.id()))
            .map(|e| EntrySummary {
                uuid: e.id().uuid(),
                title: e.get_title().unwrap_or_default().to_string(),
                username: e.get_username().unwrap_or_default().to_string(),
                group_path: Self::group_path(&self.db, e.id()),
            })
            .collect();
        out.sort_by(|a, b| a.group_path.cmp(&b.group_path).then(a.title.cmp(&b.title)));
        out
    }

    /// Liest einen Eintrag. Der Fundort in der Gruppenstruktur ist gleichgültig.
    pub fn credential(&self, uuid: Uuid) -> Option<Credential> {
        let id = EntryId::from_uuid(uuid);
        if Self::in_recycle_bin(&self.db, id) {
            return None;
        }
        let e = self.db.entry(id)?;
        let get = |k: &str| e.fields.get(k).map(|v| v.get().clone());
        Some(Credential {
            uuid,
            data: CredentialData {
                title: get(fields::TITLE).unwrap_or_default(),
                username: get(fields::USERNAME).unwrap_or_default(),
                password: Secret::from_bytes(get(fields::PASSWORD).unwrap_or_default().into_bytes()),
                url: get(fields::URL),
                notes: get(fields::NOTES),
                otp: get(fields::OTP).map(|s| Secret::from_bytes(s.into_bytes())),
            },
            attachments: e.attachments_named().map(|(n, _)| n.to_string()).collect(),
            group_path: Self::group_path(&self.db, id),
        })
    }

    /// Löst einen Index-Verweis auf; fehlt der Eintrag, kommt die Meldung mit
    /// dem zuletzt bekannten Titel.
    pub fn resolve(&self, r: &CredentialRef) -> Result<Credential, MissingCredential> {
        self.credential(r.entry)
            .ok_or_else(|| MissingCredential { uuid: r.entry, last_title: r.last_title.clone() })
    }

    /// Anhang eines Eintrags (z. B. Schlüsseldatei, WireGuard-Konfiguration).
    pub fn attachment(&self, uuid: Uuid, name: &str) -> Option<Secret> {
        let e = self.db.entry(EntryId::from_uuid(uuid))?;
        let a = e.attachment_by_name(name)?;
        Some(Secret::from_bytes(a.data.get().clone()))
    }

    fn apply(db: &mut Database, op: &EntryOp) -> Result<(), VaultError> {
        fn write(e: &mut keepass::db::Entry, d: &CredentialData) {
            e.set_unprotected(fields::TITLE, d.title.clone());
            e.set_unprotected(fields::USERNAME, d.username.clone());
            // Vorher in `record` auf UTF-8 geprüft; keine Ersetzung.
            e.set(fields::PASSWORD, Value::protected(d.password.as_str().unwrap_or_default().to_string()));
            match &d.url {
                Some(u) => e.set_unprotected(fields::URL, u.clone()),
                None => {
                    e.fields.remove(fields::URL);
                }
            }
            match &d.notes {
                Some(n) => e.set_unprotected(fields::NOTES, n.clone()),
                None => {
                    e.fields.remove(fields::NOTES);
                }
            }
            if let Some(otp) = &d.otp {
                e.set(fields::OTP, Value::protected(otp.as_str().unwrap_or_default().to_string()));
            }
            let now = chrono::Utc::now().naive_utc();
            e.times.last_modification = Some(now);
        }
        match op {
            EntryOp::Create { uuid, data } => {
                let gid = Self::ensure_group(db);
                let mut group = db.group_mut(gid).expect("Gruppe vorhanden");
                let mut e = match group.add_entry_with_id(EntryId::from_uuid(*uuid)) {
                    Ok(e) => e,
                    // Bereits vorhanden (z. B. nach Zusammenführen): wie Änderung behandeln.
                    Err(_) => return Self::apply(db, &EntryOp::Update { uuid: *uuid, data: data.clone() }),
                };
                write(&mut e, data);
            }
            EntryOp::Update { uuid, data } => {
                let id = EntryId::from_uuid(*uuid);
                if db.entry(id).is_none() {
                    return Err(VaultError::NoSuchEntry(*uuid));
                }
                if !Self::in_own_group(db, id) {
                    return Err(VaultError::ForeignEntry(*uuid));
                }
                let mut e = db.entry_mut(id).expect("geprüft");
                write(&mut e, data);
            }
            EntryOp::Attach { uuid, name, data } => {
                let id = EntryId::from_uuid(*uuid);
                if !Self::in_own_group(db, id) {
                    return Err(VaultError::ForeignEntry(*uuid));
                }
                let mut e = db.entry_mut(id).ok_or(VaultError::NoSuchEntry(*uuid))?;
                e.remove_attachment_by_name(name);
                e.add_attachment(name.clone(), Value::protected(data.as_bytes().to_vec()));
            }
        }
        Ok(())
    }

    fn record(&mut self, op: EntryOp) -> Result<(), VaultError> {
        if let EntryOp::Create { data, .. } | EntryOp::Update { data, .. } = &op {
            if data.password.as_str().is_none() || data.otp.as_ref().is_some_and(|o| o.as_str().is_none()) {
                return Err(VaultError::PasswordEncoding);
            }
        }
        Self::apply(&mut self.db, &op)?;
        self.ops.push(op);
        Ok(())
    }

    /// Legt einen neuen Eintrag in der Gruppe `remotejoe` an.
    pub fn add_credential(&mut self, data: CredentialData) -> Result<Uuid, VaultError> {
        let uuid = Uuid::new_v4();
        self.record(EntryOp::Create { uuid, data })?;
        Ok(uuid)
    }

    /// Ändert einen Eintrag; nur innerhalb der Gruppe `remotejoe` erlaubt.
    pub fn update_credential(&mut self, uuid: Uuid, data: CredentialData) -> Result<(), VaultError> {
        self.record(EntryOp::Update { uuid, data })
    }

    /// Fügt einen Anhang hinzu oder ersetzt ihn.
    pub fn set_attachment(&mut self, uuid: Uuid, name: &str, data: Secret) -> Result<(), VaultError> {
        self.record(EntryOp::Attach { uuid, name: name.to_string(), data })
    }

    /// Einträge, auf die kein Verweis des Index zeigt („Nicht zugeordnet“).
    pub fn unassigned(&self) -> Vec<EntrySummary> {
        let used: BTreeSet<Uuid> = self.index.index.referenced_entries();
        self.entries().into_iter().filter(|e| !used.contains(&e.uuid)).collect()
    }

    /// Verweise des Index, deren Eintrag fehlt.
    pub fn missing_references(&self) -> Vec<MissingCredential> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        self.index.index.for_each_settings(|s| {
            for r in s.credentials.values() {
                if seen.insert(r.entry) && self.credential(r.entry).is_none() {
                    out.push(MissingCredential { uuid: r.entry, last_title: r.last_title.clone() });
                }
            }
        });
        out
    }

    /// Aktualisiert die zwischengespeicherten Titel im Index (nur im Speicher;
    /// der Index gilt danach als geändert, wenn sich etwas ändert).
    pub fn refresh_cached_titles(&mut self) -> Result<(), VaultError> {
        let mut idx = self.index.index.clone();
        fn walk(f: &mut rj_core::Folder, v: &Vault) {
            fix(&mut f.settings, v);
            for n in &mut f.children {
                match n {
                    rj_core::Node::Folder(sub) => walk(sub, v),
                    rj_core::Node::Target(t) => fix(&mut t.settings, v),
                }
            }
        }
        fn fix(s: &mut rj_core::Settings, v: &Vault) {
            for r in s.credentials.values_mut() {
                if let Some(c) = v.credential(r.entry) {
                    r.last_title = Some(c.data.title);
                }
            }
        }
        walk(&mut idx.root, self);
        self.set_index(idx)
    }

    /// Speichert, wenn es Änderungen gibt. Wurde die Datei von außen
    /// verändert, wird sie neu eingelesen und zusammengeführt; bei einem
    /// Index-Konflikt ohne Wahl kommt [`VaultError::Conflict`] zurück und es
    /// wird nichts geschrieben.
    pub fn save(&mut self, choice: Option<ConflictChoice>) -> Result<bool, VaultError> {
        if !self.is_dirty() && !self.changed_on_disk()? {
            return Ok(false);
        }
        if self.index.read_only && self.index_dirty {
            return Err(VaultError::IndexReadOnly);
        }
        let _lock = WriteLock::try_acquire(&self.path)?;
        let mut backup_theirs: Option<Index> = None;

        if self.disk.is_some() && self.changed_on_disk()? {
            let bytes = fs::read(&self.path)?;
            let mut foreign = Database::parse(&bytes, self.key.to_db_key()?)?;
            let (theirs, theirs_key) = Self::load_index(self.storage, &self.path, &self.key, &foreign)?;
            if theirs_key.is_some() {
                self.index_file_key = theirs_key;
            }
            let theirs_sha = theirs.as_ref().map(|t| t.1);
            let theirs_changed = theirs_sha != self.base_index_sha;
            if theirs_changed {
                let (their_idx, _) = theirs.expect("geändert heißt vorhanden");
                if self.index_dirty {
                    let mine_rev = self.index.index.revision;
                    let their_rev = their_idx.index.revision;
                    match choice {
                        None => return Err(VaultError::Conflict { mine: mine_rev, theirs: their_rev }),
                        Some(ConflictChoice::KeepMine) => {
                            self.index.index.revision = mine_rev.max(their_rev);
                        }
                        Some(ConflictChoice::KeepBoth) => {
                            self.index.index.revision = mine_rev.max(their_rev);
                            backup_theirs = Some(their_idx.index);
                        }
                        Some(ConflictChoice::LoadTheirs) => {
                            self.index = their_idx;
                            self.index_dirty = false;
                        }
                    }
                } else {
                    self.index = their_idx;
                }
            }
            for op in &self.ops {
                Self::apply(&mut foreign, op)?;
            }
            self.db = foreign;
            if !theirs_changed || self.index_dirty {
                // Eigener Index wird (erneut) geschrieben.
                self.index_dirty = self.index_dirty || theirs_changed;
            }
        }

        let mut index_sha = self.base_index_sha;
        if self.index_dirty {
            self.index.index.revision += 1;
            let json = self.index.to_json_for_save().map_err(|_| VaultError::IndexReadOnly)?;
            match self.storage {
                IndexStorage::KdbxCustomData => {
                    index_store::write(&mut self.db, &json)?;
                    if let Some(t) = &backup_theirs {
                        let mut mine_backup = json.clone();
                        index_store::write_backup(&mut self.db, "mine", &mine_backup)?;
                        mine_backup.clear();
                        index_store::write_backup(&mut self.db, "theirs", &rj_core::schema::to_json(t).unwrap_or_default())?;
                    }
                }
                IndexStorage::IndexFile => {
                    let params = self.index_file_params();
                    index_file::write(&index_file::path_for(&self.path), &self.key, params, &mut self.index_file_key, &json)?;
                    if let Some(t) = &backup_theirs {
                        let p = index_file::path_for(&self.path).with_extension("rjindex-fremd");
                        let mut own_key = None;
                        index_file::write(&p, &self.key, params, &mut own_key, &rj_core::schema::to_json(t).unwrap_or_default())?;
                    }
                }
            }
            index_sha = Some(Sha256::digest(&json).into());
        }

        let mut out = Vec::new();
        self.db.save(&mut out, self.key.to_db_key()?)?;
        if let Some(dir) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        atomic_write_locked(&self.path, &out)?;
        self.disk = Some(fingerprint(&out));
        self.base_index_sha = index_sha;
        self.index_dirty = false;
        self.ops.clear();
        Ok(true)
    }

    /// Verwirft alle Änderungen und liest die Datei neu.
    pub fn reload(&mut self) -> Result<(), VaultError> {
        let fresh = Vault::open(&self.path, self.key.clone(), self.storage)?;
        *self = fresh;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
