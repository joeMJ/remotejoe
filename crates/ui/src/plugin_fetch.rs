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
use std::sync::atomic::AtomicBool;

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

/// Releasenotes der Erweiterungen in der Quelle (neben dem Katalog, über die signierten `SHA256SUMS` geprüft) und im
/// Datenordner (`<Datenordner>/plugins/RELEASENOTES-ERWEITERUNGEN.md`).
pub const NOTES_FILE: &str = "RELEASENOTES-ERWEITERUNGEN.md";

/// Zuletzt geladene Releasenotes der Erweiterungen (Markdown), sonst leer.
pub fn notes_text() -> String {
    plugins_dir().and_then(|d| std::fs::read(d.join(NOTES_FILE)).ok()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

/// Kennung des neuesten Eintrags (erste Überschrift „## …“), für „schon gesehen“.
pub fn notes_head(text: &str) -> String {
    text.lines().find_map(|l| l.strip_prefix("## ")).map(|h| h.trim().to_string()).unwrap_or_default()
}

fn notes_speichern(b: &[u8]) {
    if let Some(d) = plugins_dir() {
        let _ = std::fs::create_dir_all(&d);
        let _ = rj_core::fsutil::atomic_write_with_backup(&d.join(NOTES_FILE), b);
    }
}

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

/// Zustand des Absturzwächters für die Oberfläche: `aktiv` (dieser Start ohne Erweiterungen), `grund` („absturz“
/// bzw. „schalter“), `zuletzt` (Namen der zuletzt geladenen Erweiterungen) und `ordner` (Ordner der Erweiterungen,
/// für den Notfall-Hinweis).
pub fn safe_mode_json() -> Value {
    let ordner = plugins_dir().map(|d| d.display().to_string()).unwrap_or_default();
    match crate::plugins::safe_mode() {
        None => json!({ "aktiv": false, "ordner": ordner }),
        Some(crate::plugins::SafeMode::Requested) => json!({ "aktiv": true, "grund": "schalter", "zuletzt": [], "ordner": ordner }),
        Some(crate::plugins::SafeMode::Crash(ids)) => {
            let k = katalog();
            let names: Vec<String> = ids.iter().map(|id| k.name_of(id).map(str::to_string).unwrap_or_else(|| id.clone())).collect();
            json!({ "aktiv": true, "grund": "absturz", "zuletzt": names, "ordner": ordner })
        }
    }
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

/// Zwischenstand einer Erweiterung nach dem Laden aus der Quelle (noch nichts eingespielt).
#[derive(Debug)]
enum Geladen {
    /// Neuere passende Fassung geladen und geprüft (Signatur, Prüfsumme), liegt im Speicher.
    Neu { version: String, datei: String, data: Vec<u8> },
    Aktuell(String),
    /// Keine Erweiterung dieser Kennung für diese Plattform in der Quelle.
    Fehlt(String),
    /// Index da, aber keine Fassung passt zu dieser Programmversion.
    Passt(String),
    Fehler(String),
}

/// Endergebnis einer Erweiterung.
#[derive(Debug)]
enum Out {
    Installed(String, PathBuf),
    UpToDate(String),
    Unavailable(String),
    Incompatible(String),
    Error(String),
}

/// Zusammengehörige Erweiterungen: verbunden über `braucht` (in beide Richtungen). Reihenfolge wie in `ids`.
fn gruppen(ids: &[String], braucht: &std::collections::HashMap<String, Vec<String>>) -> Vec<Vec<String>> {
    let mut gruppe: Vec<usize> = (0..ids.len()).collect();
    fn wurzel(g: &mut [usize], mut i: usize) -> usize {
        while g[i] != i {
            g[i] = g[g[i]];
            i = g[i];
        }
        i
    }
    for (i, id) in ids.iter().enumerate() {
        for b in braucht.get(id).into_iter().flatten() {
            if let Some(j) = ids.iter().position(|x| x == b) {
                let (a, c) = (wurzel(&mut gruppe, i), wurzel(&mut gruppe, j));
                gruppe[a] = c;
            }
        }
    }
    let mut out: Vec<(usize, Vec<String>)> = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let w = wurzel(&mut gruppe, i);
        match out.iter_mut().find(|(r, _)| *r == w) {
            Some((_, v)) => v.push(id.clone()),
            None => out.push((w, vec![id.clone()])),
        }
    }
    out.into_iter().map(|(_, v)| v).collect()
}

/// Alle oder keine: Ist in einer Gruppe zusammengehöriger Erweiterungen eine gescheitert (Fehler, keine passende
/// Fassung oder installiert, aber nicht mehr in der Quelle), wird keine neue Fassung der Gruppe eingespielt – der
/// bisherige Stand aller bleibt.
fn zurueckhalten(stand: &mut [(String, Geladen)], braucht: &std::collections::HashMap<String, Vec<String>>, installiert: &dyn Fn(&str) -> bool) {
    let ids: Vec<String> = stand.iter().map(|(id, _)| id.clone()).collect();
    for g in gruppen(&ids, braucht) {
        let gescheitert: Vec<String> = stand
            .iter()
            .filter(|(id, s)| g.contains(id) && (matches!(s, Geladen::Fehler(_) | Geladen::Passt(_)) || (matches!(s, Geladen::Fehlt(_)) && installiert(id))))
            .map(|(id, _)| format!("„{id}“"))
            .collect();
        if gescheitert.is_empty() {
            continue;
        }
        for (id, s) in stand.iter_mut().filter(|(id, _)| g.contains(id)) {
            if let Geladen::Neu { version, .. } = s {
                *s = Geladen::Fehler(format!(
                    "Fassung {version} von „{id}“ nicht eingespielt: gehört mit {} zusammen, das nicht geladen werden konnte. Der bisherige Stand bleibt.",
                    gescheitert.join(", ")
                ));
            }
        }
    }
}

/// Geladene Fassungen einspielen (`unpack(id, version, datei, data)` entpackt und prüft nach `<id>.neu`, `remove` entfernt so einen Ordner).
/// Scheitert das Einspielen einer Erweiterung, werden die schon eingespielten ihrer Gruppe wieder entfernt und
/// die übrigen nicht mehr eingespielt.
fn einspielen(
    stand: Vec<(String, Geladen)>,
    braucht: &std::collections::HashMap<String, Vec<String>>,
    unpack: &dyn Fn(&str, &str, &str, &[u8]) -> Result<PathBuf, String>,
    remove: &dyn Fn(&Path),
) -> Vec<(String, Out)> {
    let ids: Vec<String> = stand.iter().map(|(id, _)| id.clone()).collect();
    let mut neu: std::collections::HashMap<String, (String, String, Vec<u8>)> = std::collections::HashMap::new();
    let mut out: Vec<(String, Option<Out>)> = Vec::new();
    for (id, s) in stand {
        let o = match s {
            Geladen::Neu { version, datei, data } => {
                neu.insert(id.clone(), (version, datei, data));
                None
            }
            Geladen::Aktuell(v) => Some(Out::UpToDate(v)),
            Geladen::Fehlt(w) => Some(Out::Unavailable(w)),
            Geladen::Passt(w) => Some(Out::Incompatible(w)),
            Geladen::Fehler(e) => Some(Out::Error(e)),
        };
        out.push((id, o));
    }
    let mut set = |id: &str, o: Out| {
        if let Some((_, slot)) = out.iter_mut().find(|(x, _)| x == id) {
            *slot = Some(o);
        }
    };
    for g in gruppen(&ids, braucht) {
        let mut done: Vec<(String, String, PathBuf)> = Vec::new();
        let mut failed: Option<String> = None;
        for id in &g {
            let Some((version, datei, data)) = neu.remove(id) else { continue };
            if let Some(f) = &failed {
                set(id, Out::Error(format!("Fassung {version} von „{id}“ nicht eingespielt: gehört mit „{f}“ zusammen, das sich nicht einspielen ließ. Der bisherige Stand bleibt.")));
                continue;
            }
            match unpack(id, &version, &datei, &data) {
                Ok(dir) => done.push((id.clone(), version, dir)),
                Err(e) => {
                    set(id, Out::Error(e));
                    failed = Some(id.clone());
                }
            }
        }
        for (id, version, dir) in done {
            match &failed {
                Some(f) => {
                    remove(&dir);
                    set(&id, Out::Error(format!("Fassung {version} von „{id}“ wieder herausgenommen: gehört mit „{f}“ zusammen, das sich nicht einspielen ließ. Der bisherige Stand bleibt.")));
                }
                None => set(&id, Out::Installed(version, dir)),
            }
        }
    }
    out.into_iter().map(|(id, o)| (id, o.unwrap_or_else(|| Out::Error("nicht eingespielt".into())))).collect()
}

/// Kennungen aus `braucht` (ohne Mindestversion).
fn dep_ids(braucht: &[String]) -> Vec<String> {
    braucht.iter().map(|b| rj_plugin_api::dependency(b).0.to_string()).collect()
}

/// Eine Erweiterung aus der Quelle laden und prüfen (Index, Wahl der Fassung, Paket mit Signatur und Prüfsumme),
/// ohne einzuspielen. Liefert dazu, was die gewählte Fassung braucht.
fn laden(s: &ablage::Sitzung, id: &str, platform: &str, key: &sign::VerifyingKey, current: &Version, mode: FetchMode, cancel: &AtomicBool) -> (Geladen, Vec<String>) {
    let r = (|| -> Result<(Geladen, Vec<String>), String> {
        let dir = quelle_dir(id, platform)?;
        // Erst den Ordner listen: eine Erweiterung, die es in der Quelle (noch) nicht gibt, ist kein Fehler; ein
        // Netzfehler beim Auflisten schon.
        let has_index = s.ordner(&dir, Some(cancel))?.is_some_and(|e| e.iter().any(|x| x.name == INDEX_FILE));
        if !has_index {
            return Ok((Geladen::Fehlt(format!("In der Update-Quelle liegt keine Erweiterung „{id}“ für diese Plattform.")), Vec::new()));
        }
        let small = Limits { max_bytes: 1024 * 1024, timeout: std::time::Duration::from_secs(60) };
        let index = Index::parse(&ablage::datei_geprueft(s, &dir, INDEX_FILE, key, small, Some(cancel), None)?)?;
        let Some(f) = index.pick(current, rj_plugin_api::API_VERSION) else {
            let newest = index.fassungen.first().map(|f| format!(" (neueste {} braucht remotejoe {} oder neuer, Schnittstelle {})", f.version, f.min_remotejoe, f.api)).unwrap_or_default();
            return Ok((Geladen::Passt(format!("Keine Fassung der Erweiterung „{id}“ passt zu remotejoe {current}{newest}.")), Vec::new()));
        };
        let version = Version::parse(&f.version).map_err(|e| e.to_string())?;
        if let Some(inst) = installed_manifest(id).and_then(|m| Version::parse(&m.version).ok()) {
            if inst >= version || (mode == FetchMode::Update && inst == version) {
                return Ok((Geladen::Aktuell(inst.to_string()), dep_ids(&f.braucht)));
            }
        }
        let limits = Limits { max_bytes: 400 * 1024 * 1024, timeout: std::time::Duration::from_secs(30 * 60) };
        let data = ablage::datei_geprueft(s, &dir, &f.datei, key, limits, Some(cancel), None)?;
        Ok((Geladen::Neu { version: f.version.clone(), datei: f.datei.clone(), data }, dep_ids(&f.braucht)))
    })();
    r.unwrap_or_else(|e| (Geladen::Fehler(e), Vec::new()))
}

/// Ergebnis einer Erweiterung als JSON für die Oberfläche (`JobResult` „plugin_fetch“) mit `state`: `installed`
/// (Version, gilt ab dem nächsten Start), `uptodate`, `unavailable` (keine Fassung für diese Plattform bzw. kein
/// Index), `incompatible` (Index da, nichts passt zu dieser Programmversion), `error`.
fn result_json(id: &str, o: Out) -> Value {
    match o {
        Out::Installed(version, neu) => {
            // Noch nichts geladen (z. B. erste Lizenz): sofort an Ort und Stelle; sonst beim nächsten Start.
            let swapped = !is_loaded(id) && {
                swap_pending(neu.parent().unwrap_or(&neu));
                dirs(id).is_some_and(|(d, _)| d.join(library_name(id)).is_file())
            };
            json!({ "state": "installed", "id": id, "version": version, "swapped": swapped, "neustart": true })
        }
        Out::UpToDate(version) => json!({ "state": "uptodate", "id": id, "version": version }),
        Out::Unavailable(why) => json!({ "state": "unavailable", "id": id, "error": why }),
        Out::Incompatible(why) => json!({ "state": "incompatible", "id": id, "error": why }),
        Out::Error(e) => json!({ "state": "error", "id": id, "error": e }),
    }
}

impl App {
    /// Lizenzierte Erweiterungen, deren Abhängigkeiten ebenfalls lizenziert sind (eine Lizenz mit einer
    /// Hersteller-Erweiterung ohne ihre Oberfunktion wird gemeldet, nicht still übergangen). `ids`: Module der Lizenz.
    fn fetch_candidates(ids: &[String]) -> (Vec<String>, Vec<(String, String)>) {
        let manifests = crate::plugins::manifests();
        let mut ok = Vec::new();
        let mut bad = Vec::new();
        for id in ids {
            let missing: Vec<String> = manifests.iter().find(|m| &m.id == id).map(|m| m.braucht_ids().into_iter().filter(|b| !ids.contains(b)).collect()).unwrap_or_default();
            if missing.is_empty() {
                ok.push(id.clone());
            } else {
                bad.push((id.clone(), format!("braucht {}, das die Lizenz nicht enthält", missing.iter().map(|b| format!("„{b}“")).collect::<Vec<_>>().join(", "))));
            }
        }
        (ok, bad)
    }

    /// Erweiterungen, die dieser Abgleich holen soll; beim Start nur fehlende bzw. nicht geladene.
    fn kandidaten(ids: &[String], anlass: SyncAnlass) -> (Vec<String>, Vec<(String, String)>) {
        let (mut ok, bad) = App::fetch_candidates(ids);
        if anlass == SyncAnlass::Start {
            let pending = pending_ids();
            ok.retain(|id| !is_loaded(id) && !pending.contains(id));
        }
        (ok, bad)
    }

    /// Abgleich mit der Quelle als **ein** Auftrag mit einer Anmeldung: Katalog laden (für Lizenzen ohne Modulliste
    /// und für Namen), dann die lizenzierten Erweiterungen nacheinander laden und prüfen. Eingespielt wird erst danach
    /// und nur gemeinsam: Scheitert eine von mehreren zusammengehörigen Erweiterungen, bleibt der alte Stand aller
    /// (siehe [`zurueckhalten`], [`einspielen`]). Erweiterungen mit fehlender Oberfunktion in der Lizenz erscheinen
    /// als Fehler auf der Lizenzseite.
    fn plugins_sync(&mut self, zugang: (String, String), current: Version, anlass: SyncAnlass, lizenz_ids: Vec<String>) -> Result<Value, String> {
        if self.demo {
            return Err("Vorführung".into());
        }
        if !self.plugin_jobs.is_empty() {
            return Err("Die Erweiterungen werden bereits heruntergeladen.".into());
        }
        let key_text = self.update.key_override.clone().unwrap_or_else(|| crate::update::UPDATE_KEY.to_string());
        let key = sign::public_key_from_text(&key_text).map_err(|e| e.to_string())?;
        rj_update::proxy::set_mode(rj_update::proxy::ProxyMode::from_setting(&self.config.update.proxy));
        let host = host_of(&zugang.0);
        let (url, kennwort) = zugang;
        let platform = rj_update::platform();
        let mode = if anlass == SyncAnlass::Update { FetchMode::Update } else { FetchMode::Install };
        // Voraussichtliche Erweiterungen (für „wird heruntergeladen“ und Meldungen, falls schon die Anmeldung scheitert).
        let vorab = App::kandidaten(&if lizenz_ids.is_empty() { katalog().ids() } else { lizenz_ids.clone() }, anlass).0;
        let laufend = vorab.clone();
        let job = self.jobs.spawn_cancellable("Erweiterungen mit der Update-Quelle abgleichen".to_string(), move |job, cancel| {
            let r = (|| -> Result<(Vec<(String, Out)>, Vec<(String, String)>), String> {
                let s = ablage::Sitzung::anmelden(&url, &kennwort, Some(&*cancel))?;
                // Katalog: fehlt er in der Quelle oder scheitert er, bleibt der zuletzt geladene. Ebenso die Releasenotes
                // der Erweiterungen (nur mit gültiger Signatur übernommen).
                let liste = s.ordner(QUELLE_ERWEITERUNGEN, Some(&*cancel));
                let small = Limits { max_bytes: 1024 * 1024, timeout: std::time::Duration::from_secs(60) };
                let k = (|| -> Result<Option<Katalog>, String> {
                    if !liste.as_ref().map_err(Clone::clone)?.as_ref().is_some_and(|e| e.iter().any(|x| x.name == KATALOG_FILE)) {
                        return Ok(None);
                    }
                    Ok(Some(Katalog::parse(&ablage::datei_geprueft(&s, QUELLE_ERWEITERUNGEN, KATALOG_FILE, &key, small, Some(&*cancel), None)?)?))
                })();
                if liste.as_ref().ok().and_then(|l| l.as_ref()).is_some_and(|e| e.iter().any(|x| x.name == NOTES_FILE)) {
                    match ablage::datei_geprueft(&s, QUELLE_ERWEITERUNGEN, NOTES_FILE, &key, small, Some(&*cancel), None) {
                        Ok(b) => notes_speichern(&b),
                        Err(e) => eprintln!("remotejoe: Releasenotes der Erweiterungen nicht geladen: {e}"),
                    }
                }
                match &k {
                    Ok(Some(k)) => katalog_speichern(k),
                    Ok(None) => {}
                    Err(e) => eprintln!("remotejoe: Katalog der Erweiterungen nicht geladen: {e}"),
                }
                let ids = if lizenz_ids.is_empty() { katalog().ids() } else { lizenz_ids };
                let (ok, bad) = App::kandidaten(&ids, anlass);
                let mut stand = Vec::new();
                let mut braucht = std::collections::HashMap::new();
                for id in &ok {
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err("Abgebrochen".into());
                    }
                    let (g, b) = laden(&s, id, &platform, &key, &current, mode, &cancel);
                    let mut deps = installed_manifest(id).map(|m| m.braucht_ids()).unwrap_or_default();
                    deps.extend(b);
                    braucht.insert(id.clone(), deps);
                    stand.push((id.clone(), g));
                }
                eprintln!("remotejoe: Erweiterungen mit der Update-Quelle abgeglichen ({} Verbindung(en))", s.verbindungen());
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Abgebrochen".into());
                }
                zurueckhalten(&mut stand, &braucht, &|id| installed_manifest(id).is_some());
                let unpack = |id: &str, version: &str, datei: &str, data: &[u8]| -> Result<PathBuf, String> {
                    let neu = unpack_verified(id, datei, data, &key)?;
                    let m = installed_manifest(id).ok_or("Manifest im Paket fehlt")?;
                    if m.version != version {
                        let _ = std::fs::remove_dir_all(&neu);
                        return Err(format!("Das Paket nennt {}, der Index {version}", m.version));
                    }
                    Ok(neu)
                };
                Ok((einspielen(stand, &braucht, &unpack, &|d| {
                    let _ = std::fs::remove_dir_all(d);
                }), bad))
            })();
            Box::new(move |app: &mut App, cancelled| {
                app.plugin_jobs.clear();
                if cancelled {
                    if let Ok((out, _)) = &r {
                        for (_, o) in out {
                            if let Out::Installed(_, neu) = o {
                                let _ = std::fs::remove_dir_all(neu);
                            }
                        }
                    }
                    return Vec::new();
                }
                let (out, bad) = match r {
                    Ok(x) => x,
                    // Schon Anmeldung oder Abbruch gescheitert: Meldung bei jeder erwarteten Erweiterung.
                    Err(e) => {
                        eprintln!("remotejoe: Erweiterungen nicht abgeglichen: {e}");
                        (vorab.into_iter().map(|id| (id, Out::Error(e.clone()))).collect(), Vec::new())
                    }
                };
                let mut events = Vec::new();
                let all = bad.into_iter().map(|(id, e)| (id, Out::Error(e))).chain(out);
                for (id, o) in all {
                    let json = result_json(&id, o);
                    app.plugin_fetch_last.insert(id, json.clone());
                    events.push(UiEvent::JobResult { job, kind: "plugin_fetch".into(), target: String::new(), json: json.to_string() });
                }
                events
            })
        });
        for id in laufend {
            self.plugin_jobs.insert(id, job);
        }
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
    fn notes_head_is_the_newest_heading() {
        assert_eq!(notes_head("## Erweiterung A 1.3.1 – Neu\n\n- a\n\n## Erweiterung A 1.3.0\n"), "Erweiterung A 1.3.1 – Neu");
        assert_eq!(notes_head("ohne Überschrift"), "");
    }

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

    fn neu(v: &str) -> Geladen {
        Geladen::Neu { version: v.into(), datei: format!("paket-{v}"), data: v.as_bytes().to_vec() }
    }

    fn deps(list: &[(&str, &[&str])]) -> std::collections::HashMap<String, Vec<String>> {
        list.iter().map(|(id, b)| (id.to_string(), b.iter().map(|x| x.to_string()).collect())).collect()
    }

    /// Gruppen über `braucht` in beide Richtungen, auch über mehrere Stufen; Unabhängige bleiben allein.
    #[test]
    fn groups_follow_dependencies() {
        let ids: Vec<String> = ["basis", "netz", "kind", "allein"].iter().map(|s| s.to_string()).collect();
        let g = gruppen(&ids, &deps(&[("netz", &["basis"]), ("kind", &["netz"]), ("allein", &[])]));
        assert_eq!(g, vec![vec!["basis".to_string(), "netz".into(), "kind".into()], vec!["allein".to_string()]]);
    }

    /// Scheitert eine zusammengehörige Erweiterung (Fehler, keine passende Fassung, installiert aber nicht mehr in
    /// der Quelle), wird die neue Fassung der anderen nicht eingespielt; Unabhängige und Aktuelle bleiben unberührt.
    #[test]
    fn all_or_nothing_holds_back_partners() {
        let b = deps(&[("netz", &["basis"])]);
        for failure in [Geladen::Fehler("Zeitüberschreitung".into()), Geladen::Passt("zu neu".into()), Geladen::Fehlt("fehlt".into())] {
            let mut st = vec![("basis".to_string(), neu("1.3.0")), ("netz".to_string(), failure), ("allein".to_string(), neu("2.0.0"))];
            zurueckhalten(&mut st, &b, &|_| true);
            assert!(matches!(&st[0].1, Geladen::Fehler(e) if e.contains("„netz“") && e.contains("bisherige Stand")), "{:?}", st[0].1);
            assert!(matches!(st[2].1, Geladen::Neu { .. }), "unabhängige Erweiterung");
        }
        // Nicht installiert und nicht in der Quelle: kein Grund, den Partner zurückzuhalten.
        let mut st = vec![("basis".to_string(), neu("1.3.0")), ("netz".to_string(), Geladen::Fehlt("fehlt".into()))];
        zurueckhalten(&mut st, &b, &|id| id == "basis");
        assert!(matches!(st[0].1, Geladen::Neu { .. }));
        // Partner aktuell: neue Fassung darf kommen.
        let mut st = vec![("basis".to_string(), neu("1.3.0")), ("netz".to_string(), Geladen::Aktuell("1.2.0".into()))];
        zurueckhalten(&mut st, &b, &|_| true);
        assert!(matches!(st[0].1, Geladen::Neu { .. }));
    }

    /// Einspielen: Die erste Erweiterung gelingt, die zweite derselben Gruppe scheitert beim Entpacken – die erste
    /// wird wieder herausgenommen, der alte Stand beider bleibt. Eine unabhängige wird eingespielt.
    #[test]
    fn failed_unpack_removes_the_group() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().to_path_buf();
        let unpack = |id: &str, version: &str, _datei: &str, _data: &[u8]| -> Result<PathBuf, String> {
            if id == "netz" {
                return Err("Signatur oder Prüfsumme der Bibliothek: falsch".into());
            }
            let d = root.join(format!("{id}.neu"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("version"), version).unwrap();
            Ok(d)
        };
        let remove = |d: &Path| std::fs::remove_dir_all(d).unwrap();
        let st = vec![("basis".to_string(), neu("1.3.0")), ("netz".to_string(), neu("1.3.0")), ("allein".to_string(), neu("2.0.0")), ("rest".to_string(), Geladen::Aktuell("1.0.0".into()))];
        let out = einspielen(st, &deps(&[("netz", &["basis"])]), &unpack, &remove);
        let get = |id: &str| &out.iter().find(|(x, _)| x == id).unwrap().1;
        assert!(matches!(get("basis"), Out::Error(e) if e.contains("herausgenommen") && e.contains("„netz“")), "{:?}", get("basis"));
        assert!(matches!(get("netz"), Out::Error(e) if e.contains("Signatur")));
        assert!(!root.join("basis.neu").exists(), "erste Erweiterung wieder entfernt");
        assert!(matches!(get("allein"), Out::Installed(v, d) if v == "2.0.0" && d.exists()));
        assert!(matches!(get("rest"), Out::UpToDate(_)));
        // Reihenfolge andersherum: scheitert die erste, wird die zweite gar nicht erst eingespielt.
        let st = vec![("netz".to_string(), neu("1.3.0")), ("basis".to_string(), neu("1.3.0"))];
        let out = einspielen(st, &deps(&[("netz", &["basis"])]), &unpack, &remove);
        assert!(out.iter().all(|(_, o)| matches!(o, Out::Error(_))));
        assert!(!root.join("basis.neu").exists());
    }

    /// Netzfehler beim Auflisten des Ordners einer Erweiterung ist ein Fehler, kein „nicht in der Quelle“; nur ein
    /// fehlender Ordner ist „nicht in der Quelle“.
    #[test]
    fn listing_error_is_not_reported_as_missing() {
        let platform = "linux-x86_64";
        let base = ablage::test_server(vec![("/api/v2.1/share-links/tok/dirents/?path=/erweiterungen/netz/", "schliessen")]);
        let s = ablage::test_sitzung(&base);
        let key = sign::generate().unwrap().verifying_key();
        let v = Version::parse("0.1.67").unwrap();
        let cancel = AtomicBool::new(false);
        let (g, _) = laden(&s, "netz", platform, &key, &v, FetchMode::Install, &cancel);
        assert!(matches!(&g, Geladen::Fehler(e) if e.contains("beim Auflisten")), "{g:?}");
        let (g, _) = laden(&s, "fehlt", platform, &key, &v, FetchMode::Install, &cancel);
        assert!(matches!(g, Geladen::Fehlt(_)), "{g:?}");
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
