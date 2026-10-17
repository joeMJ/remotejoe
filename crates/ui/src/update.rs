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
}

/// Die einzige Quelle: die öffentlichen Veröffentlichungen von remotejoe auf GitHub.
pub fn github_source() -> Source {
    Source { kind: SourceKind::Github, base_url: String::new(), repo: "joeMJ/remotejoe".into() }
}

impl App {
    fn program_dir(&self) -> Option<PathBuf> {
        self.update.dir_override.clone().or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())))
    }

    /// Quelle der Updates: immer GitHub (außer in Tests und Vorführung).
    fn update_source(&self) -> Source {
        self.update.source_override.clone().unwrap_or_else(github_source)
    }

    /// Zustand für den Tab: Versionen, Quelle (ohne Schlüssel), Installationsart.
    pub fn update_status(&self) -> Value {
        let plan = self.program_dir().map(|d| install::plan(&d));
        json!({
            "current": rj_update::CURRENT_VERSION,
            "platform": rj_update::platform(),
            "check_on_start": self.config.update.check_on_start,
            "package_managed": matches!(plan, Some(install::Plan::PackageManaged { .. })),
            "development": matches!(plan, Some(install::Plan::Development { .. })),
            "busy": self.update.job.is_some_and(|j| self.jobs.is_pending(j)),
            "found": self.update.found.as_ref().map(|f| found_json(f)),
            "sessions": self.sessions.ids().len(),
        })
    }

    /// Einstellungen ändern: nur „Beim Start im Hintergrund prüfen“.
    pub fn update_settings(&mut self, c: &Value) -> Result<Value, String> {
        if let Some(v) = c.get("check_on_start").and_then(Value::as_bool) {
            self.config.update.check_on_start = v;
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
