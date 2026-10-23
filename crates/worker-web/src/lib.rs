//! Web-Ziele: Adressprüfung, Formular-Anmeldung per Skript, eigenes
//! Profil je Ziel, Zertifikatsentscheidung und die Erweiterung des Kanals
//! für Web-Sitzungen. Die WebView selbst steckt im Worker (`main.rs`,
//! Merkmal `webview`); sie rendert ohne sichtbares Fenster in den
//! geteilten Bildspeicher.
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
    /// Zertifikatsfehler (unbekannte Zertifizierungsstelle, falscher Name, abgelaufen …)
    /// ohne Rückfrage übergehen. Nur für Geräte, an denen man nichts ändern kann.
    pub ignore_certificate: bool,
    /// Schlüssel des Profilverzeichnisses (Kennung des Ziels).
    pub profile: Option<String>,
    /// Ohne Fenster im Hauptprogramm laufen (Felderkennung): Unter Windows gibt es dann kein Elternfenster
    /// zum Einbetten, der Worker legt ein unsichtbares an.
    pub headless: bool,
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

/// Hilfsfunktionen: Suche auch in offenen Webkomponenten (Shadow-DOM) und gleichartigen Rahmen.
const HELPERS: &str = r####"function roots(){var r=[document],i=0;while(i<r.length&&r.length<200){var d=r[i++],els;try{els=d.querySelectorAll('*');}catch(x){continue;}
for(var k=0;k<els.length;k++){var e=els[k];if(e.shadowRoot)r.push(e.shadowRoot);var t=e.tagName;if(t==='IFRAME'||t==='FRAME'){try{if(e.contentDocument)r.push(e.contentDocument);}catch(x){}}}}return r;}
function qa(s){var out=[];roots().forEach(function(d){try{[].push.apply(out,d.querySelectorAll(s));}catch(x){}});return out;}
function q(s){if(!s)return null;var e=null;try{e=document.querySelector(s);}catch(x){}if(e)return e;var a=qa(s);return a.length?a[0]:null;}
function vis(e){var w=(e.ownerDocument&&e.ownerDocument.defaultView)||window,r=e.getBoundingClientRect(),c=w.getComputedStyle(e);return r.width>0&&r.height>0&&c.visibility!=='hidden'&&c.display!=='none'&&!e.disabled;}
var BTNRE=/^(anmelden|anmeldung|login|log in|sign in|einloggen)$/i;
function btn(pe){var box=pe.form||pe.getRootNode();if(!box.querySelector)return null;
var s=box.querySelector('[type=submit],button:not([type=button]):not([type=reset])');if(s)return s;
function txt(e){return String(e.value||e.textContent||'').replace(/\s+/g,' ').trim();}
var c=[].slice.call(box.querySelectorAll('button,input[type=button],[role=button]'));
for(var i=0;i<c.length;i++){if(BTNRE.test(txt(c[i]))&&vis(c[i]))return c[i];}
c=[].slice.call(box.querySelectorAll('a,div,span'));
for(i=0;i<c.length;i++){if(BTNRE.test(txt(c[i]))&&vis(c[i])&&![].some.call(c[i].children,function(k){return BTNRE.test(txt(k));}))return c[i];}return null;}
function fill(el,v){if(!el)return false;el.focus();var p=Object.getPrototypeOf(el);var d=Object.getOwnPropertyDescriptor(p,'value');
if(d&&d.set){d.set.call(el,v);}else{el.value=v;}
el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));return true;}"####;

/// Ergebnisse der Prüfung, ob nach dem Abschicken wieder ein Kennwortfeld sichtbar ist.
pub const RESULT_PASSWORD_VISIBLE: &str = "rj:kennwortfeld-da";

/// Prüfskript ohne Zugangsdaten: ist ein sichtbares Kennwortfeld vorhanden?
pub fn presence_script() -> String {
    format!("(function(){{{HELPERS}return qa('input[type=password]').filter(vis).length?'{RESULT_PASSWORD_VISIBLE}':'rj:kennwortfeld-weg';}})()")
}

/// Erzeugt das Anmeldeskript. Der Text enthält das Kennwort und wird nach
/// Gebrauch überschrieben.
pub fn login_script(l: &FormLogin, username: &str, password: &Secret) -> Result<Zeroizing<String>, WebError> {
    let pw = password.as_str().ok_or(WebError::PasswordNotUtf8)?;
    // Hilfsfunktionen: Suche auch in offenen Webkomponenten (Shadow-DOM) und gleichartigen Rahmen.
    let helpers = HELPERS;
    let body = if let Some(custom) = l.script.as_ref().filter(|s| !s.trim().is_empty()) {
        format!("var rj={{username:u,password:p,fill:function(s,v){{return fill(q(s),v);}}}};var r=(function(rj){{{custom}\n}})(rj);return r===undefined?'{RESULT_OK}':String(r);")
    } else {
        // Leere Selektoren: automatische Erkennung (sichtbares Kennwortfeld, davor liegendes
        // Textfeld als Benutzerfeld, Knopf im selben Formular).
        format!(
            "var us={us},ps={ps},ss={ss};\
var pe=ps?q(ps):(qa('input[type=password]').filter(vis)[0]||null);if(!pe)return '{RESULT_NO_PASSWORD_FIELD}';\
var ue=null;if(us){{ue=q(us);if(!ue)return '{RESULT_NO_USERNAME_FIELD}';}}else{{var box=pe.form||pe.getRootNode(),all=[].slice.call(box.querySelectorAll('input')).filter(vis),i=all.indexOf(pe);\
for(i=i-1;i>=0;i--){{var t=(all[i].type||'text').toLowerCase();if(['text','email','tel','search','url'].indexOf(t)>=0){{ue=all[i];break;}}}}}}\
if(ue)fill(ue,u);fill(pe,p);\
var se=ss?q(ss):btn(pe);\
if(se){{se.click();}}else if(pe.form){{if(pe.form.requestSubmit)pe.form.requestSubmit();else pe.form.submit();}}\
return '{RESULT_OK}';",
            us = js_opt(&l.username_selector),
            ps = js_opt(&l.password_selector),
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

// ------------------------------------------------------------ Anmeldung in Schritten

/// Schritte der Anmeldung wie von Hand. Die Skripte enthalten nie Zugangsdaten: Sie suchen,
/// fokussieren und klicken nur; getippt wird vom Worker als echte Eingabe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Benutzerfeld fokussieren und markieren.
    FocusUser,
    /// Kennwortfeld fokussieren und markieren.
    FocusPassword,
    /// Länge der eingetragenen Werte prüfen (nie der Inhalt).
    Check,
    /// Feld verlassen, auf den freigegebenen Knopf warten, dann klicken.
    Submit,
    /// Knopf blieb gesperrt: Kennwortfeld fokussieren, danach Enter als echte Taste.
    FocusForEnter,
}

/// Welches Feld der Worker tippen soll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    User,
    Password,
}

/// Art der echten Eingabe: zuerst Text einfügen wie eine Eingabehilfe, im zweiten Durchgang
/// Taste für Taste (für Seiten, die nur auf Tastendrücke reagieren).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeMode {
    Insert,
    Keys,
}

pub const RESULT_FOCUS_USER: &str = "rj:fokus-benutzer";
pub const RESULT_NO_USER_AUTO: &str = "rj:kein-benutzerfeld-auto";
pub const RESULT_FOCUS_PASSWORD: &str = "rj:fokus-kennwort";
pub const RESULT_INPUT_OK: &str = "rj:eingabe-ok";
pub const RESULT_INPUT_MISSING: &str = "rj:eingabe-fehlt";
pub const RESULT_BUTTON_LOCKED: &str = "rj:knopf-gesperrt";

/// Suche der Felder: gleiche Regeln wie im alten Anmeldeskript (Selektor oder Automatik).
const FIELDS: &str = r####"function fields(us,ps){var pe=ps?q(ps):(qa('input[type=password]').filter(vis)[0]||null);if(!pe)return null;var ue=null;
if(us){ue=q(us);}else{var box=pe.form||pe.getRootNode(),all=[].slice.call(box.querySelectorAll('input')).filter(vis),i=all.indexOf(pe);
for(i=i-1;i>=0;i--){var t=(all[i].type||'text').toLowerCase();if(['text','email','tel','search','url'].indexOf(t)>=0){ue=all[i];break;}}}return {u:ue,p:pe};}
function focusSel(e){try{e.scrollIntoView({block:'center'});}catch(x){}e.focus();try{if(e.select)e.select();}catch(x){}}
function locked(b){return !b||b.disabled||b.getAttribute('aria-disabled')==='true'||/(^|\s)(disabled|is-disabled)(\s|$)/.test(String(b.className||''));}"####;

/// Wiederholung eines Schritts: (Abstand, Höchstzahl) für Ergebnisse, die „noch nicht so weit“ bedeuten.
pub fn step_retry(result: &str) -> Option<(std::time::Duration, u32)> {
    match result.trim_matches('"') {
        // Formular wird erst nach dem Laden aufgebaut: alle 500 ms, höchstens 10 Sekunden.
        RESULT_NO_PASSWORD_FIELD => Some((std::time::Duration::from_millis(500), 20)),
        // Knopf noch gesperrt (Seite prüft die Eingabe): alle 100 ms, höchstens 3 Sekunden.
        RESULT_BUTTON_LOCKED => Some((std::time::Duration::from_millis(100), 30)),
        _ => None,
    }
}

/// Skript eines Schritts. `user_len` und `pw_len` (Zeichen) braucht nur `Check`.
pub fn step_script(l: &FormLogin, step: Step, user_len: usize, pw_len: usize) -> String {
    let (us, ps, ss) = (js_opt(&l.username_selector), js_opt(&l.password_selector), js_opt(&l.submit_selector));
    let body = match step {
        Step::FocusUser => format!(
            "var f=fields({us},{ps});if(!f)return '{RESULT_NO_PASSWORD_FIELD}';if(!f.u)return {us}?'{RESULT_NO_USERNAME_FIELD}':'{RESULT_NO_USER_AUTO}';focusSel(f.u);return '{RESULT_FOCUS_USER}';"
        ),
        Step::FocusPassword => format!("var f=fields({us},{ps});if(!f)return '{RESULT_NO_PASSWORD_FIELD}';focusSel(f.p);return '{RESULT_FOCUS_PASSWORD}';"),
        Step::Check => format!(
            "var f=fields({us},{ps});if(!f)return '{RESULT_NO_PASSWORD_FIELD}';var n=function(v){{return Array.from(String(v)).length;}},m=[];if(f.u&&n(f.u.value)!=={user_len})m.push('benutzer');if(n(f.p.value)!=={pw_len})m.push('kennwort');return m.length?'{RESULT_INPUT_MISSING}:'+m.join(','):'{RESULT_INPUT_OK}';"
        ),
        // Erst das Feld verlassen (manche Seiten übernehmen Werte erst dann), dann den Knopf
        // klicken, sobald er freigegeben ist; der Klick folgt mit kurzer Verzögerung.
        Step::Submit => format!(
            "var f=fields({us},{ps});if(!f)return '{RESULT_NO_PASSWORD_FIELD}';var a=f.p.ownerDocument.activeElement;if(a&&a.blur){{a.blur();}}f.p.dispatchEvent(new FocusEvent('focusout',{{bubbles:true}}));var se={ss}?q({ss}):btn(f.p);if(se){{if(locked(se))return '{RESULT_BUTTON_LOCKED}';setTimeout(function(){{se.click();}},400);return '{RESULT_OK}';}}if(f.p.form){{setTimeout(function(){{if(f.p.form.requestSubmit)f.p.form.requestSubmit();else f.p.form.submit();}},400);return '{RESULT_OK}';}}return '{RESULT_BUTTON_LOCKED}';"
        ),
        Step::FocusForEnter => format!("var f=fields({us},{ps});if(!f)return '{RESULT_NO_PASSWORD_FIELD}';focusSel(f.p);return '{RESULT_FOCUS_PASSWORD}';"),
    };
    format!("(function(){{{HELPERS}{FIELDS}{body}}})()")
}

/// Was der Worker nach einem Ereignis tun soll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowAction {
    None,
    /// Anmeldeskript ausführen.
    InjectLogin,
    /// Hinweis an die Oberfläche.
    Notice(String),
    /// Nach dem Abschicken prüfen, ob wieder ein Kennwortfeld erscheint (Anmeldung abgelehnt?).
    CheckRejected,
    /// Schritt-Skript ausführen (Ergebnis an `step_result`).
    Script(Step),
    /// Feld als echte Eingabe tippen (danach `typed`).
    Type(Field, TypeMode),
    /// Enter als echte Taste im fokussierten Feld (danach `typed`).
    PressEnter,
}

/// Ablauf der Formular-Anmeldung: höchstens ein Versuch je Seitenaufruf
/// und höchstens drei insgesamt, damit eine abgelehnte Anmeldung keine
/// Schleife auslöst.
#[derive(Debug, Clone)]
pub struct LoginFlow {
    login: Option<FormLogin>,
    attempts: u32,
    done: bool,
    /// Formular wurde abgeschickt; kein weiterer Versuch, nur Prüfung auf Ablehnung.
    submitted: bool,
    rejected_noted: bool,
    /// Hinweis „abgelehnt“ wurde wieder zurückgenommen (Anmeldung später doch gelungen).
    rejected_cleared: bool,
    /// Nachprüfungen, solange der Hinweis steht (manche Seiten laden nach der Anmeldung nicht neu).
    rechecks: u32,
    /// Laufende Anmeldung in Schritten: Art der Eingabe und zuletzt getipptes Feld.
    mode: TypeMode,
    last: Option<Typed>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Typed {
    User,
    Password,
    Enter,
}

impl LoginFlow {
    pub const MAX_ATTEMPTS: u32 = 3;

    pub fn new(login: Option<FormLogin>) -> Self {
        LoginFlow { login, attempts: 0, done: false, submitted: false, rejected_noted: false, rejected_cleared: false, rechecks: 0, mode: TypeMode::Insert, last: None }
    }

    pub fn page_loaded(&mut self, url: &str) -> FlowAction {
        let Some(l) = &self.login else { return FlowAction::None };
        // Die leere Startseite (vor dem ersten Laden) zählt nicht.
        if url.is_empty() || url == "about:blank" {
            return FlowAction::None;
        }
        if self.done {
            let pending = !self.rejected_noted || !self.rejected_cleared;
            return if self.submitted && pending { FlowAction::CheckRejected } else { FlowAction::None };
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
            RESULT_OK => {
                // Einmal eingetragen und abgeschickt: auf späteren Seiten nicht erneut versuchen
                // (eine abgelehnte Anmeldung darf kein Konto sperren).
                self.done = true;
                self.submitted = true;
                FlowAction::None
            }
            RESULT_NO_PASSWORD_FIELD => FlowAction::Notice("Auf dieser Seite wurde auch nach 10 Sekunden kein Kennwortfeld gefunden (bei Bedarf unter Verbindung „Felder erkennen“ oder den Selektor eintragen).".into()),
            RESULT_NO_USERNAME_FIELD => FlowAction::Notice("Das eingetragene Benutzerfeld wurde nicht gefunden (Selektor unter Verbindung prüfen oder leeren für die Automatik).".into()),
            other if other.starts_with("rj:fertig") => {
                self.done = true;
                FlowAction::None
            }
            other => FlowAction::Notice(format!("Anmeldeskript meldet: {}", rj_core::charset::mark_visible(other, false))),
        }
    }

    /// Höchstzahl der Nachprüfungen (alle 2 s, also etwa 5 Minuten), solange der Hinweis steht.
    pub const MAX_RECHECKS: u32 = 150;

    /// Ergebnis der Prüfung nach `CheckRejected`. Steht der Hinweis „abgelehnt“, wird weiter
    /// nachgeprüft (`CheckRejected`), bis kein Kennwortfeld mehr da ist; dann wird er mit einem
    /// leeren Hinweis zurückgenommen.
    pub fn rejected_check(&mut self, result: &str) -> FlowAction {
        let visible = result.trim_matches('"') == RESULT_PASSWORD_VISIBLE;
        if self.rejected_noted {
            if self.rejected_cleared {
                return FlowAction::None;
            }
            if !visible {
                self.rejected_cleared = true;
                return FlowAction::Notice(String::new());
            }
            self.rechecks += 1;
            return if self.rechecks < Self::MAX_RECHECKS { FlowAction::CheckRejected } else { FlowAction::None };
        }
        if !visible {
            return FlowAction::None;
        }
        self.rejected_noted = true;
        FlowAction::Notice("Nach dem Abschicken zeigt die Seite wieder ein Kennwortfeld: Die Anmeldung wurde vermutlich abgelehnt oder die Seite hat neu geladen. Es wird nicht erneut abgeschickt; bitte Zugangsdaten prüfen oder von Hand anmelden.".into())
    }

    /// Beginn der Anmeldung nach `InjectLogin`: mit eigenem Skript der alte Weg (`InjectLogin`),
    /// sonst Schritte mit echter Eingabe.
    pub fn begin(&mut self) -> FlowAction {
        let Some(l) = &self.login else { return FlowAction::None };
        if l.script.as_ref().is_some_and(|s| !s.trim().is_empty()) {
            return FlowAction::InjectLogin;
        }
        self.mode = TypeMode::Insert;
        self.last = None;
        FlowAction::Script(Step::FocusUser)
    }

    /// Ergebnis eines Schritt-Skripts (nach den Wiederholungen aus `step_retry`).
    pub fn step_result(&mut self, step: Step, result: &str) -> FlowAction {
        let r = result.trim_matches('"');
        match (step, r) {
            (Step::FocusUser, RESULT_FOCUS_USER) => FlowAction::Type(Field::User, self.mode),
            (Step::FocusUser, RESULT_NO_USER_AUTO) => FlowAction::Script(Step::FocusPassword),
            (Step::FocusPassword, RESULT_FOCUS_PASSWORD) => FlowAction::Type(Field::Password, self.mode),
            (Step::Check, RESULT_INPUT_OK) => FlowAction::Script(Step::Submit),
            (Step::Check, m) if m.starts_with(RESULT_INPUT_MISSING) => match self.mode {
                // Einfügen kam nicht an: zweiter Durchgang Taste für Taste.
                TypeMode::Insert => {
                    self.mode = TypeMode::Keys;
                    FlowAction::Script(Step::FocusUser)
                }
                // Auch Tastendrücke kamen nicht an: alter Weg über das Skript.
                TypeMode::Keys => FlowAction::InjectLogin,
            },
            (Step::Submit, RESULT_BUTTON_LOCKED) => FlowAction::Script(Step::FocusForEnter),
            (Step::FocusForEnter, RESULT_FOCUS_PASSWORD) => FlowAction::PressEnter,
            (Step::Submit, RESULT_OK) => self.script_result(RESULT_OK),
            _ => self.script_result(r),
        }
    }

    /// Der Worker hat ein Feld getippt bzw. Enter gedrückt.
    pub fn typed(&mut self, action: &FlowAction) -> FlowAction {
        match action {
            FlowAction::Type(Field::User, _) => {
                self.last = Some(Typed::User);
                FlowAction::Script(Step::FocusPassword)
            }
            FlowAction::Type(Field::Password, _) => {
                self.last = Some(Typed::Password);
                FlowAction::Script(Step::Check)
            }
            FlowAction::PressEnter => {
                self.last = Some(Typed::Enter);
                self.script_result(RESULT_OK)
            }
            _ => FlowAction::None,
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
    Ask { text: String, detail: String, fingerprint: String, previous: Option<String> },
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

    /// Schlüssel eines Dienstes: Rechner, bei einem anderen Port als 443 mit Port (`host:port`, IPv6 `[h]:port`).
    /// Ein Ziel mit mehreren Web-Wegen (z. B. 443 und 4343) hat so je Dienst ein eigenes Zertifikat.
    pub fn key(host: &str, port: u16) -> String {
        match port {
            443 => host.to_string(),
            p if host.contains(':') => format!("[{host}]:{p}"),
            p => format!("{host}:{p}"),
        }
    }

    /// Rechner aus einem Schlüssel (ohne Port).
    fn host_of(key: &str) -> &str {
        match key.rsplit_once(':') {
            Some((h, p)) if p.parse::<u16>().is_ok() && (h.starts_with('[') || !h.contains(':')) => h.trim_start_matches('[').trim_end_matches(']'),
            _ => key,
        }
    }

    /// Bewertung eines Zertifikats, das die Systemprüfung nicht bestanden hat
    /// (`problems` leer: gültig). `host` ist der Schlüssel aus [`AcceptedCerts::key`].
    pub fn decide(&self, host: &str, der: &[u8], problems: &[String]) -> CertDecision {
        if problems.is_empty() {
            return CertDecision::Valid;
        }
        let fp = Self::fingerprint(der);
        // Ältere Einträge stehen ohne Port: Sie gelten, wenn der Fingerabdruck passt; ein anderer Fingerabdruck
        // gehört dann womöglich zu einem anderen Dienst desselben Rechners und ist keine Änderung.
        let plain = Self::host_of(host);
        if plain != host && !self.hosts.contains_key(host) {
            return match self.hosts.get(plain) {
                Some(known) if *known == fp => CertDecision::Accepted,
                _ => CertDecision::Ask {
                    text: format!("Das Zertifikat von {host} ist nicht vertrauenswürdig."),
                    detail: format!("Probleme: {}\nFingerabdruck (SHA-256): {fp}", problems.join(", ")),
                    fingerprint: fp.clone(),
                    previous: None,
                },
            };
        }
        match self.hosts.get(host) {
            Some(known) if *known == fp => CertDecision::Accepted,
            Some(known) => CertDecision::Ask {
                text: format!("WARNUNG: Das Zertifikat von {host} hat sich geändert und ist nicht vertrauenswürdig."),
                detail: format!("Probleme: {}\nFingerabdruck (SHA-256): {fp}\nbisher angenommen: {known}", problems.join(", ")),
                fingerprint: fp.clone(),
                previous: Some(known.clone()),
            },
            None => CertDecision::Ask {
                text: format!("Das Zertifikat von {host} ist nicht vertrauenswürdig."),
                detail: format!("Probleme: {}\nFingerabdruck (SHA-256): {fp}", problems.join(", ")),
                fingerprint: fp.clone(),
                previous: None,
            },
        }
    }

    pub fn accept(&mut self, host: &str, der: &[u8]) {
        self.hosts.insert(host.to_string(), Self::fingerprint(der));
    }
}

/// Befehl der Oberfläche an den Web-Worker (`ToWorker::Custom`, JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WebCommand {
    /// Zurück, Vor, Neu laden, Anhalten.
    Nav { action: NavAction },
    /// Die Ansicht im Hauptfenster hat den Tastaturfokus erhalten oder verloren.
    Focus { on: bool },
    /// Bearbeiten: Kopieren, Ausschneiden, Einfügen, Alles auswählen.
    Edit { action: EditAction },
    /// Eingabefelder und Knöpfe der Seite erfassen (Antwort: `WebEvent::Fields`).
    Detect,
    /// Auswahl in einem Menü des Workers (`index` bei Auswahllisten,
    /// `action` beim Kontextmenü; beides leer: abgebrochen).
    Menu {
        id: u32,
        #[serde(default)]
        index: Option<u32>,
        #[serde(default)]
        action: Option<String>,
    },
    /// Windows: Fenster des Hauptprogramms (Tab-Bereich), in das die Seite als Kindfenster gesetzt wird.
    /// `hwnd` 0: lösen. Wechselt die Kennung (Tab abgerissen), kommt der Befehl erneut.
    Embed { hwnd: i64 },
    /// Windows: Bereich der Seite im Elternfenster in Bildpunkten; `scale` ist das Geräte-Pixelverhältnis
    /// des Hauptfensters in Prozent (150: 1,5; Seitenmaßstab), 0: unverändert.
    Bounds {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        #[serde(default)]
        scale: u32,
    },
    /// Windows: Seite zeigen oder ausblenden (Tabwechsel).
    Visible { on: bool },
    /// Windows: Tasten, die der Anwendung gehören (Kurzbefehle des Hauptfensters); sie gehen nicht an die Seite.
    Keys { keys: Vec<ReservedKey> },
    /// Text in der Seite suchen und markieren (nächster bzw. mit `backwards` voriger Treffer; Antwort `Found`).
    /// `restart`: Auswahl aufheben und vom Seitenanfang suchen (beim Tippen).
    Find { text: String, #[serde(default)] backwards: bool, #[serde(default)] restart: bool },
}

/// Skript für `WebCommand::Find`: `window.find` (Groß-/Kleinschreibung egal, am Ende umbrechen); Ergebnis "1"/"0".
pub fn find_script(text: &str, backwards: bool, restart: bool) -> String {
    let t = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    let reset = if restart { "try{window.getSelection().removeAllRanges();}catch(e){}" } else { "" };
    format!("(function(){{{reset}try{{return window.find({t},false,{backwards},true,false,false,false)?'1':'0';}}catch(e){{return '0';}}}})()")
}

/// Tastenkombination (Windows-Tastencode) für `WebCommand::Keys`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservedKey {
    pub vk: u32,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavAction {
    Back,
    Forward,
    Reload,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditAction {
    Copy,
    Cut,
    Paste,
    SelectAll,
}

/// Eintrag eines Menüs, das der Worker im Hauptfenster anzeigen lässt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuEntry {
    /// Kontextmenü: Aktion (`back`, `copy` …), Auswahlliste: leer.
    pub action: String,
    /// Text der Auswahlliste (auffällige Zeichen markiert); beim Kontextmenü leer,
    /// die Oberfläche nimmt ihre eigenen Texte.
    pub label: String,
    pub enabled: bool,
    pub selected: bool,
}

/// Meldung des Web-Workers an die Oberfläche (`FromWorker::Custom`, JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case")]
pub enum WebEvent {
    /// Seitentitel, Ladezustand und Navigationsmöglichkeiten.
    Status { title: String, uri: String, loading: bool, progress: u8, error: Option<String>, back: bool, forward: bool },
    /// Menü an der Stelle `x`, `y` (Bildpunkte der Seite) anzeigen;
    /// `kind`: `context` oder `option` (Auswahlliste).
    Menu { id: u32, kind: String, x: i32, y: i32, items: Vec<MenuEntry> },
    /// Gefundene Felder der Seite (Antwort auf `WebCommand::Detect`).
    Fields { users: Vec<FieldCandidate>, passwords: Vec<FieldCandidate>, submits: Vec<FieldCandidate> },
    /// Windows: Eine Taste aus `WebCommand::Keys` wurde in der Seite gedrückt und ist nicht an die Seite gegangen.
    Key { vk: u32, ctrl: bool, alt: bool, shift: bool },
    /// Windows: Die Seite hat den Tastaturfokus des Systems bekommen (`on`) oder will ihn abgeben (`on` falsch,
    /// `forward`: mit Tab weiter, sonst zurück).
    Focus { on: bool, forward: bool },
    /// Windows: Download; `state`: `started`, `progress`, `done`, `failed`. `opened`: Datei wurde mit dem
    /// zugeordneten Programm gestartet (nur .ica).
    Download { id: u32, state: String, name: String, dir: String, received: u64, total: u64, error: Option<String>, opened: bool },
    /// Ergebnis der Suche in der Seite (`WebCommand::Find`).
    Found { found: bool },
}

/// Gefundenes Feld oder gefundener Knopf mit Selektor und lesbarer Beschriftung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldCandidate {
    pub selector: String,
    pub label: String,
}

/// Skript (eigene Welt des Workers), das die sichtbaren Eingabefelder und Knöpfe der Seite
/// als JSON-Text liefert: `{users, passwords, submits}`, je Eintrag `{selector, label}`.
/// Werte der Felder werden nie gelesen.
pub const DETECT_SCRIPT: &str = r####"(function(){
function roots(){var r=[document],i=0;while(i<r.length&&r.length<200){var d=r[i++],els;try{els=d.querySelectorAll('*');}catch(x){continue;}
 for(var k=0;k<els.length;k++){var e=els[k];if(e.shadowRoot)r.push(e.shadowRoot);var t=e.tagName;if(t==='IFRAME'||t==='FRAME'){try{if(e.contentDocument)r.push(e.contentDocument);}catch(x){}}}}return r;}
function qa(s){var out=[];roots().forEach(function(d){try{[].push.apply(out,d.querySelectorAll(s));}catch(x){}});return out;}
function vis(e){var w=(e.ownerDocument&&e.ownerDocument.defaultView)||window,r=e.getBoundingClientRect(),c=w.getComputedStyle(e);return r.width>0&&r.height>0&&c.visibility!=='hidden'&&c.display!=='none'&&!e.disabled;}
function esc(s){return(window.CSS&&CSS.escape)?CSS.escape(s):s.replace(/[^a-zA-Z0-9_-]/g,'\\$&');}
function uniq(el,q){try{return el.getRootNode().querySelectorAll(q).length===1;}catch(x){return false;}}
// Zufällig erzeugte IDs (wechseln bei jedem Laden, z. B. „tHyP31-real“) taugen nicht als Selektor.
function randomId(id){return /\d[a-zA-Z]/.test(id)||/[a-z][A-Z][a-z]*[A-Z]/.test(id)||(/^[a-z0-9]{8,}$/i.test(id)&&/\d/.test(id)&&/[a-z]/i.test(id));}
function attr(el,k){var v=el.getAttribute(k);if(!v)return null;var q=el.tagName.toLowerCase()+'['+k+'="'+v.replace(/"/g,'\\"')+'"]';return uniq(el,q)?q:null;}
function sel(el){
 var root=el.getRootNode(),tag=el.tagName.toLowerCase(),q;
 // Feste Merkmale zuerst: sprechende ID, Name, Autofill-Angabe, Platzhalter, Beschriftung, Feldtyp, Klasse.
 if(el.id&&!randomId(el.id)){q='#'+esc(el.id);if(uniq(el,q))return q;}
 if((q=attr(el,'name')))return q;
 if((q=attr(el,'autocomplete'))&&!/^(on|off)$/i.test(el.getAttribute('autocomplete')))return q;
 if((q=attr(el,'placeholder')))return q;
 if((q=attr(el,'aria-label')))return q;
 var cls=String(el.className||'').split(/\s+/).filter(function(c){return c&&!randomId(c);});
 for(var ci=0;ci<cls.length;ci++){q=tag+'.'+esc(cls[ci]);if(uniq(el,q))return q;}
 if(el.type&&(q=attr(el,'type')))return q;
 if(el.id){q='#'+esc(el.id);if(uniq(el,q))return q;}
 if(root!==document){return tag+(el.type?'[type="'+el.type+'"]':'');}
 var p=[],e=el;
 while(e&&e.nodeType===1&&e!==document.body){var i=1,s=e;while((s=s.previousElementSibling))if(s.tagName===e.tagName)i++;p.unshift(e.tagName.toLowerCase()+':nth-of-type('+i+')');e=e.parentElement;}
 return 'body > '+p.join(' > ');
}
function lab(el){
 var t=el.getAttribute('aria-label')||el.getAttribute('placeholder')||'';
 if(!t&&el.id){try{var l=el.getRootNode().querySelector('label[for="'+el.id.replace(/"/g,'')+'"]');if(l)t=l.textContent;}catch(x){}}
 if(!t)t=el.getAttribute('name')||el.id||el.type||el.tagName;
 return String(t).trim().slice(0,40);
}
var us=[],ps=[],ss=[];
qa('input').filter(vis).forEach(function(e){
 var t=(e.type||'text').toLowerCase();
 if(t==='password')ps.push({selector:sel(e),label:lab(e)});
 else if(['text','email','tel','search','url'].indexOf(t)>=0)us.push({selector:sel(e),label:lab(e)});
});
qa('button,input[type=submit],input[type=button],a[role=button]').filter(vis).forEach(function(e){
 var t=String(e.value||e.textContent||e.getAttribute('aria-label')||'').trim().slice(0,40);
 ss.push({selector:sel(e),label:t||sel(e)});
});
var BTNRE=/^(anmelden|anmeldung|login|log in|sign in|einloggen)$/i,have=ss.map(function(x){return x.selector;});
qa('div,span,a,td').filter(vis).forEach(function(e){
 var t=String(e.textContent||'').replace(/\s+/g,' ').trim();
 // Nur das innerste Element mit dem Text (nicht den umgebenden Block).
 var inner=![].some.call(e.children,function(c){return BTNRE.test(String(c.textContent||'').replace(/\s+/g,' ').trim());});
 if(BTNRE.test(t)&&inner){var k=sel(e);if(have.indexOf(k)<0){have.push(k);ss.push({selector:k,label:t});}}
});
// Eigene Ausgabe statt JSON.stringify: Seiten mit prototype.js (toJSON an Array/String) verpacken sonst doppelt.
function q2(v){return '"'+String(v).replace(/[\\"\u0000-\u001f\u2028\u2029]/g,function(c){return '\\u'+('0000'+c.charCodeAt(0).toString(16)).slice(-4);})+'"';}
function lst(a){var o=[];for(var i=0;i<a.length;i++)o.push('{"selector":'+q2(a[i].selector)+',"label":'+q2(a[i].label)+'}');return '['+o.join(',')+']';}
return '{"users":'+lst(us)+',"passwords":'+lst(ps)+',"submits":'+lst(ss)+'}';
})()"####;

impl WebEvent {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("JSON")
    }
}

/// Aktionen des Kontextmenüs in der angezeigten Reihenfolge (leer: Trennlinie).
pub fn context_menu(back: bool, forward: bool, editable: bool, selection: bool, can_paste: bool) -> Vec<MenuEntry> {
    let e = |a: &str, enabled: bool| MenuEntry { action: a.into(), label: String::new(), enabled, selected: false };
    vec![
        e("back", back),
        e("forward", forward),
        e("reload", true),
        e("", false),
        e("cut", editable && selection),
        e("copy", selection || editable),
        e("paste", editable && can_paste),
        e("select_all", true),
    ]
}

/// Skript (eigene Welt des Workers) für den markierten Text; aus
/// Kennwortfeldern wird nichts kopiert.
pub const SELECTION_SCRIPT: &str = "(function(){var e=document.activeElement;\
if(e&&(e.tagName==='INPUT'||e.tagName==='TEXTAREA')&&typeof e.selectionStart==='number'){\
if(e.type==='password')return '';return e.value.substring(e.selectionStart,e.selectionEnd);}\
var s=window.getSelection();return s?s.toString():'';})()";

/// Modifikatoren aus X11-Keysyms (Umschalt, Strg, Alt).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Modifiers {
    /// Wertet eine Taste aus; `true`, wenn sie ein Modifikator ist.
    pub fn update(&mut self, keysym: u32, down: bool) -> bool {
        match keysym {
            0xffe1 | 0xffe2 => self.shift = down,
            0xffe3 | 0xffe4 => self.ctrl = down,
            0xffe9 | 0xffea | 0xffe7 | 0xffe8 => self.alt = down,
            _ => return false,
        }
        true
    }

    /// Strg+C/X/V/A, die der Worker selbst ausführt, statt sie an die Seite zu geben.
    pub fn shortcut(&self, keysym: u32) -> Option<EditAction> {
        if !self.ctrl || self.alt {
            return None;
        }
        match keysym {
            0x63 | 0x43 => Some(EditAction::Copy),
            0x78 | 0x58 => Some(EditAction::Cut),
            0x76 | 0x56 => Some(EditAction::Paste),
            _ => None,
        }
    }
}

/// Vereinigung geänderter Bereiche, beschnitten auf die Bildgröße.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Damage(pub Option<(i32, i32, i32, i32)>);

impl Damage {
    pub fn add(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.0 = Some(match self.0 {
            None => (x, y, x + w, y + h),
            Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x + w), d.max(y + h)),
        });
    }

    /// Nimmt den Bereich als (x, y, w, h) heraus, beschnitten auf `width`×`height`.
    pub fn take(&mut self, width: i32, height: i32) -> Option<(i32, i32, i32, i32)> {
        let (a, b, c, d) = self.0.take()?;
        let (a, b, c, d) = (a.max(0), b.max(0), c.min(width), d.min(height));
        (c > a && d > b).then_some((a, b, c - a, d - b))
    }
}

/// Startdateien (`.ica`) im Download-Ordner, die älter als `max_age` sind. Sie enthalten kurzlebige Anmeldetickets
/// und werden nach dem Öffnen nicht mehr gebraucht.
pub fn stale_launch_files(dir: &std::path::Path, max_age: std::time::Duration) -> Vec<std::path::PathBuf> {
    let now = std::time::SystemTime::now();
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("ica")))
        .filter(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age >= max_age)))
        .map(|e| e.path())
        .collect()
}

#[cfg(test)]
mod tests;
