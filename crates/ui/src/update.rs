//! Update-Funktion im Hauptprogramm: Quelle aus der Konfiguration,
//! Zugriffsschlüssel aus der Schlüsseldatenbank, Prüfen und Installieren als
//! Hintergrundaufgaben. Ergebnisse kommen als `JobResult` (Art `update_check`
//! bzw. `update_install`) an den Tab „Nach Updates suchen“.

use std::path::PathBuf;

use rj_update::release::{self, Check, Source, SourceKind};
use rj_update::{install, Limits, Version};
use serde_json::{json, Value};
use zeroize::Zeroizing;

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
    /// Automatische Prüfung beim Start ist erledigt.
    auto_done: bool,
}

/// Quelle aus den Einstellungen.
pub fn source(s: &rj_core::config::UpdateSettings) -> Result<Source, String> {
    let kind = match s.source.as_str() {
        "gitea" => SourceKind::Gitea,
        "github" => SourceKind::Github,
        "" => return Err(rj_update::UpdateError::NoSource.to_string()),
        other => return Err(format!("Unbekannte Update-Quelle „{other}“")),
    };
    let src = Source { kind, base_url: s.base_url.trim().to_string(), repo: s.repo.trim().to_string() };
    src.latest_url().map_err(|e| e.to_string())?;
    Ok(src)
}

impl App {
    fn program_dir(&self) -> Option<PathBuf> {
        self.update.dir_override.clone().or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())))
    }

    /// Zugriffsschlüssel aus der Schlüsseldatenbank (nie protokolliert).
    fn update_token(&self) -> Result<Option<Zeroizing<String>>, String> {
        let Some(id) = self.config.update.token_entry else { return Ok(None) };
        let vault = self.vault.as_ref().ok_or("Die Schlüsseldatenbank ist gesperrt; der Zugriffsschlüssel für Updates steht darin.")?;
        let c = vault.credential(id).ok_or("Der Eintrag mit dem Zugriffsschlüssel für Updates fehlt in der Schlüsseldatenbank.")?;
        release::token_text(c.data.password.as_bytes()).map(Some).map_err(|e| e.to_string())
    }

    /// Zustand für den Tab: Versionen, Quelle (ohne Schlüssel), Installationsart.
    pub fn update_status(&self) -> Value {
        let s = &self.config.update;
        let token_title = s.token_entry.and_then(|id| self.vault.as_ref().and_then(|v| v.credential(id)).map(|c| rj_core::charset::mark_visible(&c.data.title, false).into_owned()));
        let plan = self.program_dir().map(|d| install::plan(&d));
        json!({
            "current": rj_update::CURRENT_VERSION,
            "platform": rj_update::platform(),
            "source": s.source,
            "base_url": s.base_url,
            "repo": s.repo,
            "token_entry": s.token_entry,
            "token_title": token_title,
            "check_on_start": s.check_on_start,
            "package_managed": matches!(plan, Some(install::Plan::PackageManaged { .. })),
            "development": matches!(plan, Some(install::Plan::Development { .. })),
            "busy": self.update.job.is_some_and(|j| self.jobs.is_pending(j)),
            "found": self.update.found.as_ref().map(|f| found_json(f)),
            "sessions": self.sessions.ids().len(),
            "entries": self.vault.as_ref().map(|v| v.entries().into_iter().map(|e| json!({ "uuid": e.uuid, "title": rj_core::charset::mark_visible(&e.title, false), "group": e.group_path.join("/") })).collect::<Vec<_>>()).unwrap_or_default(),
            "locked": self.vault.is_none(),
        })
    }

    /// Einstellungen ändern (Quelle, Adresse, Repository, Schlüsseleintrag, Prüfung beim Start).
    pub fn update_settings(&mut self, c: &Value) -> Result<Value, String> {
        let mut s = self.config.update.clone();
        let text = |k: &str| c.get(k).and_then(Value::as_str).map(str::to_string);
        if let Some(v) = text("source") {
            s.source = v;
        }
        if let Some(v) = text("base_url") {
            s.base_url = v.trim().to_string();
        }
        if let Some(v) = text("repo") {
            s.repo = v.trim().to_string();
        }
        if let Some(v) = c.get("token_entry") {
            s.token_entry = match v.as_str() {
                Some(t) if !t.is_empty() => Some(t.parse().map_err(|_| "Eintrag ungültig".to_string())?),
                _ => None,
            };
        }
        if let Some(v) = c.get("check_on_start").and_then(Value::as_bool) {
            s.check_on_start = v;
        }
        if !s.source.is_empty() {
            source(&s)?;
        }
        self.config.update = s;
        if !self.demo {
            if let Some(cp) = self.config_path.clone() {
                self.config.save(&cp).map_err(|e| e.to_string())?;
            }
        }
        Ok(self.update_status())
    }

    /// Startet die Prüfung im Hintergrund; Ergebnis als `JobResult` „update_check“.
    pub fn update_check(&mut self) -> Result<Value, String> {
        if self.update.job.is_some_and(|j| self.jobs.is_pending(j)) {
            return Err("Es läuft bereits eine Update-Prüfung oder -Installation.".into());
        }
        let src = source(&self.config.update)?;
        let token = self.update_token()?;
        let current = Version::parse(rj_update::CURRENT_VERSION).map_err(|e| e.to_string())?;
        let platform = rj_update::platform();
        let job = self.jobs.spawn_cancellable("Nach Updates suchen", move |job, cancel| {
            let r = release::check(&src, token.as_deref().map(|t| t.as_str()), &current, &platform, Some(&*cancel));
            drop(token);
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
        let src = source(&self.config.update)?;
        let token = self.update_token()?;
        let key_text = self.update.key_override.clone().unwrap_or_else(|| UPDATE_KEY.to_string());
        let key = rj_update::sign::public_key_from_text(&key_text).map_err(|e| e.to_string())?;
        let job = self.jobs.spawn_cancellable("Update herunterladen und installieren", move |job, cancel| {
            let r = release::download_verified(&src, token.as_deref().map(|t| t.as_str()), &found, &key, Limits::default(), Some(&*cancel))
                .and_then(|data| if cancel.load(std::sync::atomic::Ordering::Relaxed) { Err(rj_update::UpdateError::Cancelled) } else { Ok(data) })
                .and_then(|data| install::install(&found.archive.name, &data, &dir));
            drop(token);
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

    /// Prüfung beim Start (wenn eingeschaltet): einmal, im Hintergrund; mit
    /// Zugriffsschlüssel erst nach dem Entsperren.
    pub fn maybe_auto_check(&mut self) {
        let s = &self.config.update;
        if self.update.auto_done || self.demo || !s.check_on_start || s.source.is_empty() {
            return;
        }
        if s.token_entry.is_some() && self.vault.is_none() {
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
