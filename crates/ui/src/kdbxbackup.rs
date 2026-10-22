//! Sicherungen der KDBX nach dem Speichern und vor einem Update (Einstellungen → Sicherung). Gesichert werden die
//! verschlüsselten Bytes, wie sie auf der Platte liegen (dazu ein eigener Index `.rjindex`); Kennwort und
//! Schlüsseldatei bleiben dieselben. Die Schlüsseldatei wird bewusst nicht mitgesichert: Neben der KDBX auf einem
//! NAS wäre sie kein zweiter Faktor mehr.
//!
//! Die Kopien laufen im Hintergrund, damit ein langsamer oder nicht erreichbarer zweiter Ort Speichern und Beenden
//! nicht aufhält; beim Beenden wartet die Oberfläche höchstens kurz auf eine laufende Sicherung ([`wait`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rj_core::backup;
use serde_json::{json, Value};

use crate::app::App;
use crate::sessions::UiEvent;

static RUNNING: AtomicUsize = AtomicUsize::new(0);
static LAST: Mutex<Option<Value>> = Mutex::new(None);

/// Zählt eine laufende Sicherung, auch wenn die Arbeit abbricht.
struct Running;

impl Running {
    fn start() -> Running {
        RUNNING.fetch_add(1, Ordering::SeqCst);
        Running
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Läuft gerade eine Sicherung?
pub fn busy() -> bool {
    RUNNING.load(Ordering::SeqCst) > 0
}

/// Wartet höchstens `limit` auf laufende Sicherungen; `true`, wenn keine mehr läuft.
pub fn wait(limit: Duration) -> bool {
    let end = Instant::now() + limit;
    while busy() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    !busy()
}

/// Ordner der lokalen Sicherungen zur KDBX `kdbx`.
pub fn local_dir(kdbx: &Path) -> PathBuf {
    kdbx.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).join(backup::LOCAL_DIR)
}

/// Inhalt einer Sicherung: die KDBX und, falls vorhanden, der Index daneben.
fn snapshot(kdbx: &Path) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let mut files = vec![("kdbx", std::fs::read(kdbx).map_err(|e| format!("{}: {e}", kdbx.display()))?)];
    let index = kdbx.with_extension("rjindex");
    if index.is_file() {
        files.push(("rjindex", std::fs::read(&index).map_err(|e| format!("{}: {e}", index.display()))?));
    }
    Ok(files)
}

/// Schreibt die Sicherung nach `dir` und räumt dort auf.
fn store(dir: &Path, stem: &str, base: &str, today: &str, files: &[(&str, &[u8])], r: backup::Retention) -> Value {
    match backup::write(dir, base, files).and_then(|p| backup::prune(dir, stem, today, r).map(|_| p)) {
        Ok(p) => json!({ "dir": dir, "ok": true, "file": p }),
        Err(e) => json!({ "dir": dir, "ok": false, "error": e.to_string() }),
    }
}

impl App {
    /// Startet eine Sicherung im Hintergrund: nach dem Speichern, mit `update` vor der Installation dieser Version,
    /// mit `manual` auf Knopfdruck (auch wenn die Sicherung ausgeschaltet ist). Ohne gespeicherte KDBX und im
    /// Vorführmodus geschieht nichts.
    pub(crate) fn backup_start(&mut self, update: Option<&str>, manual: bool) -> Result<Option<u64>, String> {
        let s = self.config.backup.clone();
        if self.demo || !(s.enabled || manual) {
            return Ok(None);
        }
        let Some(kdbx) = self.vault.as_ref().map(|v| v.path().to_path_buf()).filter(|p| p.is_file()) else {
            return Ok(None);
        };
        let stem = kdbx.file_stem().and_then(|s| s.to_str()).ok_or("KDBX ohne Dateinamen")?.to_string();
        // Die Bytes jetzt lesen: Die Kopie zeigt genau den eben gespeicherten Stand, auch wenn danach erneut
        // gespeichert wird.
        let files = snapshot(&kdbx)?;
        let mut dirs = vec![local_dir(&kdbx)];
        if s.second_enabled && !s.second_dir.trim().is_empty() {
            dirs.push(PathBuf::from(s.second_dir.trim()));
        }
        let now = chrono::Local::now();
        let stamp = now.format("%Y%m%d-%H%M%S").to_string();
        let base = backup::base_name(&stem, &stamp, update);
        let r = s.retention();
        let guard = Running::start();
        let job = self.jobs.spawn("KDBX sichern", move |job| {
            let files: Vec<(&str, &[u8])> = files.iter().map(|(e, d)| (*e, d.as_slice())).collect();
            let results: Vec<Value> = dirs.iter().map(|d| store(d, &stem, &base, &stamp, &files, r)).collect();
            drop(guard);
            let json = json!({
                "ok": results.iter().all(|r| r["ok"] == true),
                "at": now.format("%d.%m.%Y %H:%M:%S").to_string(),
                "update": update_label(&base),
                "targets": results,
            });
            *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(json.clone());
            Box::new(move |_app: &mut App, _cancelled| vec![UiEvent::JobResult { job, kind: "backup".into(), target: String::new(), json: json.to_string() }])
        });
        Ok(Some(job))
    }

    /// Einstellungen und Stand der Sicherung für die Oberfläche.
    pub(crate) fn backup_settings_json(&self) -> Value {
        let s = &self.config.backup;
        let kdbx = self.vault.as_ref().map(|v| v.path().to_path_buf()).or_else(|| self.config.kdbx_path.clone());
        json!({
            "enabled": s.enabled,
            "keep_last": s.keep_last,
            "keep_days": s.keep_days,
            "keep_update": s.keep_update,
            "second_enabled": s.second_enabled,
            "second_dir": s.second_dir,
            "local_dir": kdbx.as_deref().map(local_dir),
            "key_file": self.config.key_file.is_some(),
            "last": LAST.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            "busy": busy(),
        })
    }

    /// Übernimmt geänderte Einstellungen aus `c` (nur vorhandene Felder).
    pub(crate) fn backup_apply_settings(&mut self, c: &Value) {
        let s = &mut self.config.backup;
        let num = |k: &str, lo: u64, hi: u64| c.get(k).and_then(Value::as_u64).map(|n| n.clamp(lo, hi) as u32);
        if let Some(b) = c.get("backup_enabled").and_then(Value::as_bool) {
            s.enabled = b;
        }
        if let Some(n) = num("backup_keep_last", 1, 100) {
            s.keep_last = n;
        }
        if let Some(n) = num("backup_keep_days", 0, 90) {
            s.keep_days = n;
        }
        if let Some(n) = num("backup_keep_update", 1, 20) {
            s.keep_update = n;
        }
        if let Some(b) = c.get("backup_second_enabled").and_then(Value::as_bool) {
            s.second_enabled = b;
        }
        if let Some(d) = c.get("backup_second_dir").and_then(Value::as_str) {
            s.second_dir = d.trim().to_string();
        }
    }
}

/// Version aus dem Namen einer Update-Sicherung, sonst `null`.
fn update_label(base: &str) -> Value {
    base.split_once("-vor-update-").and_then(|(_, rest)| rest.rsplit_once('-')).and_then(|(v, _)| v.rsplit_once('-')).map_or(Value::Null, |(v, _)| json!(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_label_from_name() {
        assert_eq!(update_label("ziele-vor-update-0.1.21-20261022-143000"), json!("0.1.21"));
        assert_eq!(update_label("ziele-20261022-143000"), Value::Null);
    }

    #[test]
    fn local_dir_next_to_kdbx() {
        assert_eq!(local_dir(Path::new("/daten/ziele.kdbx")), PathBuf::from("/daten/Sicherungen"));
        assert_eq!(local_dir(Path::new("ziele.kdbx")), PathBuf::from("./Sicherungen"));
    }
}
