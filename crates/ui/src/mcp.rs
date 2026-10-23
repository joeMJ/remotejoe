//! Claude-Anbindung (MCP): Antworten auf Werkzeugaufrufe, die `rj-mcp` über den lokalen Kanal weiterreicht.
//!
//! - Ab Werk aus (Einstellungen → Zusatzfunktionen). Antworten gibt es nur bei entsperrter Schlüsseldatenbank.
//! - Claude sieht nur Ziele in freigegebenen Ordnern (je Schlüsseldatenbank). Adressen und Seriennummern lassen
//!   sich ausblenden. Zugangsdaten, Benutzernamen, Notizen und andere Inhalte der KDBX gehen nie hinaus.
//! - Aktionen (Verbinden, Inventar, Cluster, Nutzer) nur bei Zielen, deren wirksame Zugangsdaten aus einem für
//!   Claude freigegebenen Anmeldeobjekt stammen, und nur nach Freigabe je Aktion in der Oberfläche.
//! - Jeder Aufruf landet im Protokoll (Extras → Claude-Protokoll).

use std::sync::Mutex;

use rj_core::inherit::{resolve_credential, CredentialResolution};
use rj_core::model::Node;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app::App;

/// Protokoll der Aufrufe (neueste zuletzt), höchstens 500 Einträge.
static LOG: Mutex<Vec<Value>> = Mutex::new(Vec::new());

fn log(tool: &str, args: &Value, result: &str) {
    let mut l = LOG.lock().unwrap_or_else(|e| e.into_inner());
    l.push(json!({ "at": crate::overview::inventory::now(), "tool": tool, "args": args, "result": result }));
    let n = l.len();
    if n > 500 {
        l.drain(..n - 500);
    }
}

/// Werkzeuge, die etwas auslösen (Freigabe je Aktion nötig).
pub const ACTIONS: [&str; 4] = ["verbinden", "inventar_abfragen", "cluster_abfragen", "nutzer_anzeigen"];

/// Feldnamen des Inventars, die Adressen bzw. Seriennummern enthalten.
fn hidden_field(key: &str, hide_addr: bool, hide_serial: bool) -> bool {
    (hide_addr && matches!(key, "ip" | "mac")) || (hide_serial && key == "serial")
}

impl App {
    fn kdbx_key(&self) -> Option<String> {
        self.kdbx_path()
    }

    /// Freigegebene Ordner der geöffneten Schlüsseldatenbank.
    fn mcp_folders(&self) -> Vec<Uuid> {
        self.kdbx_key().and_then(|k| self.config.features.mcp_folders.get(&k).cloned()).unwrap_or_default().iter().filter_map(|s| Uuid::parse_str(s).ok()).collect()
    }

    /// Für Claude freigegebene Anmeldeobjekte der geöffneten Schlüsseldatenbank.
    fn mcp_logins(&self) -> Vec<Uuid> {
        self.kdbx_key().and_then(|k| self.config.features.mcp_logins.get(&k).cloned()).unwrap_or_default().iter().filter_map(|s| Uuid::parse_str(s).ok()).collect()
    }

    /// Liegt der Knoten in einem freigegebenen Ordner (oder ist er einer)?
    fn mcp_visible(&self, id: Uuid) -> bool {
        let allowed = self.mcp_folders();
        self.index.path_to(id).is_some_and(|p| p.iter().any(|n| allowed.contains(&n.id())))
    }

    fn mcp_path(&self, id: Uuid) -> String {
        self.index.path_to(id).unwrap_or_default().iter().rev().skip(1).rev().map(|n| n.name().to_string()).collect::<Vec<_>>().join(" › ")
    }

    /// Kurzbeschreibung eines Ziels für Claude (ohne Zugangsdaten; Adresse und Seriennummer je nach Einstellung).
    fn mcp_target(&self, t: &rj_core::Target, with_inventory: bool) -> Value {
        let s = &self.config.features;
        let inv = self.overview.demo_inventory.get(&t.id.to_string()).cloned().unwrap_or_else(|| self.overview.store.load_key(&t.id.to_string()));
        let mut v = json!({
            "id": t.id,
            "name": t.name,
            "typ": self.overview.devices.get(&t.id).map(|d| if *d == "aruba-ap" { "Aruba-AP" } else { *d }).unwrap_or(t.protocol.label()),
            "ordner": self.mcp_path(t.id),
            "version": inv.value("system").and_then(crate::overview::inventory::version_of),
        });
        if !s.mcp_hide_addresses {
            v["adresse"] = json!(t.host);
        }
        if with_inventory {
            let mut fields = serde_json::Map::new();
            for k in crate::overview::inventory::FIELDS {
                if hidden_field(k, s.mcp_hide_addresses, s.mcp_hide_serials) {
                    continue;
                }
                if let Some(val) = inv.value(k) {
                    fields.insert(k.to_string(), json!(val));
                }
            }
            v["inventar"] = Value::Object(fields);
            // Hauptweg zuerst, dann die weiteren Wege.
            let mut ways = vec![json!({ "name": "Hauptweg", "protokoll": t.protocol.label(), "hauptweg": true })];
            ways.extend(t.ways.iter().map(|w| json!({ "name": w.label(), "protokoll": w.protocol.label() })));
            v["zugangswege"] = json!(ways);
        } else if !s.mcp_hide_serials {
            if let Some(serial) = inv.value("serial") {
                v["seriennummer"] = json!(serial);
            }
        }
        v
    }

    fn mcp_targets(&self) -> Vec<rj_core::Target> {
        let mut out = Vec::new();
        for f in self.mcp_folders() {
            for id in rj_core::edit::targets_below(&self.index, f) {
                if let Some(t) = self.target_ref(id) {
                    if !out.iter().any(|x: &rj_core::Target| x.id == id) {
                        out.push(t.clone());
                    }
                }
            }
        }
        out
    }

    fn mcp_target_arg(&self, args: &Value) -> Result<rj_core::Target, String> {
        let id = args.get("id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()).ok_or("Parameter id (Ziel-ID aus ziele_suchen) fehlt")?;
        if !self.mcp_visible(id) {
            return Err("Dieses Ziel ist für Claude nicht freigegeben".into());
        }
        self.target_ref(id).cloned().ok_or_else(|| "Ziel nicht gefunden".into())
    }

    /// Darf Claude an diesem Ziel etwas auslösen? Nur mit freigegebenem Anmeldeobjekt als wirksamen Zugangsdaten.
    fn mcp_action_allowed(&self, t: &rj_core::Target) -> Result<String, String> {
        let allowed = self.mcp_logins();
        let r = match resolve_credential(&self.index, t.id).map_err(|e| e.to_string())? {
            CredentialResolution::Found(r) => r,
            _ => return Err("Das Ziel hat keine eindeutigen Zugangsdaten".into()),
        };
        match self.index.logins.iter().find(|l| l.entry.entry == r.reference.entry && allowed.contains(&l.id)) {
            Some(l) => Ok(l.name.clone()),
            None => Err("Aktionen nur bei Zielen, deren Zugangsdaten aus einem für Claude freigegebenen Anmeldeobjekt stammen".into()),
        }
    }

    /// Ein Werkzeugaufruf von Claude. `confirmed`: die Oberfläche hat die Aktion freigegeben.
    pub fn mcp_call(&mut self, c: &Value) -> Result<Value, String> {
        let tool = c.get("tool").and_then(Value::as_str).unwrap_or_default().to_string();
        let args = c.get("args").cloned().unwrap_or(json!({}));
        let r = self.mcp_dispatch(&tool, &args, c.get("confirmed").and_then(Value::as_bool).unwrap_or(false));
        let summary = match &r {
            Ok(v) if v.get("needs_confirm").is_some() => "wartet auf Freigabe".to_string(),
            Ok(v) if v.get("freigegeben").is_some() => "vom Nutzer freigegeben".to_string(),
            Ok(_) => "ok".to_string(),
            Err(e) => format!("abgelehnt: {e}"),
        };
        log(&tool, &args, &summary);
        r
    }

    /// Der Nutzer hat eine Aktion abgelehnt (oder nicht rechtzeitig freigegeben).
    pub fn mcp_denied(&self, c: &Value) -> Value {
        log(c.get("tool").and_then(Value::as_str).unwrap_or_default(), &c.get("args").cloned().unwrap_or(json!({})), "vom Nutzer abgelehnt");
        json!({ "ok": true })
    }

    fn mcp_dispatch(&mut self, tool: &str, args: &Value, confirmed: bool) -> Result<Value, String> {
        if !self.config.features.mcp {
            return Err("Die Claude-Anbindung ist in remotejoe ausgeschaltet (Einstellungen → Claude-Anbindung)".into());
        }
        if self.vault.is_none() && !self.demo {
            return Err("remotejoe ist gesperrt; bitte die Schlüsseldatenbank entsperren".into());
        }
        if self.mcp_folders().is_empty() {
            return Err("Für Claude ist noch kein Ordner freigegeben (Einstellungen → Claude-Anbindung)".into());
        }
        match tool {
            "ordner_liste" => {
                let list: Vec<Value> = self.mcp_folders().iter().filter_map(|f| self.index.path_to(*f).map(|p| json!({ "id": f, "pfad": p.iter().map(|n| n.name().to_string()).collect::<Vec<_>>().join(" › ") }))).collect();
                Ok(json!({ "ordner": list }))
            }
            "ziele_suchen" => {
                let text = args.get("text").and_then(Value::as_str).unwrap_or_default().to_lowercase();
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50).min(500) as usize;
                let hits: Vec<Value> = self
                    .mcp_targets()
                    .iter()
                    .filter(|t| text.is_empty() || t.name.to_lowercase().contains(&text) || (!self.config.features.mcp_hide_addresses && t.host.to_lowercase().contains(&text)) || self.mcp_path(t.id).to_lowercase().contains(&text))
                    .take(limit)
                    .map(|t| self.mcp_target(t, false))
                    .collect();
                Ok(json!({ "anzahl": hits.len(), "ziele": hits }))
            }
            "ziel_uebersicht" => {
                let t = self.mcp_target_arg(args)?;
                Ok(self.mcp_target(&t, true))
            }
            "ordner_inventar" => {
                let id = args.get("id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()).ok_or("Parameter id (Ordner-ID aus ordner_liste) fehlt")?;
                if !self.mcp_visible(id) {
                    return Err("Dieser Ordner ist für Claude nicht freigegeben".into());
                }
                let rows: Vec<Value> = rj_core::edit::targets_below(&self.index, id).iter().filter_map(|t| self.target_ref(*t)).map(|t| self.mcp_target(t, true)).collect();
                Ok(json!({ "anzahl": rows.len(), "ziele": rows }))
            }
            "ziele_abgleichen" => {
                let werte: Vec<String> = args.get("werte").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.trim().to_lowercase())).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
                if werte.is_empty() {
                    return Err("Parameter werte (Liste von Adressen, Seriennummern, MACs oder Namen) fehlt".into());
                }
                if werte.len() > 2000 {
                    return Err("Höchstens 2000 Werte je Aufruf".into());
                }
                let s = self.config.features.clone();
                let targets = self.mcp_targets();
                let invs: Vec<(rj_core::Target, crate::overview::inventory::Inventory)> = targets.into_iter().map(|t| {
                    let inv = self.overview.demo_inventory.get(&t.id.to_string()).cloned().unwrap_or_else(|| self.overview.store.load_key(&t.id.to_string()));
                    (t, inv)
                }).collect();
                let norm_mac = |m: &str| m.to_lowercase().replace(['-', '.'], ":");
                let mut result = Vec::new();
                for w in &werte {
                    let hits: Vec<Value> = invs
                        .iter()
                        .filter(|(t, inv)| {
                            t.name.to_lowercase() == *w
                                || (!s.mcp_hide_addresses && (t.host.to_lowercase() == *w || inv.value("ip").is_some_and(|ip| ip.to_lowercase() == *w) || inv.value("mac").is_some_and(|m| norm_mac(m) == norm_mac(w))))
                                || (!s.mcp_hide_serials && inv.value("serial").is_some_and(|x| x.to_lowercase() == *w))
                        })
                        .map(|(t, _)| self.mcp_target(t, false))
                        .collect();
                    result.push(json!({ "wert": w, "gefunden": !hits.is_empty(), "ziele": hits }));
                }
                let missing = result.iter().filter(|r| r["gefunden"] == false).count();
                Ok(json!({ "geprueft": werte.len(), "nicht_gefunden": missing, "ergebnis": result }))
            }
            "aruba_cluster" => {
                let t = self.mcp_target_arg(args)?;
                let mut c = self.overview.aruba.get(&t.id.to_string()).cloned().ok_or("Für diesen Controller liegt noch keine Cluster-Abfrage vor (in remotejoe „Cluster abfragen“ bzw. Werkzeug cluster_abfragen)")?;
                let s = &self.config.features;
                if let Some(aps) = c.get_mut("aps").and_then(Value::as_array_mut) {
                    for ap in aps {
                        if let Some(o) = ap.as_object_mut() {
                            if s.mcp_hide_addresses {
                                o.remove("ip");
                                o.remove("mac");
                            }
                            if s.mcp_hide_serials {
                                o.remove("serial");
                            }
                        }
                    }
                }
                if s.mcp_hide_addresses {
                    if let Some(o) = c.as_object_mut() {
                        o.remove("vc");
                        o.remove("segment");
                    }
                }
                Ok(c)
            }
            "sitzungen" => {
                let list: Vec<Value> = self
                    .sessions
                    .ids()
                    .into_iter()
                    .filter_map(|s| {
                        let t = self.sessions.target(s)?;
                        self.mcp_visible(t).then(|| json!({ "ziel": t, "titel": self.sessions.title(s), "protokoll": self.sessions.protocol(s).map(|p| p.label()), "zustand": self.sessions.state(s) }))
                    })
                    .collect();
                Ok(json!({ "sitzungen": list }))
            }
            a if ACTIONS.contains(&a) => {
                let t = self.mcp_target_arg(args)?;
                let login = self.mcp_action_allowed(&t)?;
                if !confirmed {
                    let what = match a {
                        "verbinden" => "eine Verbindung öffnen zu",
                        "inventar_abfragen" => "das Inventar abfragen von",
                        "cluster_abfragen" => "den Cluster abfragen an",
                        _ => "die Anwender anzeigen von",
                    };
                    return Ok(json!({ "needs_confirm": true, "prompt": format!("Claude möchte {what} „{}“ (mit Anmeldeobjekt „{login}“).", t.name), "target": t.id, "action": a }));
                }
                // Ausführen übernimmt die Oberfläche (sie öffnet Tabs bzw. startet die Abfrage).
                Ok(json!({ "freigegeben": true, "action": a, "target": t.id, "hinweis": "Gestartet. Ergebnisse von Abfragen später mit ziel_uebersicht bzw. aruba_cluster abrufen." }))
            }
            other => Err(format!("Unbekanntes Werkzeug „{other}“")),
        }
    }

    /// Einstellungen der Claude-Anbindung für die Oberfläche.
    pub fn mcp_settings_json(&self) -> Value {
        let s = &self.config.features;
        let folders: Vec<Value> = {
            let mut out = Vec::new();
            fn walk(f: &rj_core::model::Folder, path: &str, out: &mut Vec<Value>) {
                for c in &f.children {
                    if let Node::Folder(sub) = c {
                        let p = if path.is_empty() { sub.name.clone() } else { format!("{path} › {}", sub.name) };
                        out.push(json!({ "id": sub.id, "path": p }));
                        walk(sub, &p, out);
                    }
                }
            }
            walk(&self.index.root, "", &mut out);
            out
        };
        json!({
            "enabled": s.mcp,
            "hide_addresses": s.mcp_hide_addresses,
            "hide_serials": s.mcp_hide_serials,
            "allowed": self.mcp_folders(),
            "logins": self.mcp_logins(),
            "folders": folders,
            "all_logins": self.index.logins.iter().map(|l| json!({ "id": l.id, "name": l.name })).collect::<Vec<_>>(),
            "locked": self.kdbx_key().is_none(),
        })
    }

    pub fn mcp_apply_settings(&mut self, c: &Value) -> Result<Value, String> {
        let key = self.kdbx_key();
        let s = &mut self.config.features;
        if let Some(b) = c.get("enabled").and_then(Value::as_bool) {
            s.mcp = b;
        }
        if let Some(b) = c.get("hide_addresses").and_then(Value::as_bool) {
            s.mcp_hide_addresses = b;
        }
        if let Some(b) = c.get("hide_serials").and_then(Value::as_bool) {
            s.mcp_hide_serials = b;
        }
        let list = |k: &str| c.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>());
        if let Some(key) = &key {
            if let Some(f) = list("allowed") {
                s.mcp_folders.insert(key.clone(), f);
            }
            if let Some(l) = list("logins") {
                s.mcp_logins.insert(key.clone(), l);
            }
        }
        match &self.config_path {
            Some(p) => self.config.save(p).map(|_| json!({ "ok": true })).map_err(|e| e.to_string()),
            None => Ok(json!({ "ok": true })),
        }
    }

    pub fn mcp_log_json(&self) -> Value {
        json!({ "log": LOG.lock().unwrap_or_else(|e| e.into_inner()).clone() })
    }
}
