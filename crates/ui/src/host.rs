//! Host für Erweiterungen: `App` beantwortet die Host-Operationen (Faden der Oberfläche); `AppShared` die
//! nebenläufigen (Ereignisse, Aufträge, Befehlszeilen-Sitzungen mit Zugangsscheinen, SNMP, externe Programme).

use std::sync::{Arc, Mutex};

use rj_core::{Node, Protocol};
use rj_plugin_api::{CliOptions, CliTicket, DeviceInfo, DialogStep, Host, HostShared};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app::App;
use crate::cli_service::CliService;
use crate::jobs::ExternalJobs;
use crate::sessions::UiEvent;

fn empty_payload() -> rj_ipc::msg::CredentialPayload {
    rj_ipc::msg::CredentialPayload { username: String::new(), domain: None, password: rj_ipc::msg::SecretBytes(Vec::new()), private_key: None }
}

/// Adresse und Port aus einer Adresse oder URL (`https://host:8443/pfad`, `[::1]:443`, `host`).
fn split_host(raw: &str, port: Option<u16>) -> (String, Option<u16>) {
    let s = raw.trim();
    let s = s.split_once("://").map(|(_, r)| r).unwrap_or(s);
    let s = s.split(['/', '?', '#']).next().unwrap_or_default();
    let s = s.rsplit_once('@').map(|(_, r)| r).unwrap_or(s);
    if let Some(rest) = s.strip_prefix('[') {
        if let Some((h, tail)) = rest.split_once(']') {
            let p = tail.strip_prefix(':').and_then(|p| p.parse().ok());
            return (h.to_string(), port.or(p));
        }
    }
    match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h.to_string(), port.or(p.parse().ok())),
        _ => (s.to_string(), port),
    }
}

fn id_of(a: &Value, k: &str) -> Result<Uuid, String> {
    a.get(k).and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()).ok_or_else(|| format!("{k}: keine Kennung"))
}

/// Nebenläufiger Zugang (für Fäden der Erweiterungen).
pub struct AppShared {
    pub events: Arc<Mutex<Vec<UiEvent>>>,
    pub jobs: Arc<Mutex<ExternalJobs>>,
    pub cli: Arc<CliService>,
    pub cfgstores: Arc<crate::cfgstore::CfgStores>,
    /// Nachzuholende Host-Operationen (im Faden der Oberfläche, nächstes `poll`).
    pub deferred: Arc<Mutex<Vec<(String, Value)>>>,
}

impl HostShared for AppShared {
    fn call(&self, op: &str, a: Value) -> Result<Value, String> {
        match op {
            "event" => {
                let e = UiEvent::JobResult { job: a["job"].as_u64().unwrap_or(0), kind: a["kind"].as_str().unwrap_or_default().into(), target: a["target"].as_str().unwrap_or_default().into(), json: a["json"].as_str().unwrap_or("{}").into() };
                self.events.lock().unwrap_or_else(|e| e.into_inner()).push(e);
                Ok(Value::Null)
            }
            "tree_changed" => {
                self.events.lock().unwrap_or_else(|e| e.into_inner()).push(UiEvent::TreeChanged);
                Ok(Value::Null)
            }
            "defer" => {
                self.deferred.lock().unwrap_or_else(|e| e.into_inner()).push((a["op"].as_str().unwrap_or_default().to_string(), a["args"].clone()));
                Ok(Value::Null)
            }
            "job_begin" => Ok(json!({ "id": self.jobs.lock().unwrap_or_else(|e| e.into_inner()).begin(a["label"].as_str().unwrap_or("Erweiterung")) })),
            "job_end" => {
                self.jobs.lock().unwrap_or_else(|e| e.into_inner()).end(a["id"].as_u64().unwrap_or(0));
                self.events.lock().unwrap_or_else(|e| e.into_inner()).push(UiEvent::JobsChanged);
                Ok(Value::Null)
            }
            "job_cancelled" => Ok(json!(self.jobs.lock().unwrap_or_else(|e| e.into_inner()).cancelled(a["id"].as_u64().unwrap_or(0)))),
            "cli_run" => {
                let steps: Vec<DialogStep> = serde_json::from_value(a["steps"].clone()).map_err(|e| e.to_string())?;
                let opts: CliOptions = serde_json::from_value(a["options"].clone()).unwrap_or_default();
                let out = self.cli.run(CliTicket(a["ticket"].as_u64().unwrap_or(0)), &steps, &opts)?;
                serde_json::to_value(out).map_err(|e| e.to_string())
            }
            "cli_ticket_info" => Ok(self.cli.info(CliTicket(a["ticket"].as_u64().unwrap_or(0)))),
            "http_request" => self.cli.http(CliTicket(a["ticket"].as_u64().unwrap_or(0)), &a),
            "snmp_walk" => {
                let max = a["max"].as_u64().unwrap_or(500).min(5000) as usize;
                self.cli.snmp_walk(CliTicket(a["ticket"].as_u64().unwrap_or(0)), a["host"].as_str().unwrap_or_default(), a["prefix"].as_str().unwrap_or_default(), max)
            }
            "snmp_get" => {
                let oids: Vec<String> = a["oids"].as_array().map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                self.cli.snmp_get(CliTicket(a["ticket"].as_u64().unwrap_or(0)), a["host"].as_str().unwrap_or_default(), &oids)
            }
            "run_external" => {
                let args: Vec<String> = a["args"].as_array().map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let env = a["env"].as_object().cloned().unwrap_or_default();
                self.cli.run_external(CliTicket(a["ticket"].as_u64().unwrap_or(0)), a["program"].as_str().unwrap_or_default(), &args, &env, a["cwd"].as_str().unwrap_or_default(), a["log"].as_str().unwrap_or_default(), a["stdin"].as_str()).map(|pid| json!({ "pid": pid }))
            }
            "external_stop" => {
                self.cli.external_stop(a["pid"].as_u64().unwrap_or(0) as u32);
                Ok(Value::Null)
            }
            "external_state" => Ok(self.cli.external_state(a["pid"].as_u64().unwrap_or(0) as u32)),
            "text" => Ok(json!(crate::texts::text(a["key"].as_str().unwrap_or_default()))),
            "cfgstore_close" => {
                self.cfgstores.close(a["handle"].as_u64().unwrap_or(0));
                Ok(Value::Null)
            }
            op if op.starts_with("cfgstore_") => self.cfgstores.call(op, &a),
            _ => Err(format!("Unbekannte Host-Operation „{op}“")),
        }
    }
}

impl App {
    /// Inventar eines Ziels (in der Vorführung nur im Speicher).
    fn host_inventory(&self, key: &str) -> crate::overview::inventory::Inventory {
        if self.demo {
            self.overview.demo_inventory.get(key).cloned().unwrap_or_default()
        } else {
            self.overview.store.load_key(key)
        }
    }

    fn host_inventory_save(&mut self, key: &str, inv: crate::overview::inventory::Inventory) -> Result<Value, String> {
        if self.demo {
            self.overview.demo_inventory.insert(key.to_string(), inv);
        } else {
            self.overview.save_inventory(key, &inv)?;
        }
        Ok(Value::Null)
    }

    /// Zugangsschein für ein Ziel: SSH mit den Zugangsdaten des Ziels bzw. seines SSH-Wegs; Adresse aus der
    /// Dienstprüfung, falls abweichend.
    pub(crate) fn cli_ticket_for_target(&self, id: Uuid) -> Result<CliTicket, String> {
        self.cli_ticket_for_target_at(id, None, None)
    }

    /// Wie [`cli_ticket_for_target`](Self::cli_ticket_for_target), wahlweise für eine andere Adresse und einen anderen Port.
    pub(crate) fn cli_ticket_for_target_at(&self, id: Uuid, host: Option<&str>, port: Option<u16>) -> Result<CliTicket, String> {
        let t = self.target(id).cloned().ok_or("Ziel nicht gefunden")?;
        let (view, r) = self.inventory_view(&t)?;
        let payload = self.credential_payload(&r, Protocol::Ssh)?;
        let host = host.map(str::to_string).unwrap_or_else(|| self.overview.services_ip(&id.to_string()).unwrap_or_else(|| t.host.trim().to_string()));
        let port = port.unwrap_or(if view.protocol == Protocol::Ssh { view.effective_port() } else { 22 });
        let spec = crate::patch::JobSpec { protocol: Protocol::Ssh, host, port, credentials: payload, transport: None, legacy: self.target_legacy_of(id) };
        Ok(self.cli.issue(spec, self.known_hosts_ref(), Some(r.reference.entry)))
    }

    /// Zugangsschein für eine Adresse mit dem benannten Zugangsdatensatz `slot` eines Ordners (vererbt über die
    /// Ordnerkette bis zur Wurzel).
    fn cli_ticket_for_slot(&self, host: &str, port: u16, folder: Uuid, slot: &str, legacy: bool) -> Result<CliTicket, String> {
        let mut chain: Vec<&rj_core::Folder> = Vec::new();
        for n in self.index.path_to(folder).unwrap_or_default() {
            if let Node::Folder(f) = n {
                chain.push(f);
            }
        }
        chain.push(&self.index.root);
        let (fid, r) = chain.iter().rev().find_map(|f| f.settings.credentials.get(slot).map(|r| (f.id, r.clone()))).ok_or_else(|| format!("Anmeldeobjekt „{slot}“ fehlt am Standort"))?;
        let rc = rj_core::inherit::ResolvedCredential { name: slot.to_string(), reference: r, origin: rj_core::inherit::Origin::Folder(fid), overrides: Default::default() };
        let payload = self.credential_payload(&rc, Protocol::Ssh)?;
        let cred = rc.reference.entry;
        let spec = crate::patch::JobSpec { protocol: Protocol::Ssh, host: host.to_string(), port, credentials: payload, transport: None, legacy };
        Ok(self.cli.issue(spec, self.known_hosts_ref(), Some(cred)))
    }

    /// HTTPS-Ticket: Zugangsdaten aus einem benannten Geheimnis des Ziels (`target` + `secret`), einem Zugangsweg des
    /// Ziels (`target` + `way`: Name oder Kennung) bzw. dem Ziel selbst (`target`). Adresse aus dem Weg bzw. Ziel (bei Web aus der URL), `port` darf abweichen, Vorgabe 443; eine
    /// fremde Adresse ist nicht möglich. `auth` siehe [`crate::https_ticket::HttpAuth::from_json`].
    fn http_ticket(&self, a: &Value) -> Result<CliTicket, String> {
        let auth = crate::https_ticket::HttpAuth::from_json(&a["auth"])?;
        let (payload, host, port) = if let Some(target) = a.get("target").and_then(Value::as_str) {
            let t = self.target(Uuid::parse_str(target).map_err(|e| e.to_string())?).cloned().ok_or("Ziel nicht gefunden")?;
            let way = match a.get("way").and_then(Value::as_str).filter(|w| !w.is_empty()) {
                Some(w) => Some(t.ways.iter().find(|x| x.id.to_string() == w || x.label().eq_ignore_ascii_case(w)).cloned().ok_or_else(|| format!("Zugangsweg „{w}“ fehlt am Ziel"))?),
                None => None,
            };
            let secret = a.get("secret").and_then(Value::as_str).filter(|n| !n.is_empty());
            let payload = if auth == crate::https_ticket::HttpAuth::None {
                empty_payload()
            } else if let Some(name) = secret {
                // Benanntes Geheimnis des Ziels (z. B. API-Schlüssel).
                let r = t.settings.secrets.get(name).ok_or_else(|| format!("{name} fehlt (Eigenschaften des Ziels)"))?;
                let vault = self.vault.as_ref().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
                let c = vault.credential(r.entry).ok_or_else(|| format!("{name}: Eintrag nicht in der Schlüsseldatenbank"))?;
                if c.data.password.as_bytes().is_empty() {
                    return Err(format!("{name} fehlt (Eigenschaften des Ziels)"));
                }
                rj_ipc::msg::CredentialPayload { username: String::new(), domain: None, password: rj_ipc::msg::SecretBytes(c.data.password.as_bytes().to_vec()), private_key: None }
            } else {
                let (r, _) = self.check_credential(&t, way.as_ref())?;
                self.credential_payload(&r, Protocol::Web)?
            };
            let (h, p) = match &way {
                Some(w) if !w.host.trim().is_empty() => split_host(&w.host, w.port),
                Some(w) => split_host(&t.host, w.port),
                None => split_host(&t.host, t.port),
            };
            (payload, h, p)
        } else {
            // Nur Ziele: ihre Zugangsdaten gehen ausschließlich an ihre eigene Adresse.
            return Err("Ziel fehlt".into());
        };
        let explicit = a.get("host").and_then(Value::as_str).filter(|h| !h.trim().is_empty()).map(|h| h.trim().to_string());
        // Zugangsdaten eines Ziels gehen nur an dessen eigene Adresse (bzw. die seines Wegs), nie an eine fremde.
        if explicit.is_some() && a.get("target").is_some() {
            return Err("Bei einem Ziel kommt die Adresse aus dem Ziel bzw. Zugangsweg".into());
        }
        let host = explicit.unwrap_or(host);
        if host.is_empty() {
            return Err("Keine Adresse".into());
        }
        let port = a.get("port").and_then(Value::as_u64).map(|p| p as u16).or(port).unwrap_or(443);
        Ok(self.cli.issue_http(crate::https_ticket::HttpSpec { host, port, auth, credentials: payload }))
    }

    /// Hat der Ordner (oder einer darüber) den benannten Zugangsdatensatz `slot`?
    fn folder_has_slot(&self, folder: Uuid, slot: &str) -> bool {
        self.index.path_to(folder).unwrap_or_default().iter().any(|n| matches!(n, Node::Folder(f) if f.settings.credentials.contains_key(slot))) || self.index.root.settings.credentials.contains_key(slot)
    }

    /// Zugangsschein für eine Adresse mit einem Anmeldeobjekt (`login`).
    fn cli_ticket_for_login(&self, host: &str, port: u16, login: Uuid, legacy: bool) -> Result<CliTicket, String> {
        let l = self.index.logins.iter().find(|l| l.id == login).ok_or("Anmeldeobjekt nicht gefunden")?;
        let vault = self.vault.as_ref().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
        let c = vault.credential(l.entry.entry).ok_or("Anmeldeobjekt ohne Eintrag in der Schlüsseldatenbank")?;
        let (dom, user) = c.split_domain();
        let payload = rj_ipc::msg::CredentialPayload { username: user.to_string(), domain: dom.map(str::to_string), password: rj_ipc::msg::SecretBytes(c.data.password.as_bytes().to_vec()), private_key: None };
        let spec = crate::patch::JobSpec { protocol: Protocol::Ssh, host: host.to_string(), port, credentials: payload, transport: None, legacy };
        Ok(self.cli.issue(spec, self.known_hosts_ref(), Some(l.entry.entry)))
    }

    /// SNMPv3-Schein aus einem Anmeldeobjekt: `login`, `host`, `port` (Vorgabe 161), `auth` (`sha256`, `sha1`),
    /// `priv` (`aes128`, `none`), `lenient` (Höchstgröße der Gegenstelle begrenzen statt ablehnen). Die Werte
    /// verlassen den Kern nicht.
    fn snmp_ticket_for_login(&self, a: &Value) -> Result<CliTicket, String> {
        let login = id_of(a, "login")?;
        let host = a.get("host").and_then(Value::as_str).filter(|h| !h.is_empty()).ok_or("Adresse fehlt")?;
        let l = self.index.logins.iter().find(|l| l.id == login).ok_or("Anmeldeobjekt nicht gefunden")?;
        let vault = self.vault.as_ref().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
        let c = vault.credential(l.entry.entry).ok_or("Eintrag nicht in der Schlüsseldatenbank")?;
        let auth = match a.get("auth").and_then(Value::as_str).unwrap_or("sha256") {
            "sha1" => rj_snmp::AuthProto::Sha1,
            "sha256" => rj_snmp::AuthProto::Sha256,
            other => return Err(format!("SNMP-Authentisierung „{other}“ wird nicht unterstützt")),
        };
        let privacy = match a.get("priv").and_then(Value::as_str).unwrap_or("aes128") {
            "none" => None,
            "aes128" => Some((rj_snmp::PrivProto::Aes128, vault.field(c.uuid, rj_core::SNMP_PRIV_FIELD).unwrap_or_else(|| c.data.password.clone()))),
            other => return Err(format!("SNMP-Verschlüsselung „{other}“ wird nicht unterstützt")),
        };
        let credentials = rj_snmp::Credentials::V3 { user: c.data.username.clone(), auth, auth_password: c.data.password.clone(), privacy };
        let port = a.get("port").and_then(Value::as_u64).map(|p| p as u16).unwrap_or(161);
        let dev = rj_snmp::Device { host: host.to_string(), port, credentials, timeout: std::time::Duration::from_secs(2) };
        Ok(self.cli.issue_snmp_with(dev, a.get("lenient").and_then(Value::as_bool).unwrap_or(false)))
    }

    /// SNMP-Gerät eines Ordners (Einstellung „SNMP“ auf dem Weg zur Wurzel) mit Zugangsdaten als Schein.
    fn snmp_ticket_for_folder(&self, folder: Uuid) -> Result<(CliTicket, String), String> {
        let mut chain: Vec<&rj_core::Folder> = Vec::new();
        for n in self.index.path_to(folder).unwrap_or_default() {
            if let Node::Folder(f) = n {
                chain.push(f);
            }
        }
        chain.push(&self.index.root);
        let dev_id = chain
            .iter()
            .rev()
            .find_map(|f| match f.settings.snmp {
                Some(rj_core::SnmpSetting::Device(d)) => Some(Some(d)),
                Some(rj_core::SnmpSetting::None) => Some(None),
                None => None,
            })
            .flatten()
            .ok_or("Kein SNMP-Gerät für diesen Ordner")?;
        let d = self.index.snmp_devices.iter().find(|x| x.id == dev_id).ok_or("SNMP-Gerät nicht gefunden")?;
        let vault = self.vault.as_ref().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
        let credentials = crate::app::snmp_credentials(vault, d).ok_or("Keine SNMP-Zugangsdaten")?;
        let dev = rj_snmp::Device { host: d.host.clone(), port: d.port.unwrap_or(161), credentials, timeout: std::time::Duration::from_secs(2) };
        Ok((self.cli.issue_snmp(dev), d.name.clone()))
    }

    fn host_call(&mut self, op: &str, a: Value) -> Result<Value, String> {
        match op {
            "edit" => {
                let r: Value = serde_json::from_str(&self.edit(&a.to_string())).map_err(|e| e.to_string())?;
                match r.get("error").and_then(Value::as_str) {
                    Some(e) => Err(e.to_string()),
                    None => Ok(r),
                }
            }
            "index" => serde_json::to_value(&self.index).map_err(|e| e.to_string()),
            "index_set" => {
                self.index = serde_json::from_value(a["index"].clone()).map_err(|e| e.to_string())?;
                Ok(Value::Null)
            }
            "index_apply" => {
                let index: rj_core::Index = serde_json::from_value(a["index"].clone()).map_err(|e| e.to_string())?;
                let before = std::mem::replace(&mut self.index, index);
                self.commit(a["label"].as_str().unwrap_or("Erweiterung"), before);
                Ok(Value::Null)
            }
            "target" => {
                let id = id_of(&a, "id")?;
                serde_json::to_value(self.target(id)).map_err(|e| e.to_string())
            }
            "folder" => {
                let id = id_of(&a, "id")?;
                let f = if id == self.index.root.id { Some(&self.index.root) } else { rj_core::edit::folder(&self.index, id) };
                serde_json::to_value(f).map_err(|e| e.to_string())
            }
            "node_path" => {
                let id = id_of(&a, "id")?;
                let names: Vec<&str> = self.index.path_to(id).unwrap_or_default().iter().map(|n| n.name()).collect();
                Ok(json!({ "path": names.join(" / ") }))
            }
            "settings_extra" => serde_json::to_value(&self.config.features.extra).map_err(|e| e.to_string()),
            "set_settings_extra" => {
                if a.is_object() {
                    self.config.features.set_section(&a);
                    if let Some(cp) = self.config_path.clone() {
                        let _ = self.config.save(&cp);
                    }
                }
                Ok(Value::Null)
            }
            "settings_core" => Ok(json!({ "mcp_hide_addresses": self.config.features.mcp_hide_addresses, "mcp_hide_serials": self.config.features.mcp_hide_serials, "connect_all_parallel": self.config.connection.connect_all_parallel, "update_proxy": self.config.update.proxy })),
            "light" => {
                let id = id_of(&a, "id")?;
                Ok(serde_json::to_value(self.lights.get(&id)).unwrap_or(Value::Null))
            }
            "redact" => Ok(self.redact(a)),
            "login_kinds" => Ok(json!(crate::sitesetup::login_kinds())),
            "edition" => Ok(json!(crate::plugins::edition())),
            "program_version" => Ok(json!(rj_update::CURRENT_VERSION)),
            "mcp_target" => self.mcp_target_arg(&a).and_then(|t| serde_json::to_value(t).map_err(|e| e.to_string())),
            "plugin_op" => crate::plugins::dispatch(self, a["op"].as_str().unwrap_or_default(), &a["args"]).unwrap_or_else(|| Err("Unbekannte Operation der Erweiterung".into())),
            // Vermittlung zwischen Erweiterungen: Operation einer anderen, Anbieter einer Anschlussstelle.
            "plugin_call" => crate::plugins::call_other(self, a["to"].as_str().unwrap_or_default(), a["op"].as_str().unwrap_or_default(), &a["args"]),
            "providers" => Ok(json!(crate::plugins::providers(a["anschluss"].as_str().unwrap_or_default()))),
            "cfgstore_state" => self.cfgstore_state(id_of(&a, "folder")?, a["name"].as_str().unwrap_or_default()),
            "cfgstore_ensure_entry" => self.cfgstore_ensure_entry().map(|_| Value::Null),
            "cfgstore_open" => self.cfgstore_open(id_of(&a, "folder")?, a["name"].as_str().unwrap_or_default(), a["create"].as_bool().unwrap_or(false)).map(|h| json!({ "handle": h })),
            "login_create" => {
                let folder = id_of(&a, "folder")?;
                let name = a["name"].as_str().unwrap_or_default().to_string();
                let title = a["title"].as_str().map(str::to_string).unwrap_or_else(|| format!("Anmeldung: {name}"));
                let vault = self.vault.as_ref().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
                // Vorlage: Anmeldeobjekt gleichen Namens im Vorlagenordner (Benutzername, Kennwort übernehmen).
                let template = a["template"].as_str().and_then(|label| {
                    let tf = crate::sitesetup::login_templates_folder(&self.index)?;
                    self.index.logins.iter().find(|l| l.folder == tf && l.name == label).and_then(|l| vault.credential(l.entry.entry)).map(|c| (c.data.username.clone(), c.data.password.clone()))
                });
                let missing = a["template"].is_string() && template.is_none();
                let (username, password) = template.unwrap_or_default();
                let vault = self.vault.as_mut().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
                let entry = vault.add_credential(rj_kdbx::CredentialData { title: title.clone(), username, password, ..Default::default() }).map_err(|e| e.to_string())?;
                let lid = Uuid::new_v4();
                self.index.logins.push(rj_core::Login { id: lid, name, folder, entry: rj_core::CredentialRef { entry, last_title: Some(title) } });
                rj_core::inherit::sync_logins(&mut self.index);
                Ok(json!({ "id": lid, "entry": entry, "missing_template": missing }))
            }
            "data_dir" => Ok(json!(rj_core::config::data_dir())),
            "text" => Ok(json!(crate::texts::text(a["key"].as_str().unwrap_or_default()))),
            "demo" => Ok(json!(self.demo)),
            "vault_open" => Ok(json!(self.vault.is_some())),
            "read_only" => Ok(json!(self.vault.as_ref().map_or(true, |v| v.index().read_only))),
            "device" => {
                let id = id_of(&a, "id")?;
                serde_json::to_value(self.overview.devices.get(&id)).map_err(|e| e.to_string())
            }
            "set_device" => {
                let id = id_of(&a, "id")?;
                match serde_json::from_value::<Option<DeviceInfo>>(a["device"].clone()).map_err(|e| e.to_string())? {
                    Some(d) => {
                        self.overview.devices.insert(id, d);
                    }
                    None => {
                        self.overview.devices.remove(&id);
                    }
                }
                Ok(Value::Null)
            }
            "inventory" => {
                let key = a["key"].as_str().unwrap_or_default();
                let inv = self.host_inventory(key);
                Ok(if inv.fields.is_empty() { Value::Null } else { json!({ "fields": crate::overview::inventory_json(&inv), "web_way": inv.web_way }) })
            }
            "inventory_web_way" => {
                let key = a["key"].as_str().unwrap_or_default().to_string();
                let mut inv = self.host_inventory(&key);
                inv.web_way = true;
                self.host_inventory_save(&key, inv)
            }
            "inventory_offer" => {
                let key = a["key"].as_str().unwrap_or_default().to_string();
                let mut inv = self.host_inventory(&key);
                inv.offer(a["field"].as_str().unwrap_or_default(), a["value"].as_str().unwrap_or_default(), a["source"].as_str().unwrap_or("Erweiterung"), rj_plugin_api::now());
                self.host_inventory_save(&key, inv)
            }
            "services_ip" => Ok(json!(self.overview.services_ip(a["key"].as_str().unwrap_or_default()))),
            "cli_ticket" => {
                let t = if let Some(target) = a.get("target").and_then(Value::as_str) {
                    self.cli_ticket_for_target_at(Uuid::parse_str(target).map_err(|e| e.to_string())?, a["host"].as_str(), a["port"].as_u64().map(|p| p as u16))?
                } else {
                    let host = a["host"].as_str().unwrap_or_default();
                    let port = a["port"].as_u64().unwrap_or(22) as u16;
                    let legacy = a["legacy"].as_bool().unwrap_or(false);
                    if let Some(slot) = a.get("slot").and_then(Value::as_str) {
                        self.cli_ticket_for_slot(host, port, id_of(&a, "folder")?, slot, legacy)?
                    } else {
                        self.cli_ticket_for_login(host, port, id_of(&a, "login")?, legacy)?
                    }
                };
                Ok(json!({ "ticket": t.0 }))
            }
            "http_ticket" => Ok(json!({ "ticket": self.http_ticket(&a)?.0 })),
            "secret_state" => Ok(json!({ "set": self.secret_state(id_of(&a, "target")?, a["name"].as_str().unwrap_or_default()) })),
            // Eigene Zugangsdaten eines Zugangswegs als Geheimnis des Ziels übernehmen und den Weg entfernen (Umstellung
            // älterer Einrichtungen; der Wert verlässt den Kern dabei nicht).
            "secret_from_way" => {
                let id = id_of(&a, "target")?;
                let (way, name) = (a["way"].as_str().unwrap_or_default().to_string(), a["name"].as_str().unwrap_or_default().to_string());
                if name.is_empty() || way.is_empty() {
                    return Err("Weg und Name nötig".into());
                }
                let mut idx = self.index.clone();
                let Some(Node::Target(t)) = rj_core::inherit::find_node_mut(&mut idx.root, id) else { return Err("Ziel nicht gefunden".into()) };
                let Some(pos) = t.ways.iter().position(|w| w.label().eq_ignore_ascii_case(&way)) else { return Ok(json!({ "moved": false })) };
                let w = t.ways.remove(pos);
                if let Some(own) = w.own {
                    t.settings.secrets.entry(name).or_insert(own);
                }
                if t.open_with == Some(w.id) {
                    t.open_with = None;
                }
                let before = std::mem::replace(&mut self.index, idx);
                self.commit("Geheimnis aus Zugangsweg übernehmen", before);
                Ok(json!({ "moved": true }))
            }
            "https_accept_certificate" => {
                crate::https_ticket::accept(a["host"].as_str().unwrap_or_default(), a["fingerprint"].as_str().unwrap_or_default())?;
                Ok(Value::Null)
            }
            "folder_has_slot" => Ok(json!(self.folder_has_slot(id_of(&a, "folder")?, a["slot"].as_str().unwrap_or_default()))),
            "busy_set" => {
                self.overview.set_busy(a["key"].as_str().unwrap_or_default(), a["job"].as_u64().unwrap_or(0));
                Ok(Value::Null)
            }
            "busy_clear" => {
                self.overview.finish_busy(a["key"].as_str().unwrap_or_default());
                Ok(Value::Null)
            }
            "busy" => Ok(json!(self.overview.busy_job(a["key"].as_str().unwrap_or_default()))),
            "cli_revoke" => {
                self.cli.revoke(CliTicket(a["ticket"].as_u64().unwrap_or(0)));
                Ok(Value::Null)
            }
            "snmp_ticket" => {
                // Mit `login`: SNMPv3 über ein Anmeldeobjekt (Benutzer, Kennwort zur Authentisierung, geschütztes Feld
                // zur Verschlüsselung) für `host`; ohne: SNMP-Gerät des Ordners.
                if a.get("login").is_some() {
                    let t = self.snmp_ticket_for_login(&a)?;
                    return Ok(json!({ "ticket": t.0 }));
                }
                let (t, name) = self.snmp_ticket_for_folder(id_of(&a, "folder")?)?;
                Ok(json!({ "ticket": t.0, "name": name }))
            }
            "tree_changed" => {
                self.plugin_events.lock().unwrap_or_else(|e| e.into_inner()).push(UiEvent::TreeChanged);
                Ok(Value::Null)
            }
            other => self.shared().call(other, a),
        }
    }
}

impl Host for App {
    fn call(&mut self, op: &str, args: Value) -> Result<Value, String> {
        self.host_call(op, args)
    }

    fn shared(&self) -> Arc<dyn HostShared> {
        Arc::new(AppShared { events: self.plugin_events.clone(), jobs: self.jobs.external(), cli: self.cli.clone(), cfgstores: self.cfgstores.clone(), deferred: self.plugin_deferred.clone() })
    }

    fn index_ref(&self) -> &rj_core::Index {
        &self.index
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// Neu gebaut nur, wenn sich der Index seit dem letzten Mal geändert hat (Vergleich statt Buchführung über
    /// alle Stellen, die ihn ändern; der Vergleich kostet nur einen Bruchteil des Serialisierens).
    fn index_cached(&mut self) -> Option<(u64, Arc<String>)> {
        let fresh = matches!(&self.index_cache, Some((_, copy, _)) if *copy == self.index);
        if !fresh {
            let json = serde_json::to_string(&self.index).ok()?;
            self.index_gen += 1;
            self.index_cache = Some((self.index_gen, self.index.clone(), Arc::new(json)));
        }
        self.index_cache.as_ref().map(|(g, _, j)| (*g, j.clone()))
    }
}

#[cfg(test)]
mod https_tests {
    use super::split_host;

    #[test]
    fn host_and_port_come_from_address_or_url() {
        assert_eq!(split_host("https://192.0.2.5:8443/manage", None), ("192.0.2.5".into(), Some(8443)));
        assert_eq!(split_host("controller.local", None), ("controller.local".into(), None));
        assert_eq!(split_host("192.0.2.5:11443", Some(443)), ("192.0.2.5".into(), Some(443)));
        assert_eq!(split_host("https://[2001:db8::1]:8443/", None), ("2001:db8::1".into(), Some(8443)));
        assert_eq!(split_host("2001:db8::1", None), ("2001:db8::1".into(), None));
    }
}

#[cfg(test)]
mod secret_tests {
    use crate::testing::{app_with_vault, cmd};
    use rj_plugin_api::{Host, HostExt, HostSharedExt};
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn secret_is_stored_hidden_and_only_usable_by_ticket_to_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _p, _k) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let id = Uuid::parse_str(cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "web", "name": "Ctrl" }))["id"].as_str().unwrap()).unwrap();
        let r = cmd(&mut app, json!({ "op": "update", "id": id, "fields": { "options": { "url": "https://192.0.2.7/" }, "secrets": { "API-Schlüssel": "abc123" } } }));
        assert!(r.get("error").is_none() && r.get("errors").is_none(), "{r}");
        assert_eq!(cmd(&mut app, json!({ "op": "secret_state", "id": id, "name": "API-Schlüssel" }))["set"], true);
        // Nicht in den Eigenschaften als Text.
        assert!(!app.properties_json(id).contains("abc123"));
        let host: &mut dyn Host = &mut app;
        assert!(host.secret_state(id, "API-Schlüssel"));
        assert!(!host.secret_state(id, "anderes"));
        let t = host.http_ticket(json!({ "target": id, "secret": "API-Schlüssel", "auth": { "kind": "header", "name": "X-API-KEY" } })).unwrap();
        let info = host.shared().cli_ticket_info(t);
        assert_eq!(info["host"], "192.0.2.7");
        assert!(host.http_ticket(json!({ "target": id, "secret": "fehlt", "auth": { "kind": "header", "name": "X-API-KEY" } })).unwrap_err().contains("fehlt"));
        // Leer = unverändert, null = entfernen.
        let _ = cmd(&mut app, json!({ "op": "update", "id": id, "fields": { "secrets": { "API-Schlüssel": "" } } }));
        assert_eq!(cmd(&mut app, json!({ "op": "secret_state", "id": id, "name": "API-Schlüssel" }))["set"], true);
        let _ = cmd(&mut app, json!({ "op": "update", "id": id, "fields": { "secrets": { "API-Schlüssel": null } } }));
        assert_eq!(cmd(&mut app, json!({ "op": "secret_state", "id": id, "name": "API-Schlüssel" }))["set"], false);
    }
}
