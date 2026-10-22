//! Update-Funktion im Hauptprogramm: Quelle aus der Konfiguration,
//! Zugriffsschlüssel aus der Schlüsseldatenbank, Prüfen und Installieren als
//! Hintergrundaufgaben. Ergebnisse kommen als `JobResult` (Art `update_check`
//! bzw. `update_install`) an den Tab „Nach Updates suchen“.

use std::path::PathBuf;

use rj_update::release::{self, Check, Source, SourceKind};
use rj_update::{install, Limits, Version};
use serde_json::{json, Value};

use crate::app::App;
use crate::sessions::UiEvent;

/// Eingebauter öffentlicher Schlüssel für die Signatur der Pakete.
pub const UPDATE_KEY: &str = include_str!("../res/update-key.pub");

#[derive(Default)]
pub struct UpdateState {
    /// Zuletzt gefundene neuere Version.
    found: Option<release::Found>,
    /// Laufende Prüfung oder Installation.
    pub job: Option<u64>,
    /// Ziel der Installation (Tests); sonst das Verzeichnis des Programms.
    pub dir_override: Option<PathBuf>,
    /// Öffentlicher Schlüssel (Tests); sonst der eingebaute.
    pub key_override: Option<String>,
    /// Quelle (Tests und Vorführung); sonst immer GitHub.
    pub source_override: Option<Source>,
    /// Automatische Prüfung beim Start ist erledigt.
    auto_done: bool,
    /// Windows: zuletzt gefundene neuere Version (Angaben für den Tab).
    found_page: Option<Value>,
    /// Windows: geladene und geprüfte MSI (Pfad, Version), bereit zur Installation.
    ready: Option<(PathBuf, String)>,
    /// Windows: bisher geladene Bytes der MSI (Fortschrittsanzeige) und erwartete Größe.
    progress: std::sync::Arc<std::sync::atomic::AtomicU64>,
    total: u64,
    /// Windows, nur Tests: Installation über die MSI annehmen bzw. ausschließen (sonst nach dem Programmordner).
    pub msi_override: Option<bool>,
}

/// Windows: Das Programm ersetzt sich nicht selbst. Als MSI installiert lädt „Nach Updates suchen“ die MSI,
/// prüft sie und installiert sie nach dem Beenden; sonst (Entwicklungsstand) nennt es die neue Version und
/// öffnet die Release-Seite.
const PAGE_ONLY: bool = cfg!(windows);

/// Die einzige Quelle: die öffentlichen Veröffentlichungen von remotejoe auf GitHub.
pub fn github_source() -> Source {
    Source { kind: SourceKind::Github, base_url: String::new(), repo: "joeMJ/remotejoe".into() }
}

impl App {
    fn program_dir(&self) -> Option<PathBuf> {
        self.update.dir_override.clone().or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())))
    }

    /// Quelle der Updates: immer GitHub (außer in Tests und Vorführung). Jeder Abruf holt sie hier ab; dabei gilt
    /// auch die aktuelle Proxy-Einstellung.
    fn update_source(&self) -> Source {
        rj_update::proxy::set_mode(rj_update::proxy::ProxyMode::from_setting(&self.config.update.proxy));
        self.update.source_override.clone().unwrap_or_else(github_source)
    }

    /// Zustand für den Tab: Versionen, Quelle (ohne Schlüssel), Installationsart.
    pub fn update_status(&self) -> Value {
        let plan = self.program_dir().map(|d| install::plan(&d));
        let mut status = json!({
            "current": rj_update::CURRENT_VERSION,
            "platform": rj_update::platform(),
            "check_on_start": self.config.update.check_on_start,
            "proxy": self.config.update.proxy,
            "package_managed": matches!(plan, Some(install::Plan::PackageManaged { .. })),
            "development": matches!(plan, Some(install::Plan::Development { .. })),
            "busy": self.update.job.is_some_and(|j| self.jobs.is_pending(j)),
            "found": self.update.found.as_ref().map(|f| found_json(f)),
            "sessions": self.sessions.ids().len(),
        });
        if PAGE_ONLY {
            status["page_only"] = json!(true);
            status["page"] = json!(self.update_source().releases_page().ok());
            status["found"] = self.update.found_page.clone().unwrap_or(Value::Null);
            status["package_managed"] = json!(false);
            status["development"] = json!(false);
            #[cfg(windows)]
            if self.msi_mode() {
                status["page_only"] = json!(false);
                status["msi"] = json!(true);
                status["found"] = self.update.found.as_ref().map(|f| found_json(f)).unwrap_or(Value::Null);
                status["ready"] = json!(self.update.ready.as_ref().map(|(_, v)| v));
                status["progress"] = json!(self.update.progress.load(std::sync::atomic::Ordering::Relaxed));
                status["total"] = json!(self.update.total);
            }
        }
        status
    }

    /// Windows: nur die Version vergleichen (keine Dateiauswahl); Ergebnis wie `update_check`,
    /// zusätzlich mit der Adresse der Release-Seite.
    fn update_check_page(&mut self, src: Source, current: Version) -> Result<Value, String> {
        let page = src.releases_page().map_err(|e| e.to_string())?;
        let job = self.jobs.spawn_cancellable("Nach Updates suchen", move |job, cancel| {
            let r = release::latest(&src, None, Some(&*cancel)).and_then(|rel| Version::parse(&rel.tag_name).map(|v| (v, rel)));
            Box::new(move |app: &mut App, cancelled| {
                if cancelled {
                    return Vec::new();
                }
                let json = match r {
                    Ok((v, rel)) if !rel.draft && v > current => {
                        let found = json!({
                            "version": v.to_string(),
                            "name": rel.name.as_deref().map(|n| rj_core::charset::mark_visible(n, false).into_owned()),
                            "notes": rel.notes(),
                            "prerelease": v.is_prerelease(),
                        });
                        app.update.found_page = Some(found.clone());
                        json!({ "state": "newer", "current": current.to_string(), "found": found, "page": page })
                    }
                    Ok((v, _)) => {
                        app.update.found_page = None;
                        json!({ "state": "uptodate", "current": current.to_string(), "latest": v.to_string() })
                    }
                    Err(e) => json!({ "state": "error", "error": e.to_string() }),
                };
                vec![UiEvent::JobResult { job, kind: "update_check".into(), target: String::new(), json: json.to_string() }]
            })
        });
        self.update.job = Some(job);
        Ok(json!({ "job": job }))
    }

    /// Einstellungen ändern: „Beim Start im Hintergrund prüfen“ und Proxy.
    pub fn update_settings(&mut self, c: &Value) -> Result<Value, String> {
        if let Some(v) = c.get("check_on_start").and_then(Value::as_bool) {
            self.config.update.check_on_start = v;
        }
        if let Some(p) = c.get("proxy").and_then(Value::as_str) {
            let p = p.trim();
            if let rj_update::proxy::ProxyMode::Manual(m) = rj_update::proxy::ProxyMode::from_setting(p) {
                rj_update::proxy::check_manual(&m)?;
            }
            self.config.update.proxy = if p == "auto" { String::new() } else { p.to_string() };
        }
        if !self.demo {
            if let Some(cp) = self.config_path.clone() {
                self.config.save(&cp).map_err(|e| e.to_string())?;
            }
        }
        Ok(self.update_status())
    }

    /// Nur Vorführung: Quelle auf eine lokale Attrappe setzen (Bildschirmfotos).
    pub fn update_demo_source(&mut self, base_url: &str) -> Result<Value, String> {
        if !self.demo {
            return Err("Nur in der Vorführung".into());
        }
        self.update.source_override = Some(Source { kind: SourceKind::Gitea, base_url: base_url.trim().to_string(), repo: "team/remotejoe".into() });
        Ok(self.update_status())
    }

    /// Startet die Prüfung im Hintergrund; Ergebnis als `JobResult` „update_check“.
    pub fn update_check(&mut self) -> Result<Value, String> {
        if self.update.job.is_some_and(|j| self.jobs.is_pending(j)) {
            return Err("Es läuft bereits eine Update-Prüfung oder -Installation.".into());
        }
        let src = self.update_source();
        let current = Version::parse(rj_update::CURRENT_VERSION).map_err(|e| e.to_string())?;
        if PAGE_ONLY {
            #[cfg(windows)]
            if self.msi_mode() {
                return self.update_check_msi(src, current);
            }
            return self.update_check_page(src, current);
        }
        let platform = rj_update::platform();
        let job = self.jobs.spawn_cancellable("Nach Updates suchen", move |job, cancel| {
            let r = release::check(&src, None, &current, &platform, Some(&*cancel));
            Box::new(move |app: &mut App, cancelled| {
                if cancelled {
                    return Vec::new();
                }
                let json = match r {
                    Ok(Check::Newer(f)) => {
                        let j = json!({ "state": "newer", "current": current.to_string(), "found": found_json(&f) });
                        app.update.found = Some(*f);
                        j
                    }
                    Ok(Check::UpToDate(v)) => {
                        app.update.found = None;
                        json!({ "state": "uptodate", "current": current.to_string(), "latest": v.to_string() })
                    }
                    Err(e) => json!({ "state": "error", "error": e.to_string() }),
                };
                vec![UiEvent::JobResult { job, kind: "update_check".into(), target: String::new(), json: json.to_string() }]
            })
        });
        self.update.job = Some(job);
        Ok(json!({ "job": job }))
    }

    /// Lädt, prüft und installiert die gefundene Version. Bei offenen Sitzungen
    /// nur mit `confirm`; unter `/usr` nur ein Hinweis.
    pub fn update_install(&mut self, confirm: bool) -> Result<Value, String> {
        // Windows: kein Ersetzen im laufenden Betrieb. Als MSI installiert: MSI laden und prüfen, installiert wird
        // nach dem Beenden (`update_apply`); sonst öffnet die Oberfläche die Release-Seite.
        if PAGE_ONLY {
            #[cfg(windows)]
            if self.msi_mode() && self.update.found.is_some() {
                return self.update_download_msi();
            }
            let _ = confirm;
            return Ok(json!({ "state": "page", "url": self.update_source().releases_page().map_err(|e| e.to_string())? }));
        }
        let found = self.update.found.clone().ok_or("Keine neuere Version gefunden; bitte zuerst prüfen.")?;
        if self.update.job.is_some_and(|j| self.jobs.is_pending(j)) {
            return Err("Es läuft bereits eine Update-Prüfung oder -Installation.".into());
        }
        let dir = self.program_dir().ok_or("Programmverzeichnis unbekannt")?;
        match install::plan(&dir) {
            install::Plan::PackageManaged { dir } => return Ok(json!({ "state": "package", "dir": dir, "url": found.archive.browser_download_url })),
            install::Plan::Development { dir } => return Ok(json!({ "state": "development", "dir": dir, "url": found.archive.browser_download_url })),
            install::Plan::Replace { .. } => {}
        }
        let open = self.sessions.ids().len();
        if open > 0 && !confirm {
            return Ok(json!({ "state": "confirm", "sessions": open }));
        }
        // Vor dem Update die KDBX sichern (läuft neben dem Herunterladen; die Daten berührt das Update nicht).
        let _ = self.backup_start(Some(&found.version.to_string()), false);
        let src = self.update_source();
        let key_text = self.update.key_override.clone().unwrap_or_else(|| UPDATE_KEY.to_string());
        let key = rj_update::sign::public_key_from_text(&key_text).map_err(|e| e.to_string())?;
        let job = self.jobs.spawn_cancellable("Update herunterladen und installieren", move |job, cancel| {
            let r = release::download_verified(&src, None, &found, &key, Limits::default(), Some(&*cancel))
                .and_then(|data| if cancel.load(std::sync::atomic::Ordering::Relaxed) { Err(rj_update::UpdateError::Cancelled) } else { Ok(data) })
                .and_then(|data| install::install(&found.archive.name, &data, &dir));
            let version = found.version.to_string();
            Box::new(move |app: &mut App, _cancelled| {
                let json = match r {
                    Ok(old) => {
                        app.update.found = None;
                        json!({ "state": "installed", "version": version, "dir": dir, "old": old, "program": dir.join(install::main_program()) })
                    }
                    Err(e) => json!({ "state": "error", "error": e.to_string() }),
                };
                vec![UiEvent::JobResult { job, kind: "update_install".into(), target: String::new(), json: json.to_string() }]
            })
        });
        self.update.job = Some(job);
        Ok(json!({ "job": job }))
    }

    /// Windows: geprüfte MSI installieren. remotejoe muss sich danach beenden (die Oberfläche schließt das Fenster
    /// vorher); ein Hilfsprozess wartet darauf, installiert und startet remotejoe neu.
    pub fn update_apply(&mut self, confirm: bool) -> Result<Value, String> {
        #[cfg(windows)]
        {
            let (path, version) = self.update.ready.clone().ok_or("Kein geprüftes Update bereit; bitte zuerst herunterladen.")?;
            let open = self.sessions.ids().len();
            if open > 0 && !confirm {
                return Ok(json!({ "state": "confirm", "sessions": open }));
            }
            if !path.is_file() {
                self.update.ready = None;
                return Err("Die geladene MSI ist nicht mehr vorhanden; bitte erneut herunterladen.".into());
            }
            // Vor dem Update die KDBX sichern; das Fenster ist schon zu, remotejoe beendet sich gleich danach.
            if let Ok(Some(_)) = self.backup_start(Some(&version.to_string()), false) {
                crate::kdbxbackup::wait(std::time::Duration::from_secs(10));
            }
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            rj_update::msi::install_after_exit(&path, &exe, std::process::id()).map_err(|e| e.to_string())?;
            Ok(json!({ "state": "applying", "version": version }))
        }
        #[cfg(not(windows))]
        {
            let _ = confirm;
            Err("Nur unter Windows".into())
        }
    }

    pub fn update_cancel(&mut self) -> Value {
        let cancelled = self.update.job.take().is_some_and(|j| self.jobs.cancel(j));
        json!({ "cancelled": cancelled })
    }

    /// Prüfung beim Start (wenn eingeschaltet): einmal, im Hintergrund.
    pub fn maybe_auto_check(&mut self) {
        if self.update.auto_done || self.demo || !self.config.update.check_on_start {
            return;
        }
        self.update.auto_done = true;
        let _ = self.update_check();
    }
}

/// Windows: Update über die MSI der Veröffentlichung.
#[cfg(windows)]
impl App {
    /// Ist remotejoe über die MSI installiert (Programmordner `…\Programs\remotejoe`)?
    fn msi_mode(&self) -> bool {
        self.update.msi_override.unwrap_or_else(|| self.program_dir().is_some_and(|d| rj_update::msi::is_msi_dir(&d)))
    }

    /// Wie `update_check`, wählt aber die MSI aus. Enthält die Veröffentlichung keine MSI, gilt wie bisher die
    /// Release-Seite (`page`, `no_msi`).
    fn update_check_msi(&mut self, src: Source, current: Version) -> Result<Value, String> {
        let page = src.releases_page().ok();
        let job = self.jobs.spawn_cancellable("Nach Updates suchen", move |job, cancel| {
            let r = release::latest(&src, None, Some(&*cancel)).and_then(|rel| Version::parse(&rel.tag_name).map(|v| (v, rel)));
            Box::new(move |app: &mut App, cancelled| {
                if cancelled {
                    return Vec::new();
                }
                let json = match r {
                    Ok((v, rel)) if !rel.draft && v > current => match rj_update::msi::select_msi(&rel) {
                        Ok(f) => {
                            let j = json!({ "state": "newer", "current": current.to_string(), "found": found_json(&f), "msi": true });
                            app.update.found = Some(f);
                            app.update.ready = None;
                            j
                        }
                        Err(e) => {
                            let found = json!({ "version": v.to_string(), "notes": rel.notes(), "prerelease": v.is_prerelease() });
                            app.update.found = None;
                            app.update.found_page = Some(found.clone());
                            json!({ "state": "newer", "current": current.to_string(), "found": found, "page": page, "no_msi": e.to_string() })
                        }
                    },
                    Ok((v, _)) => {
                        app.update.found = None;
                        app.update.ready = None;
                        json!({ "state": "uptodate", "current": current.to_string(), "latest": v.to_string() })
                    }
                    Err(e) => json!({ "state": "error", "error": e.to_string() }),
                };
                vec![UiEvent::JobResult { job, kind: "update_check".into(), target: String::new(), json: json.to_string() }]
            })
        });
        self.update.job = Some(job);
        Ok(json!({ "job": job }))
    }

    /// Lädt die MSI mit Fortschritt, prüft Signatur der `SHA256SUMS`, Prüfsumme und Authenticode-Signatur und legt
    /// sie ab. Ergebnis „update_install“ mit `ready`; bei einem Fehler bleibt keine MSI liegen.
    fn update_download_msi(&mut self) -> Result<Value, String> {
        if self.update.job.is_some_and(|j| self.jobs.is_pending(j)) {
            return Err("Es läuft bereits eine Update-Prüfung oder -Installation.".into());
        }
        let found = self.update.found.clone().ok_or("Keine neuere Version gefunden; bitte zuerst prüfen.")?;
        let src = self.update_source();
        let key_text = self.update.key_override.clone().unwrap_or_else(|| UPDATE_KEY.to_string());
        let key = rj_update::sign::public_key_from_text(&key_text).map_err(|e| e.to_string())?;
        let progress = self.update.progress.clone();
        progress.store(0, std::sync::atomic::Ordering::Relaxed);
        self.update.total = found.archive.size;
        self.update.ready = None;
        let job = self.jobs.spawn_cancellable("Update herunterladen und prüfen", move |job, cancel| {
            let limits = Limits { max_bytes: 400 * 1024 * 1024, timeout: std::time::Duration::from_secs(30 * 60) };
            let dir = rj_update::msi::download_dir();
            let path = dir.join(&found.archive.name);
            let r = release::download_verified_progress(&src, None, &found, &key, limits, Some(&*cancel), Some(&progress))
                .and_then(|data| {
                    std::fs::create_dir_all(&dir)?;
                    std::fs::write(&path, &data)?;
                    rj_update::authenticode::verify_file(&path, rj_update::authenticode::PUBLISHER_THUMBPRINT)
                })
                .inspect_err(|_| {
                    let _ = std::fs::remove_file(&path);
                });
            let version = found.version.to_string();
            Box::new(move |app: &mut App, cancelled| {
                if cancelled {
                    let _ = std::fs::remove_file(&path);
                    return Vec::new();
                }
                let json = match r {
                    Ok(()) => {
                        app.update.ready = Some((path.clone(), version.clone()));
                        json!({ "state": "ready", "version": version, "path": path })
                    }
                    Err(e) => json!({ "state": "error", "error": e.to_string() }),
                };
                vec![UiEvent::JobResult { job, kind: "update_install".into(), target: String::new(), json: json.to_string() }]
            })
        });
        self.update.job = Some(job);
        Ok(json!({ "job": job, "state": "downloading" }))
    }
}

fn found_json(f: &release::Found) -> Value {
    json!({
        "version": f.version.to_string(),
        "name": f.release.name.as_deref().map(|n| rj_core::charset::mark_visible(n, false).into_owned()),
        "notes": f.release.notes(),
        "prerelease": f.version.is_prerelease(),
        "size": f.archive.size,
        "archive": f.archive.name,
    })
}
