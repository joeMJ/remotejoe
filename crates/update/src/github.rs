//! Herausgeber: den von der Paketierung angelegten Entwurf einer GitHub-Veröffentlichung
//! prüfen, mit dem lokalen privaten Schlüssel signieren und veröffentlichen.
//!
//! Ablauf (`finish_release`): Entwurf zum Tag suchen (bis die Paketierung fertig ist),
//! `SHA256SUMS` und jedes darin aufgeführte Archiv laden und die Prüfsummen selbst
//! nachrechnen, `SHA256SUMS` signieren, die Signatur gegenprüfen, hochladen, Entwurf
//! veröffentlichen. Der Zugriffsschlüssel erscheint nie in Fehlermeldungen.

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use ed25519_dalek::{SigningKey, VerifyingKey};

use crate::http::{self, Control};
use crate::release::{Asset, Release};
use crate::sign::{self, SIG, SUMS};
use crate::{Limits, Result, UpdateError, Version};

/// Adressen der Programmierschnittstellen (für Tests austauschbar).
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub api: String,
    pub uploads: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints { api: "https://api.github.com".into(), uploads: "https://uploads.github.com".into() }
    }
}

/// Ergebnis: veröffentlichte Version und die geprüften Dateien.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub tag: String,
    pub archives: Vec<String>,
}

fn headers(token: &str, accept: &str) -> Vec<(String, String)> {
    vec![
        ("Authorization".into(), format!("Bearer {token}")),
        ("Accept".into(), accept.into()),
        ("X-GitHub-Api-Version".into(), "2022-11-28".into()),
    ]
}

fn ctl(cancel: Option<&AtomicBool>, secs: u64, max: u64) -> Control<'_> {
    Control::new(Limits { max_bytes: max, timeout: Duration::from_secs(secs) }, cancel)
}

/// Entwurf mit diesem Tag samt der Dateien, die die Paketierung angelegt hat.
fn find_draft(ep: &Endpoints, repo: &str, tag: &str, token: &str, cancel: Option<&AtomicBool>) -> Result<Option<(u64, Release)>> {
    let url = format!("{}/repos/{repo}/releases?per_page=30", ep.api.trim_end_matches('/'));
    let body = http::get(&url, &headers(token, "application/vnd.github+json"), "die Veröffentlichungen", &ctl(cancel, 30, 8 * 1024 * 1024))?;
    let list: Vec<serde_json::Value> = serde_json::from_slice(&body).map_err(|e| UpdateError::Parse(e.to_string()))?;
    for v in list {
        if v["tag_name"].as_str() == Some(tag) {
            let id = v["id"].as_u64().ok_or_else(|| UpdateError::Parse("Kennung der Veröffentlichung fehlt".into()))?;
            let release = Release::parse(&serde_json::to_vec(&v).map_err(|e| UpdateError::Parse(e.to_string()))?)?;
            return Ok(Some((id, release)));
        }
    }
    Ok(None)
}

/// Zusätzliche Dateien an den Entwurf hängen und `SHA256SUMS` um sie ergänzt neu hochladen.
#[allow(clippy::too_many_arguments)]
fn add_extras(
    ep: &Endpoints,
    repo: &str,
    id: u64,
    release: &Release,
    mut sums: Vec<u8>,
    extra: &[(String, Vec<u8>)],
    token: &str,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<u8>> {
    for (name, data) in extra {
        if release.assets.iter().any(|a| a.name == *name) || sign::sums_lookup(&sums, name).is_ok() {
            return Err(UpdateError::Install(format!("{name} ist im Entwurf schon vorhanden.")));
        }
        if !sums.is_empty() && !sums.ends_with(b"\n") {
            sums.push(b'\n');
        }
        sums.extend_from_slice(format!("{}  {name}\n", sign::sha256_hex(data)).as_bytes());
    }
    let upload = |name: &str, data: &[u8]| -> Result<()> {
        let url = format!("{}/repos/{repo}/releases/{id}/assets?name={name}", ep.uploads.trim_end_matches('/'));
        let mut h = headers(token, "application/vnd.github+json");
        h.push(("Content-Type".into(), "application/octet-stream".into()));
        let r = http::request("POST", &url, &h, data, &ctl(cancel, 900, 1024 * 1024))?;
        if r.status != 201 {
            return Err(UpdateError::Status { status: r.status, what: format!("Hochladen von {name}") });
        }
        Ok(())
    };
    for (name, data) in extra {
        upload(name, data)?;
    }
    // Alte Prüfsummendatei ersetzen.
    let old = release.assets.iter().find(|a| a.name == SUMS).ok_or_else(|| UpdateError::MissingFile(SUMS.into()))?;
    let r = http::request("DELETE", &old.url, &headers(token, "application/vnd.github+json"), &[], &ctl(cancel, 60, 1024 * 1024))?;
    if r.status != 204 {
        return Err(UpdateError::Status { status: r.status, what: "Ersetzen von SHA256SUMS".into() });
    }
    upload(SUMS, &sums)?;
    Ok(sums)
}

fn asset_bytes(a: &Asset, token: &str, cancel: Option<&AtomicBool>, max: u64) -> Result<Vec<u8>> {
    if a.url.is_empty() {
        return Err(UpdateError::MissingFile(a.name.clone()));
    }
    http::get(&a.url, &headers(token, "application/octet-stream"), &a.name, &ctl(cancel, 900, max))
}

/// Prüft und signiert den Entwurf zu `tag` und veröffentlicht ihn. Wartet bis `wait`, bis die
/// Paketierung `SHA256SUMS` und die Archive angehängt hat.
pub fn finish_release(
    ep: &Endpoints,
    repo: &str,
    tag: &str,
    token: &str,
    key: &SigningKey,
    public: &VerifyingKey,
    wait: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<Finished> {
    finish_release_with(ep, repo, tag, token, key, public, wait, cancel, &[])
}

/// Wie `finish_release`; `extra` (Dateiname, Inhalt) werden vor dem Signieren an den Entwurf
/// gehängt und in `SHA256SUMS` eingetragen (z. B. die unter Windows gebaute MSI), damit auch sie
/// von der Signatur gedeckt sind.
#[allow(clippy::too_many_arguments)]
pub fn finish_release_with(
    ep: &Endpoints,
    repo: &str,
    tag: &str,
    token: &str,
    key: &SigningKey,
    public: &VerifyingKey,
    wait: Duration,
    cancel: Option<&AtomicBool>,
    extra: &[(String, Vec<u8>)],
) -> Result<Finished> {
    let version = Version::parse(tag)?;
    let deadline = Instant::now() + wait;
    let (id, release) = loop {
        // Störungen beim Warten (Netz, Zeitlimit einer Anfrage, Serverfehler, Drosselung) beenden das Warten nicht;
        // erst die Frist entscheidet. Andere Fehler (Anmeldung, unlesbare Antwort) brechen sofort ab.
        let found = match find_draft(ep, repo, tag, token, cancel) {
            Ok(f) => f,
            Err(e @ (UpdateError::Net(_) | UpdateError::Timeout)) => {
                eprintln!("Hinweis beim Warten: {e} – erneuter Versuch.");
                None
            }
            Err(e @ UpdateError::Status { status, .. }) if status >= 500 || status == 429 => {
                eprintln!("Hinweis beim Warten: {e} – erneuter Versuch.");
                None
            }
            Err(e) => return Err(e),
        };
        match found {
            Some((id, r)) if r.draft && r.assets.iter().any(|a| a.name == SUMS) && r.assets.iter().any(|a| a.name.starts_with("remotejoe-") && a.name != SUMS) => break (id, r),
            Some((_, r)) if !r.draft => return Err(UpdateError::Install(format!("Die Veröffentlichung {tag} ist schon veröffentlicht."))),
            _ => {}
        }
        if cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
            return Err(UpdateError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(UpdateError::Timeout);
        }
        std::thread::sleep(Duration::from_secs(10).min(deadline.saturating_duration_since(Instant::now())));
    };
    if release.assets.iter().any(|a| a.name == SIG) {
        return Err(UpdateError::Install(format!("Der Entwurf {tag} enthält schon eine Signatur; bitte ohne sie neu anlegen.")));
    }
    let sums_asset = release.assets.iter().find(|a| a.name == SUMS).expect("geprüft");
    let mut sums = asset_bytes(sums_asset, token, cancel, 1024 * 1024)?;
    let mut release = release;
    if !extra.is_empty() {
        sums = add_extras(ep, repo, id, &release, sums, extra, token, cancel)?;
        release = match find_draft(ep, repo, tag, token, cancel)? {
            Some((_, r)) => r,
            None => return Err(UpdateError::Install(format!("Entwurf {tag} nach dem Hochladen nicht mehr gefunden."))),
        };
    }
    // Jedes Archiv, das die Paketierung angehängt hat, muss in der Prüfsummendatei stehen und stimmen.
    let mut archives = Vec::new();
    for a in release.assets.iter().filter(|a| a.name != SUMS) {
        let want = sign::sums_lookup(&sums, &a.name)?;
        let data = asset_bytes(a, token, cancel, Limits::default().max_bytes)?;
        if sign::sha256_hex(&data) != want {
            return Err(UpdateError::BadChecksum);
        }
        archives.push(a.name.clone());
    }
    let sig = sign::sign(key, &sums);
    sign::verify(public, &sums, sig.as_bytes())?;
    // Signatur hochladen.
    let url = format!("{}/repos/{repo}/releases/{id}/assets?name={SIG}", ep.uploads.trim_end_matches('/'));
    let mut h = headers(token, "application/vnd.github+json");
    h.push(("Content-Type".into(), "application/octet-stream".into()));
    let r = http::request("POST", &url, &h, sig.as_bytes(), &ctl(cancel, 120, 1024 * 1024))?;
    if r.status != 201 {
        return Err(UpdateError::Status { status: r.status, what: "Hochladen der Signatur".into() });
    }
    // Entwurf veröffentlichen (Vorabversionen bleiben als solche gekennzeichnet).
    let url = format!("{}/repos/{repo}/releases/{id}", ep.api.trim_end_matches('/'));
    let mut h = headers(token, "application/vnd.github+json");
    h.push(("Content-Type".into(), "application/json".into()));
    let body = serde_json::json!({ "draft": false, "prerelease": version.is_prerelease(), "make_latest": if version.is_prerelease() { "false" } else { "true" } });
    let r = http::request("PATCH", &url, &h, body.to_string().as_bytes(), &ctl(cancel, 60, 1024 * 1024))?;
    if r.status != 200 {
        return Err(UpdateError::Status { status: r.status, what: "Veröffentlichen".into() });
    }
    Ok(Finished { tag: tag.to_string(), archives })
}
