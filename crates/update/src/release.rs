//! Quellen (Gitea, GitHub), Veröffentlichungen abrufen und Paket laden.

use std::sync::atomic::{AtomicBool, AtomicU64};

use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::http::{self, Control};
use crate::sign::{self, SIG, SUMS};
use crate::{asset_name, Limits, Result, UpdateError, Version};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// Keine Quelle (Voreinstellung).
    #[default]
    None,
    Gitea,
    Github,
}

/// Eine Update-Quelle (ohne Zugriffsschlüssel; der kommt aus der Schlüsseldatenbank).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub kind: SourceKind,
    /// Gitea: Basisadresse (z. B. `https://git.example.test`); GitHub: leer für
    /// `https://api.github.com`.
    pub base_url: String,
    /// `besitzer/name`.
    pub repo: String,
}

fn valid_repo(r: &str) -> bool {
    let mut parts = r.split('/');
    let ok = |p: Option<&str>| p.is_some_and(|s| !s.is_empty() && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)));
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

impl Source {
    fn base(&self) -> String {
        match self.kind {
            SourceKind::Github if self.base_url.trim().is_empty() => "https://api.github.com".into(),
            _ => self.base_url.trim().trim_end_matches('/').to_string(),
        }
    }

    /// Adresse der neuesten Veröffentlichung.
    pub fn latest_url(&self) -> Result<String> {
        if !valid_repo(&self.repo) {
            return Err(UpdateError::Url(format!("Repository „{}“ (erwartet besitzer/name)", self.repo)));
        }
        let url = match self.kind {
            SourceKind::None => return Err(UpdateError::NoSource),
            SourceKind::Gitea if self.base().is_empty() => return Err(UpdateError::NoSource),
            SourceKind::Gitea => format!("{}/api/v1/repos/{}/releases/latest", self.base(), self.repo),
            SourceKind::Github => format!("{}/repos/{}/releases/latest", self.base(), self.repo),
        };
        http::Url::parse(&url)?;
        Ok(url)
    }

    /// Webseite der neuesten Veröffentlichung (zum Öffnen im Browser), z. B.
    /// `https://github.com/joeMJ/remotejoe/releases/latest`.
    pub fn releases_page(&self) -> Result<String> {
        if !valid_repo(&self.repo) {
            return Err(UpdateError::Url(format!("Repository „{}“ (erwartet besitzer/name)", self.repo)));
        }
        let web = match self.kind {
            SourceKind::None => return Err(UpdateError::NoSource),
            SourceKind::Github if self.base_url.trim().is_empty() => "https://github.com".to_string(),
            _ if self.base().is_empty() => return Err(UpdateError::NoSource),
            _ => self.base(),
        };
        let url = format!("{web}/{}/releases/latest", self.repo);
        http::Url::parse(&url)?;
        Ok(url)
    }

    /// Kopfzeilen für die Programmierschnittstelle; der Schlüssel wird nie ausgegeben.
    pub fn headers(&self, token: Option<&str>, binary: bool) -> Vec<(String, String)> {
        let mut h = Vec::new();
        match self.kind {
            SourceKind::Github => {
                h.push(("Accept".into(), if binary { "application/octet-stream" } else { "application/vnd.github+json" }.into()));
                if let Some(t) = token.filter(|t| !t.is_empty()) {
                    h.push(("Authorization".into(), format!("Bearer {t}")));
                }
            }
            _ => {
                h.push(("Accept".into(), if binary { "application/octet-stream" } else { "application/json" }.into()));
                if let Some(t) = token.filter(|t| !t.is_empty()) {
                    h.push(("Authorization".into(), format!("token {t}")));
                }
            }
        }
        h
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    /// Öffentliche Download-Adresse.
    #[serde(default)]
    pub browser_download_url: String,
    /// GitHub: Adresse der Programmierschnittstelle (für private Repositorys).
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub size: u64,
}

/// Antwort von `/releases/latest` (Gitea und GitHub haben dieselben Felder).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn parse(json: &[u8]) -> Result<Release> {
        serde_json::from_slice(json).map_err(|e| UpdateError::Parse(e.to_string()))
    }

    /// Änderungstext als reiner Text (Steuerzeichen außer Zeilenumbruch und Tab markiert).
    pub fn notes(&self) -> String {
        let body = self.body.clone().unwrap_or_default().replace("\r\n", "\n");
        body.split('\n').map(|l| rj_core::charset::mark_visible(l, false).into_owned()).collect::<Vec<_>>().join("\n")
    }
}

/// Gefundene neuere Veröffentlichung mit den drei Dateien.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub release: Release,
    pub version: Version,
    pub archive: Asset,
    pub sums: Asset,
    pub sig: Asset,
}

/// Wählt Archiv, Prüfsummen und Signatur für die Plattform.
pub fn select(release: &Release, platform: &str) -> Result<Found> {
    let version = Version::parse(&release.tag_name)?;
    let want = asset_name(&version.to_string(), platform);
    let find = |n: &str| release.assets.iter().find(|a| a.name == n).cloned();
    let archive = find(&want).ok_or_else(|| UpdateError::NoAsset(platform.into()))?;
    let sums = find(SUMS).ok_or_else(|| UpdateError::MissingFile(SUMS.into()))?;
    let sig = find(SIG).ok_or_else(|| UpdateError::MissingFile(SIG.into()))?;
    Ok(Found { release: release.clone(), version, archive, sums, sig })
}

/// Ergebnis einer Prüfung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// Neuere Version gefunden.
    Newer(Box<Found>),
    /// Die neueste Veröffentlichung ist nicht neuer (Version angegeben).
    UpToDate(Version),
}

/// Fragt die neueste Veröffentlichung ab (ohne Auswahl der Dateien).
pub fn latest(source: &Source, token: Option<&str>, cancel: Option<&AtomicBool>) -> Result<Release> {
    let ctl = Control::new(Limits { max_bytes: 4 * 1024 * 1024, timeout: std::time::Duration::from_secs(30) }, cancel);
    let body = http::get(&source.latest_url()?, &source.headers(token, false), "die neueste Veröffentlichung", &ctl)?;
    Release::parse(&body)
}

/// Fragt die neueste Veröffentlichung ab und vergleicht mit `current`.
/// Entwürfe zählen nicht; ältere Versionen werden nie angeboten.
pub fn check(source: &Source, token: Option<&str>, current: &Version, platform: &str, cancel: Option<&AtomicBool>) -> Result<Check> {
    let release = latest(source, token, cancel)?;
    let v = Version::parse(&release.tag_name)?;
    if release.draft || v <= *current {
        return Ok(Check::UpToDate(v));
    }
    Ok(Check::Newer(Box::new(select(&release, platform)?)))
}

fn fetch(source: &Source, token: Option<&str>, a: &Asset, ctl: &Control) -> Result<Vec<u8>> {
    // Privates GitHub-Repository: Datei über die Programmierschnittstelle laden.
    let url = if source.kind == SourceKind::Github && token.is_some() && !a.url.is_empty() { &a.url } else { &a.browser_download_url };
    if url.is_empty() {
        return Err(UpdateError::MissingFile(a.name.clone()));
    }
    http::get(url, &source.headers(token, true), &a.name, ctl)
}

/// Lädt Signatur, Prüfsummen und Archiv; prüft erst die Signatur, dann die
/// Prüfsumme. Liefert die geprüften Bytes des Archivs.
pub fn download_verified(source: &Source, token: Option<&str>, found: &Found, key: &VerifyingKey, limits: Limits, cancel: Option<&AtomicBool>) -> Result<Vec<u8>> {
    download_verified_progress(source, token, found, key, limits, cancel, None)
}

/// Wie `download_verified`; `progress` zählt die gelesenen Bytes des Pakets mit (Fortschrittsanzeige).
pub fn download_verified_progress(
    source: &Source,
    token: Option<&str>,
    found: &Found,
    key: &VerifyingKey,
    limits: Limits,
    cancel: Option<&AtomicBool>,
    progress: Option<&AtomicU64>,
) -> Result<Vec<u8>> {
    let mut ctl = Control::new(limits, cancel);
    ctl.progress = progress;
    let small = Control { limits: Limits { max_bytes: 1024 * 1024, ..limits }, cancel, deadline: ctl.deadline, progress: None };
    let sig = fetch(source, token, &found.sig, &small)?;
    let sums = fetch(source, token, &found.sums, &small)?;
    sign::verify(key, &sums, &sig)?;
    let want = sign::sums_lookup(&sums, &found.archive.name)?;
    if found.archive.size > limits.max_bytes {
        return Err(UpdateError::TooLarge(limits.max_bytes));
    }
    let data = fetch(source, token, &found.archive, &ctl)?;
    if sign::sha256_hex(&data) != want {
        return Err(UpdateError::BadChecksum);
    }
    Ok(data)
}

/// Zugriffsschlüssel als UTF-8 (unverändert, ohne Kürzen).
pub fn token_text(secret: &[u8]) -> Result<Zeroizing<String>> {
    std::str::from_utf8(secret).map(|s| Zeroizing::new(s.to_string())).map_err(|_| UpdateError::Parse("Zugriffsschlüssel ist kein gültiges UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GITEA: &str = r#"{"id":7,"tag_name":"v0.3.0","name":"0.3.0","body":"Neu:\r\n- Web im Tab\u0007","draft":false,"prerelease":false,
"assets":[{"id":1,"name":"remotejoe-0.3.0-linux-x86_64.tar.zst","size":10,"browser_download_url":"https://git.example.test/a/b/releases/download/v0.3.0/remotejoe-0.3.0-linux-x86_64.tar.zst"},
{"id":2,"name":"SHA256SUMS","size":1,"browser_download_url":"https://git.example.test/x/SHA256SUMS"},
{"id":3,"name":"SHA256SUMS.sig","size":1,"browser_download_url":"https://git.example.test/x/SHA256SUMS.sig"}]}"#;
    const GITHUB: &str = r#"{"url":"https://api.github.com/repos/a/b/releases/1","tag_name":"1.0.0-rc.2","name":null,"body":null,"draft":false,"prerelease":true,
"assets":[{"url":"https://api.github.com/repos/a/b/releases/assets/9","name":"remotejoe-1.0.0-rc.2-windows-x86_64.zip","size":5,"browser_download_url":"https://github.com/a/b/releases/download/1.0.0-rc.2/remotejoe-1.0.0-rc.2-windows-x86_64.zip"},
{"url":"u","name":"SHA256SUMS","size":1,"browser_download_url":"https://github.com/s"},{"url":"u","name":"SHA256SUMS.sig","size":1,"browser_download_url":"https://github.com/t"}]}"#;

    #[test]
    fn parse_and_select() {
        let r = Release::parse(GITEA.as_bytes()).unwrap();
        let f = select(&r, "linux-x86_64").unwrap();
        assert_eq!(f.version.to_string(), "0.3.0");
        assert_eq!(f.archive.name, "remotejoe-0.3.0-linux-x86_64.tar.zst");
        assert!(matches!(select(&r, "windows-x86_64"), Err(UpdateError::NoAsset(_))));
        assert_eq!(r.notes(), "Neu:\n- Web im Tab⟦U+0007⟧");
        let r = Release::parse(GITHUB.as_bytes()).unwrap();
        let f = select(&r, "windows-x86_64").unwrap();
        assert!(f.version.is_prerelease());
        assert_eq!(f.archive.url, "https://api.github.com/repos/a/b/releases/assets/9");
        assert!(Release::parse(b"{}").is_err());
    }

    #[test]
    fn urls_and_headers() {
        let g = Source { kind: SourceKind::Gitea, base_url: "https://git.example.test/".into(), repo: "team/remotejoe".into() };
        assert_eq!(g.latest_url().unwrap(), "https://git.example.test/api/v1/repos/team/remotejoe/releases/latest");
        assert!(g.headers(Some("abc"), false).contains(&("Authorization".into(), "token abc".into())));
        assert!(!g.headers(None, false).iter().any(|(k, _)| k == "Authorization"));
        let h = Source { kind: SourceKind::Github, base_url: String::new(), repo: "a/b".into() };
        assert_eq!(h.latest_url().unwrap(), "https://api.github.com/repos/a/b/releases/latest");
        assert_eq!(h.releases_page().unwrap(), "https://github.com/a/b/releases/latest");
        assert_eq!(g.releases_page().unwrap(), "https://git.example.test/team/remotejoe/releases/latest");
        assert!(Source { repo: "../b".into(), ..h.clone() }.releases_page().is_err());
        for bad in ["a", "a/b/c", "../b", "a/ b"] {
            assert!(Source { repo: bad.into(), ..g.clone() }.latest_url().is_err(), "{bad}");
        }
        assert!(matches!(Source { kind: SourceKind::None, ..g.clone() }.latest_url(), Err(UpdateError::NoSource)));
        assert!(matches!(Source { base_url: String::new(), ..g.clone() }.latest_url(), Err(UpdateError::NoSource)));
        assert!(matches!(Source { base_url: "http://git.example.test".into(), ..g }.latest_url(), Err(UpdateError::Insecure(_))));
    }
}
