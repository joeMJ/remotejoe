//! Updates aus der Ablage der Lizenz (Freigabe-Link und Kennwort aus der Lizenz): Programm und Erweiterungen. Teil von `update`.
//!
//! Ohne gültige Lizenz oder nach Ablauf der Wartung: keine Suche, nur ein Hinweis (ein Update von GitHub würde die
//! Fassung durch die ohne Module ersetzen). Sonst wie bei GitHub: neueste Version aus den Ordnernamen, Paket der
//! Plattform laden, Signatur und Prüfsumme prüfen; Linux ersetzt das Programmverzeichnis, Windows lädt die MSI
//! (Authenticode geprüft) und installiert sie nach dem Beenden (`update_apply`).

use super::*;
use crate::license::{self as lizenz, ablage};

/// Zuletzt in der Ablage gefundene neuere Version.
#[derive(Default, Clone)]
pub(crate) struct AblageFund {
    pub version: Option<Version>,
}

/// Zugang zur Ablage oder die Meldung, warum es keine Updates gibt.
fn zugang() -> Result<(String, String), String> {
    if let Some(z) = lizenz::ablage() {
        return Ok(z);
    }
    Err(match lizenz::status() {
        lizenz::Status::Gueltig(l) => format!("Die Wartung der Lizenz ist am {} abgelaufen – keine Updates mehr. Die installierte Version läuft weiter bis {}.", l.wartung_bis, l.version_bis),
        lizenz::Status::Abgelaufen(l) => format!("Die Lizenz ist am {} abgelaufen – keine Updates.", l.version_bis),
        _ => "Keine gültige Lizenz – Updates aus der Ablage gibt es nur mit Lizenz (Einstellungen → Lizenz).".into(),
    })
}

/// Automatische Prüfung beim Start möglich?
pub(crate) fn auto_moeglich() -> bool {
    lizenz::ablage().is_some()
}

enum Ergebnis {
    Neuer { version: Version, notes: String, archive: String, size: u64 },
    Aktuell(Version),
}

impl App {
    pub(crate) fn update_check_ablage(&mut self) -> Result<Value, String> {
        let (url, kennwort) = zugang()?;
        let current = Version::parse(rj_update::CURRENT_VERSION).map_err(|e| e.to_string())?;
        rj_update::proxy::set_mode(rj_update::proxy::ProxyMode::from_setting(&self.config.update.proxy));
        let platform = rj_update::platform();
        let cur = current.clone();
        let job = self.jobs.spawn_cancellable("Nach Updates suchen", move |job, cancel| {
            let r = (|| -> Result<Ergebnis, String> {
                let s = ablage::Sitzung::anmelden(&url, &kennwort, Some(&*cancel))?;
                let v = ablage::neueste(&s.ordner("/", Some(&*cancel))?).ok_or("Bei der Update-Quelle liegt keine Version.")?;
                if v <= cur {
                    return Ok(Ergebnis::Aktuell(v));
                }
                let (ordner, name) = ablage::paket(&v, &platform)?;
                let size = s.ordner(&format!("/{v}/{ordner}/"), Some(&*cancel))?.into_iter().find(|e| e.name == name).map(|e| e.groesse)
                    .ok_or_else(|| format!("Version {v}: kein Paket für diese Plattform ({name})"))?;
                let notes = ablage::notizen(&s, &v, Some(&*cancel)).unwrap_or_default();
                Ok(Ergebnis::Neuer { version: v, notes, archive: name, size })
            })();
            Box::new(move |app: &mut App, cancelled| {
                if cancelled {
                    return Vec::new();
                }
                let json = match r {
                    Ok(Ergebnis::Neuer { version, notes, archive, size }) => {
                        app.update.ablage.version = Some(version.clone());
                        app.update.total = size;
                        let found = json!({ "version": version.to_string(), "name": format!("remotejoe {version}"), "notes": notes,
                                            "prerelease": false, "size": size, "archive": archive });
                        json!({ "state": "newer", "current": current.to_string(), "found": found, "msi": cfg!(windows) })
                    }
                    Ok(Ergebnis::Aktuell(v)) => {
                        app.update.ablage.version = None;
                        json!({ "state": "uptodate", "current": current.to_string(), "latest": v.to_string() })
                    }
                    Err(e) => json!({ "state": "error", "error": e }),
                };
                vec![UiEvent::JobResult { job, kind: "update_check".into(), target: String::new(), json: json.to_string() }]
            })
        });
        self.update.job = Some(job);
        Ok(json!({ "job": job }))
    }

    /// Gefundene Version laden, prüfen und installieren (Linux) bzw. als geprüfte MSI bereitlegen (Windows).
    pub(crate) fn update_install_ablage(&mut self, confirm: bool) -> Result<Value, String> {
        let version = self.update.ablage.version.clone().ok_or("Keine neuere Version gefunden; bitte zuerst prüfen.")?;
        if self.update.job.is_some_and(|j| self.jobs.is_pending(j)) {
            return Err("Es läuft bereits eine Update-Prüfung oder -Installation.".into());
        }
        let (url, kennwort) = zugang()?;
        let platform = rj_update::platform();
        let key_text = self.update.key_override.clone().unwrap_or_else(|| UPDATE_KEY.to_string());
        let key = rj_update::sign::public_key_from_text(&key_text).map_err(|e| e.to_string())?;
        rj_update::proxy::set_mode(rj_update::proxy::ProxyMode::from_setting(&self.config.update.proxy));
        #[cfg(windows)]
        {
            let _ = confirm;
            let progress = self.update.progress.clone();
            progress.store(0, std::sync::atomic::Ordering::Relaxed);
            self.update.ready = None;
            let job = self.jobs.spawn_cancellable("Update herunterladen und prüfen", move |job, cancel| {
                let limits = Limits { max_bytes: 400 * 1024 * 1024, timeout: std::time::Duration::from_secs(30 * 60) };
                let dir = rj_update::msi::download_dir();
                let r = (|| -> Result<PathBuf, String> {
                    let s = ablage::Sitzung::anmelden(&url, &kennwort, Some(&*cancel))?;
                    let (name, data) = ablage::laden_geprueft(&s, &version, &platform, &key, limits, Some(&*cancel), Some(&progress))?;
                    let path = dir.join(&name);
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    std::fs::write(&path, &data).map_err(|e| e.to_string())?;
                    if let Err(e) = rj_update::authenticode::verify_file(&path, rj_update::authenticode::PUBLISHER_THUMBPRINT) {
                        let _ = std::fs::remove_file(&path);
                        return Err(e.to_string());
                    }
                    Ok(path)
                })();
                let version = version.to_string();
                Box::new(move |app: &mut App, cancelled| {
                    let json = match r {
                        Ok(path) if cancelled => {
                            let _ = std::fs::remove_file(&path);
                            return Vec::new();
                        }
                        Ok(path) => {
                            app.update.ready = Some((path.clone(), version.clone()));
                            json!({ "state": "ready", "version": version, "path": path })
                        }
                        Err(_) if cancelled => return Vec::new(),
                        Err(e) => json!({ "state": "error", "error": e }),
                    };
                    vec![UiEvent::JobResult { job, kind: "update_install".into(), target: String::new(), json: json.to_string() }]
                })
            });
            self.update.job = Some(job);
            return Ok(json!({ "job": job, "state": "downloading" }));
        }
        let dir = self.program_dir().ok_or("Programmverzeichnis unbekannt")?;
        match install::plan(&dir) {
            install::Plan::PackageManaged { dir } => return Ok(json!({ "state": "package", "dir": dir })),
            install::Plan::Development { dir } => return Ok(json!({ "state": "development", "dir": dir })),
            install::Plan::Replace { .. } => {}
        }
        let open = self.sessions.ids().len();
        if open > 0 && !confirm {
            return Ok(json!({ "state": "confirm", "sessions": open }));
        }
        let _ = self.backup_start(Some(&version.to_string()), false);
        let job = self.jobs.spawn_cancellable("Update herunterladen und installieren", move |job, cancel| {
            let r = (|| -> Result<PathBuf, String> {
                let s = ablage::Sitzung::anmelden(&url, &kennwort, Some(&*cancel))?;
                let (name, data) = ablage::laden_geprueft(&s, &version, &platform, &key, Limits::default(), Some(&*cancel), None)?;
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Abgebrochen".into());
                }
                install::install(&name, &data, &dir).map_err(|e| e.to_string())
            })();
            let version = version.to_string();
            Box::new(move |app: &mut App, _cancelled| {
                let json = match r {
                    Ok(old) => {
                        app.update.ablage.version = None;
                        // Erweiterungen für die neue Version mitholen (liegen bis zum Neustart in `<id>.neu`).
                        if let Ok(v) = Version::parse(&version) {
                            app.plugin_fetch_for_update(&v);
                        }
                        json!({ "state": "installed", "version": version, "dir": dir, "old": old, "program": dir.join(install::main_program()) })
                    }
                    Err(e) => json!({ "state": "error", "error": e }),
                };
                vec![UiEvent::JobResult { job, kind: "update_install".into(), target: String::new(), json: json.to_string() }]
            })
        });
        self.update.job = Some(job);
        Ok(json!({ "job": job }))
    }
}
