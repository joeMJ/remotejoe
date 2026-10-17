//! Web-Ziele: Adressprüfung, Formular-Anmeldung per Skript, eigenes
//! Profil je Ziel und Zertifikatsentscheidung. Die WebView selbst steckt im
//! Worker (`main.rs`, Merkmal `webview`).
//!
//! Das Kennwort gelangt nur über die Skript-Schnittstelle in die Seite
//! (als JSON-Zeichenkette im eingespeisten Skript), nie in die Adresse.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rj_core::Secret;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WebError {
    #[error("Adresse ungültig: {0}")]
    Url(String),
    #[error("Die Adresse enthält Zugangsdaten; diese gehören in den Zugangsdatensatz")]
    CredentialsInUrl,
    #[error("Kennwort ist kein gültiges UTF-8 und kann nicht in ein Formular eingetragen werden")]
    PasswordNotUtf8,
    #[error("Für die Formular-Anmeldung fehlt der Selektor des Kennwortfelds")]
    NoPasswordSelector,
}

/// Formular-Anmeldung: CSS-Selektoren oder ein eigenes Skript.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FormLogin {
    pub username_selector: Option<String>,
    pub password_selector: Option<String>,
    pub submit_selector: Option<String>,
    /// Nur anmelden, wenn die geladene Adresse diesen Text enthält.
    pub url_contains: Option<String>,
    /// Eigenes Skript; erhält `rj.username`, `rj.password`, `rj.fill(sel, wert)`.
    pub script: Option<String>,
}

/// Optionen eines Web-Ziels (JSON aus dem Index).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebOptions {
    /// Vollständige Adresse; sonst `https://host:port/`.
    pub url: Option<String>,
    pub login: Option<FormLogin>,
    /// Unverschlüsseltes HTTP ohne Rückfrage erlauben.
    pub allow_http: bool,
    /// Schlüssel des Profilverzeichnisses (Kennung des Ziels).
    pub profile: Option<String>,
}

/// Geprüfte Adresse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUrl {
    pub url: String,
    pub https: bool,
    pub host: String,
    pub port: u16,
}

/// Prüft eine Adresse: nur http/https, keine Zugangsdaten, Host vorhanden.
pub fn check_url(url: &str) -> Result<WebUrl, WebError> {
    let (scheme, rest) = url.split_once("://").ok_or_else(|| WebError::Url(url.into()))?;
    let https = match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => false,
        _ => return Err(WebError::Url(format!("Schema „{scheme}“ nicht erlaubt"))),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err(WebError::CredentialsInUrl);
    }
    let (host, port) = if let Some(r) = authority.strip_prefix('[') {
        let (h, p) = r.split_once(']').ok_or_else(|| WebError::Url(url.into()))?;
        (h.to_string(), p.strip_prefix(':').map(str::to_string))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p.to_string())),
            None => (authority.to_string(), None),
        }
    };
    if host.is_empty() {
        return Err(WebError::Url(url.into()));
    }
    let port = match port {
        Some(p) => p.parse().map_err(|_| WebError::Url(format!("Port „{p}“")))?,
        None if https => 443,
        None => 80,
    };
    Ok(WebUrl { url: url.to_string(), https, host, port })
}

/// Startadresse aus Optionen oder Host/Port.
pub fn start_url(opts: &WebOptions, host: &str, port: u16) -> Result<WebUrl, WebError> {
    match &opts.url {
        Some(u) if !u.is_empty() => check_url(u),
        _ => {
            let h = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() };
            let u = if port == 443 { format!("https://{h}/") } else { format!("https://{h}:{port}/") };
            check_url(&u)
        }
    }
}

/// Eigenes Profilverzeichnis je Ziel (Cookies, Speicher, Zwischenspeicher).
pub fn profile_dir(base: &Path, target_id: &str) -> PathBuf {
    let safe: String = target_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    base.join("web-profile").join(if safe.is_empty() { "_".to_string() } else { safe })
}

fn js(s: &str) -> String {
    // JSON ist gültiges JavaScript; U+2028/U+2029 zusätzlich maskieren.
    serde_json::to_string(s).expect("Zeichenkette").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

fn js_opt(s: &Option<String>) -> String {
    match s {
        Some(v) if !v.is_empty() => js(v),
        _ => "null".into(),
    }
}

/// Ergebnis des eingespeisten Skripts.
pub const RESULT_OK: &str = "rj:ok";
pub const RESULT_NO_PASSWORD_FIELD: &str = "rj:kein-kennwortfeld";
pub const RESULT_NO_USERNAME_FIELD: &str = "rj:kein-benutzerfeld";

/// Erzeugt das Anmeldeskript. Der Text enthält das Kennwort und wird nach
/// Gebrauch überschrieben.
pub fn login_script(l: &FormLogin, username: &str, password: &Secret) -> Result<Zeroizing<String>, WebError> {
    let pw = password.as_str().ok_or(WebError::PasswordNotUtf8)?;
    let helpers = "function q(s){return s?document.querySelector(s):null;}\
function fill(el,v){if(!el)return false;el.focus();var p=Object.getPrototypeOf(el);var d=Object.getOwnPropertyDescriptor(p,'value');\
if(d&&d.set){d.set.call(el,v);}else{el.value=v;}\
el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));return true;}";
    let body = if let Some(custom) = l.script.as_ref().filter(|s| !s.trim().is_empty()) {
        format!("var rj={{username:u,password:p,fill:function(s,v){{return fill(q(s),v);}}}};var r=(function(rj){{{custom}\n}})(rj);return r===undefined?'{RESULT_OK}':String(r);")
    } else {
        let psel = js_opt(&l.password_selector);
        if psel == "null" {
            return Err(WebError::NoPasswordSelector);
        }
        format!(
            "var us={us},ps={psel},ss={ss};var pe=q(ps);if(!pe)return '{RESULT_NO_PASSWORD_FIELD}';\
var ue=q(us);if(us&&!ue)return '{RESULT_NO_USERNAME_FIELD}';if(ue)fill(ue,u);fill(pe,p);\
var se=q(ss);if(se){{se.click();}}else if(pe.form){{if(pe.form.requestSubmit)pe.form.requestSubmit();else pe.form.submit();}}\
return '{RESULT_OK}';",
            us = js_opt(&l.username_selector),
            ss = js_opt(&l.submit_selector),
        )
    };
    let mut s = Zeroizing::new(String::new());
    s.push_str("(function(){");
    s.push_str(helpers);
    s.push_str("var u=");
    s.push_str(&js(username));
    s.push_str(",p=");
    let pj = Zeroizing::new(js(pw));
    s.push_str(&pj);
    s.push(';');
    s.push_str(&body);
    s.push_str("})()");
    Ok(s)
}

/// Was der Worker nach einem Ereignis tun soll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowAction {
    None,
    /// Anmeldeskript ausführen.
    InjectLogin,
    /// Hinweis an die Oberfläche.
    Notice(String),
}

/// Ablauf der Formular-Anmeldung: höchstens ein Versuch je Seitenaufruf
/// und höchstens drei insgesamt, damit eine abgelehnte Anmeldung keine
/// Schleife auslöst.
#[derive(Debug, Clone)]
pub struct LoginFlow {
    login: Option<FormLogin>,
    attempts: u32,
    done: bool,
}

impl LoginFlow {
    pub const MAX_ATTEMPTS: u32 = 3;

    pub fn new(login: Option<FormLogin>) -> Self {
        LoginFlow { login, attempts: 0, done: false }
    }

    pub fn page_loaded(&mut self, url: &str) -> FlowAction {
        let Some(l) = &self.login else { return FlowAction::None };
        if self.done {
            return FlowAction::None;
        }
        if let Some(m) = l.url_contains.as_ref().filter(|m| !m.is_empty()) {
            if !url.contains(m.as_str()) {
                return FlowAction::None;
            }
        }
        if self.attempts >= Self::MAX_ATTEMPTS {
            self.done = true;
            return FlowAction::Notice("Automatische Anmeldung nach drei Versuchen abgebrochen; bitte von Hand anmelden.".into());
        }
        self.attempts += 1;
        FlowAction::InjectLogin
    }

    pub fn script_result(&mut self, result: &str) -> FlowAction {
        let r = result.trim_matches('"');
        match r {
            RESULT_OK => FlowAction::None,
            RESULT_NO_PASSWORD_FIELD => FlowAction::Notice("Kennwortfeld nicht gefunden (Selektor prüfen).".into()),
            RESULT_NO_USERNAME_FIELD => FlowAction::Notice("Benutzerfeld nicht gefunden (Selektor prüfen).".into()),
            other if other.starts_with("rj:fertig") => {
                self.done = true;
                FlowAction::None
            }
            other => FlowAction::Notice(format!("Anmeldeskript meldet: {}", rj_core::charset::mark_visible(other, false))),
        }
    }

    pub fn login(&self) -> Option<&FormLogin> {
        self.login.as_ref()
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

/// Entscheidung zu einem Serverzertifikat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertDecision {
    /// Gültig nach Systemprüfung.
    Valid,
    /// Ungültig, aber dieser Fingerabdruck wurde für den Host angenommen.
    Accepted,
    /// Rückfrage mit Warnung.
    Ask { text: String, detail: String },
}

/// Angenommene Zertifikate je Profil (Host → SHA-256-Fingerabdruck).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedCerts {
    pub hosts: BTreeMap<String, String>,
}

impl AcceptedCerts {
    pub fn path(profile: &Path) -> PathBuf {
        profile.join("zertifikate.json")
    }

    pub fn load(profile: &Path) -> AcceptedCerts {
        std::fs::read(Self::path(profile)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, profile: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(profile)?;
        rj_core::fsutil::atomic_write_with_backup(&Self::path(profile), &serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)
    }

    pub fn fingerprint(der: &[u8]) -> String {
        use sha2::Digest;
        sha2::Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
    }

    /// Bewertung eines Zertifikats, das die Systemprüfung nicht bestanden hat
    /// (`problems` leer: gültig).
    pub fn decide(&self, host: &str, der: &[u8], problems: &[String]) -> CertDecision {
        if problems.is_empty() {
            return CertDecision::Valid;
        }
        let fp = Self::fingerprint(der);
        match self.hosts.get(host) {
            Some(known) if *known == fp => CertDecision::Accepted,
            Some(known) => CertDecision::Ask {
                text: format!("WARNUNG: Das Zertifikat von {host} hat sich geändert und ist nicht vertrauenswürdig."),
                detail: format!("Probleme: {}\nFingerabdruck (SHA-256): {fp}\nbisher angenommen: {known}", problems.join(", ")),
            },
            None => CertDecision::Ask {
                text: format!("Das Zertifikat von {host} ist nicht vertrauenswürdig."),
                detail: format!("Probleme: {}\nFingerabdruck (SHA-256): {fp}", problems.join(", ")),
            },
        }
    }

    pub fn accept(&mut self, host: &str, der: &[u8]) {
        self.hosts.insert(host.to_string(), Self::fingerprint(der));
    }
}

#[cfg(test)]
mod tests;
