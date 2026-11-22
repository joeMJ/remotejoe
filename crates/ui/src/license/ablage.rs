//! Zugang zur Ablage der Fassung mit Modulen: kennwortgeschützter Freigabe-Link (SeaFile, `/d/<token>/`).
//!
//! Ablauf wie im Browser: Seite laden (Cookie `csrftoken`, verborgene Felder), Kennwort per POST senden
//! (Sitzungs-Cookie), dann Ordner über `/api/v2.1/share-links/<token>/dirents/?path=…` lesen und Dateien über
//! `/d/<token>/files/?p=…&dl=1` laden. Aufbau der Ablage: je Version ein Ordner (`0.1.50/`), darin je Plattform
//! `windows/`, `ubuntu2404/`, `ubuntu2604/` mit Paket, `SHA256SUMS` und `SHA256SUMS.sig`.
//! Geprüft wird wie bei jedem Update: erst die Signatur von `SHA256SUMS` mit dem eingebauten Update-Schlüssel,
//! dann die Prüfsumme des Pakets. Das Kennwort steht nie in Protokollen oder Fehlermeldungen.

use std::sync::atomic::{AtomicBool, AtomicU64};

use rj_update::http::{self, Control};
use rj_update::Version;
use rj_update::Limits;

/// Angemeldete Sitzung an einem Freigabe-Link.
pub struct Sitzung {
    base: String,
    link: String,
    token: String,
    cookies: Vec<(String, String)>,
}

/// Ein Eintrag eines Ordners der Ablage.
#[derive(Debug, Clone, PartialEq)]
pub struct Eintrag {
    pub name: String,
    pub ordner: bool,
    pub groesse: u64,
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn form_enc(s: &str) -> String {
    enc(s).replace('/', "%2F")
}

/// `https://host/d/<token>/` → (Basis `https://host`, Token).
pub fn parse_link(url: &str) -> Result<(String, String), String> {
    let u = url.trim();
    let rest = u.strip_prefix("https://").ok_or("Der Freigabe-Link muss mit https:// beginnen.")?;
    let (host, path) = rest.split_once('/').ok_or("Freigabe-Link ohne Pfad")?;
    let token = path.strip_prefix("d/").map(|t| t.trim_end_matches('/')).filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric()))
        .ok_or("Freigabe-Link nicht im Format https://<server>/d/<kennung>/")?;
    Ok((format!("https://{host}"), token.to_string()))
}

/// Wert eines verborgenen Formularfelds (`name="…" value="…"`).
pub fn field(html: &str, name: &str) -> Option<String> {
    let key = format!("name=\"{name}\"");
    let i = html.find(&key)?;
    let tag_start = html[..i].rfind('<')?;
    let tag_end = i + html[i..].find('>')?;
    let tag = &html[tag_start..tag_end];
    let v = tag.find("value=\"")? + 7;
    let end = tag[v..].find('"')? + v;
    Some(tag[v..end].to_string())
}

/// `Set-Cookie`-Zeilen in die Sammlung übernehmen (nur Name und Wert).
pub fn merge_cookies(cookies: &mut Vec<(String, String)>, headers: &[(String, String)]) {
    for (k, v) in headers {
        if !k.eq_ignore_ascii_case("set-cookie") {
            continue;
        }
        let first = v.split(';').next().unwrap_or("").trim();
        if let Some((n, val)) = first.split_once('=') {
            let (n, val) = (n.trim().to_string(), val.trim().to_string());
            cookies.retain(|(k, _)| *k != n);
            if !val.is_empty() {
                cookies.push((n, val));
            }
        }
    }
}

fn is_password_form(html: &str) -> bool {
    html.contains("share-passwd-form")
}

impl Sitzung {
    fn cookie_header(&self) -> (String, String) {
        ("Cookie".into(), self.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; "))
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![self.cookie_header(), ("Referer".into(), self.link.clone()), ("Accept".into(), "application/json, */*".into())]
    }

    /// Am Freigabe-Link anmelden.
    pub fn anmelden(url: &str, kennwort: &str, cancel: Option<&AtomicBool>) -> Result<Sitzung, String> {
        let (base, token) = parse_link(url)?;
        let link = format!("{base}/d/{token}/");
        let ctl = Control::new(Limits { max_bytes: 2 * 1024 * 1024, timeout: std::time::Duration::from_secs(30) }, cancel);
        let mut s = Sitzung { base, link: link.clone(), token, cookies: Vec::new() };
        let r = http::request("GET", &link, &[], &[], &ctl).map_err(|e| format!("Update-Quelle nicht erreichbar: {e}"))?;
        if r.status == 404 {
            return Err("Freigabe-Link nicht gefunden (abgelaufen oder gelöscht).".into());
        }
        merge_cookies(&mut s.cookies, &r.headers);
        let html = String::from_utf8_lossy(&r.body).into_owned();
        if !is_password_form(&html) {
            return Ok(s);
        }
        let csrf = field(&html, "csrfmiddlewaretoken").ok_or("Anmeldeseite der Update-Quelle unbekannt (kein csrfmiddlewaretoken)")?;
        let form_token = field(&html, "token").unwrap_or_else(|| s.token.clone());
        let body = format!("csrfmiddlewaretoken={}&token={}&password={}", form_enc(&csrf), form_enc(&form_token), form_enc(kennwort));
        let mut hs = s.headers();
        hs.push(("Content-Type".into(), "application/x-www-form-urlencoded".into()));
        let r = http::request("POST", &link, &hs, body.as_bytes(), &ctl).map_err(|e| format!("Anmeldung an der Update-Quelle: {e}"))?;
        merge_cookies(&mut s.cookies, &r.headers);
        // Erfolg: Weiterleitung oder Seite ohne Kennwortformular.
        let ok = matches!(r.status, 301 | 302 | 303) || (r.status == 200 && !is_password_form(&String::from_utf8_lossy(&r.body)));
        if !ok {
            return Err("Zugang zur Update-Quelle abgelehnt – das Kennwort stimmt nicht mehr. Bitte eine neue Lizenz einfügen.".into());
        }
        Ok(s)
    }

    /// Einträge eines Ordners (`/` bzw. `/0.1.50/windows/`).
    pub fn ordner(&self, pfad: &str, cancel: Option<&AtomicBool>) -> Result<Vec<Eintrag>, String> {
        let ctl = Control::new(Limits { max_bytes: 2 * 1024 * 1024, timeout: std::time::Duration::from_secs(30) }, cancel);
        let url = format!("{}/api/v2.1/share-links/{}/dirents/?path={}", self.base, self.token, enc(pfad));
        let body = http::get(&url, &self.headers(), "Ordner der Update-Quelle", &ctl).map_err(|e| e.to_string())?;
        parse_dirents(&body)
    }

    /// Datei laden (mit Größengrenze und optionaler Fortschrittsanzeige).
    pub fn datei(&self, pfad: &str, limits: Limits, cancel: Option<&AtomicBool>, progress: Option<&AtomicU64>) -> Result<Vec<u8>, String> {
        let mut ctl = Control::new(limits, cancel);
        ctl.progress = progress;
        let url = format!("{}/d/{}/files/?p={}&dl=1", self.base, self.token, form_enc(pfad));
        http::get(&url, &self.headers(), pfad, &ctl).map_err(|e| e.to_string())
    }
}

/// Antwort von `dirents` lesen.
pub fn parse_dirents(body: &[u8]) -> Result<Vec<Eintrag>, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "Antwort der Update-Quelle unlesbar")?;
    let list = v.get("dirent_list").and_then(|l| l.as_array()).ok_or("Antwort der Update-Quelle ohne Liste")?;
    Ok(list
        .iter()
        .map(|e| {
            let ordner = e.get("is_dir").and_then(|b| b.as_bool()).unwrap_or(false);
            let name = e.get(if ordner { "folder_name" } else { "file_name" }).and_then(|n| n.as_str()).unwrap_or("").to_string();
            Eintrag { name, ordner, groesse: e.get("size").and_then(|s| s.as_u64()).unwrap_or(0) }
        })
        .filter(|e| !e.name.is_empty())
        .collect())
}

/// Höchste Version unter den Ordnernamen (andere Ordner und Vorabversionen zählen nicht).
pub fn neueste(eintraege: &[Eintrag]) -> Option<Version> {
    eintraege.iter().filter(|e| e.ordner).filter_map(|e| Version::parse(&e.name).ok()).filter(|v| !v.is_prerelease()).max()
}

/// Unterordner einer Version für diese Plattform (`rj_update::platform()`).
pub fn ordner_for(platform: &str) -> Result<&'static str, String> {
    match platform {
        "windows-x86_64" => Ok("windows"),
        "linux-x86_64-ubuntu2404" => Ok("ubuntu2404"),
        "linux-x86_64" => Ok("ubuntu2604"),
        other => Err(format!("Für diese Plattform ({other}) gibt es keine Pakete bei der Update-Quelle.")),
    }
}

/// Unterordner und Paketname des Programms für diese Plattform.
pub fn paket(version: &Version, platform: &str) -> Result<(&'static str, String), String> {
    let v = version.to_string();
    let ordner = ordner_for(platform)?;
    let name = match platform {
        "windows-x86_64" => format!("remotejoe-{v}-windows-x86_64.msi"),
        _ => format!("remotejoe-{v}-{platform}.tar.zst"),
    };
    Ok((ordner, name))
}

/// Datei `name` aus dem Ablageordner `dir` (mit `/` am Ende) laden und prüfen: Signatur der `SHA256SUMS` des
/// Ordners mit `key`, dann die Prüfsumme der Datei.
pub fn datei_geprueft(
    s: &Sitzung,
    dir: &str,
    name: &str,
    key: &ed25519_dalek::VerifyingKey,
    limits: Limits,
    cancel: Option<&AtomicBool>,
    progress: Option<&AtomicU64>,
) -> Result<Vec<u8>, String> {
    let small = Limits { max_bytes: 1024 * 1024, ..limits };
    let sig = s.datei(&format!("{dir}{}", rj_update::sign::SIG), small, cancel, None)?;
    let sums = s.datei(&format!("{dir}{}", rj_update::sign::SUMS), small, cancel, None)?;
    rj_update::sign::verify(key, &sums, &sig).map_err(|e| format!("Signatur der Prüfsummen: {e}"))?;
    let want = rj_update::sign::sums_lookup(&sums, name).map_err(|e| e.to_string())?;
    let data = s.datei(&format!("{dir}{name}"), limits, cancel, progress)?;
    if rj_update::sign::sha256_hex(&data) != want {
        return Err("Prüfsumme des Pakets stimmt nicht – nichts installiert.".into());
    }
    Ok(data)
}

/// Paket laden und prüfen: Signatur der `SHA256SUMS` (eingebauter Update-Schlüssel), dann Prüfsumme.
pub fn laden_geprueft(
    s: &Sitzung,
    version: &Version,
    platform: &str,
    key: &ed25519_dalek::VerifyingKey,
    limits: Limits,
    cancel: Option<&AtomicBool>,
    progress: Option<&AtomicU64>,
) -> Result<(String, Vec<u8>), String> {
    let (ordner, name) = paket(version, platform)?;
    let data = datei_geprueft(s, &format!("/{version}/{ordner}/"), &name, key, limits, cancel, progress)?;
    Ok((name, data))
}

/// Releasenotes einer Version (`RELEASENOTES-<v>.md`), falls vorhanden.
pub fn notizen(s: &Sitzung, version: &Version, cancel: Option<&AtomicBool>) -> Option<String> {
    let lim = Limits { max_bytes: 256 * 1024, timeout: std::time::Duration::from_secs(20) };
    s.datei(&format!("/{version}/RELEASENOTES-{version}.md"), lim, cancel, None).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><head><title>Private Seafile</title></head><body>
<form action="/d/a1b2c3/" method="post" id="share-passwd-form">
<input type="hidden" name="csrfmiddlewaretoken" value="Xy12abc">
<input type="hidden" name="token" value="a1b2c3" />
<input type="password" name="password" id="password" class="form-control" autofocus />
</form></body></html>"#;

    #[test]
    fn link_form_and_cookies() {
        assert_eq!(parse_link("https://ablage.example.org/d/a1b2c3/").unwrap(), ("https://ablage.example.org".into(), "a1b2c3".into()));
        assert!(parse_link("http://ablage.example.org/d/a1b2c3/").is_err());
        assert!(parse_link("https://ablage.example.org/seafdav/").is_err());
        assert!(is_password_form(PAGE));
        assert_eq!(field(PAGE, "csrfmiddlewaretoken").as_deref(), Some("Xy12abc"));
        assert_eq!(field(PAGE, "token").as_deref(), Some("a1b2c3"));
        assert_eq!(field(PAGE, "fehlt"), None);
        let mut c = Vec::new();
        merge_cookies(&mut c, &[("Set-Cookie".into(), "csrftoken=abc; Path=/; SameSite=Lax".into())]);
        merge_cookies(&mut c, &[("set-cookie".into(), "sessionid=s1; HttpOnly".into()), ("Set-Cookie".into(), "csrftoken=neu; Path=/".into())]);
        assert_eq!(c, vec![("sessionid".to_string(), "s1".to_string()), ("csrftoken".to_string(), "neu".to_string())]);
        assert_eq!(form_enc("a b&c=d/ä"), "a%20b%26c%3Dd%2F%C3%A4");
    }

    #[test]
    fn dirents_versions_and_packages() {
        let body = br#"{"dirent_list":[{"is_dir":true,"folder_name":"0.1.40","size":0},{"is_dir":true,"folder_name":"0.1.50","size":0},
            {"is_dir":true,"folder_name":"0.1.60-beta.1","size":0},{"is_dir":true,"folder_name":"alt","size":0},{"is_dir":false,"file_name":"0.1.99","size":3}]}"#;
        let e = parse_dirents(body).unwrap();
        assert_eq!(e.len(), 5);
        assert_eq!(neueste(&e).map(|v| v.to_string()).as_deref(), Some("0.1.50"));
        let v = Version::parse("0.1.50").unwrap();
        assert_eq!(paket(&v, "windows-x86_64").unwrap(), ("windows", "remotejoe-0.1.50-windows-x86_64.msi".into()));
        assert_eq!(paket(&v, "linux-x86_64-ubuntu2404").unwrap().0, "ubuntu2404");
        assert_eq!(paket(&v, "linux-x86_64").unwrap(), ("ubuntu2604", "remotejoe-0.1.50-linux-x86_64.tar.zst".into()));
        assert!(paket(&v, "macos-aarch64").is_err());
        assert!(parse_dirents(b"{}").is_err());
    }

    /// Nur von Hand (`--ignored`): echte Ablage, Kennwort aus dem Schlüsselbund (`dienst remotejoe-freigabe`),
    /// Link aus `RJ_ABLAGE_LINK`. Lädt und prüft das Paket für Ubuntu 26.04 der neuesten Version, installiert nichts.
    #[test]
    #[ignore]
    fn live_ablage_laden_und_pruefen() {
        let link = std::env::var("RJ_ABLAGE_LINK").expect("RJ_ABLAGE_LINK");
        let out = std::process::Command::new("secret-tool").args(["lookup", "dienst", "remotejoe-freigabe"]).output().unwrap();
        let pw = String::from_utf8(out.stdout).unwrap();
        assert!(!pw.is_empty(), "Kennwort im Schlüsselbund fehlt");
        let s = Sitzung::anmelden(&link, &pw, None).unwrap();
        let v = neueste(&s.ordner("/", None).unwrap()).expect("keine Version");
        let key = rj_update::sign::public_key_from_text(crate::update::UPDATE_KEY).unwrap();
        let (name, data) = laden_geprueft(&s, &v, "linux-x86_64", &key, Limits::default(), None, None).unwrap();
        eprintln!("neueste Version {v}: {name}, {} Bytes, Signatur und Prüfsumme in Ordnung", data.len());
        assert!(Sitzung::anmelden(&link, "falsch", None).is_err(), "falsches Kennwort muss scheitern");
    }
}
