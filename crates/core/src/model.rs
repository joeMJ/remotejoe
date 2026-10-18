//! Datenmodell des Index.
//!
//! Der Index beschreibt Struktur und Einstellungen (Ordner, Ziele, Tunnel,
//! SNMP-Geräte, Aufgaben, Makros, Ansichten). Geheimnisse liegen nie im Index,
//! sondern in der KDBX; der Index verweist per UUID auf KDBX-Einträge.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// Aktuelle Schema-Version des Index.
pub const SCHEMA_VERSION: u32 = 3;

/// Name des Zugangsdatensatzes, wenn das Ziel keinen nennt.
pub const DEFAULT_CREDENTIAL: &str = "default";

/// Gesamter Index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub schema_version: u32,
    /// Revisionszähler; wird bei jedem Speichern erhöht.
    #[serde(default)]
    pub revision: u64,
    pub root: Folder,
    #[serde(default)]
    pub tunnels: Vec<TunnelDef>,
    #[serde(default)]
    pub snmp_devices: Vec<SnmpDevice>,
    #[serde(default)]
    pub tasks: Vec<TaskDef>,
    #[serde(default)]
    pub macros: Vec<MacroDef>,
    #[serde(default)]
    pub views: Vec<View>,
    /// Vorlagen für neue Ziele (ohne Kennwörter).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<Template>,
    /// Favoriten (Ziele oder Ordner) in Anzeigereihenfolge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub favorites: Vec<Uuid>,
}

impl Default for Index {
    fn default() -> Self {
        Index {
            schema_version: SCHEMA_VERSION,
            revision: 0,
            root: Folder::new("Wurzel"),
            tunnels: Vec::new(),
            snmp_devices: Vec::new(),
            tasks: Vec::new(),
            macros: Vec::new(),
            views: Vec::new(),
            templates: Vec::new(),
            favorites: Vec::new(),
        }
    }
}

/// Ordner im Baum. Die Reihenfolge der Kinder ist fest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: Uuid,
    pub name: String,
    #[serde(default, skip_serializing_if = "Settings::is_empty")]
    pub settings: Settings,
    #[serde(default, skip_serializing_if = "Meta::is_empty")]
    pub meta: Meta,
    #[serde(default)]
    pub children: Vec<Node>,
}

impl Folder {
    pub fn new(name: impl Into<String>) -> Self {
        Folder {
            id: Uuid::new_v4(),
            name: name.into(),
            settings: Settings::default(),
            meta: Meta::default(),
            children: Vec::new(),
        }
    }
}

/// Knoten im Baum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Node {
    Folder(Folder),
    Target(Target),
}

impl Node {
    pub fn id(&self) -> Uuid {
        match self {
            Node::Folder(f) => f.id,
            Node::Target(t) => t.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Node::Folder(f) => &f.name,
            Node::Target(t) => &t.name,
        }
    }

    pub fn settings(&self) -> &Settings {
        match self {
            Node::Folder(f) => &f.settings,
            Node::Target(t) => &t.settings,
        }
    }
}

/// Protokoll eines Ziels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Rdp,
    Ssh,
    Vnc,
    Web,
    Proxmox,
    Hyperv,
    Psrp,
}

impl Protocol {
    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Rdp | Protocol::Hyperv => 3389,
            Protocol::Ssh => 22,
            Protocol::Vnc => 5900,
            Protocol::Web => 443,
            Protocol::Proxmox => 8006,
            Protocol::Psrp => 5986,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Rdp => "rdp",
            Protocol::Ssh => "ssh",
            Protocol::Vnc => "vnc",
            Protocol::Web => "web",
            Protocol::Proxmox => "proxmox",
            Protocol::Hyperv => "hyperv",
            Protocol::Psrp => "psrp",
        }
    }
}

/// Ziel einer Verbindung.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub id: Uuid,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    /// Fehlt der Port, gilt der Standardport des Protokolls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "ProtocolOptions::is_none")]
    pub options: ProtocolOptions,
    /// Name des benötigten Zugangsdatensatzes; sonst `default`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, skip_serializing_if = "Settings::is_empty")]
    pub settings: Settings,
    #[serde(default, skip_serializing_if = "Meta::is_empty")]
    pub meta: Meta,
}

impl Target {
    pub fn new(name: impl Into<String>, protocol: Protocol, host: impl Into<String>) -> Self {
        Target {
            id: Uuid::new_v4(),
            name: name.into(),
            protocol,
            host: host.into(),
            port: None,
            options: ProtocolOptions::None,
            credential: None,
            settings: Settings::default(),
            meta: Meta::default(),
        }
    }

    pub fn effective_port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.protocol.default_port())
    }

    pub fn credential_name(&self) -> &str {
        self.credential.as_deref().unwrap_or(DEFAULT_CREDENTIAL)
    }
}

/// Protokollspezifische Optionen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ProtocolOptions {
    #[default]
    None,
    Rdp(RdpOptions),
    Ssh(SshOptions),
    Vnc(VncOptions),
    Web(WebOptions),
}

impl ProtocolOptions {
    pub fn is_none(&self) -> bool {
        matches!(self, ProtocolOptions::None)
    }
}

/// Richtung der Zwischenablage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardDirection {
    Off,
    HostToTarget,
    TargetToHost,
    #[default]
    Both,
}

/// Grafikkanal (RDPGFX) einer RDP-Verbindung. GNOME-Remote-Desktop verlangt ihn; `Auto` versucht
/// es zuerst ohne und bei einem Abbruch im Verbindungsaufbau einmal mit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GfxMode {
    #[default]
    Auto,
    On,
    Off,
}

impl GfxMode {
    pub fn is_auto(&self) -> bool {
        *self == GfxMode::Auto
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RdpOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default)]
    pub clipboard: ClipboardDirection,
    /// Größenlimit der Zwischenablage in Bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_layout: Option<String>,
    #[serde(default)]
    pub ignore_certificate: bool,
    #[serde(default, skip_serializing_if = "GfxMode::is_auto")]
    pub gfx: GfxMode,
    /// Tastenfolge, die nach dem Verbinden automatisch gesendet wird.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_keys: Option<crate::autokeys::AutoKeys>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SshOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term: Option<String>,
    /// OSC 52: Schreiben der lokalen Zwischenablage durch das Ziel.
    #[serde(default = "yes")]
    pub osc52_write: bool,
    /// OSC 52: Lesen der lokalen Zwischenablage (fragt nach).
    #[serde(default)]
    pub osc52_read: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive_secs: Option<u32>,
    /// Ältere, unsichere SSH-Verfahren zusätzlich zulassen (für alte Geräte).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legacy_algorithms: bool,
    /// Tastenfolge, die nach dem Verbinden automatisch gesendet wird (z. B. Verzeichniswechsel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_keys: Option<crate::autokeys::AutoKeys>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VncOptions {
    #[serde(default)]
    pub clipboard: ClipboardDirection,
    #[serde(default)]
    pub view_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WebOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_selector: Option<String>,
    #[serde(default)]
    pub ignore_certificate: bool,
}

/// Verweis auf einen KDBX-Eintrag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRef {
    /// UUID des KDBX-Eintrags.
    pub entry: Uuid,
    /// Zuletzt bekannter Titel des Eintrags (für Meldungen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_title: Option<String>,
}

/// Überschreibbare Felder eines Zugangsdatensatzes. Das Kennwort gehört
/// ausdrücklich nicht dazu.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

impl FieldOverrides {
    pub fn is_empty(&self) -> bool {
        self.username.is_none() && self.domain.is_none()
    }
}

/// Tunnel-Einstellung an Ordner oder Ziel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelSetting {
    /// Vererbten Tunnel abschalten.
    None,
    /// Tunnel aus der Tunnelliste des Index.
    Use(Uuid),
}

impl Serialize for TunnelSetting {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            TunnelSetting::None => s.serialize_str("none"),
            TunnelSetting::Use(id) => s.serialize_str(&id.to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for TunnelSetting {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == "none" {
            Ok(TunnelSetting::None)
        } else {
            Uuid::parse_str(&s)
                .map(TunnelSetting::Use)
                .map_err(|_| serde::de::Error::custom(format!("ungültiger Tunnelverweis: {s}")))
        }
    }
}

/// Transport für PowerShell-Remoting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PsrpTransport {
    Winrm {
        #[serde(default = "yes")]
        https: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        auth: WinrmAuth,
        /// Zertifikatsprüfung; abschalten nur mit Warnung.
        #[serde(default = "yes")]
        verify_certificate: bool,
    },
    Ssh {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
    /// Automatisch: unter Windows WinRM über HTTPS, sonst SSH (PowerShell 7 hat unter Linux
    /// keinen WinRM-Client).
    Auto,
}

impl PsrpTransport {
    /// Transport, den dieses Betriebssystem ohne Vorgabe nimmt (`winrm_port` nur für WinRM).
    pub fn for_this_os(winrm_port: Option<u16>) -> Self {
        if cfg!(windows) {
            PsrpTransport::Winrm { https: true, port: winrm_port, auth: WinrmAuth::Ntlm, verify_certificate: true }
        } else {
            PsrpTransport::Ssh { port: None }
        }
    }

    /// `Auto` durch den Transport dieses Betriebssystems ersetzen.
    pub fn resolved(self, winrm_port: Option<u16>) -> Self {
        match self {
            PsrpTransport::Auto => Self::for_this_os(winrm_port),
            t => t,
        }
    }

    /// Fehlende Vorgabe oder `Auto` → Transport dieses Betriebssystems.
    pub fn effective(t: Option<Self>, winrm_port: Option<u16>) -> Self {
        t.map(|t| t.resolved(winrm_port)).unwrap_or_else(|| Self::for_this_os(winrm_port))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WinrmAuth {
    Kerberos,
    Ntlm,
    Basic,
}

/// SNMP-Zuordnung: Gerät aus der Geräteliste des Index oder abgeschaltet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type", content = "device")]
pub enum SnmpSetting {
    None,
    Device(Uuid),
}

/// Vererbbare Einstellungen an Ordner oder Ziel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Benannte Zugangsdatensätze (Name → KDBX-Eintrag).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub credentials: BTreeMap<String, CredentialRef>,
    /// Überschriebene Felder je Zugangsdatensatz-Name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, FieldOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel: Option<TunnelSetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psrp_transport: Option<PsrpTransport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snmp: Option<SnmpSetting>,
    /// Zugangsdaten an dieser Stelle ausdrücklich abgeschaltet (auch für alle
    /// Unterobjekte, die nichts Eigenes festlegen).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub credentials_off: bool,
    /// Name des Zugangsdatensatzes für Unterobjekte ohne eigenen Namen
    /// (Suche nach oben, z. B. `admin`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_name: Option<String>,
    /// Vorgabe der Zwischenablage-Richtung für neue Ziele in diesem Ordner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<ClipboardDirection>,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self.credentials.is_empty()
            && self.overrides.is_empty()
            && self.tunnel.is_none()
            && self.psrp_transport.is_none()
            && self.snmp.is_none()
            && !self.credentials_off
            && self.credential_name.is_none()
            && self.clipboard.is_none()
    }
}

/// Tunneldefinition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TunnelDef {
    pub id: Uuid,
    pub name: String,
    pub kind: TunnelKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TunnelKind {
    /// SSH-Tunnel über eine oder mehrere Stufen (Sprungserver der Reihe nach).
    Ssh { hops: Vec<SshHop> },
    /// WireGuard im Benutzerbereich; Konfiguration als Anhang eines KDBX-Eintrags.
    Wireguard { config_entry: CredentialRef },
    /// Vorhandener Systemtunnel: es wird nur die Erreichbarkeit geprüft.
    System { probe_host: String, probe_port: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshHop {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Zugangsdatensatz-Name für diese Stufe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// Ältere, unsichere SSH-Verfahren für diese Stufe zusätzlich zulassen (alte Geräte als Sprungserver).
    /// Gilt auch, wenn das Ziel hinter dem Tunnel „Ältere Verfahren“ eingeschaltet hat.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legacy_algorithms: bool,
}

/// SNMP-Gerät.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnmpDevice {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub version: SnmpVersion,
    /// KDBX-Eintrag mit Community bzw. v3-Benutzer und Schlüsseln: v2c
    /// Community im Kennwortfeld; v3 Benutzername, Kennwort zur
    /// Authentisierung im Kennwortfeld, Kennwort zur Verschlüsselung im
    /// geschützten Feld `SNMP-Verschlüsselung` (fehlt es, gilt das Kennwort
    /// zur Authentisierung).
    pub credential: CredentialRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<u32>,
    /// v3: Verfahren der Authentisierung (Vorgabe SHA-1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_proto: Option<SnmpAuth>,
    /// v3: Verfahren der Verschlüsselung (Vorgabe AES-128).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priv_proto: Option<SnmpPriv>,
}

/// SNMPv3-Authentisierung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnmpAuth {
    Md5,
    Sha1,
    Sha256,
}

/// SNMPv3-Verschlüsselung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnmpPriv {
    None,
    Des,
    Aes128,
}

/// Geschütztes KDBX-Feld mit dem SNMPv3-Kennwort zur Verschlüsselung.
pub const SNMP_PRIV_FIELD: &str = "SNMP-Verschlüsselung";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnmpVersion {
    V2c,
    V3,
}

/// Aufgabe (z. B. Aktualisierungen).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskDef {
    pub id: Uuid,
    pub name: String,
    /// Kennung der eingebauten Aufgabe oder `script`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

/// Makro: Folge von Schritten.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroDef {
    pub id: Uuid,
    pub name: String,
    pub steps: Vec<serde_json::Value>,
}

/// Beschreibende Angaben an Ordner oder Ziel (keine Geheimnisse).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// Eigene Felder (Name → Wert).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
    /// Zugeordnete Aufgaben (IDs aus `Index::tasks`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<Uuid>,
    /// Zugeordnete Makros (IDs aus `Index::macros`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<Uuid>,
}

impl Meta {
    pub fn is_empty(&self) -> bool {
        self.description.is_empty() && self.notes.is_empty() && self.fields.is_empty() && self.tasks.is_empty() && self.macros.is_empty()
    }
}

/// Art der Zugangsdaten in einer Vorlage. Vorlagen verweisen nie auf
/// KDBX-Einträge und enthalten keine Kennwörter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "name", rename_all = "lowercase")]
pub enum TemplateCredential {
    #[default]
    Inherit,
    None,
    Named(String),
}

/// Vorlage für neue Ziele eines Protokolls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub id: Uuid,
    pub name: String,
    pub protocol: Protocol,
    /// Standardeinstellung des Protokolls (eine je Protokoll).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "ProtocolOptions::is_none")]
    pub options: ProtocolOptions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel: Option<TunnelSetting>,
    #[serde(default)]
    pub credential: TemplateCredential,
}

/// Ansicht: benannte Auswahl von Zielen; ein Ziel darf in mehreren Ansichten stehen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub id: Uuid,
    pub name: String,
    pub targets: Vec<Uuid>,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Index {
    /// Sucht einen Knoten und liefert den Pfad von der Wurzel (ohne Wurzel)
    /// bis einschließlich des Knotens.
    pub fn path_to(&self, id: Uuid) -> Option<Vec<&Node>> {
        fn walk<'a>(f: &'a Folder, id: Uuid, path: &mut Vec<&'a Node>) -> bool {
            for n in &f.children {
                path.push(n);
                if n.id() == id {
                    return true;
                }
                if let Node::Folder(sub) = n {
                    if walk(sub, id, path) {
                        return true;
                    }
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        walk(&self.root, id, &mut path).then_some(path)
    }

    /// Ruft `f` für jedes Ziel in Baumreihenfolge auf.
    /// Repariert Web-Ziele aus älteren Importen, bei denen die ganze Adresse im Host-Feld stand
    /// (`http://…`): Adresse in die Optionen, Host und Port daraus. Liefert die Zahl der Ziele.
    pub fn repair_web_targets(&mut self) -> usize {
        fn walk(folder: &mut Folder, n: &mut usize) {
            for c in &mut folder.children {
                match c {
                    Node::Folder(sub) => walk(sub, n),
                    Node::Target(t) if t.protocol == Protocol::Web && t.host.contains("://") => {
                        if let Some((h, p)) = crate::weburl::endpoint(&t.host) {
                            let url = t.host.trim().to_string();
                            t.host = h;
                            t.port = (p != Protocol::Web.default_port()).then_some(p);
                            match &mut t.options {
                                ProtocolOptions::Web(o) if o.url.as_deref().is_some_and(|u| !u.is_empty()) => {}
                                ProtocolOptions::Web(o) => o.url = Some(url),
                                other => *other = ProtocolOptions::Web(WebOptions { url: Some(url), ..Default::default() }),
                            }
                            *n += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut n = 0;
        walk(&mut self.root, &mut n);
        n
    }

    pub fn for_each_target<'a>(&'a self, mut f: impl FnMut(&'a Target)) {
        fn walk<'a>(folder: &'a Folder, f: &mut impl FnMut(&'a Target)) {
            for n in &folder.children {
                match n {
                    Node::Folder(sub) => walk(sub, f),
                    Node::Target(t) => f(t),
                }
            }
        }
        walk(&self.root, &mut f);
    }

    /// Ruft `f` für jede Einstellungsgruppe (Wurzel, Ordner, Ziele) auf.
    pub fn for_each_settings<'a>(&'a self, mut f: impl FnMut(&'a Settings)) {
        fn walk<'a>(folder: &'a Folder, f: &mut impl FnMut(&'a Settings)) {
            f(&folder.settings);
            for n in &folder.children {
                match n {
                    Node::Folder(sub) => walk(sub, f),
                    Node::Target(t) => f(&t.settings),
                }
            }
        }
        walk(&self.root, &mut f);
    }

    /// Alle KDBX-UUIDs, auf die der Index verweist.
    pub fn referenced_entries(&self) -> std::collections::BTreeSet<Uuid> {
        let mut out = std::collections::BTreeSet::new();
        self.for_each_settings(|s| out.extend(s.credentials.values().map(|c| c.entry)));
        for t in &self.tunnels {
            if let TunnelKind::Wireguard { config_entry } = &t.kind {
                out.insert(config_entry.entry);
            }
        }
        for d in &self.snmp_devices {
            out.insert(d.credential.entry);
        }
        out
    }
}

#[cfg(test)]
mod gfx_tests {
    use super::*;

    #[test]
    fn gfx_option_defaults_to_auto_and_is_not_written_then() {
        let o: RdpOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(o.gfx, GfxMode::Auto);
        assert!(!serde_json::to_string(&o).unwrap().contains("gfx"));
        let o: RdpOptions = serde_json::from_str(r#"{"gfx":"on"}"#).unwrap();
        assert_eq!(o.gfx, GfxMode::On);
        assert!(serde_json::to_string(&o).unwrap().contains(r#""gfx":"on""#));
        assert!(serde_json::from_str::<RdpOptions>(r#"{"gfx":"x"}"#).is_err());
    }
}

#[cfg(test)]
mod web_repair_tests {
    use super::*;

    #[test]
    fn old_import_with_the_url_in_the_host_field_is_repaired() {
        let mut idx = Index::default();
        let mut f = Folder::new("alt");
        f.children.push(Node::Target(Target::new("a", Protocol::Web, "http://host-a.example.test:3000/x")));
        f.children.push(Node::Target(Target::new("b", Protocol::Web, "host-b.example.test")));
        f.children.push(Node::Target(Target::new("c", Protocol::Ssh, "ssh://nicht-anfassen")));
        idx.root.children.push(Node::Folder(f));
        assert_eq!(idx.repair_web_targets(), 1);
        let mut got = Vec::new();
        idx.for_each_target(|t| got.push((t.name.clone(), t.host.clone(), t.port, matches!(&t.options, ProtocolOptions::Web(o) if o.url.as_deref() == Some("http://host-a.example.test:3000/x")))));
        assert_eq!(got[0], ("a".into(), "host-a.example.test".into(), Some(3000), true));
        assert_eq!(got[1].1, "host-b.example.test");
        assert_eq!(got[2].1, "ssh://nicht-anfassen", "andere Protokolle bleiben unberührt");
        assert_eq!(idx.repair_web_targets(), 0, "zweiter Lauf ändert nichts");
    }
}
