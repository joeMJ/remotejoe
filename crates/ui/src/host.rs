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
}
