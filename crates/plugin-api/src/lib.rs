//! Schnittstelle zwischen dem Kern von remotejoe und seinen Erweiterungen (Plugins).
//!
//! Der Kern kennt keine Hersteller und keine Fachfunktionen der Erweiterungen. Er stellt einer Erweiterung einen [`Host`] zur Verfügung
//! (Index, Einstellungen, Inventar, Texte, Datenordner, Bearbeitungsbefehle) und einen nebenläufig nutzbaren
//! [`HostShared`] (Ereignisse, Aufträge, Befehlszeilen-Sitzungen zu Geräten mit den Zugangsdaten des Kerns, SNMP).
//! Die Erweiterung sieht nie Kennwörter: Sie löst im Kern einen Zugangsschein (`cli_ticket`) für ein Ziel, und der
//! Kern baut damit die Verbindung auf.
//!
//! Alle Operationen sind JSON rein, JSON raus, damit dieselbe Schnittstelle später über eine C-Schnittstelle zu
//! Bibliotheken in `plugins/` des Datenordners führt.

pub mod cli;
pub mod ffi;
pub mod netutil;

use std::any::Any;
use std::sync::Arc;

use rj_core::{Folder, Index, Target};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

pub use rj_core;

/// Gefundene Inventarwerte: (Feld, Wert).
pub type Found = Vec<(&'static str, String)>;

/// Sekunden seit 1970 (Zeitstempel von Inventar und Ständen).
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Virtueller Eintrag unter einem Ordner (kein Ziel; eine Erweiterung öffnet ihn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualChild {
    pub id: Uuid,
    pub name: String,
}

/// Geräteart eines Ziels, wie eine Erweiterung sie aus dem Inventar erkennt (neutral für Baum, Symbole, Übersicht).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceInfo {
    /// Klasse: `switch`, `controller`, `ap`, `gateway`.
    pub class: String,
    /// Anzeigename, z. B. „WiFi-Controller (Serie X)“.
    pub label: String,
    /// Schlüssel der Erweiterung (undurchsichtig für den Kern), z. B. „hersteller-adapter/serie-8“.
    pub key: String,
    /// Port der Web-Oberfläche, falls abweichend.
    pub web_port: Option<u16>,
}

impl DeviceInfo {
    pub fn is_switch(&self) -> bool {
        self.class == "switch"
    }

    pub fn is_controller(&self) -> bool {
        self.class == "controller"
    }

    /// Klasse als fester Schlüssel für Baum und Symbole.
    pub fn class_key(&self) -> &'static str {
        match self.class.as_str() {
            "switch" => "switch",
            "controller" => "controller",
            "ap" => "ap",
            "gateway" => "gateway",
            _ => "device",
        }
    }
}

/// Ausgaben einer Befehlszeilen-Sitzung: je gesendetem Befehl eine Ausgabe, dazu das bereinigte Protokoll.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CliOutput {
    pub outputs: Vec<String>,
    /// Sekunden ab Sitzungsbeginn, zu denen die Ausgabe je Befehl vollständig war.
    pub times: Vec<f32>,
    /// Die Zeitgrenze wurde erreicht, bevor alle Befehle beantwortet waren.
    pub timed_out: bool,
    pub log: String,
    /// Hinweis der Verbindung (z. B. übernommener Host-Schlüssel, ältere Verfahren).
    pub note: Option<String>,
    /// Tatsächlich benutzter Weg: `ssh` oder `telnet`.
    pub via: String,
}

/// Schritt mit möglicher Rückfrage des Geräts (Teiltext der Rückfrage und Antwort).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DialogStep {
    pub command: String,
    pub prompt: Option<String>,
    pub answer: Option<String>,
}

impl DialogStep {
    pub fn cmd(c: &str) -> Self {
        DialogStep { command: c.to_string(), prompt: None, answer: None }
    }
    pub fn ask(c: &str, prompt: &str, answer: &str) -> Self {
        DialogStep { command: c.to_string(), prompt: Some(prompt.to_string()), answer: Some(answer.to_string()) }
    }
}

/// Zugangsschein für Befehlszeilen-Sitzungen: der Kern hat die Zugangsdaten eines Ziels aufgelöst und verwahrt sie
/// unter dieser Nummer bis zum Sperren; die Erweiterung reicht nur die Nummer weiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CliTicket(pub u64);

/// Wie die Sitzung aufgebaut wird.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CliOptions {
    /// Bei abgelehntem Schlüsseltausch mit älteren SSH-Verfahren erneut versuchen.
    pub legacy_retry: bool,
    /// Bei abgelehnter SSH-Verbindung Telnet (Port 23) versuchen.
    pub telnet: bool,
    /// Wechsel des Verfahrens im Protokoll vermerken.
    pub marks: bool,
    /// Breite des Terminals (Spalten).
    pub cols: u32,
    /// Gesamtfrist in Sekunden.
    pub limit_secs: u64,
    /// Schnittstelle mit Begrüßung und Seitenumbruch (Switch-CLI) statt einfacher Eingabeaufforderung.
    pub switch_cli: bool,
    /// Offene Verbindung des Kerns vorher neu aufbauen (Geräte, die nach einem Befehl den Kanal schließen).
    pub fresh: bool,
    /// Andere Adresse als die im Schein (gleiche Zugangsdaten), z. B. APs eines Controllers oder eine Suche im Netz.
    pub host: Option<String>,
    /// Nur Telnet (Port 23), kein SSH.
    pub telnet_only: bool,
}

impl Default for CliOptions {
    fn default() -> Self {
        CliOptions { legacy_retry: true, telnet: false, marks: false, cols: 200, limit_secs: 30, switch_cli: true, fresh: false, host: None, telnet_only: false }
    }
}

/// Zugang des Kerns, nur im Faden der Oberfläche (die Operationen sehen den Zustand des Programms).
pub trait Host {
    /// Operation des Kerns. Jede Bearbeitungsoperation des Kerns (wie sie die Oberfläche schickt) und die
    /// Host-Operationen (siehe `HostExt`).
    fn call(&mut self, op: &str, args: Value) -> Result<Value, String>;
    /// Nebenläufig nutzbarer Zugang (Fäden der Erweiterung).
    fn shared(&self) -> Arc<dyn HostShared>;
    /// Der Index (lesend, ohne Kopie). Entfällt mit dem Laden aus Bibliotheken zugunsten von `index` mit Stand.
    fn index_ref(&self) -> &Index;
    /// Übergang: Zugriff auf das Programm selbst, solange eine Erweiterung noch nicht vollständig über den Host
    /// spricht. Entfällt mit dem Laden aus Bibliotheken.
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn as_any(&self) -> &dyn Any;
}

/// Zugang des Kerns aus beliebigen Fäden: Ereignisse an die Oberfläche, Aufträge in der Auftragsliste,
/// Befehlszeilen-Sitzungen mit den Zugangsdaten des Kerns, SNMP, externe Programme mit Zugangsdaten.
pub trait HostShared: Send + Sync {
    fn call(&self, op: &str, args: Value) -> Result<Value, String>;
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Typisierte Hilfen über den JSON-Operationen des Hosts.
pub trait HostExt {
    fn call_json(&mut self, op: &str, args: Value) -> Result<Value, String>;

    /// Der ganze Index (Kopie).
    fn index(&mut self) -> Index {
        self.index_owned()
    }
    fn index_owned(&self) -> Index;
    /// Laufende Abfrage eines Ziels merken bzw. beenden (keine doppelten Starts; die Übersicht zeigt „läuft“).
    fn busy_set(&mut self, key: &str, job: u64) {
        let _ = self.call_json("busy_set", json!({ "key": key, "job": job }));
    }
    fn busy_clear(&mut self, key: &str) {
        let _ = self.call_json("busy_clear", json!({ "key": key }));
    }
    /// Läuft für das Ziel schon eine Abfrage? Dann ihre Auftragsnummer.
    fn busy(&mut self, key: &str) -> Option<u64> {
        self.call_json("busy", json!({ "key": key })).ok().and_then(|v| v.as_u64())
    }
    /// Geänderten Index übernehmen (Rückgängig-Eintrag `label`, Ablage im Tresor, Baum neu laden).
    fn index_apply(&mut self, index: &Index, label: &str) -> Result<(), String> {
        self.call_json("index_apply", json!({ "index": index, "label": label })).map(|_| ())
    }
    /// Geänderten Index einsetzen, ohne Rückgängig-Eintrag: innerhalb einer Operation des Kerns, die danach selbst
    /// übernimmt (z. B. Eigenschaften eines Ordners speichern).
    fn index_set(&mut self, index: &Index) -> Result<(), String> {
        self.call_json("index_set", json!({ "index": index })).map(|_| ())
    }
    /// Bearbeitungsbefehl des Kerns (wie aus der Oberfläche).
    fn edit(&mut self, cmd: Value) -> Result<Value, String> {
        self.call_json("edit", cmd)
    }
    fn target(&mut self, id: Uuid) -> Option<Target> {
        self.call_json("target", json!({ "id": id })).ok().and_then(|v| serde_json::from_value(v).ok())
    }
    fn folder(&mut self, id: Uuid) -> Option<Folder> {
        self.call_json("folder", json!({ "id": id })).ok().and_then(|v| serde_json::from_value(v).ok())
    }
    /// Einstellungsabschnitt der Erweiterung (Zusatzschlüssel in `[features]`).
    fn settings_section<T: DeserializeOwned + Default>(&mut self) -> T {
        self.call_json("settings_extra", Value::Null).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    fn set_settings_section<T: Serialize>(&mut self, v: &T) {
        let _ = self.call_json("set_settings_extra", serde_json::to_value(v).unwrap_or(Value::Null));
    }
    /// Einstellungen des Kerns als JSON (`mcp_hide_addresses`, …).
    fn core_settings(&mut self) -> Value {
        self.call_json("settings_core", Value::Null).unwrap_or(Value::Null)
    }
    fn data_dir(&mut self) -> std::path::PathBuf {
        std::path::PathBuf::from(self.call_json("data_dir", Value::Null).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default())
    }
    fn text(&mut self, key: &str) -> String {
        self.call_json("text", json!({ "key": key })).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| key.to_string())
    }
    fn demo(&mut self) -> bool {
        self.call_json("demo", Value::Null).ok().and_then(|v| v.as_bool()).unwrap_or(false)
    }
    /// Schlüsseldatenbank geöffnet (nicht gesperrt)?
    fn vault_open(&mut self) -> bool {
        self.call_json("vault_open", Value::Null).ok().and_then(|v| v.as_bool()).unwrap_or(false)
    }
    fn read_only(&mut self) -> bool {
        self.call_json("read_only", Value::Null).ok().and_then(|v| v.as_bool()).unwrap_or(true)
    }
    /// Geräteart je Ziel (aus dem Inventar).
    fn device(&mut self, id: Uuid) -> Option<DeviceInfo> {
        self.call_json("device", json!({ "id": id })).ok().and_then(|v| serde_json::from_value(v).ok())
    }
    fn set_device(&mut self, id: Uuid, d: Option<&DeviceInfo>) {
        let _ = self.call_json("set_device", json!({ "id": id, "device": d }));
    }
    /// Gespeichertes Inventar eines Ziels bzw. Gasts (`key`): `{ fields: [...], web_way }` (Felder wie die Übersicht).
    fn inventory(&mut self, key: &str) -> Option<Value> {
        self.call_json("inventory", json!({ "key": key })).ok().filter(|v| !v.is_null())
    }
    /// Wert anbieten (wie eine Quelle des Kerns): `source` nennt die Herkunft.
    fn inventory_offer(&mut self, key: &str, field: &str, value: &str, source: &str) -> Result<(), String> {
        self.call_json("inventory_offer", json!({ "key": key, "field": field, "value": value, "source": source })).map(|_| ())
    }
    /// Zuletzt geprüfte Adresse eines Ziels (Dienstprüfung), falls sie von der eingetragenen abweicht.
    fn services_ip(&mut self, key: &str) -> Option<String> {
        self.call_json("services_ip", json!({ "key": key })).ok().and_then(|v| v.as_str().map(str::to_string))
    }
    /// Zugangsschein für Befehlszeilen-Sitzungen zu einem Ziel (SSH mit den Zugangsdaten des Ziels bzw. Wegs).
    fn cli_ticket(&mut self, target: Uuid) -> Result<CliTicket, String> {
        self.call_json("cli_ticket", json!({ "target": target })).and_then(|v| v.get("ticket").and_then(Value::as_u64).map(CliTicket).ok_or_else(|| "Kein Ticket".to_string()))
    }
    /// Zugangsschein mit den Zugangsdaten eines Ziels, aber für eine andere Adresse (z. B. AP des Controllers).
    fn cli_ticket_at(&mut self, target: Uuid, host: &str, port: u16) -> Result<CliTicket, String> {
        self.call_json("cli_ticket", json!({ "target": target, "host": host, "port": port })).and_then(|v| v.get("ticket").and_then(Value::as_u64).map(CliTicket).ok_or_else(|| "Kein Ticket".to_string()))
    }
    /// Ampel eines Ziels (wie im Baum), falls bekannt.
    fn light(&mut self, id: Uuid) -> Value {
        self.call_json("light", json!({ "id": id })).unwrap_or(Value::Null)
    }
    /// Zugangsschein für eine Adresse mit einem Anmeldeobjekt (`login`) oder dem Anmeldeobjekt eines Ordners für einen
    /// Zugangsdaten-Slot (`folder` + `slot`).
    fn cli_ticket_for(&mut self, host: &str, port: u16, how: Value) -> Result<CliTicket, String> {
        let mut a = how;
        a["host"] = json!(host);
        a["port"] = json!(port);
        self.call_json("cli_ticket", a).and_then(|v| v.get("ticket").and_then(Value::as_u64).map(CliTicket).ok_or_else(|| "Kein Ticket".to_string()))
    }
    /// Hat der Ordner (oder einer darüber) den benannten Zugangsdatensatz `slot`?
    fn folder_has_slot(&mut self, folder: Uuid, slot: &str) -> bool {
        self.call_json("folder_has_slot", json!({ "folder": folder, "slot": slot })).ok().and_then(|v| v.as_bool()).unwrap_or(false)
    }
    /// HTTPS-Ticket: Der Kern ruft HTTPS mit Zugangsdaten auf, die die Erweiterung nie sieht. `how`: `target`
    /// (Kennung) mit optionalem `secret` (benanntes Geheimnis des Ziels) oder `way` (Name oder Kennung eines
    /// Zugangswegs) und `port`; die Adresse kommt immer aus dem
    /// Ziel bzw. Weg. `auth`: `{"kind":"header","name":"X-API-KEY"}`, `{"kind":"bearer"}`, `{"kind":"basic"}` oder
    /// `{"kind":"none"}`. Anfragen mit [`HostShared::http_request`].
    fn http_ticket(&mut self, how: Value) -> Result<CliTicket, String> {
        self.call_json("http_ticket", how).and_then(|v| v.get("ticket").and_then(Value::as_u64).map(CliTicket).ok_or_else(|| "Kein HTTPS-Ticket".to_string()))
    }
    /// Ist am Ziel das benannte Geheimnis (z. B. „API-Schlüssel“) hinterlegt? Der Wert selbst ist nie lesbar; gesetzt
    /// wird er über die Eigenschaften (Feld `secrets` beim Speichern) und genutzt über ein HTTPS-Ticket
    /// (`{"target", "secret"}`).
    fn secret_state(&mut self, target: Uuid, name: &str) -> bool {
        self.call_json("secret_state", json!({ "target": target, "name": name })).ok().and_then(|v| v["set"].as_bool()).unwrap_or(false)
    }
    /// Eigene Zugangsdaten des Zugangswegs `way` als Geheimnis `name` des Ziels übernehmen und den Weg entfernen
    /// (Umstellung älterer Einrichtungen; der Wert verlässt den Kern nicht). `true`, wenn umgestellt.
    fn secret_from_way(&mut self, target: Uuid, way: &str, name: &str) -> Result<bool, String> {
        self.call_json("secret_from_way", json!({ "target": target, "way": way, "name": name })).map(|v| v["moved"].as_bool().unwrap_or(false))
    }
    /// Zertifikat (`host:port`, SHA-256-Fingerabdruck) für HTTPS-Tickets annehmen – nur nach Bestätigung durch den
    /// Anwender (Rückfrage der Erweiterung bei einer Antwort `{"certificate":{…}}`) und nur den Abdruck, den der Server
    /// zuletzt vorgelegt hat.
    fn https_accept_certificate(&mut self, host_port: &str, fingerprint: &str) -> Result<(), String> {
        self.call_json("https_accept_certificate", json!({ "host": host_port, "fingerprint": fingerprint })).map(|_| ())
    }
    /// Zugangsdaten eines Ziels bzw. Ordners für SNMP als Schein.
    fn snmp_ticket(&mut self, folder: Uuid) -> Result<CliTicket, String> {
        self.call_json("snmp_ticket", json!({ "folder": folder })).and_then(|v| v.get("ticket").and_then(Value::as_u64).map(CliTicket).ok_or_else(|| "Kein SNMP-Zugang".to_string()))
    }
    /// Konfigablage eines Ordners: Stand (`path`, `exists`, `needs_entry`).
    fn cfgstore_state(&mut self, folder: Uuid, name: &str) -> Result<Value, String> {
        self.call_json("cfgstore_state", json!({ "folder": folder, "name": name }))
    }
    /// Eintrag „Konfigsicherung“ in der Haupt-KDBX anlegen (nach Rückfrage in der Oberfläche).
    fn cfgstore_ensure_entry(&mut self) -> Result<(), String> {
        self.call_json("cfgstore_ensure_entry", Value::Null).map(|_| ())
    }
    /// Konfigablage öffnen (`create`: auch anlegen); Griff für die nebenläufigen Operationen, `None` = keine Datei.
    fn cfgstore_open(&mut self, folder: Uuid, name: &str, create: bool) -> Result<Option<u64>, String> {
        self.call_json("cfgstore_open", json!({ "folder": folder, "name": name, "create": create })).map(|v| v["handle"].as_u64())
    }
    /// Anmeldeobjekt anlegen (Zugangsdaten aus der Vorlage `template`, sonst leer); liefert `id`, `entry`,
    /// `missing_template`.
    fn login_create(&mut self, folder: Uuid, name: &str, title: &str, template: Option<&str>) -> Result<Value, String> {
        self.call_json("login_create", json!({ "folder": folder, "name": name, "title": title, "template": template }))
    }
    /// Operation einer anderen Erweiterung (`to` = Kennung) über den Kern als Vermittler; Fehler, wenn sie nicht
    /// geladen und freigeschaltet ist oder die Operation nicht kennt.
    fn plugin_call(&mut self, to: &str, op: &str, args: Value) -> Result<Value, String> {
        self.call_json("plugin_call", json!({ "to": to, "op": op, "args": args }))
    }
    /// Kennungen der freigeschalteten Erweiterungen, die die Anschlussstelle `anschluss` anbieten (Manifest
    /// `stellt_bereit`), in Ladereihenfolge.
    fn providers(&mut self, anschluss: &str) -> Vec<String> {
        self.call_json("providers", json!({ "anschluss": anschluss })).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    /// Arten der Standard-Anmeldeobjekte (Kennung, Vorlagenname, Netzwerkordner?) – Kern plus Erweiterungen.
    fn login_kinds(&mut self) -> Vec<(String, String, bool)> {
        self.call_json("login_kinds", Value::Null).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    /// Name des Programms mit Fassung (Über, Berichte).
    fn edition(&mut self) -> String {
        self.call_json("edition", Value::Null).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "remotejoe".into())
    }
    /// Fassung des laufenden Programms (z. B. „0.1.69“), nicht die der Erweiterung. Ein älterer Kern ohne diese Abfrage
    /// liefert die Fassung des Kerns, gegen den die Erweiterung gebaut wurde.
    fn program_version(&mut self) -> String {
        self.call_json("program_version", Value::Null).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| rj_core::schema::PROGRAM_VERSION.into())
    }
    /// Baum neu laden lassen (nach Änderungen am Index durch die Erweiterung).
    fn tree_changed(&mut self) {
        let _ = self.call_json("tree_changed", Value::Null);
    }
    /// Kurzform: Name und Pfad eines Knotens.
    fn node_path(&mut self, id: Uuid) -> String {
        self.call_json("node_path", json!({ "id": id })).ok().map(|v| s(&v, "path")).unwrap_or_default()
    }
}

impl HostExt for dyn Host + '_ {
    fn call_json(&mut self, op: &str, args: Value) -> Result<Value, String> {
        self.call(op, args)
    }
    fn index_owned(&self) -> Index {
        self.index_ref().clone()
    }
}

/// Gespeichertes Inventar eines Ziels (Sicht der Erweiterung): Felder mit Wert und Quelle.
#[derive(Debug, Clone, Default)]
pub struct InvView {
    pub fields: Vec<Value>,
    /// Geräteart erkannt und ein Zugangsweg von einer Erweiterung geprüft bzw. angelegt (einmalig).
    pub web_way: bool,
}

impl InvView {
    pub fn load(host: &mut dyn Host, key: &str) -> InvView {
        let v = host.inventory(key).unwrap_or(Value::Null);
        InvView::from_json(&v)
    }
    pub fn from_json(v: &Value) -> InvView {
        InvView { fields: v["fields"].as_array().or(v.as_array()).cloned().unwrap_or_default(), web_way: v["web_way"].as_bool().unwrap_or(false) }
    }
    /// Gültiger Wert eines Felds (wie die Übersicht ihn zeigt).
    pub fn value(&self, key: &str) -> Option<String> {
        self.fields.iter().find(|f| f["key"] == key).and_then(|f| f["value"].as_str()).filter(|s| !s.is_empty()).map(str::to_string)
    }
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// Typisierte Hilfen über dem nebenläufigen Zugang.
pub trait HostSharedExt {
    fn call_json(&self, op: &str, args: Value) -> Result<Value, String>;

    /// Ereignis an die Oberfläche: Ergebnis eines Auftrags (`kind`, Ziel, JSON).
    fn job_result(&self, job: u64, kind: &str, target: &str, json: &Value) {
        let _ = self.call_json("event", json!({ "job": job, "kind": kind, "target": target, "json": json.to_string() }));
    }
    fn tree_changed(&self) {
        let _ = self.call_json("tree_changed", Value::Null);
    }
    /// Host-Operation im Faden der Oberfläche nachholen (Inventar anbieten, Geräteart setzen, Index übernehmen, …).
    fn defer(&self, op: &str, args: Value) {
        let _ = self.call_json("defer", json!({ "op": op, "args": args }));
    }
    /// Eigene Operation der Erweiterung im Faden der Oberfläche nachholen (mit Host).
    fn defer_plugin_op(&self, op: &str, args: Value) {
        self.defer("plugin_op", json!({ "op": op, "args": args }));
    }
    /// Auftrag in der Auftragsliste anmelden (die Erweiterung führt ihn in einem eigenen Faden aus).
    fn job_begin(&self, label: &str) -> u64 {
        self.call_json("job_begin", json!({ "label": label })).ok().and_then(|v| v.get("id").and_then(Value::as_u64)).unwrap_or(0)
    }
    fn job_end(&self, id: u64) {
        let _ = self.call_json("job_end", json!({ "id": id }));
    }
    fn job_cancelled(&self, id: u64) -> bool {
        self.call_json("job_cancelled", json!({ "id": id })).ok().and_then(|v| v.as_bool()).unwrap_or(false)
    }
    /// Befehlszeilen-Sitzung mit einem Zugangsschein: je Aufruf eine Verbindung, die Befehle, dann trennen.
    fn cli_run(&self, ticket: CliTicket, cmds: &[&str], opts: &CliOptions) -> Result<CliOutput, String> {
        let steps: Vec<DialogStep> = cmds.iter().map(|c| DialogStep::cmd(c)).collect();
        self.cli_run_steps(ticket, &steps, opts)
    }
    fn cli_run_steps(&self, ticket: CliTicket, steps: &[DialogStep], opts: &CliOptions) -> Result<CliOutput, String> {
        let v = self.call_json("cli_run", json!({ "ticket": ticket.0, "steps": steps, "options": opts }))?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
    /// HTTPS-Anfrage über ein HTTPS-Ticket. `req`: `method`, `path` (mit Abfrage), `body` (JSON) oder `body_text`,
    /// `headers`, `timeout_secs`. Antwort: `status`, `headers`, `json` bzw. `body`/`body_b64`. Bei unbekanntem oder
    /// geändertem Zertifikat ist der Fehler JSON `{"certificate":{"host","fingerprint","old"}}`.
    fn http_request(&self, ticket: CliTicket, req: Value) -> Result<Value, String> {
        let mut a = req;
        a["ticket"] = json!(ticket.0);
        self.call_json("http_request", a)
    }
    /// Adresse und Port hinter einem Schein (ohne Zugangsdaten), z. B. für Protokolle.
    fn cli_ticket_info(&self, ticket: CliTicket) -> Value {
        self.call_json("cli_ticket_info", json!({ "ticket": ticket.0 })).unwrap_or(Value::Null)
    }
    /// SNMP: Werte lesen (`oids`) über den Schein eines Ordners.
    fn snmp_get(&self, ticket: CliTicket, host: &str, oids: &[&str]) -> Result<Value, String> {
        self.call_json("snmp_get", json!({ "ticket": ticket.0, "host": host, "oids": oids }))
    }
    /// Externes Programm mit den Zugangsdaten eines Scheins starten: `{{user}}`/`{{password}}` in Argumenten,
    /// Umgebungsvariablen und der ersten Eingabezeile (`stdin`) werden vom Kern ersetzt. Liefert die Prozessnummer;
    /// Ausgabe und Fehlerausgabe landen in `log_path`.
    fn run_external(&self, ticket: CliTicket, program: &str, args: &[String], env: &Map<String, Value>, cwd: &str, log_path: &str, stdin: Option<&str>) -> Result<u32, String> {
        self.call_json("run_external", json!({ "ticket": ticket.0, "program": program, "args": args, "env": env, "cwd": cwd, "log": log_path, "stdin": stdin })).and_then(|v| v.get("pid").and_then(Value::as_u64).map(|p| p as u32).ok_or_else(|| "Kein Prozess".into()))
    }
    fn external_stop(&self, pid: u32) {
        let _ = self.call_json("external_stop", json!({ "pid": pid }));
    }
    /// `{ running, code, known }` eines gestarteten Programms.
    fn external_state(&self, pid: u32) -> Value {
        self.call_json("external_state", json!({ "pid": pid })).unwrap_or(Value::Null)
    }
    fn text(&self, key: &str) -> String {
        self.call_json("text", json!({ "key": key })).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| key.to_string())
    }
    /// Konfigablage: Stand ablegen (`changed`, `stands`), speichern (`size`), lesen, schließen.
    fn cfgstore_add(&self, handle: u64, device: Uuid, group: &str, title: &str, url: &str, series: &str, stamp: &str, reason: &str, text: &str, keep: usize) -> Result<(bool, usize), String> {
        self.call_json("cfgstore_add", json!({ "handle": handle, "device": device, "group": group, "title": title, "url": url, "series": series, "stamp": stamp, "reason": reason, "text": text, "keep": keep }))
            .map(|v| (v["changed"].as_bool().unwrap_or(false), v["stands"].as_u64().unwrap_or(0) as usize))
    }
    fn cfgstore_save(&self, handle: u64) -> Result<u64, String> {
        self.call_json("cfgstore_save", json!({ "handle": handle })).map(|v| v["size"].as_u64().unwrap_or(0))
    }
    fn cfgstore_close(&self, handle: u64) {
        let _ = self.call_json("cfgstore_close", json!({ "handle": handle }));
    }
    fn cfgstore_devices(&self, handle: u64) -> Vec<Uuid> {
        self.call_json("cfgstore_devices", json!({ "handle": handle })).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    /// Stände eines Geräts: `[{ name, size, reason }]`, neueste zuerst.
    fn cfgstore_stands(&self, handle: u64, device: Uuid) -> Vec<Value> {
        self.call_json("cfgstore_stands", json!({ "handle": handle, "device": device })).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
    }
    fn cfgstore_checked(&self, handle: u64, device: Uuid) -> Option<String> {
        self.call_json("cfgstore_checked", json!({ "handle": handle, "device": device })).ok().and_then(|v| v.as_str().map(str::to_string))
    }
    fn cfgstore_text(&self, handle: u64, device: Uuid, name: &str) -> Option<String> {
        self.call_json("cfgstore_text", json!({ "handle": handle, "device": device, "name": name })).ok().and_then(|v| v.as_str().map(str::to_string))
    }
}

impl HostSharedExt for dyn HostShared + '_ {
    fn call_json(&self, op: &str, args: Value) -> Result<Value, String> {
        self.call(op, args)
    }
}

impl HostSharedExt for Arc<dyn HostShared> {
    fn call_json(&self, op: &str, args: Value) -> Result<Value, String> {
        self.as_ref().call(op, args)
    }
}

/// Angaben einer Erweiterung beim Anmelden.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PluginInfo {
    /// Kennung der Bibliothek, z. B. „netz“.
    pub id: String,
    /// Module darin (Lizenzkennungen).
    pub modules: Vec<ModuleInfo>,
    /// Fassung der Schnittstelle, gegen die sie gebaut wurde.
    pub api: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModuleInfo {
    pub id: String,
    /// Anzeigename (Lizenzseite, Über).
    pub name: String,
}

/// Fassung dieser Schnittstelle (Kern und Erweiterung müssen übereinstimmen).
pub const API_VERSION: u32 = 1;

/// Datei mit dem Manifest im Ordner einer Erweiterung (im Paket, von den signierten Prüfsummen erfasst).
pub const MANIFEST_FILE: &str = "manifest.json";

/// Manifest einer Erweiterungs-Bibliothek: was sie ist, was sie braucht, was sie anbietet. Dieselbe Datei liegt
/// im Paket (`manifest.json`, für Lader und Quelle) und ist in die Bibliothek eingebaut (`rj_plugin_manifest`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    /// Kennung, zugleich Lizenzmodul und Ordnername (Kleinbuchstaben, Ziffern, Unterstrich).
    pub id: String,
    /// Anzeigename.
    pub name: String,
    /// Eigene Version der Erweiterung (SemVer), unabhängig von remotejoe.
    pub version: String,
    /// Fassung der Schnittstelle, gegen die sie gebaut ist (muss `API_VERSION` des Kerns gleichen).
    pub api: u32,
    /// Älteste Version von remotejoe, mit der sie läuft.
    pub min_remotejoe: String,
    /// Erweiterungen, die geladen und lizenziert sein müssen, z. B. eine Hersteller-Erweiterung ihre Oberfunktion:
    /// Kennung, wahlweise mit Mindestversion („standorte>=1.2.0“), siehe [`dependency`].
    #[serde(default)]
    pub braucht: Vec<String>,
    /// Anschlussstellen, die sie anderen Erweiterungen anbietet, mit Fassung („adapter@3“; ohne Angabe Fassung 1),
    /// siehe [`interface`].
    #[serde(default)]
    pub stellt_bereit: Vec<String>,
    /// Anschlussstellen anderer Erweiterungen, die sie nutzt, mit der Fassung, gegen die sie gebaut ist
    /// („adapter@3“; ohne Angabe Fassung 1). Der Kern lädt sie nur, wenn der Anbieter genau diese Fassung anbietet.
    #[serde(default)]
    pub nutzt: Vec<String>,
}

/// Angabe aus `braucht` zerlegen: Kennung und Mindestversion („standorte“ → ("standorte", None),
/// „standorte>=1.2.0“ → ("standorte", Some("1.2.0"))).
pub fn dependency(entry: &str) -> (&str, Option<&str>) {
    match entry.split_once(">=") {
        Some((id, min)) => (id.trim(), Some(min.trim())),
        None => (entry.trim(), None),
    }
}

/// Anschlussstelle zerlegen: Name und Fassung („adapter“ → ("adapter", 1), „adapter@3“ → ("adapter", 3)). Eine
/// unlesbare Fassung zählt als 0 (passt zu nichts; [`Manifest::parse`] lehnt sie ab).
pub fn interface(entry: &str) -> (&str, u32) {
    match entry.split_once('@') {
        Some((name, v)) => (name.trim(), v.trim().parse().unwrap_or(0)),
        None => (entry.trim(), 1),
    }
}

impl Manifest {
    /// Kennungen aus `braucht` (ohne Mindestversion).
    pub fn braucht_ids(&self) -> Vec<String> {
        self.braucht.iter().map(|b| dependency(b).0.to_string()).collect()
    }

    /// Manifest aus JSON lesen und auf Vollständigkeit prüfen (Kennung, Name, Versionen).
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let m: Manifest = serde_json::from_str(text).map_err(|e| format!("Manifest unlesbar: {e}"))?;
        if m.id.is_empty() || !m.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
            return Err(format!("Manifest: Kennung „{}“ ungültig (Kleinbuchstaben, Ziffern, Unterstrich)", m.id));
        }
        if m.name.is_empty() || m.version.is_empty() || m.min_remotejoe.is_empty() {
            return Err(format!("Manifest der Erweiterung „{}“: Name, Version oder min_remotejoe fehlt", m.id));
        }
        if m.braucht_ids().iter().any(|b| b == &m.id) {
            return Err(format!("Manifest der Erweiterung „{}“ braucht sich selbst", m.id));
        }
        if let Some(b) = m.braucht.iter().find(|b| dependency(b).0.is_empty() || dependency(b).1.is_some_and(str::is_empty)) {
            return Err(format!("Manifest der Erweiterung „{}“: Angabe „{b}“ in braucht unlesbar", m.id));
        }
        if let Some(a) = m.stellt_bereit.iter().chain(&m.nutzt).find(|a| interface(a).0.is_empty() || interface(a).1 == 0) {
            return Err(format!("Manifest der Erweiterung „{}“: Anschlussstelle „{a}“ unlesbar (Name oder Name@Fassung)", m.id));
        }
        Ok(m)
    }
}

/// Eine Erweiterung. Alles, was der Kern von ihr wissen kann, läuft über diese Methoden. Jede Methode mit `host`
/// läuft im Faden der Oberfläche.
pub trait Plugin: Sync + Send {
    fn info(&self) -> PluginInfo;
    /// Freischaltung durch den Kern (Lizenz, Vorführung): `modules` = freigeschaltete Modulkennungen.
    fn configure(&self, active: &[String], demo: bool);
    /// Freigeschaltete Module (nach `configure`).
    fn active_modules(&self) -> Vec<String>;
    /// Nach dem Laden und Freischalten aller Erweiterungen (Faden der Oberfläche, in Ladereihenfolge): hier holt
    /// sich eine Hersteller-Erweiterung den Griff ihrer Oberfunktion und meldet sich dort an. Ein Fehler erscheint
    /// auf der Lizenzseite; die Erweiterung bleibt geladen.
    fn attach(&self, _host: &mut dyn Host) -> Result<(), String> {
        Ok(())
    }
    /// Operation ausführen; `None`, wenn die Erweiterung sie nicht kennt.
    fn handle(&self, host: &mut dyn Host, op: &str, c: &Value) -> Option<Result<Value, String>>;
    /// Oberflächentexte (JSON Schlüssel → Text) je Modul.
    fn texts(&self) -> Vec<&'static str> {
        Vec::new()
    }
    fn settings_json(&self, _host: &mut dyn Host, _m: &mut Map<String, Value>) {}
    fn set_settings(&self, _host: &mut dyn Host, _c: &Value) {}
    /// Trägt der Ordner Angaben der Erweiterung (eigenes Symbol, eigene Karte)?
    fn folder_marked(&self, _f: &Folder) -> bool {
        false
    }
    fn folder_virtual_children(&self, _f: &Folder) -> Vec<VirtualChild> {
        Vec::new()
    }
    /// Angaben für die Eigenschaften eines Ordners ergänzen.
    fn folder_editor_json(&self, _host: &mut dyn Host, _f: &Folder, _v: &mut Value) {}
    /// Formular der Eigenschaften eines Ordners übernehmen.
    fn folder_update(&self, _host: &mut dyn Host, _id: Uuid, _form: &Value) -> Result<(), String> {
        Ok(())
    }
    /// Ziel vor dem Speichern ergänzen (z. B. Adresse eines Zugangswegs).
    fn target_saving(&self, _t: &mut Target) {}
    /// Anteile an der Übersicht eines Ziels (`key`).
    fn overview_extra(&self, _host: &mut dyn Host, _key: &str) -> Map<String, Value> {
        Map::new()
    }
    /// Arten von Standard-Anmeldeobjekten (Kennung, Vorlagenname, Netzwerkordner?).
    fn login_kinds(&self) -> Vec<(&'static str, &'static str, bool)> {
        Vec::new()
    }
    /// Geräteart aus dem System-Feld des Inventars.
    fn device_info(&self, _system: &str) -> Option<DeviceInfo> {
        None
    }
    /// Zusätzliche Werkzeuge der Claude-Anbindung (Name, Beschreibung, Eingabe) und Aktionen mit Freigabe
    /// (Name, Beschreibung „Claude möchte … „Ziel““).
    fn mcp_tools(&self) -> Vec<Value> {
        Vec::new()
    }
    fn mcp_actions(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }
    fn mcp_call(&self, _host: &mut dyn Host, _tool: &str, _args: &Value) -> Option<Result<Value, String>> {
        None
    }
    /// Eigene Arten beim Hinzufügen (`kind` von „add“).
    fn add_kinds(&self) -> Vec<&'static str> {
        Vec::new()
    }
    fn add_target(&self, _host: &mut dyn Host, _kind: &str, _parent: Uuid, _c: &Value) -> Option<Result<Value, String>> {
        None
    }
    /// Nach einer Inventur eines Ziels (z. B. Zugangsweg anlegen); `true` = etwas geändert.
    fn after_inventory(&self, _host: &mut dyn Host, _id: Uuid) -> bool {
        false
    }
    /// Inventar eines Ziels über eine offene SSH-Sitzung des Kerns (`ticket` läuft Befehle darauf), wenn Linux und
    /// Windows nicht geantwortet haben. `ip`: geprüfte Adresse. Liefert Felder, Quelle und Hinweis.
    fn ssh_inventory(&self, _shared: &Arc<dyn HostShared>, _ticket: CliTicket, _ip: Option<&str>) -> Option<Result<(Found, &'static str, Option<String>), String>> {
        None
    }
    /// Inventar eines Switches über seine Befehlszeile (Sammelinventur): Zugangsschein des Ziels.
    fn switch_inventory(&self, _shared: &Arc<dyn HostShared>, _ticket: CliTicket) -> Option<Result<Found, String>> {
        None
    }
    /// „Inventar abfragen“ eines Ziels selbst übernehmen (Hintergrund); `None` = Kernweg.
    fn inventory_query(&self, _host: &mut dyn Host, _target: &Target, _key: &str) -> Option<Result<Value, String>> {
        None
    }
    /// Bezeichnung eines Ports in der Dienstprüfung (z. B. 4343).
    fn port_label(&self, _port: u16) -> Option<&'static str> {
        None
    }
    /// Beim Sperren: Geheimnisse und laufende Stände verwerfen.
    fn wipe(&self) {}
}
