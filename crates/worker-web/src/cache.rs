//! Zwischenspeicher der Web-Profile: Größe ermitteln, leeren, „zuletzt benutzt“ festhalten und aufräumen.
//!
//! Ein Profil (`daten/web-profile/<Ziel>`) enthält die Anmeldung (Cookies, Speicher der Seite), die angenommenen
//! Zertifikate (`zertifikate.json`) und den Zwischenspeicher der Browser-Engine. Nur Letzterer wird geleert: Er
//! beschleunigt das Laden und wird bei Bedarf neu geholt.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Ordner des Zwischenspeichers relativ zum Profil eines Ziels (Linux, WebKitGTK bis 0.1.66; seit 0.1.67 hält die
/// Web-Engine ihren Zwischenspeicher nur im Arbeitsspeicher).
pub const CACHE_DIRS: &[&str] = &["WebKitCache", "CacheStorage"];

/// Windows (WebView2): gemeinsamer Browser-Ordner aller Web-Ziele unter `web-profile`. Jedes Ziel ist darin ein
/// eigenes Profil (`EBWebView/WV2Profile_<Ziel>`, eigene Cookies und Anmeldungen); Komponenten der Web-Engine liegen nur einmal
/// da statt in jedem Ziel (bis 0.1.66: ein ganzer Browser-Ordner je Ziel, rund 70 MB Komponenten je Ziel).
pub const SHARED_WEBVIEW2: &str = "_webview2";

/// Zwischenspeicher im Profil eines Ziels im gemeinsamen Browser-Ordner (relativ zu `EBWebView/WV2Profile_<Ziel>`).
pub const ENGINE_CACHE_DIRS: &[&str] = &["Cache", "Code Cache", "GPUCache", "DawnGraphiteCache", "DawnWebGPUCache", "Service Worker/CacheStorage", "Service Worker/ScriptCache"];

/// Gemeinsame Zwischenspeicher und nachgeladene Komponenten im gemeinsamen Browser-Ordner (relativ zu `EBWebView`),
/// die remotejoe nicht braucht (Kopierschutz-Modul, Filterlisten, Spracherkennung, Silbentrennung, Nutzungsmesswerte).
/// Werden nur entfernt, wenn keine Web-Sitzung läuft.
pub const SHARED_JUNK_DIRS: &[&str] = &[
    "ShaderCache",
    "GrShaderCache",
    "GraphiteDawnCache",
    "BrowserMetrics",
    "WidevineCdm",
    "component_crx_cache",
    "Subresource Filter",
    "Speech Recognition",
    "hyphen-data",
];

/// Vorsilbe, die WebView2 dem Profilnamen im Ordner gibt (`EBWebView/WV2Profile_<Name>`, auf der VM gemessen).
pub const WEBVIEW2_PROFILE_PREFIX: &str = "WV2Profile_";

/// Profil eines Ziels im gemeinsamen Browser-Ordner (Windows); `profile` ist der Ordner des Ziels unter `web-profile`.
pub fn engine_profile(profile: &Path) -> Option<PathBuf> {
    let name = profile.file_name()?.to_string_lossy();
    Some(profile.parent()?.join(SHARED_WEBVIEW2).join("EBWebView").join(format!("{WEBVIEW2_PROFILE_PREFIX}{name}")))
}

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

/// Ordner des Zwischenspeichers eines Ziels (im Ordner des Ziels und, unter Windows, in seinem Profil im
/// gemeinsamen Browser-Ordner).
fn cache_paths(profile: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = CACHE_DIRS.iter().map(|d| profile.join(d)).collect();
    if let Some(e) = engine_profile(profile) {
        v.extend(ENGINE_CACHE_DIRS.iter().map(|d| e.join(d)));
    }
    v
}

/// Größe des Zwischenspeichers eines Profils.
pub fn cache_size(profile: &Path) -> u64 {
    cache_paths(profile).iter().map(|p| dir_size(p)).sum()
}

/// Zwischenspeicher eines Profils leeren; liefert die frei gewordenen Bytes. Gesperrte Dateien (laufende Sitzung)
/// bleiben stehen.
pub fn clear_cache(profile: &Path) -> u64 {
    let mut freed = 0;
    for p in cache_paths(profile) {
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

/// Daten der Web-Engine aus Fassungen bis 0.1.66 im Profil (Linux, WebKitGTK mit dauerhaftem Profil). Seit 0.1.67
/// läuft die Web-Engine flüchtig ([`crate::cookies`]); diese Ordner werden nicht mehr benutzt. Darin lag unter
/// anderem das Schreibprotokoll des lokalen Speichers, das ohne Grenze wuchs (mehrere GB).
/// Unter Windows bis 0.1.66 der eigene Browser-Ordner je Ziel (`EBWebView`, samt Komponenten).
pub const LEGACY_ENGINE_DATA: &[&str] = &["EBWebView", "localstorage", "storage", "databases", "WebKitCache", "CacheStorage", "mediakeys", "hsts-storage.sqlite", "deviceidhashsalts"];

/// Altbestand eines Profils entfernen, das gerade nicht benutzt wird: alte Cookies einmal nach `cookies.json`
/// übernehmen, dann die alten Daten der Web-Engine löschen. Liefert die frei gewordenen Bytes.
pub fn remove_legacy(profile: &Path) -> u64 {
    let legacy = profile.join(crate::cookies::LEGACY_FILE);
    if legacy.is_file() {
        if !profile.join(crate::cookies::FILE).is_file() {
            let c = crate::cookies::load(profile);
            if crate::cookies::save(profile, &c).is_err() {
                // Ohne gesicherte Cookies nichts löschen (Anmeldungen gingen sonst verloren).
                return 0;
            }
        }
        let _ = std::fs::remove_file(&legacy);
    }
    let mut freed = 0;
    for d in LEGACY_ENGINE_DATA {
        let p = profile.join(d);
        let before = dir_size(&p);
        if before == 0 && !p.exists() {
            continue;
        }
        let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        freed += before.saturating_sub(dir_size(&p));
    }
    freed
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
    // Gemeinsamer Browser-Ordner (Windows): enthält die Profile der Ziele (oben schon als Zwischenspeicher gezählt)
    // und die gemeinsamen Teile.
    let shared = root.join(SHARED_WEBVIEW2);
    total += dir_size(&shared);
    cache += SHARED_JUNK_DIRS.iter().map(|d| dir_size(&shared.join("EBWebView").join(d))).sum::<u64>();
    (total, cache, n)
}

/// Ordner der Ziele (ohne den gemeinsamen Browser-Ordner).
fn profiles(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .map(|rd| rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.file_name() != SHARED_WEBVIEW2).map(|e| e.path()).collect())
        .unwrap_or_default()
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
        // Altbestand der Web-Engine (bis 0.1.66) immer entfernen.
        let legacy = remove_legacy(&p);
        if legacy > 0 {
            r.freed += legacy;
            r.cleared += 1;
        }
        let probe = name.ends_with(PROBE_SUFFIX);
        let orphan = c.known.is_some_and(|k| !k.contains(&name));
        if probe || (orphan && (c.all || expired(&p))) {
            // Mit dem Ziel auch sein Profil im gemeinsamen Browser-Ordner.
            if let Some(e) = engine_profile(&p).filter(|e| e.exists()) {
                let before = dir_size(&e);
                let _ = std::fs::remove_dir_all(&e);
                r.freed += before.saturating_sub(dir_size(&e));
            }
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
    // Gemeinsame Teile des Browser-Ordners nur, wenn keine Web-Sitzung läuft.
    if c.in_use.is_empty() {
        let shared = root.join(SHARED_WEBVIEW2).join("EBWebView");
        for d in SHARED_JUNK_DIRS {
            let p = shared.join(d);
            let before = dir_size(&p);
            if before > 0 {
                let _ = std::fs::remove_dir_all(&p);
                r.freed += before.saturating_sub(dir_size(&p));
            }
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
        file(&p.join("cookies.json"), 10);
        file(&p.join("zertifikate.json"), 20);
        let e = engine_profile(&p).unwrap();
        file(&e.join("Network/Cookies"), 40);
        file(&e.join("Cache/Cache_Data/a"), cache);
        file(&e.join("Code Cache/js/b"), cache);
        touch(&p);
        p
    }

    fn kept(p: &Path) -> bool {
        ["cookies.json", "zertifikate.json"].iter().all(|f| p.join(f).is_file()) && engine_profile(p).unwrap().join("Network/Cookies").is_file()
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

    /// Altbestand bis 0.1.66 (Linux): alte Cookies werden übernommen, die Daten der Web-Engine (u. a. das riesige
    /// Schreibprotokoll des lokalen Speichers) entfernt; Zertifikate und „zuletzt benutzt“ bleiben. Ein Profil in
    /// Benutzung bleibt unberührt.
    #[test]
    fn legacy_engine_data_is_removed_and_cookies_are_kept() {
        let root = tempfile::tempdir().unwrap();
        let future = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600;
        for name in ["alt", "offen"] {
            let p = root.path().join(name);
            file(&p.join("localstorage/https_192.0.2.1_0.localstorage-wal"), 5000);
            file(&p.join("WebKitCache/x"), 100);
            file(&p.join("hsts-storage.sqlite"), 10);
            file(&p.join("zertifikate.json"), 10);
            std::fs::write(p.join("cookies"), format!("192.0.2.1\tFALSE\t/\tTRUE\t{future}\tTOKEN\tabc\n")).unwrap();
        }
        let known: HashSet<String> = ["alt".to_string(), "offen".to_string()].into();
        let in_use: HashSet<String> = ["offen".to_string()].into();
        let r = cleanup(root.path(), &Cleanup { limit_mb: 50, keep_days: 7, known: Some(&known), in_use: &in_use, all: false, now: SystemTime::now() });
        assert!(r.freed >= 5110, "{r:?}");
        let alt = root.path().join("alt");
        assert!(!alt.join("localstorage").exists() && !alt.join("WebKitCache").exists() && !alt.join("hsts-storage.sqlite").exists() && !alt.join("cookies").exists());
        assert!(alt.join("zertifikate.json").exists());
        assert_eq!(crate::cookies::load(&alt)[0].value, "abc", "Anmeldung übernommen");
        assert!(root.path().join("offen/localstorage").exists(), "Profil in Benutzung bleibt unberührt");
    }


    /// Windows: gemeinsamer Browser-Ordner. Der alte Browser-Ordner je Ziel (bis 0.1.66, samt Komponenten) wird
    /// entfernt, nachgeladene Komponenten im gemeinsamen Ordner ebenfalls (nur ohne laufende Sitzung); Anmeldungen
    /// der Ziele bleiben; ein gelöschtes Ziel verschwindet samt seinem Profil im gemeinsamen Ordner.
    #[test]
    fn shared_webview2_folder_is_cleaned_without_losing_logins() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let shared = root.join(SHARED_WEBVIEW2).join("EBWebView");
        file(&shared.join("WidevineCdm/4.10/widevinecdm.dll"), 5000);
        file(&shared.join("component_crx_cache/x"), 4000);
        file(&shared.join("Local State"), 10);
        for name in ["ziel", "geloescht"] {
            file(&root.join(name).join("zertifikate.json"), 10);
            file(&root.join(name).join("EBWebView/WidevineCdm/4.10/widevinecdm.dll"), 3000);
            file(&shared.join(format!("WV2Profile_{name}")).join("Network/Cookies"), 40);
            file(&shared.join(format!("WV2Profile_{name}")).join("Cache/Cache_Data/a"), 700);
        }
        assert_eq!(stats(root).2, 2, "der gemeinsame Ordner ist kein Ziel");
        let known: HashSet<String> = ["ziel".to_string()].into();
        let none = HashSet::new();
        let r = cleanup(root, &Cleanup { limit_mb: 50, keep_days: 7, known: Some(&known), in_use: &none, all: true, now: SystemTime::now() });
        assert!(r.freed >= 5000 + 4000 + 3000 + 700, "{r:?}");
        assert!(!root.join("ziel/EBWebView").exists(), "alter Browser-Ordner des Ziels entfernt");
        assert!(!shared.join("WidevineCdm").exists() && !shared.join("component_crx_cache").exists());
        assert!(shared.join("Local State").exists() && shared.join("WV2Profile_ziel/Network/Cookies").exists(), "Anmeldung bleibt");
        assert_eq!(dir_size(&shared.join("WV2Profile_ziel/Cache")), 0);
        assert!(!root.join("geloescht").exists() && !shared.join("WV2Profile_geloescht").exists(), "gelöschtes Ziel samt Profil entfernt");
        // Läuft eine Sitzung, bleiben die gemeinsamen Teile.
        file(&shared.join("WidevineCdm/neu"), 100);
        let in_use: HashSet<String> = ["ziel".to_string()].into();
        cleanup(root, &Cleanup { limit_mb: 50, keep_days: 7, known: Some(&known), in_use: &in_use, all: true, now: SystemTime::now() });
        assert!(shared.join("WidevineCdm/neu").exists());
    }

}
