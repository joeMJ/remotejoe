//! Einstellungen → Daten → Web-Zwischenspeicher: Größe anzeigen, Grenze und Aufbewahrung einstellen, aufräumen
//! (Knopf und einmal nach dem Entsperren im Hintergrund). Die Arbeit selbst steckt in [`rj_web::cache`].

use std::collections::HashSet;

use rj_core::Protocol;
use rj_web::cache;
use serde_json::{json, Value};

use crate::app::App;

impl App {
    /// Profile laufender Web-Sitzungen (auch der Felderkennung): bleiben beim Aufräumen unberührt.
    fn web_profiles_in_use(&self) -> HashSet<String> {
        self.sessions
            .ids()
            .into_iter()
            .filter(|s| self.sessions.protocol(*s) == Some(Protocol::Web))
            .filter_map(|s| self.sessions.spec(s))
            .filter_map(|spec| serde_json::from_str::<Value>(&spec.request.options_json).ok())
            .filter_map(|o| o["profile"].as_str().map(str::to_string))
            .collect()
    }

    /// Kennungen aller Ziele der geöffneten Datenbank (`None`, solange sie gesperrt ist).
    fn known_target_ids(&self) -> Option<HashSet<String>> {
        if self.locked || self.vault.is_none() {
            return None;
        }
        let mut ids = HashSet::new();
        self.index.for_each_target(|t| {
            ids.insert(t.id.to_string());
        });
        Some(ids)
    }

    pub(crate) fn webcache_status(&self) -> Value {
        let s = &self.config.web_cache;
        let (total, cache_bytes, profiles) = rj_core::config::data_dir().map(|d| cache::stats(&cache::profiles_root(&d))).unwrap_or_default();
        json!({ "limit_mb": s.limit_mb, "keep_days": s.keep_days, "bytes": total, "cache_bytes": cache_bytes, "profiles": profiles, "dir": rj_core::config::data_dir().map(|d| cache::profiles_root(&d)) })
    }

    /// Übernimmt geänderte Einstellungen aus `c` (nur vorhandene Felder).
    pub(crate) fn webcache_apply_settings(&mut self, c: &Value) {
        if let Some(n) = c.get("webcache_limit_mb").and_then(Value::as_u64) {
            self.config.web_cache.limit_mb = n.clamp(10, 2000) as u32;
        }
        if let Some(n) = c.get("webcache_keep_days").and_then(Value::as_u64) {
            self.config.web_cache.keep_days = n.clamp(1, 365) as u32;
        }
    }

    /// Aufräumen: `all` = Knopf (alle Zwischenspeicher leeren, verwaiste Profile sofort entfernen), sonst nur über
    /// der Grenze bzw. nach Ablauf der Aufbewahrung.
    pub(crate) fn webcache_cleanup(&self, all: bool) -> Value {
        let Some(dir) = rj_core::config::data_dir() else { return json!({ "error": "Kein Datenordner" }) };
        let known = self.known_target_ids();
        let in_use = self.web_profiles_in_use();
        let s = &self.config.web_cache;
        let r = cache::cleanup(
            &cache::profiles_root(&dir),
            &cache::Cleanup { limit_mb: s.limit_mb, keep_days: s.keep_days, known: known.as_ref(), in_use: &in_use, all, now: std::time::SystemTime::now() },
        );
        json!({ "freed": r.freed, "cleared": r.cleared, "removed": r.removed, "locked": known.is_none() })
    }

    /// Einmal nach dem Entsperren: im Hintergrund aufräumen (Grenze, Ablauf, Reste der Felderkennung).
    pub(crate) fn webcache_cleanup_background(&self) {
        if self.demo {
            return;
        }
        let Some(dir) = rj_core::config::data_dir() else { return };
        let known = self.known_target_ids();
        let in_use = self.web_profiles_in_use();
        let s = self.config.web_cache.clone();
        std::thread::spawn(move || {
            cache::cleanup(
                &cache::profiles_root(&dir),
                &cache::Cleanup { limit_mb: s.limit_mb, keep_days: s.keep_days, known: known.as_ref(), in_use: &in_use, all: false, now: std::time::SystemTime::now() },
            );
        });
    }

    /// Profil einer beendeten Felderkennung entfernen (der Worker kann es noch kurz belegen: einige Versuche).
    pub(crate) fn remove_probe_profile(&self, target: uuid::Uuid) {
        let Some(dir) = rj_core::config::data_dir() else { return };
        let p = rj_web::profile_dir(&dir, &format!("{target}{}", cache::PROBE_SUFFIX));
        std::thread::spawn(move || {
            for _ in 0..20 {
                if !p.exists() || std::fs::remove_dir_all(&p).is_ok() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::{app, cmd};
    use serde_json::json;

    #[test]
    fn settings_are_clamped_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let s = cmd(&mut app, json!({ "op": "webcache_status" }));
        assert_eq!((s["limit_mb"].as_u64(), s["keep_days"].as_u64()), (Some(50), Some(7)), "{s}");
        cmd(&mut app, json!({ "op": "set_settings", "webcache_limit_mb": 5, "webcache_keep_days": 0 }));
        assert_eq!((app.config.web_cache.limit_mb, app.config.web_cache.keep_days), (10, 1));
        cmd(&mut app, json!({ "op": "set_settings", "webcache_limit_mb": 80, "webcache_keep_days": 14 }));
        let s = cmd(&mut app, json!({ "op": "webcache_status" }));
        assert_eq!((s["limit_mb"].as_u64(), s["keep_days"].as_u64()), (Some(80), Some(14)), "{s}");
    }
}
