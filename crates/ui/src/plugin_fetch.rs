//! Erweiterungen aus der Update-Quelle der Lizenz herunterladen. Je Erweiterung und Plattform liegt dort ein Ordner
//! `/erweiterungen/<kennung>/<plattformordner>/` mit einem Index (`index.json`: alle Fassungen mit Version,
//! Schnittstelle, Mindestversion von remotejoe, Abhängigkeiten, Dateiname), den Paketen
//! `remotejoe-<kennung>-<version>-<plattform>.tar.zst` (Windows `.zip`) und signierten `SHA256SUMS` über Index und
//! Pakete. remotejoe nimmt die **neueste passende** Fassung (Schnittstelle gleich, Mindestversion erfüllt); die
//! Programmversion steht nicht mehr im Pfad. Ein Paket enthält Bibliothek, `manifest.json` und eigene signierte
//! `SHA256SUMS`/`SHA256SUMS.sig`, die der Lader bei jedem Start prüft.
//!
//! Heruntergeladen wird nur mit gültiger Lizenz **und laufender Wartung**: nach der Lizenzeingabe, beim Start
//! (lizenzierte Erweiterung fehlt), über „Erweiterungen aktualisieren“ (nur Neueres) und nach einem Update des
//! Programms (passend zur neuen Version). Nach Ablauf der Wartung bleibt Installiertes nutzbar, solange die Lizenz gilt.
//!
//! Eine geladene Bibliothek wird nie an Ort und Stelle ersetzt: das neue Paket landet in `<id>.neu`, der Tausch
//! geschieht beim nächsten Start vor dem Laden (`swap_pending`).

use std::path::{Path, PathBuf};

use rj_plugin_api::Manifest;
use rj_update::sign::{self, SIG, SUMS};
use rj_update::{Limits, Version};
use serde_json::{json, Value};

use crate::app::App;
use crate::license::ablage;
use crate::plugins::{library_name, plugins_dir};
use crate::sessions::UiEvent;

pub use rj_update::erweiterungen::{Fassung, Index, Katalog, INDEX_FILE, KATALOG_FILE};

/// Name des Pakets einer Erweiterung in der Quelle.
pub fn package_name(id: &str, version: &Version, platform: &str) -> String {
    let ext = if platform.starts_with("windows") { "zip" } else { "tar.zst" };
    format!("remotejoe-{id}-{version}-{platform}.{ext}")
}

/// Ordner einer Erweiterung in der Quelle für diese Plattform.
pub fn quelle_dir(id: &str, platform: &str) -> Result<String, String> {
    Ok(format!("/erweiterungen/{id}/{}/", ablage::ordner_for(platform)?))
}

/// Ordner der Erweiterungen in der Quelle (Katalog).
pub const QUELLE_ERWEITERUNGEN: &str = "/erweiterungen/";

/// Zuletzt aus der Quelle geladener Katalog (`<Datenordner>/plugins/katalog.json`), sonst leer.
pub fn katalog() -> Katalog {
    plugins_dir().and_then(|d| std::fs::read(d.join(KATALOG_FILE)).ok()).and_then(|b| Katalog::parse(&b).ok()).unwrap_or_default()
}

fn katalog_speichern(k: &Katalog) {
    if let Some(d) = plugins_dir() {
        let _ = std::fs::create_dir_all(&d);
        let _ = rj_core::fsutil::atomic_write_with_backup(&d.join(KATALOG_FILE), k.to_text().as_bytes());
    }
}

/// Ordner einer Erweiterung im Datenordner und der Ordner des wartenden Pakets.
pub fn dirs(id: &str) -> Option<(PathBuf, PathBuf)> {
    let base = plugins_dir()?;
    Some((base.join(id), base.join(format!("{id}.neu"))))
}

/// Wartende Pakete (`<id>.neu`) an die Stelle der alten setzen – vor dem Laden, wenn noch nichts geladen ist.
pub fn swap_pending(base: &Path) {
    let Ok(rd) = std::fs::read_dir(base) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(id) = name.strip_suffix(".neu") else { continue };
        if !p.is_dir() {
            continue;
        }
        let target = base.join(id);
        let old = base.join(format!("{id}.alt"));
        let _ = std::fs::remove_dir_all(&old);
        if target.exists() && std::fs::rename(&target, &old).is_err() {
            eprintln!("remotejoe: Erweiterung {id}: alte Fassung nicht beiseitegestellt");
            continue;
        }
        match rename_retry(&p, &target) {
            Ok(()) => {
                let _ = std::fs::remove_dir_all(&old);
            }
            Err(e) => {
                eprintln!("remotejoe: Erweiterung {id}: neue Fassung nicht übernommen: {e}");
                let _ = std::fs::rename(&old, &target);
            }
        }
    }
}

/// Umbenennen mit Wiederholung: unter Windows hält der Virenscanner frisch geschriebene Dateien kurz fest
/// („Zugriff verweigert“), danach klappt es.
fn rename_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for i in 0..10 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(150 * (i + 1)));
            }
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("Umbenennen fehlgeschlagen")))
}

/// Entpackten Ordner einer Erweiterung prüfen (Prüfsummen signiert mit `key`, Bibliothek und Manifest vorhanden
/// und unverändert, Manifest mit der Kennung `id`), ohne die Bibliothek zu laden. Liefert das Manifest.
pub fn verify_dir(dir: &Path, id: &str, key: &sign::VerifyingKey) -> Result<rj_plugin_api::Manifest, String> {
    let lib = library_name(id);
    let sums = std::fs::read(dir.join(SUMS)).map_err(|e| format!("{SUMS}: {e}"))?;
    let sig = std::fs::read(dir.join(SIG)).map_err(|e| format!("{SIG}: {e}"))?;
    let data = std::fs::read(dir.join(&lib)).map_err(|e| format!("{lib}: {e}"))?;
    sign::verify_package(key, &sums, &sig, &lib, &data).map_err(|e| format!("Signatur oder Prüfsumme der Bibliothek: {e}"))?;
    let mtext = std::fs::read(dir.join(rj_plugin_api::MANIFEST_FILE)).map_err(|e| format!("{}: {e}", rj_plugin_api::MANIFEST_FILE))?;
    sign::verify_package(key, &sums, &sig, rj_plugin_api::MANIFEST_FILE, &mtext).map_err(|e| format!("Prüfsumme des Manifests: {e}"))?;
    let m = rj_plugin_api::Manifest::parse(&String::from_utf8_lossy(&mtext))?;
    if m.id != id {
        return Err(format!("Manifest nennt „{}“, erwartet „{id}“", m.id));
    }
    Ok(m)
}

/// Paket `data` (Name `name`) nach `<id>.neu` entpacken und prüfen; bei Fehler bleibt nichts zurück.
pub fn unpack_verified(id: &str, name: &str, data: &[u8], key: &sign::VerifyingKey) -> Result<PathBuf, String> {
    let (_, neu) = dirs(id).ok_or("Kein Datenordner")?;
    let _ = std::fs::remove_dir_all(&neu);
    let r = (|| -> Result<(), String> {
        rj_update::archive::unpack(name, data, &neu).map_err(|e| e.to_string())?;
        verify_dir(&neu, id, key).map(|_| ())
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_dir_all(&neu);
        return Err(e);
    }
    Ok(neu)
}

/// Ist die Erweiterung `id` in diesem Lauf geladen? Dann darf ihr Ordner erst beim nächsten Start ersetzt werden.
fn is_loaded(id: &str) -> bool {
    crate::plugins::loaded().is_loaded(id)
}

/// Manifest der installierten Fassung (Ordner der Erweiterung; ein wartendes Paket zählt als installiert).
pub fn installed_manifest(id: &str) -> Option<Manifest> {
    let (dir, neu) = dirs(id)?;
    for d in [neu, dir] {
        if let Ok(t) = std::fs::read_to_string(d.join(rj_plugin_api::MANIFEST_FILE)) {
            if let Ok(m) = Manifest::parse(&t) {
                return Some(m);
            }
        }
    }
    None
}

/// Erweiterungen mit wartendem Paket (`<id>.neu`), gelten nach dem nächsten Start.
pub fn pending_ids() -> Vec<String> {
    let Some(base) = plugins_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(base) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".neu")).map(str::to_string))
        .filter(|id| dirs(id).is_some_and(|(_, neu)| neu.join(library_name(id)).is_file()))
        .collect()
}

/// Erweiterungen, die laut Lizenz dazugehören (Kennungen der Module). Eine ältere Lizenz ohne Liste nennt alle
/// Erweiterungen, die remotejoe kennt (angemeldete Manifeste).
pub fn licensed_ids() -> Vec<String> {
    crate::license::module_ids()
}

/// Wer den Abgleich mit der Quelle angestoßen hat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncAnlass {
    /// Beim Start: nur fehlende bzw. nicht ladbare Erweiterungen.
    Start,
    /// Nach der Lizenzeingabe (Module aus dem neuen Lizenztext).
    Lizenz,
    /// „Erweiterungen aktualisieren“ bzw. nach einem Programm-Update: nur Neueres.
    Update,
}

/// Wie ein Download entschieden wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMode {
    /// Holen, wenn nichts installiert ist oder die Quelle Neueres hat (Start, Lizenzeingabe).
    Install,
    /// Nur holen, wenn die Quelle Neueres hat als installiert („Erweiterungen aktualisieren“, nach Programm-Update).
    Update,
}

/// Rechner der Quelle für Meldungen („wird heruntergeladen von …“).
fn host_of(url: &str) -> String {
    ablage::parse_link(url).map(|(base, _)| base.trim_start_matches("https://").to_string()).unwrap_or_default()
}

impl App {
    /// Paket der Erweiterung `id` aus der Quelle holen (Zugang aus der Lizenz oder `zugang`; `current` =
    /// Programmversion für die Wahl der Fassung). Ergebnis als `JobResult` „plugin_fetch“ mit `state`: `installed`
    /// (Version, gilt ab dem nächsten Start), `uptodate`, `unavailable` (keine Fassung für diese Plattform bzw.
    /// keinen Index), `incompatible` (Index da, nichts passt zu dieser Programmversion), `error`.
    pub fn plugin_fetch(&mut self, id: &str, zugang: Option<(String, String)>, current: Version, mode: FetchMode) -> Result<Value, String> {
        if self.demo {
            return Err("Vorführung".into());
        }
        let (url, kennwort) = match zugang.or_else(ablage) {
            Some(z) => z,
            None => return Err(crate::license::wartung_grund().unwrap_or_else(|| "Keine gültige Lizenz mit Wartung – Erweiterungen gibt es nur mit Lizenz.".into())),
        };
        if self.plugin_jobs.contains_key(id) {
            return Err(format!("Die Erweiterung „{id}“ wird bereits heruntergeladen."));
        }
        let platform = rj_update::platform();
        let key_text = self.update.key_override.clone().unwrap_or_else(|| crate::update::UPDATE_KEY.to_string());
        let key = sign::public_key_from_text(&key_text).map_err(|e| e.to_string())?;
        rj_update::proxy::set_mode(rj_update::proxy::ProxyMode::from_setting(&self.config.update.proxy));
        let installed = installed_manifest(id).and_then(|m| Version::parse(&m.version).ok());
        let id_s = id.to_string();
        let host = host_of(&url);
        let job = self.jobs.spawn_cancellable(format!("Erweiterung {id} herunterladen"), move |job, cancel| {
            enum Out {
                Installed(String, PathBuf),
                UpToDate(String),
                Unavailable(String),
                Incompatible(String),
            }
            let r = (|| -> Result<Out, String> {
                let s = ablage::Sitzung::anmelden(&url, &kennwort, Some(&*cancel))?;
                let dir = quelle_dir(&id_s, &platform)?;
                // Erst den Ordner listen: eine Erweiterung, die es in der Quelle (noch) nicht gibt, ist kein Fehler.
                // (Für einen fehlenden Ordner liefert die Freigabe keine 404, sondern eine Seite.)
                let has_index = match s.ordner(&dir, Some(&*cancel)) {
                    Ok(e) => e.iter().any(|x| x.name == INDEX_FILE),
                    Err(_) => false,
                };
                if !has_index {
                    return Ok(Out::Unavailable(format!("In der Update-Quelle liegt keine Erweiterung „{id_s}“ für diese Plattform.")));
                }
                let small = Limits { max_bytes: 1024 * 1024, timeout: std::time::Duration::from_secs(60) };
                let index = Index::parse(&ablage::datei_geprueft(&s, &dir, INDEX_FILE, &key, small, Some(&*cancel), None)?)?;
                let Some(f) = index.pick(&current, rj_plugin_api::API_VERSION) else {
                    let newest = index.fassungen.first().map(|f| format!(" (neueste {} braucht remotejoe {} oder neuer, Schnittstelle {})", f.version, f.min_remotejoe, f.api)).unwrap_or_default();
                    return Ok(Out::Incompatible(format!("Keine Fassung der Erweiterung „{id_s}“ passt zu remotejoe {current}{newest}.")));
                };
                let version = Version::parse(&f.version).map_err(|e| e.to_string())?;
                if let Some(inst) = &installed {
                    if inst >= &version || (mode == FetchMode::Update && inst == &version) {
                        return Ok(Out::UpToDate(inst.to_string()));
                    }
                }
                let limits = Limits { max_bytes: 400 * 1024 * 1024, timeout: std::time::Duration::from_secs(30 * 60) };
                let data = ablage::datei_geprueft(&s, &dir, &f.datei, &key, limits, Some(&*cancel), None)?;
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Abgebrochen".into());
                }
                let neu = unpack_verified(&id_s, &f.datei, &data, &key)?;
                let m = installed_manifest(&id_s).ok_or("Manifest im Paket fehlt")?;
                if m.version != f.version {
                    let _ = std::fs::remove_dir_all(&neu);
                    return Err(format!("Das Paket nennt {} , der Index {}", m.version, f.version));
                }
                Ok(Out::Installed(f.version.clone(), neu))
            })();
            Box::new(move |app: &mut App, cancelled| {
                app.plugin_jobs.remove(&id_s);
                if cancelled {
                    if let Some((_, neu)) = dirs(&id_s) {
                        let _ = std::fs::remove_dir_all(neu);
                    }
                    return Vec::new();
                }
                let json = match r {
                    Ok(Out::Installed(version, neu)) => {
                        // Noch nichts geladen (z. B. erste Lizenz): sofort an Ort und Stelle; sonst beim nächsten Start.
                        let swapped = !is_loaded(&id_s) && {
                            swap_pending(neu.parent().unwrap_or(&neu));
                            dirs(&id_s).is_some_and(|(d, _)| d.join(library_name(&id_s)).is_file())
                        };
                        json!({ "state": "installed", "id": id_s, "version": version, "swapped": swapped, "neustart": true })
                    }
                    Ok(Out::UpToDate(version)) => json!({ "state": "uptodate", "id": id_s, "version": version }),
                    Ok(Out::Unavailable(why)) => json!({ "state": "unavailable", "id": id_s, "error": why }),
                    Ok(Out::Incompatible(why)) => json!({ "state": "incompatible", "id": id_s, "error": why }),
                    Err(e) => json!({ "state": "error", "id": id_s, "error": e }),
                };
                app.plugin_fetch_last.insert(id_s.clone(), json.clone());
                vec![UiEvent::JobResult { job, kind: "plugin_fetch".into(), target: String::new(), json: json.to_string() }]
            })
        });
        self.plugin_jobs.insert(id.to_string(), job);
        Ok(json!({ "job": job, "id": id, "host": host }))
    }

    /// Lizenzierte Erweiterungen, deren Abhängigkeiten ebenfalls lizenziert sind (eine Lizenz mit einer
    /// Hersteller-Erweiterung ohne ihre Oberfunktion wird gemeldet, nicht still übergangen). `ids`: Module der Lizenz.
    fn fetch_candidates(ids: &[String]) -> (Vec<String>, Vec<(String, String)>) {
        let manifests = crate::plugins::manifests();
        let mut ok = Vec::new();
        let mut bad = Vec::new();
        for id in ids {
            let missing: Vec<&String> = manifests.iter().find(|m| &m.id == id).map(|m| m.braucht.iter().filter(|b| !ids.contains(b)).collect()).unwrap_or_default();
            if missing.is_empty() {
                ok.push(id.clone());
            } else {
                bad.push((id.clone(), format!("braucht {}, das die Lizenz nicht enthält", missing.iter().map(|b| format!("„{b}“")).collect::<Vec<_>>().join(", "))));
            }
        }
        (ok, bad)
    }

    /// Abgleich mit der Quelle als ein Auftrag: Katalog laden (für Lizenzen ohne Modulliste und für Namen), dann je
    /// lizenzierter Erweiterung, deren Abhängigkeiten ebenfalls lizenziert sind, einen Download-Auftrag starten.
    /// Erweiterungen mit fehlender Oberfunktion in der Lizenz erscheinen als Fehler auf der Lizenzseite.
    fn plugins_sync(&mut self, zugang: (String, String), current: Version, anlass: SyncAnlass, lizenz_ids: Vec<String>) -> Result<Value, String> {
        if self.demo {
            return Err("Vorführung".into());
        }
        let key_text = self.update.key_override.clone().unwrap_or_else(|| crate::update::UPDATE_KEY.to_string());
        let key = sign::public_key_from_text(&key_text).map_err(|e| e.to_string())?;
        rj_update::proxy::set_mode(rj_update::proxy::ProxyMode::from_setting(&self.config.update.proxy));
        let host = host_of(&zugang.0);
        let (url, kennwort) = zugang.clone();
        let job = self.jobs.spawn_cancellable("Erweiterungen mit der Update-Quelle abgleichen".to_string(), move |_job, cancel| {
            // Katalog: fehlt er in der Quelle, bleibt der zuletzt geladene.
            let k = (|| -> Result<Option<Katalog>, String> {
                let s = ablage::Sitzung::anmelden(&url, &kennwort, Some(&*cancel))?;
                if !s.ordner(QUELLE_ERWEITERUNGEN, Some(&*cancel)).map(|e| e.iter().any(|x| x.name == KATALOG_FILE)).unwrap_or(false) {
                    return Ok(None);
                }
                let small = Limits { max_bytes: 1024 * 1024, timeout: std::time::Duration::from_secs(60) };
                Ok(Some(Katalog::parse(&ablage::datei_geprueft(&s, QUELLE_ERWEITERUNGEN, KATALOG_FILE, &key, small, Some(&*cancel), None)?)?))
            })();
            Box::new(move |app: &mut App, cancelled| {
                if cancelled {
                    return Vec::new();
                }
                if let Ok(Some(k)) = &k {
                    katalog_speichern(k);
                }
                let ids = if lizenz_ids.is_empty() { katalog().ids() } else { lizenz_ids };
                let (ok, bad) = App::fetch_candidates(&ids);
                let mut events = Vec::new();
                for (id, e) in bad {
                    let json = json!({ "state": "error", "id": id, "error": e });
                    app.plugin_fetch_last.insert(id, json.clone());
                    events.push(UiEvent::JobResult { job: 0, kind: "plugin_fetch".into(), target: String::new(), json: json.to_string() });
                }
                let pending = pending_ids();
                for id in ok {
                    if anlass == SyncAnlass::Start && (is_loaded(&id) || pending.contains(&id)) {
                        continue;
                    }
                    let mode = if anlass == SyncAnlass::Update { FetchMode::Update } else { FetchMode::Install };
                    if let Err(e) = app.plugin_fetch(&id, Some(zugang.clone()), current.clone(), mode) {
                        let json = json!({ "state": "error", "id": id, "error": e });
                        app.plugin_fetch_last.insert(id, json.clone());
                        events.push(UiEvent::JobResult { job: 0, kind: "plugin_fetch".into(), target: String::new(), json: json.to_string() });
                    }
                }
                if let Err(e) = &k {
                    eprintln!("remotejoe: Katalog der Erweiterungen nicht geladen: {e}");
                }
                events
            })
        });
        Ok(json!({ "job": job, "quelle": host, "gestartet": [ { "job": job } ], "fehler": [] }))
    }

    /// Beim Start: lizenzierte Erweiterungen holen, die fehlen oder nicht geladen werden konnten (Lizenz mit Wartung
    /// vorausgesetzt; Wartendes zählt als da).
    pub fn plugin_fetch_auto(&mut self) {
        if self.plugin_auto_done || self.demo {
            return;
        }
        self.plugin_auto_done = true;
        let Some(zugang) = ablage() else { return };
        let Ok(current) = Version::parse(rj_update::CURRENT_VERSION) else { return };
        let _ = self.plugins_sync(zugang, current, SyncAnlass::Start, licensed_ids());
    }

    /// Nach der Lizenzeingabe: alle Erweiterungen dieser Lizenz holen (Abgleich als Auftrag).
    pub fn plugin_fetch_after_license(&mut self, text: &str) -> Value {
        let Some(zugang) = crate::license::zugang_aus(text) else { return json!({ "gestartet": [], "quelle": null }) };
        let Ok(current) = Version::parse(rj_update::CURRENT_VERSION) else { return Value::Null };
        let ids = crate::license::module_ids_aus(text);
        self.plugins_sync(zugang, current, SyncAnlass::Lizenz, ids).unwrap_or_else(|e| json!({ "gestartet": [], "fehler": [ { "id": "", "error": e } ] }))
    }

    /// Nach einem Update des Programms: Erweiterungen passend zur neuen Version holen (liegen bis zum Neustart in
    /// `<id>.neu`), nur Neueres.
    pub fn plugin_fetch_for_update(&mut self, version: &Version) {
        if let Some(zugang) = ablage() {
            let _ = self.plugins_sync(zugang, version.clone(), SyncAnlass::Update, licensed_ids());
        }
    }

    /// „Erweiterungen aktualisieren“ (Lizenzseite): alle lizenzierten Erweiterungen gegen die Quelle prüfen, nur
    /// Neueres holen. Ohne laufende Wartung ein Fehler mit Grund.
    pub fn plugins_update(&mut self) -> Result<Value, String> {
        if let Some(grund) = crate::license::wartung_grund() {
            return Err(grund);
        }
        let zugang = ablage().ok_or("Keine Update-Quelle in der Lizenz")?;
        let current = Version::parse(rj_update::CURRENT_VERSION).map_err(|e| e.to_string())?;
        self.plugins_sync(zugang, current, SyncAnlass::Update, licensed_ids())
    }

    /// Läuft gerade ein Download einer Erweiterung? Kennungen.
    pub fn plugin_fetch_running(&self) -> Vec<String> {
        self.plugin_jobs.keys().cloned().collect()
    }

    /// Letzte Ergebnisse je Erweiterung in diesem Lauf (Lizenzseite).
    pub fn plugin_fetch_results(&self) -> Value {
        json!(self.plugin_fetch_last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_and_source_folders_follow_the_platform() {
        let v = Version::parse("1.0.0").unwrap();
        assert_eq!(package_name("kind", &v, "linux-x86_64"), "remotejoe-kind-1.0.0-linux-x86_64.tar.zst");
        assert_eq!(package_name("kind", &v, "linux-x86_64-ubuntu2404"), "remotejoe-kind-1.0.0-linux-x86_64-ubuntu2404.tar.zst");
        assert_eq!(package_name("kind", &v, "windows-x86_64"), "remotejoe-kind-1.0.0-windows-x86_64.zip");
        assert_eq!(quelle_dir("kind", "windows-x86_64").unwrap(), "/erweiterungen/kind/windows/");
        assert_eq!(quelle_dir("basis", "linux-x86_64").unwrap(), "/erweiterungen/basis/ubuntu2604/");
        assert!(quelle_dir("basis", "freebsd").is_err());
    }

    /// Ein signierter Ordner wird angenommen, ein veränderter oder fremd signierter nicht; `swap_pending` setzt das
    /// wartende Paket an die Stelle des alten und räumt auf.
    #[test]
    fn verify_and_swap_pending() {
        let sk = sign::generate().unwrap();
        let base = tempfile::tempdir().unwrap();
        let mk = |dir: &Path, bytes: &[u8], key: &sign::SigningKey| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(library_name("netz")), bytes).unwrap();
            let m = r#"{"id":"netz","name":"Netz","version":"1.0.0","api":1,"min_remotejoe":"0.1.60"}"#;
            std::fs::write(dir.join(rj_plugin_api::MANIFEST_FILE), m).unwrap();
            let sums = sign::sums_text(&[(library_name("netz"), sign::sha256_hex(bytes)), (rj_plugin_api::MANIFEST_FILE.into(), sign::sha256_hex(m.as_bytes()))]);
            std::fs::write(dir.join(SUMS), &sums).unwrap();
            std::fs::write(dir.join(SIG), sign::sign(key, sums.as_bytes())).unwrap();
        };
        let old = base.path().join("netz");
        let neu = base.path().join("netz.neu");
        mk(&old, b"alt", &sk);
        mk(&neu, b"neu", &sk);
        assert_eq!(verify_dir(&old, "netz", &sk.verifying_key()).unwrap().version, "1.0.0");
        assert!(verify_dir(&neu, "netz", &sk.verifying_key()).is_ok());
        assert!(verify_dir(&neu, "anders", &sk.verifying_key()).is_err(), "falsche Kennung");
        let other = sign::generate().unwrap();
        assert!(verify_dir(&neu, "netz", &other.verifying_key()).unwrap_err().contains("Signatur"));
        std::fs::write(old.join(library_name("netz")), b"alt!").unwrap();
        assert!(verify_dir(&old, "netz", &sk.verifying_key()).is_err(), "veränderte Bibliothek");
        swap_pending(base.path());
        assert_eq!(std::fs::read(old.join(library_name("netz"))).unwrap(), b"neu");
        assert!(!neu.exists() && !base.path().join("netz.alt").exists());
        // Paket entpacken und prüfen: Archiv mit Ordnerinhalt.
        let src = tempfile::tempdir().unwrap();
        mk(src.path(), b"paket", &sk);
        let data = rj_update::archive::pack_tar_zst(src.path()).unwrap();
        let dest = tempfile::tempdir().unwrap();
        rj_update::archive::unpack("x.tar.zst", &data, dest.path()).unwrap();
        assert!(verify_dir(dest.path(), "netz", &sk.verifying_key()).is_ok());
    }
}
