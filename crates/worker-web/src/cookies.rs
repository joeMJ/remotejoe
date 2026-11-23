//! Cookies eines Web-Ziels, von remotejoe selbst gespeichert (`<Profil>/cookies.json`).
//!
//! Unter Linux läuft jedes Web-Ziel mit einem flüchtigen Profil der Web-Engine: WebKitGTK leert sonst das
//! Schreibprotokoll seines lokalen Speichers nie (`…localstorage-wal`), und eine Seite, die ständig schreibt
//! (z. B. eine Controller-Oberfläche), füllt die Platte mit über 1 GB pro Stunde. Ein flüchtiges Profil erlaubt
//! aber keine dauerhaften Cookies. Damit Anmeldungen erhalten bleiben, sichert remotejoe die dauerhaften Cookies
//! (mit Ablaufdatum) selbst und spielt sie beim Öffnen wieder ein; Sitzungs-Cookies gelten wie im Browser nur bis
//! zum Schließen.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Datei der gesicherten Cookies im Profil.
pub const FILE: &str = "cookies.json";
/// Cookie-Datei der Web-Engine aus Fassungen bis 0.1.66 (Textformat, wird einmal übernommen).
pub const LEGACY_FILE: &str = "cookies";

/// Ein dauerhaftes Cookie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    /// Ablauf (Sekunden seit 1970).
    pub expires: i64,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    /// „none“, „lax“ oder „strict“.
    #[serde(default)]
    pub same_site: String,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Gesicherte, noch gültige Cookies des Profils; übernimmt beim ersten Mal die alte Cookie-Datei der Web-Engine.
pub fn load(profile: &Path) -> Vec<StoredCookie> {
    let t = now();
    let list: Vec<StoredCookie> = match std::fs::read(profile.join(FILE)) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
        Err(_) => std::fs::read_to_string(profile.join(LEGACY_FILE)).map(|s| from_legacy(&s)).unwrap_or_default(),
    };
    list.into_iter().filter(|c| c.expires > t).collect()
}

/// Dauerhafte, noch gültige Cookies sichern (ersetzt den bisherigen Stand; atomar).
pub fn save(profile: &Path, cookies: &[StoredCookie]) -> std::io::Result<()> {
    let t = now();
    let mut list: Vec<&StoredCookie> = cookies.iter().filter(|c| c.expires > t && !c.name.is_empty()).collect();
    list.sort_by(|a, b| (&a.domain, &a.path, &a.name).cmp(&(&b.domain, &b.path, &b.name)));
    list.dedup_by(|a, b| a.domain == b.domain && a.path == b.path && a.name == b.name);
    let tmp: PathBuf = profile.join(format!("{FILE}.neu"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&list).unwrap_or_default())?;
    std::fs::rename(&tmp, profile.join(FILE))
}

/// Alte Cookie-Datei der Web-Engine (Textformat wie `cookies.txt`: Domäne, Unterdomänen, Pfad, sicher, Ablauf,
/// Name, Wert; `#HttpOnly_` vor der Domäne) lesen.
pub fn from_legacy(text: &str) -> Vec<StoredCookie> {
    let mut out = Vec::new();
    for line in text.lines() {
        let (http_only, line) = match line.strip_prefix("#HttpOnly_") {
            Some(rest) => (true, rest),
            None if line.starts_with('#') || line.trim().is_empty() => continue,
            None => (false, line),
        };
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let Ok(expires) = f[4].parse::<i64>() else { continue };
        out.push(StoredCookie {
            domain: f[0].to_string(),
            path: f[2].to_string(),
            secure: f[3].eq_ignore_ascii_case("TRUE"),
            expires,
            name: f[5].to_string(),
            value: f[6..].join("\t"),
            http_only,
            same_site: String::new(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_file_is_taken_over_once_and_expired_cookies_are_dropped() {
        let d = tempfile::tempdir().unwrap();
        let future = now() + 3600;
        std::fs::write(
            d.path().join(LEGACY_FILE),
            format!("# Netscape HTTP Cookie File\n#HttpOnly_192.0.2.10\tFALSE\t/\tTRUE\t{future}\tTOKEN\tabc\tdef\n.example.test\tTRUE\t/x\tFALSE\t{future}\tsprache\tde\nalt.test\tFALSE\t/\tFALSE\t1000\tabgelaufen\t1\nkaputt\n"),
        )
        .unwrap();
        let c = load(d.path());
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!((c[0].domain.as_str(), c[0].name.as_str(), c[0].value.as_str(), c[0].http_only, c[0].secure), ("192.0.2.10", "TOKEN", "abc\tdef", true, true));
        assert_eq!((c[1].domain.as_str(), c[1].path.as_str(), c[1].secure), (".example.test", "/x", false));
        // Gesichert: ab jetzt gilt cookies.json, die alte Datei zählt nicht mehr.
        save(d.path(), &c[..1]).unwrap();
        assert_eq!(load(d.path()), c[..1].to_vec());
        save(d.path(), &[]).unwrap();
        assert!(load(d.path()).is_empty(), "Abmeldung (keine Cookies) bleibt leer, die alte Datei kommt nicht zurück");
    }

    #[test]
    fn save_keeps_one_cookie_per_name_domain_and_path() {
        let d = tempfile::tempdir().unwrap();
        let c = |name: &str, v: &str, exp: i64| StoredCookie { name: name.into(), value: v.into(), domain: "h".into(), path: "/".into(), expires: exp, secure: false, http_only: false, same_site: "lax".into() };
        let f = now() + 60;
        save(d.path(), &[c("a", "1", f), c("a", "1", f), c("b", "2", f), c("alt", "x", 5)]).unwrap();
        let l = load(d.path());
        assert_eq!(l.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }
}
