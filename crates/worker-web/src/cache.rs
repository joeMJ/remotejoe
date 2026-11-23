//! Zwischenspeicher der Web-Profile: Größe ermitteln, leeren, „zuletzt benutzt“ festhalten und aufräumen.
//!
//! Ein Profil (`daten/web-profile/<Ziel>`) enthält die Anmeldung (Cookies, Speicher der Seite), die angenommenen
//! Zertifikate (`zertifikate.json`) und den Zwischenspeicher der Browser-Engine. Nur Letzterer wird geleert: Er
//! beschleunigt das Laden und wird bei Bedarf neu geholt.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Ordner des Zwischenspeichers relativ zum Profil: WebKitGTK (Linux) und WebView2 (Windows, `EBWebView`).
pub const CACHE_DIRS: &[&str] = &[
    "WebKitCache",
    "CacheStorage",
    "EBWebView/Default/Cache",
    "EBWebView/Default/Code Cache",
    "EBWebView/Default/GPUCache",
    "EBWebView/Default/DawnGraphiteCache",
    "EBWebView/Default/DawnWebGPUCache",
    "EBWebView/Default/Service Worker/CacheStorage",
    "EBWebView/Default/Service Worker/ScriptCache",
    "EBWebView/ShaderCache",
    "EBWebView/GrShaderCache",
    "EBWebView/GraphiteDawnCache",
    // Nutzungsmesswerte der Browser-Engine (wachsen mit jeder Sitzung, werden nie gebraucht).
    "EBWebView/BrowserMetrics",
];

/// Merkdatei „zuletzt benutzt“ im Profil (Inhalt ohne Belang, es zählt die Änderungszeit).
pub const LAST_USED: &str = "zuletzt-benutzt";

/// Endung der Profile der Felderkennung (nur einmal gebraucht).
pub const PROBE_SUFFIX: &str = "-erkennen";

const MB: u64 = 1024 * 1024;

/// Belegter Platz unter `p` (Dateien, rekursiv; Verknüpfungen werden nicht verfolgt).
pub fn dir_size(p: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(p) else { return 0 };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0)
}

/// Größe des Zwischenspeichers eines Profils.
pub fn cache_size(profile: &Path) -> u64 {
    CACHE_DIRS.iter().map(|d| dir_size(&profile.join(d))).sum()
}

/// Zwischenspeicher eines Profils leeren; liefert die frei gewordenen Bytes. Gesperrte Dateien (laufende Sitzung)
/// bleiben stehen.
pub fn clear_cache(profile: &Path) -> u64 {
    let mut freed = 0;
    for d in CACHE_DIRS {
        let p = profile.join(d);
        let before = dir_size(&p);
        if before == 0 {
            continue;
        }
        let _ = std::fs::remove_dir_all(&p);
        freed += before.saturating_sub(dir_size(&p));
    }
    freed
}

/// Zwischenspeicher leeren, wenn er größer als `limit_mb` ist (0 = keine Grenze).
pub fn trim(profile: &Path, limit_mb: u32) -> u64 {
    if limit_mb == 0 || cache_size(profile) <= u64::from(limit_mb) * MB {
        return 0;
    }
    clear_cache(profile)
}

/// „Zuletzt benutzt“ auf jetzt setzen (beim Öffnen einer Sitzung). Die Änderungszeit wird ausdrücklich gesetzt:
/// Unter Windows ändert das Schreiben einer leeren Datei in eine leere Datei sie nicht.
pub fn touch(profile: &Path) {
    let p = profile.join(LAST_USED);
    if let Ok(f) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&p) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Zuletzt benutzt: Merkdatei, sonst die Änderungszeit des Profilordners.
pub fn last_used(profile: &Path) -> Option<SystemTime> {
    std::fs::metadata(profile.join(LAST_USED)).or_else(|_| std::fs::metadata(profile)).and_then(|m| m.modified()).ok()
}

/// Grenze und Aufbewahrung aus der Konfiguration (für den Worker; Vorgabe 50 MB, 7 Tage).
pub fn settings() -> rj_core::config::WebCacheSettings {
    rj_core::config::default_config_path()
        .and_then(|p| rj_core::config::Config::load(&p).ok())
        .map(|c| c.web_cache)
        .unwrap_or_default()
}

/// Ordner aller Web-Profile.
pub fn profiles_root(data_dir: &Path) -> PathBuf {
    data_dir.join("web-profile")
}

/// Gesamtgröße, davon Zwischenspeicher, und Zahl der Profile.
pub fn stats(root: &Path) -> (u64, u64, usize) {
    let mut total = 0;
    let mut cache = 0;
    let mut n = 0;
    for p in profiles(root) {
        total += dir_size(&p);
        cache += cache_size(&p);
        n += 1;
    }
    (total, cache, n)
}

fn profiles(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root).map(|rd| rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect()).unwrap_or_default()
}

/// Wie aufgeräumt wird.
pub struct Cleanup<'a> {
    pub limit_mb: u32,
    pub keep_days: u32,
    /// Kennungen der Ziele der geöffneten Datenbank; `None` (gesperrt): keine Profile entfernen.
    pub known: Option<&'a HashSet<String>>,
    /// Profile laufender Sitzungen: bleiben unberührt.
    pub in_use: &'a HashSet<String>,
    /// Alle Zwischenspeicher leeren (Knopf in den Einstellungen), sonst nur über der Grenze bzw. nach Ablauf.
    pub all: bool,
    pub now: SystemTime,
}

/// Ergebnis des Aufräumens.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub freed: u64,
    /// Profile mit geleertem Zwischenspeicher.
    pub cleared: usize,
    /// Ganz entfernte Profile (Felderkennung, gelöschte Ziele).
    pub removed: usize,
}

/// Profile aufräumen:
/// - Profile der Felderkennung (`<Ziel>-erkennen`) werden immer entfernt.
/// - Profile ohne Ziel in der geöffneten Datenbank werden nach `keep_days` ohne Benutzung entfernt (mit `all` sofort).
/// - Sonst: Zwischenspeicher leeren, wenn das Ziel `keep_days` nicht geöffnet wurde, über der Grenze liegt oder `all`.
pub fn cleanup(root: &Path, c: &Cleanup) -> Report {
    let mut r = Report::default();
    let expired = |p: &Path| c.keep_days > 0 && last_used(p).and_then(|t| c.now.duration_since(t).ok()).is_some_and(|age| age > Duration::from_secs(u64::from(c.keep_days) * 86_400));
    for p in profiles(root) {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if c.in_use.contains(&name) {
            continue;
        }
        let probe = name.ends_with(PROBE_SUFFIX);
        let orphan = c.known.is_some_and(|k| !k.contains(&name));
        if probe || (orphan && (c.all || expired(&p))) {
            let before = dir_size(&p);
            if std::fs::remove_dir_all(&p).is_ok() {
                r.freed += before;
                r.removed += 1;
                continue;
            }
            r.freed += before.saturating_sub(dir_size(&p));
            continue;
        }
        let freed = if c.all || expired(&p) { clear_cache(&p) } else { trim(&p, c.limit_mb) };
        if freed > 0 {
            r.freed += freed;
            r.cleared += 1;
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(p: &Path, n: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; n]).unwrap();
    }

    fn profile(root: &Path, name: &str, cache: usize) -> PathBuf {
        let p = root.join(name);
        file(&p.join("cookies"), 10);
        file(&p.join("zertifikate.json"), 20);
        file(&p.join("storage/local.db"), 30);
        file(&p.join("EBWebView/Default/Network/Cookies"), 40);
        file(&p.join("WebKitCache/a"), cache);
        file(&p.join("EBWebView/Default/Cache/Cache_Data/b"), cache);
        touch(&p);
        p
    }

    fn kept(p: &Path) -> bool {
        ["cookies", "zertifikate.json", "storage/local.db", "EBWebView/Default/Network/Cookies"].iter().all(|f| p.join(f).is_file())
    }

    #[test]
    fn cleanup_clears_only_caches_and_removes_probe_and_orphan_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let small = profile(root, "ziel-klein", 1000);
        let big = profile(root, "ziel-gross", (MB as usize) * 2);
        let running = profile(root, "ziel-offen", (MB as usize) * 2);
        let probe = profile(root, "ziel-klein-erkennen", 100);
        let orphan = profile(root, "geloescht", 100);
        let known: HashSet<String> = ["ziel-klein", "ziel-gross", "ziel-offen"].iter().map(|s| s.to_string()).collect();
        let in_use: HashSet<String> = ["ziel-offen".to_string()].into();
        let now = SystemTime::now();
        // Grenze 1 MB: nur das große Profil wird geleert; das verwaiste bleibt (noch nicht abgelaufen).
        let r = cleanup(root, &Cleanup { limit_mb: 1, keep_days: 7, known: Some(&known), in_use: &in_use, all: false, now });
        assert_eq!((r.cleared, r.removed), (1, 1), "{r:?}");
        assert!(r.freed >= 4 * MB);
        assert!(!probe.exists() && orphan.exists());
        assert_eq!(cache_size(&big), 0);
        assert!(kept(&big) && kept(&small));
        assert!(cache_size(&small) > 0 && cache_size(&running) > 0);
        // Nach Ablauf: verwaistes Profil weg, Zwischenspeicher der übrigen leer (außer der laufenden Sitzung).
        let later = now + Duration::from_secs(8 * 86_400);
        let r = cleanup(root, &Cleanup { limit_mb: 1, keep_days: 7, known: Some(&known), in_use: &in_use, all: false, now: later });
        assert_eq!((r.cleared, r.removed), (1, 1), "{r:?}");
        assert!(!orphan.exists() && cache_size(&small) == 0 && kept(&small));
        assert!(cache_size(&running) > 0 && kept(&running));
        // Gesperrte Datenbank: nichts wird entfernt, nur Zwischenspeicher.
        let other = profile(root, "andere-db", 10);
        let r = cleanup(root, &Cleanup { limit_mb: 1, keep_days: 7, known: None, in_use: &in_use, all: true, now });
        assert_eq!(r.removed, 0);
        assert!(other.exists() && kept(&other) && cache_size(&other) == 0);
        let (total, cache, n) = stats(root);
        assert_eq!(n, 4);
        assert!(total > 0 && cache == cache_size(&running));
    }

    #[test]
    fn touch_moves_the_last_use_forward() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        touch(p);
        let old = SystemTime::now() - Duration::from_secs(30 * 86_400);
        std::fs::File::options().write(true).open(p.join(LAST_USED)).unwrap().set_modified(old).unwrap();
        assert!(last_used(p).unwrap() <= old + Duration::from_secs(1));
        touch(p);
        assert!(last_used(p).unwrap() > SystemTime::now() - Duration::from_secs(60), "zweites Öffnen zählt");
    }

    #[test]
    fn trim_respects_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let p = profile(dir.path(), "z", (MB as usize) / 2);
        assert_eq!(trim(&p, 1), 0);
        assert_eq!(trim(&p, 0), 0, "0 = keine Grenze");
        file(&p.join("CacheStorage/x"), MB as usize);
        assert!(trim(&p, 1) > MB);
        assert!(cache_size(&p) == 0 && kept(&p));
    }
}
