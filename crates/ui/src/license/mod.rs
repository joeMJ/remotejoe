//! Lizenz der Fassung mit Modulen: signierter Textblock (ausgestellt mit dem internen Werkzeug `rj-lizenz`).
//!
//! Format: Kopf- und Fußzeile, dazwischen Base64 von 64 Byte Ed25519-Signatur + JSON-Inhalt; signiert wird
//! `remotejoe-lizenz-v1\0` + JSON-Inhalt. Der öffentliche Schlüssel ist eingebaut (`lizenz-key.pub`).
//! Abgelegt wird der Block als `lizenz.txt` im Konfigurationsordner; er gilt ab dem nächsten Start.
//!
//! - Gültig bis einschließlich `version_bis`: Module aktiv. Sonst (fehlt, ungültig, abgelaufen): Rückfall auf
//!   die Funktionen ohne Module; gespeicherte Daten der Module bleiben unberührt.
//! - Bis einschließlich `wartung_bis`: Updates aus der Ablage (Freigabe-Link und Kennwort aus der Lizenz).
//! - Vorführung (Beispieldaten, Bildschirmfotos) und Tests: Module aktiv ohne Lizenz.

pub mod ablage;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine;
use chrono::NaiveDate;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const BEGIN: &str = "-----BEGIN REMOTEJOE LIZENZ-----";
const END: &str = "-----END REMOTEJOE LIZENZ-----";
const DOMAIN: &[u8] = b"remotejoe-lizenz-v1\0";
const PUB_TAG: &str = "remotejoe-lizenz-ed25519";
const KEY_TEXT: &str = include_str!("lizenz-key.pub");
const FILE: &str = "lizenz.txt";
/// Eintrag der Haupt-KDBX (Gruppe `remotejoe`), in dem die Lizenz mitwandert.
pub const ENTRY_TITLE: &str = "remotejoe Lizenz";
const FIELD: &str = "Lizenz";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Lizenz {
    pub v: u32,
    pub id: String,
    /// Optional; fehlt in älteren Lizenzen und bei Einzelpersonen.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unternehmen: String,
    pub name: String,
    pub email: String,
    pub url: String,
    pub kennwort: String,
    pub ausgestellt: String,
    pub version_bis: String,
    pub wartung_bis: String,
    /// Freigeschaltete Erweiterungen (Kennungen); fehlt in älteren Lizenzen → alle angemeldeten Erweiterungen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub module: Vec<String>,
}

/// Schaltet die Lizenz diese Erweiterung frei? Der Kern kennt keine Kennungen; eine Lizenz ohne Liste gilt für alle.
pub fn hat_modul(l: &Lizenz, id: &str) -> bool {
    l.module.is_empty() || l.module.iter().any(|m| m == id)
}

/// Anzeigename einer Erweiterung (aus der Registrierung; unbekannte Kennungen erscheinen als Kennung).
fn name_of(id: &str) -> String {
    if let Some(m) = crate::plugins::modules().into_iter().find(|m| m.id == id) {
        return m.name;
    }
    if let Some(m) = crate::plugin_fetch::installed_manifest(id) {
        return m.name;
    }
    // Noch nicht installiert: Name aus dem Katalog der Quelle (zuletzt geladen), sonst die Kennung.
    crate::plugin_fetch::katalog().name_of(id).unwrap_or(id).to_string()
}

/// Kennungen für die Lizenzseite: angemeldete Erweiterungen plus die in der Lizenz genannten.
/// Module der aktuellen Lizenz (für die Stand-Liste), leer ohne Lizenz.
fn known_modules_of_status() -> Vec<String> {
    match status() {
        Status::Gueltig(l) | Status::Abgelaufen(l) => known_ids(l),
        _ => Vec::new(),
    }
}

fn known_ids(l: &Lizenz) -> Vec<String> {
    let mut v: Vec<String> = crate::plugins::modules().into_iter().map(|m| m.id).collect();
    if l.module.is_empty() {
        for k in crate::plugin_fetch::katalog().ids() {
            if !v.contains(&k) {
                v.push(k);
            }
        }
    }
    for m in &l.module {
        if !v.contains(m) {
            v.push(m.clone());
        }
    }
    v
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Fehlt,
    Ungueltig(String),
    Abgelaufen(Lizenz),
    Gueltig(Lizenz),
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn builtin_key() -> Result<VerifyingKey, String> {
    key_from_text(KEY_TEXT)
}

fn key_from_text(text: &str) -> Result<VerifyingKey, String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).ok_or("Schlüssel leer")?;
    let (t, v) = line.split_once(' ').ok_or("Schlüsselformat")?;
    if t != PUB_TAG {
        return Err("Schlüsselkennung".into());
    }
    let raw = b64().decode(v.trim()).map_err(|_| "Schlüssel Base64")?;
    let arr: [u8; 32] = raw.try_into().map_err(|_| "Schlüssellänge")?;
    VerifyingKey::from_bytes(&arr).map_err(|_| "Schlüssel ungültig".into())
}

/// Textblock prüfen und lesen (Copy-Paste: Leerzeichen, Einrückung, Zeilenenden egal).
pub fn decode(text: &str, key: &VerifyingKey) -> Result<Lizenz, String> {
    let start = text.find(BEGIN).ok_or("Kopfzeile „BEGIN REMOTEJOE LIZENZ“ fehlt")? + BEGIN.len();
    let end = text[start..].find(END).ok_or("Fußzeile „END REMOTEJOE LIZENZ“ fehlt")? + start;
    let body: String = text[start..end].chars().filter(|c| !c.is_whitespace()).collect();
    let raw = b64().decode(body).map_err(|_| "Inhalt unlesbar (kein Base64)")?;
    if raw.len() < 65 {
        return Err("Inhalt zu kurz".into());
    }
    let (sig, payload) = raw.split_at(64);
    let sig = Signature::from_slice(sig).map_err(|_| "Signatur unlesbar")?;
    let mut msg = DOMAIN.to_vec();
    msg.extend_from_slice(payload);
    key.verify_strict(&msg, &sig).map_err(|_| "Signatur ungültig – Lizenz verändert oder nicht von remotejoe")?;
    let l: Lizenz = serde_json::from_slice(payload).map_err(|e| format!("Inhalt: {e}"))?;
    if l.v != 1 || date(&l.version_bis).is_none() || date(&l.wartung_bis).is_none() {
        return Err("Inhalt: Version oder Datum ungültig".into());
    }
    Ok(l)
}

/// Zustand eines Textblocks an einem Tag.
pub fn status_of(text: Option<&str>, key: &VerifyingKey, today: NaiveDate) -> Status {
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else { return Status::Fehlt };
    match decode(text, key) {
        Err(e) => Status::Ungueltig(e),
        Ok(l) if date(&l.version_bis).is_some_and(|d| today <= d) => Status::Gueltig(l),
        Ok(l) => Status::Abgelaufen(l),
    }
}

/// Nutzung ohne Ende (Version bis 9999-12-31): als „unbeschränkt“ anzeigen.
pub fn unbeschraenkt(l: &Lizenz) -> bool {
    l.version_bis.as_str() >= "9999-01-01"
}

/// Wartung (Updates von der Update-Quelle) noch an diesem Tag?
pub fn wartung_aktiv(l: &Lizenz, today: NaiveDate) -> bool {
    date(&l.wartung_bis).is_some_and(|d| today <= d)
}

#[cfg(test)]
thread_local! {
    /// Tests: Lizenzdatei in einem eigenen Ordner statt im Konfigurationsordner des Benutzers.
    pub static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn path() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(d) = TEST_DIR.with(|t| t.borrow().clone()) {
        return Some(d.join(FILE));
    }
    // Tests ohne eigenen Ordner: keine Lizenzdatei (nie die echte lesen oder schreiben).
    #[cfg(test)]
    return None;
    #[allow(unreachable_code)]
    rj_core::config::config_dir().map(|d| d.join(FILE))
}

/// Nur den Block selbst (ohne umgebenden Text aus dem Einfügen).
fn block_only(text: &str) -> String {
    let start = text.find(BEGIN).unwrap_or(0);
    let end = text.find(END).map(|e| e + END.len()).unwrap_or(text.len());
    format!("{}\n", text[start..end].trim())
}

/// Lizenzdatei nur für den Benutzer lesbar (sie enthält das Kennwort für die Updates).
fn restrict(p: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = p;
}

fn write_file(text: &str) -> Result<(), String> {
    let p = path().ok_or("Kein Konfigurationsordner")?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("txt.neu");
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        use std::io::Write;
        o.open(&tmp).and_then(|mut f| f.write_all(block_only(text).as_bytes())).map_err(|e| e.to_string())?;
    }
    restrict(&tmp);
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

static STATUS: OnceLock<Status> = OnceLock::new();
static DEMO: AtomicBool = AtomicBool::new(false);
/// Seit dem Start aus der Schlüsseldatenbank übernommen (gilt nach dem Neustart).
static AUS_KDBX: AtomicBool = AtomicBool::new(false);
/// Lizenz in diesem Lauf entfernt (gilt nach dem Neustart; die Seite zeigt schon „Keine Lizenz“).
static ENTFERNT: AtomicBool = AtomicBool::new(false);

/// Beim Anlegen der App: Vorführung merken (Module dann ohne Lizenz aktiv).
pub fn set_demo(demo: bool) {
    if demo {
        DEMO.store(true, Ordering::Relaxed);
    }
}

/// Zustand beim Start (einmal gelesen; eine neue Lizenz gilt ab dem nächsten Start).
pub fn status() -> &'static Status {
    STATUS.get_or_init(|| {
        if let Some(p) = path().filter(|p| p.exists()) {
            restrict(&p);
        }
        let text = path().and_then(|p| std::fs::read_to_string(p).ok());
        match builtin_key() {
            Ok(k) => status_of(text.as_deref(), &k, today()),
            Err(e) => Status::Ungueltig(format!("eingebauter Schlüssel: {e}")),
        }
    })
}

#[cfg(test)]
thread_local! {
    /// Tests: Module gelten als lizenziert, außer ein Test schaltet es aus.
    pub static TEST_AKTIV: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Ist dieses Modul aktiv (gültige Lizenz, die es freischaltet; Vorführung oder Test)?
pub fn modul_aktiv(id: &str) -> bool {
    #[cfg(test)]
    {
        let _ = id;
        TEST_AKTIV.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        DEMO.load(Ordering::Relaxed) || matches!(status(), Status::Gueltig(l) if hat_modul(l, id))
    }
}

/// Irgendeine Erweiterung freigeschaltet (Vorführung oder gültige Lizenz)?
pub fn aktiv() -> bool {
    DEMO.load(Ordering::Relaxed) || matches!(status(), Status::Gueltig(_))
}

/// Warum Erweiterungen nicht heruntergeladen oder aktualisiert werden (keine gültige Lizenz, Wartung abgelaufen);
/// `None`, wenn alles passt.
pub fn wartung_grund() -> Option<String> {
    match status() {
        Status::Gueltig(l) if wartung_aktiv(l, today()) => None,
        Status::Gueltig(l) => Some(format!("Wartung am {} abgelaufen – Erweiterungen werden nicht mehr heruntergeladen oder aktualisiert; installierte bleiben nutzbar.", l.wartung_bis)),
        Status::Abgelaufen(l) => Some(format!("Lizenz am {} abgelaufen.", l.version_bis)),
        Status::Ungueltig(e) => Some(format!("Lizenz ungültig: {e}")),
        Status::Fehlt => Some("Keine Lizenz – Erweiterungen gibt es nur mit Lizenz.".into()),
    }
}

/// Module der gültigen Lizenz (Kennungen); leer ohne Lizenz oder bei einer älteren Lizenz ohne Liste.
pub fn module_ids() -> Vec<String> {
    match status() {
        Status::Gueltig(l) => module_ids_of(l),
        _ => Vec::new(),
    }
}

/// Module einer Lizenz; eine ältere Lizenz ohne Liste umfasst alle Erweiterungen des Katalogs der Quelle.
fn module_ids_of(l: &Lizenz) -> Vec<String> {
    if l.module.is_empty() { crate::plugin_fetch::katalog().ids() } else { l.module.clone() }
}

/// Module aus einem Lizenztext (für das Holen direkt nach der Eingabe).
pub fn module_ids_aus(text: &str) -> Vec<String> {
    builtin_key().ok().and_then(|k| decode(text, &k).ok()).map(|l| module_ids_of(&l)).unwrap_or_default()
}

/// Gültige Lizenz mit laufender Wartung: Zugang zur Ablage (Freigabe-Link, Kennwort).
pub fn ablage() -> Option<(String, String)> {
    match status() {
        Status::Gueltig(l) if wartung_aktiv(l, today()) => Some((l.url.clone(), l.kennwort.clone())),
        _ => None,
    }
}

/// Angaben für Einstellungen → Lizenz (Kennwort nie im Klartext).
pub fn json() -> Value {
    let t = today();
    let fields = |l: &Lizenz| {
        json!({ "module": known_ids(l).iter().map(|id| json!({ "id": id, "name": name_of(id), "lizenziert": hat_modul(l, id), "fehlt": crate::plugins::missing_dependencies(id) })).collect::<Vec<_>>(),
                "id": l.id, "unternehmen": l.unternehmen, "unbeschraenkt": unbeschraenkt(l), "name": l.name, "email": l.email, "url": l.url, "kennwort": !l.kennwort.is_empty(),
                "ausgestellt": l.ausgestellt, "version_bis": l.version_bis, "wartung_bis": l.wartung_bis,
                "wartung": wartung_aktiv(l, t) })
    };
    let pending = path().and_then(|p| std::fs::read_to_string(p).ok());
    let entfernt = ENTFERNT.load(Ordering::Relaxed);
    let mut v = match status() {
        _ if entfernt => json!({ "status": "fehlt", "entfernt": true, "neustart": true }),
        Status::Fehlt => json!({ "status": "fehlt" }),
        Status::Ungueltig(e) => json!({ "status": "ungueltig", "fehler": e }),
        Status::Abgelaufen(l) => json!({ "status": "abgelaufen", "lizenz": fields(l) }),
        Status::Gueltig(l) => json!({ "status": "gueltig", "lizenz": fields(l) }),
    };
    // Geladene Erweiterungen und Ladefehler (Lizenzseite).
    let loaded = crate::plugins::loaded();
    v["erweiterungen"] = json!(loaded.libs.iter().filter(|l| l.geladen).collect::<Vec<_>>());
    let mut fehler: Vec<Value> = loaded.errors().iter().map(|(p, e)| json!({ "pfad": p, "fehler": e })).collect();
    fehler.extend(crate::plugins::attach_errors().into_iter().map(|(id, e)| json!({ "pfad": id, "fehler": e })));
    v["ladefehler"] = json!(fehler);
    let wartend = crate::plugin_fetch::pending_ids();
    v["erweiterung_wartet"] = json!(!wartend.is_empty());
    v["wartend"] = json!(wartend);
    v["wartung_grund"] = json!(wartung_grund());
    // Stand je bekannter Erweiterung: installierte Fassung (Manifest im Datenordner), geladen, wartend.
    v["installiert"] = json!(crate::plugins::manifests().into_iter().map(|m| m.id).chain(known_modules_of_status()).collect::<std::collections::BTreeSet<_>>().into_iter().map(|id| {
        let inst = crate::plugin_fetch::installed_manifest(&id);
        json!({ "id": id, "version": inst.as_ref().map(|m| m.version.clone()), "name": inst.as_ref().map(|m| m.name.clone()), "geladen": loaded.is_loaded(&id), "wartet": wartend.contains(&id) })
    }).collect::<Vec<_>>());
    // Seit dem Start eine andere Lizenz abgelegt: gilt nach dem Neustart.
    let at_start = match status() { Status::Gueltig(l) | Status::Abgelaufen(l) => Some(l.id.clone()), _ => None };
    if let (Some(text), Ok(k)) = (pending, builtin_key()) {
        if let Ok(l) = decode(&text, &k) {
            if Some(&l.id) != at_start.as_ref() {
                v["neustart"] = json!(true);
            }
        }
    }
    v["alle_module"] = json!(crate::plugins::modules().into_iter().map(|m| json!({ "id": m.id, "name": m.name, "lizenziert": false })).collect::<Vec<_>>());
    v["vorfuehrung"] = json!(DEMO.load(Ordering::Relaxed));
    v["aus_kdbx"] = json!(AUS_KDBX.load(Ordering::Relaxed));
    v
}

/// Eingefügten Textblock prüfen und ablegen; gilt ab dem nächsten Start.
pub fn save(text: &str) -> Result<Value, String> {
    let key = builtin_key()?;
    let l = decode(text, &key)?;
    write_file(text)?;
    ENTFERNT.store(false, Ordering::Relaxed);
    let abgelaufen = !date(&l.version_bis).is_some_and(|d| today() <= d);
    Ok(json!({ "ok": true, "unbeschraenkt": unbeschraenkt(&l), "name": l.name, "version_bis": l.version_bis, "wartung_bis": l.wartung_bis, "abgelaufen": abgelaufen, "neustart": true }))
}

/// Lizenz entfernen: Datei löschen und die Kopie in der Schlüsseldatenbank leeren (sonst käme sie beim nächsten
/// Entsperren zurück). Erweiterungen und ihre Daten bleiben liegen; ohne Lizenz lädt remotejoe sie nach dem Neustart
/// nicht mehr.
pub fn remove(app: &mut crate::app::App) -> Result<Value, String> {
    let p = path().ok_or("Kein Konfigurationsordner")?;
    let hatte = p.exists();
    if hatte {
        std::fs::remove_file(&p).map_err(|e| format!("Lizenzdatei nicht gelöscht: {e}"))?;
    }
    let mut kdbx = Value::Null;
    if !app.demo {
        if let Some(v) = app.vault.as_mut() {
            let existing = v.entries().into_iter().find(|e| e.title == ENTRY_TITLE && e.group_path.first().map(String::as_str) == Some(rj_kdbx::GROUP_NAME)).map(|e| e.uuid);
            if let Some(u) = existing {
                kdbx = match v.set_protected_field(u, FIELD, rj_core::Secret::from_str_exact("")).map_err(|e| e.to_string()).and_then(|_| app.save().map(|_| ()).map_err(|e| e.to_string())) {
                    Ok(()) => Value::Bool(true),
                    Err(e) => Value::String(e),
                };
            }
        }
    }
    ENTFERNT.store(true, Ordering::Relaxed);
    Ok(json!({ "ok": true, "hatte": hatte, "kdbx": kdbx, "neustart": true }))
}

/// Zugang zur Ablage aus einem Lizenztext (für das Holen der Erweiterungen direkt nach der Eingabe, bevor die
/// Lizenz mit dem nächsten Start gilt).
pub fn zugang_aus(text: &str) -> Option<(String, String)> {
    let l = decode(text, &builtin_key().ok()?).ok()?;
    if l.url.is_empty() || !wartung_aktiv(&l, today()) {
        return None;
    }
    Some((l.url, l.kennwort))
}

/// Lizenz in der geöffneten Haupt-KDBX ablegen (Eintrag „remotejoe Lizenz“, geschütztes Feld „Lizenz“) und speichern.
pub fn in_kdbx(app: &mut crate::app::App, text: &str) -> Result<(), String> {
    if app.demo {
        return Err("Vorführung".into());
    }
    let block = rj_core::Secret::from_str_exact(&block_only(text));
    let v = app.vault.as_mut().ok_or("Keine Schlüsseldatenbank geöffnet")?;
    if v.index().read_only {
        return Err("Schlüsseldatenbank schreibgeschützt".into());
    }
    let existing = v.entries().into_iter().find(|e| e.title == ENTRY_TITLE && e.group_path.first().map(String::as_str) == Some(rj_kdbx::GROUP_NAME)).map(|e| e.uuid);
    let uuid = match existing {
        Some(u) => u,
        None => v
            .add_credential(rj_kdbx::CredentialData {
                title: ENTRY_TITLE.into(),
                username: "remotejoe".into(),
                notes: Some("Lizenz von remotejoe (Feld „Lizenz“). Wird beim Entsperren auf weiteren Rechnern übernommen.".into()),
                ..Default::default()
            })
            .map_err(|e| e.to_string())?,
    };
    v.set_protected_field(uuid, FIELD, block).map_err(|e| e.to_string())?;
    app.save().map(|_| ()).map_err(|e| format!("Schlüsseldatenbank nicht gespeichert: {e}"))
}

/// Nach dem Entsperren: Lizenz aus der KDBX übernehmen, wenn sie lokal fehlt oder eine andere ist.
/// Liefert `true`, wenn übernommen wurde (gilt nach dem Neustart).
pub fn aus_kdbx(app: &crate::app::App) -> bool {
    match builtin_key() {
        Ok(k) => aus_kdbx_mit(app, &k),
        Err(_) => false,
    }
}

fn aus_kdbx_mit(app: &crate::app::App, key: &VerifyingKey) -> bool {
    if app.demo {
        return false;
    }
    let Some(v) = app.vault.as_ref() else { return false };
    let Some(e) = v.entries().into_iter().find(|e| e.title == ENTRY_TITLE && e.group_path.first().map(String::as_str) == Some(rj_kdbx::GROUP_NAME)) else { return false };
    let Some(text) = v.field(e.uuid, FIELD).and_then(|s| s.as_str().map(str::to_string)) else { return false };
    let Ok(neu) = decode(&text, key) else { return false };
    let lokal = path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| decode(&t, key).ok());
    if lokal.as_ref().is_some_and(|l| l.id == neu.id) {
        return false;
    }
    if write_file(&text).is_ok() {
        AUS_KDBX.store(true, Ordering::Relaxed);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn sample(version_bis: &str) -> Lizenz {
        Lizenz { v: 1, id: "id-1".into(), unternehmen: "Beispiel GmbH".into(), name: "Erika Muster".into(), email: "lizenz@example.org".into(), url: "https://example.org/d/abc/".into(),
            kennwort: "geheim".into(), ausgestellt: "2026-11-19".into(), version_bis: version_bis.into(), wartung_bis: "2027-06-30".into(), module: Vec::new() }
    }

    /// Gleiches Format wie `rj-lizenz ausstellen`.
    fn block(l: &Lizenz, k: &SigningKey) -> String {
        let payload = serde_json::to_vec(l).unwrap();
        let mut msg = DOMAIN.to_vec();
        msg.extend_from_slice(&payload);
        let mut raw = k.sign(&msg).to_bytes().to_vec();
        raw.extend_from_slice(&payload);
        let t = b64().encode(raw);
        let body: Vec<&str> = t.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
        format!("{BEGIN}\n{}\n{END}\n", body.join("\n"))
    }

    fn d(s: &str) -> NaiveDate {
        date(s).unwrap()
    }

    #[test]
    fn license_travels_with_the_key_database() {
        let dir = tempfile::tempdir().unwrap();
        TEST_DIR.with(|t| *t.borrow_mut() = Some(dir.path().join("rechner-b")));
        let k = SigningKey::from_bytes(&[6u8; 32]);
        let b = block(&sample("9999-12-31"), &k);
        // Rechner A: Lizenz in die KDBX schreiben.
        let (mut app, kdbx, key) = crate::editor::tests::app_with_vault(dir.path());
        app.config_path = None;
        in_kdbx(&mut app, &format!("Lizenz:\n{b}\nGruß")).unwrap();
        drop(app);
        // Rechner B: dieselbe KDBX öffnen, keine lokale Lizenz → übernehmen (0600), danach nicht erneut.
        let mut app = crate::app::App::new(false);
        app.config_path = None;
        app.vault = Some(rj_kdbx::Vault::open(&kdbx, key.clone(), rj_core::config::IndexStorage::KdbxCustomData).unwrap());
        assert!(aus_kdbx_mit(&app, &k.verifying_key()));
        let p = path().unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), block_only(&b));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(!aus_kdbx_mit(&app, &k.verifying_key()), "gleiche Lizenz: nichts zu tun");
        // Fremd signierter Block in der KDBX wird nicht übernommen.
        assert!(!aus_kdbx_mit(&app, &SigningKey::from_bytes(&[7u8; 32]).verifying_key()) );
        TEST_DIR.with(|t| *t.borrow_mut() = None);
    }

    #[test]
    fn block_only_strips_surroundings() {
        assert_eq!(block_only(&format!("Hallo\n  {BEGIN}\nAAAA\n{END}  \nTschüss")), format!("{BEGIN}\nAAAA\n{END}\n"));
    }

    #[test]
    fn builtin_key_is_readable() {
        assert!(builtin_key().is_ok());
    }

    #[test]
    fn valid_expired_tampered_and_foreign() {
        let k = SigningKey::from_bytes(&[3u8; 32]);
        let pk = k.verifying_key();
        let b = block(&sample("2027-12-31"), &k);
        assert_eq!(status_of(Some(&b), &pk, d("2027-12-31")), Status::Gueltig(sample("2027-12-31")));
        assert_eq!(status_of(Some(&b), &pk, d("2028-01-01")), Status::Abgelaufen(sample("2027-12-31")));
        assert_eq!(status_of(None, &pk, d("2027-01-01")), Status::Fehlt);
        assert_eq!(status_of(Some("  \n"), &pk, d("2027-01-01")), Status::Fehlt);
        // Copy-Paste mit Einrückung und Windows-Zeilenenden.
        let pasted = format!("Lizenz:\r\n{}", b.replace('\n', "\r\n   "));
        assert!(matches!(status_of(Some(&pasted), &pk, d("2027-01-01")), Status::Gueltig(_)));
        // Fremder Schlüssel und verändertes Zeichen: ungültig.
        assert!(matches!(status_of(Some(&b), &SigningKey::from_bytes(&[4u8; 32]).verifying_key(), d("2027-01-01")), Status::Ungueltig(_)));
        let line = b.lines().nth(2).unwrap();
        let mut bad = line.to_string();
        let c = if bad.ends_with('A') { 'B' } else { 'A' };
        bad.pop();
        bad.push(c);
        assert!(matches!(status_of(Some(&b.replacen(line, &bad, 1)), &pk, d("2027-01-01")), Status::Ungueltig(_)));
        assert!(matches!(status_of(Some("kein Block"), &pk, d("2027-01-01")), Status::Ungueltig(_)));
    }

    #[test]
    fn company_is_optional_and_unlimited_is_recognised() {
        let k = SigningKey::from_bytes(&[5u8; 32]);
        let mut l = sample("9999-12-31");
        l.unternehmen.clear();
        // Ohne Unternehmen (wie ältere Lizenzen): das Feld fehlt im signierten Inhalt und bleibt leer.
        let b = block(&l, &k);
        let payload = b64().decode(b.lines().filter(|x| !x.starts_with("-----")).collect::<String>()).unwrap();
        assert!(!String::from_utf8_lossy(&payload[64..]).contains("unternehmen"));
        match status_of(Some(&b), &k.verifying_key(), d("2030-01-01")) {
            Status::Gueltig(g) => assert!(g.unternehmen.is_empty() && unbeschraenkt(&g)),
            other => panic!("{other:?}"),
        }
        assert!(!unbeschraenkt(&sample("2027-12-31")));
    }

    #[test]
    fn modules_per_license_and_legacy_default() {
        let mut l = sample("2030-01-01");
        assert!(hat_modul(&l, "basis") && hat_modul(&l, "kind") && hat_modul(&l, "drittes"), "ältere Lizenz ohne Liste: alle Erweiterungen");
        l.module = vec!["kind".into(), "drittes".into()];
        assert!(hat_modul(&l, "kind") && hat_modul(&l, "drittes") && !hat_modul(&l, "basis"));
        let k = SigningKey::from_bytes(&[2u8; 32]);
        match status_of(Some(&block(&l, &k)), &k.verifying_key(), d("2029-01-01")) {
            Status::Gueltig(g) => assert_eq!(g.module, ["kind", "drittes"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn maintenance_ends_independently() {
        let l = sample("2027-12-31");
        assert!(wartung_aktiv(&l, d("2027-06-30")));
        assert!(!wartung_aktiv(&l, d("2027-07-01")));
    }
}
