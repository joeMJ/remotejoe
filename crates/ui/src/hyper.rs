//! Hypervisoren im Baum: Gäste von Proxmox VE und Hyper-V, Aktionen und
//! Konsole (Proxmox: VNC über den Websocket, Hyper-V: RDP auf den Gast).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rj_core::{Protocol, Secret};
use rj_ipc::{ConnectRequest, CredentialPayload, SecretBytes};
use rj_core::inherit::{resolve_credential, CredentialResolution};
use serde_json::json;
use uuid::Uuid;

use crate::app::App;
use crate::sessions::{ConnectSpec, UiEvent};
use crate::workspace::SessionId;

/// Angenommene Proxmox-Zertifikate (Host:Port → SHA-256-Fingerabdruck).
fn known_path() -> Option<PathBuf> {
    rj_core::config::data_dir().map(|d| d.join("proxmox-zertifikate.json"))
}

fn known() -> BTreeMap<String, String> {
    known_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Proxmox-Anmeldung aus dem Zugangsdatensatz: `benutzer@realm!token` ⇒
/// API-Token (Kennwort = Geheimnis), sonst Ticket mit Kennwort (Realm
/// `pam`, wenn keiner angegeben ist).
pub fn proxmox_auth(c: &CredentialPayload) -> rj_proxmox::Auth {
    let secret = Secret::from_bytes(c.password.0.clone());
    if c.username.contains('!') {
        return rj_proxmox::Auth::Token { id: c.username.clone(), secret };
    }
    let (user, realm) = match c.username.rsplit_once('@') {
        Some((u, r)) => (u.to_string(), r.to_string()),
        None => (c.username.clone(), "pam".to_string()),
    };
    rj_proxmox::Auth::Password { username: user, realm, password: secret }
}

fn guest_of(id: &str) -> Option<rj_proxmox::Guest> {
    let mut it = id.splitn(3, '/');
    let node = it.next()?.to_string();
    let kind = match it.next()? {
        "qemu" => rj_proxmox::GuestKind::Qemu,
        "lxc" => rj_proxmox::GuestKind::Lxc,
        _ => return None,
    };
    let vmid = it.next()?.parse().ok()?;
    Some(rj_proxmox::Guest { vmid, name: String::new(), node, kind, status: String::new(), template: false, cpu: 0.0, mem: 0, maxmem: 0 })
}

struct Ctx {
    protocol: Protocol,
    host: String,
    port: u16,
    name: String,
    creds: CredentialPayload,
    transport: Option<rj_core::PsrpTransport>,
}

impl App {
    fn hyper_ctx(&self, target: Uuid) -> Result<Ctx, String> {
        let t = self.target_ref(target).ok_or("Ziel nicht gefunden")?.clone();
        if !matches!(t.protocol, Protocol::Proxmox | Protocol::Hyperv) {
            return Err("Kein Hypervisor".into());
        }
        let r = match resolve_credential(&self.index, target).map_err(|e| e.to_string())? {
            CredentialResolution::Found(r) => r,
            CredentialResolution::Ambiguous(_) => return Err("Mehrere Zugangsdatensätze; bitte am Ziel festlegen".into()),
            CredentialResolution::NotFound { name } => return Err(format!("Kein Zugangsdatensatz „{name}“")),
            CredentialResolution::Disabled => return Err("Für dieses Ziel sind keine Zugangsdaten festgelegt".into()),
        };
        let creds = self.credential_payload(&r, t.protocol)?;
        let transport = rj_core::inherit::resolve_psrp_transport(&self.index, target).ok().flatten();
        Ok(Ctx { protocol: t.protocol, host: t.host.clone(), port: t.effective_port(), name: t.name.clone(), creds, transport })
    }

    /// Fehler eines Proxmox-Aufrufs; bei unbekanntem Zertifikat eine Rückfrage.
    fn proxmox_error(c: &Ctx, e: rj_proxmox::ProxmoxError, seen: &Arc<Mutex<Option<String>>>) -> String {
        if let Some(fp) = seen.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let key = format!("{}:{}", c.host, c.port);
            let old = known().get(&key).cloned();
            if old.as_deref() != Some(fp.as_str()) {
                return json!({ "certificate": { "host": key, "fingerprint": fp, "old": old } }).to_string();
            }
        }
        json!({ "error": e.to_string() }).to_string()
    }

    /// Fingerabdruck eines Proxmox-Hosts annehmen.
    pub fn accept_proxmox_certificate(&self, host: &str, fingerprint: &str) -> bool {
        let Some(p) = known_path() else { return false };
        let mut k = known();
        k.insert(host.to_string(), fingerprint.to_string());
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
        rj_core::fsutil::atomic_write_with_backup(&p, &serde_json::to_vec_pretty(&k).unwrap_or_default()).is_ok()
    }

    /// Gäste eines Hypervisors: `{"guests":[{id,name,state,detail,console}]}`,
    /// `{"certificate":{…}}`, `{"error":"…"}` oder `{"pending":job}`; das
    /// Ergebnis einer Hintergrundaufgabe kommt als `JobResult` (Art `guests`).
    pub fn guests_json(&mut self, target: Uuid) -> String {
        if self.demo {
            return json!({ "guests": [
                { "id": "pve1/qemu/100", "name": "web-01", "state": "running", "detail": "pve1 · VM 100", "console": true },
                { "id": "pve1/lxc/200", "name": "behälter-1", "state": "stopped", "detail": "pve1 · CT 200", "console": true }
            ] })
            .to_string();
        }
        let c = match self.hyper_ctx(target) {
            Ok(c) => c,
            Err(e) => return json!({ "error": e }).to_string(),
        };
        let timeout = Duration::from_secs(self.config.connection.connect_timeout_secs as u64);
        let label = format!("Gäste von {} abrufen", c.name);
        let job = self.jobs.spawn(label, move |job| {
            let result = fetch_guests(&c, timeout);
            Box::new(move |_app: &mut App, cancelled| {
                if cancelled {
                    return Vec::new();
                }
                vec![UiEvent::JobResult { job, kind: "guests".into(), target: target.to_string(), json: result }]
            })
        });
        json!({ "pending": job }).to_string()
    }

    /// Aktion `start`, `shutdown`, `stop` (hart) oder `reboot` im Hintergrund;
    /// Ergebnis als `JobResult` (Art `guest_action`).
    pub fn guest_action(&mut self, target: Uuid, guest: &str, action: &str) -> String {
        if self.demo {
            return json!({ "ok": true }).to_string();
        }
        let c = match self.hyper_ctx(target) {
            Ok(c) => c,
            Err(e) => return json!({ "error": e }).to_string(),
        };
        let timeout = Duration::from_secs(self.config.connection.connect_timeout_secs as u64);
        let (guest, action) = (guest.to_string(), action.to_string());
        let job = self.jobs.spawn(format!("{} – {action}", c.name), move |job| {
            let result = run_action(&c, &guest, &action, timeout);
            Box::new(move |_app: &mut App, cancelled| {
                if cancelled {
                    return Vec::new();
                }
                vec![UiEvent::JobResult { job, kind: "guest_action".into(), target: target.to_string(), json: result }]
            })
        });
        json!({ "pending": job }).to_string()
    }

    /// Konsole eines Gasts öffnen; `console_host` (Hyper-V) aus der Liste.
    /// Proxmox: Konsole wird im Hintergrund geöffnet (`{"pending":job}`).
    pub fn guest_console(&mut self, target: Uuid, guest: &str, title: &str, console_host: &str, width: u32, height: u32) -> String {
        let c = match self.hyper_ctx(target) {
            Ok(c) => c,
            Err(e) => return json!({ "error": e }).to_string(),
        };
        let title = format!("{} – {}", c.name, title);
        let request = ConnectRequest {
            host: String::new(),
            port: 0,
            width: width.max(200),
            height: height.max(200),
            options_json: String::new(),
            connect_timeout_secs: self.config.connection.connect_timeout_secs,
            keepalive_secs: self.config.connection.keepalive_secs,
        };
        match c.protocol {
            Protocol::Proxmox => {
                let Some(g) = guest_of(guest) else { return json!({ "error": "unbekannter Gast" }).to_string() };
                let opts = json!({ "prefer_tls": false, "clipboard_limit": self.config.connection.clipboard_limit }).to_string();
                let timeout = Duration::from_secs(self.config.connection.connect_timeout_secs as u64);
                let guest = guest.to_string();
                let job = self.jobs.spawn(format!("Konsole {title} öffnen"), move |job| {
                    let r = open_console(&c, &g, timeout);
                    Box::new(move |app: &mut App, cancelled| {
                        let bridge = match r {
                            Ok(b) if !cancelled => b,
                            Ok(_) => return Vec::new(),
                            Err(e) => {
                                app.job_results.insert(job, Err(e.clone()));
                                if cancelled {
                                    return Vec::new();
                                }
                                return vec![UiEvent::JobResult { job, kind: "console".into(), target: target.to_string(), json: if e.starts_with('{') { e } else { json!({ "error": e }).to_string() } }];
                            }
                        };
                        let creds = CredentialPayload { username: String::new(), domain: None, password: SecretBytes(bridge.password.as_bytes().to_vec()), private_key: None };
                        let request = ConnectRequest { host: "127.0.0.1".into(), port: bridge.port, options_json: opts, ..request };
                        let s = app.open_session(ConnectSpec { target, title, protocol: Protocol::Vnc, request, credentials: creds, named_credentials: Default::default() }, None);
                        app.console_bridges.insert(s, bridge);
                        app.console_sources.insert(s, (target, guest));
                        app.sessions.set_hold(s, true);
                        app.job_results.insert(job, Ok(s));
                        vec![UiEvent::SessionOpened { s, job }]
                    })
                });
                json!({ "pending": job }).to_string()
            }
            _ => {
                if console_host.is_empty() {
                    return json!({ "error": "Der Gast meldet keine Adresse (Integrationsdienste prüfen)" }).to_string();
                }
                let opts = json!({ "clipboard_limit": self.config.connection.clipboard_limit }).to_string();
                let request = ConnectRequest { host: console_host.to_string(), port: 3389, options_json: opts, ..request };
                let s = self.open_session(ConnectSpec { target, title, protocol: Protocol::Rdp, request, credentials: c.creds.clone(), named_credentials: Default::default() }, None);
                json!({ "session": s }).to_string()
            }
        }
    }

    /// Proxmox-Konsole neu öffnen (neues Ticket, neue Brücke) und die Sitzung
    /// damit neu verbinden. Die alte Brücke wird dabei verworfen.
    pub(crate) fn reopen_console(&mut self, s: SessionId) {
        let Some((target, guest)) = self.console_sources.get(&s).cloned() else { return };
        // Alte Brücke sofort schließen: keine Reste, auch wenn das Öffnen scheitert.
        self.console_bridges.remove(&s);
        let c = match self.hyper_ctx(target) {
            Ok(c) => c,
            Err(e) => {
                self.sessions.push_notice(s, &format!("Konsole: {e}"));
                return;
            }
        };
        let Some(g) = guest_of(&guest) else { return };
        let timeout = Duration::from_secs(self.config.connection.connect_timeout_secs as u64);
        self.jobs.spawn(format!("Konsole {} neu öffnen", c.name), move |_| {
            let r = open_console(&c, &g, timeout);
            Box::new(move |app: &mut App, cancelled| {
                if cancelled || !app.console_sources.contains_key(&s) {
                    return Vec::new();
                }
                match r {
                    Ok(bridge) => {
                        app.sessions.set_endpoint(s, "127.0.0.1", bridge.port);
                        app.sessions.set_credentials(s, CredentialPayload { username: String::new(), domain: None, password: SecretBytes(bridge.password.as_bytes().to_vec()), private_key: None });
                        app.console_bridges.insert(s, bridge);
                        app.sessions.reconnect(s);
                        Vec::new()
                    }
                    Err(e) => vec![UiEvent::Notice { s, text: format!("Konsole: {}", if e.starts_with('{') { "Zertifikat des Hosts prüfen".to_string() } else { e }) }],
                }
            })
        });
    }
}

/// Proxmox-Client mit Prüfung des Fingerabdrucks (läuft im Hintergrund).
fn proxmox_client(c: &Ctx, seen: Arc<Mutex<Option<String>>>, timeout: Duration) -> Result<rj_proxmox::Client, String> {
    let key = format!("{}:{}", c.host, c.port);
    let accepted = known().get(&key).cloned();
    let check: rj_proxmox::CertCheck = Arc::new(move |fp: &str| {
        *seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(fp.to_string());
        accepted.as_deref() == Some(fp)
    });
    let host = if c.host.contains(':') && !c.host.starts_with('[') { format!("[{}]", c.host) } else { c.host.clone() };
    rj_proxmox::Client::new(&format!("https://{host}:{}", c.port), proxmox_auth(&c.creds), check, timeout).map_err(|e| e.to_string())
}

fn open_console(c: &Ctx, g: &rj_proxmox::Guest, timeout: Duration) -> Result<rj_proxmox::ConsoleBridge, String> {
    let seen = Arc::new(Mutex::new(None));
    proxmox_client(c, seen.clone(), timeout).and_then(|cl| cl.console(g).map_err(|e| App::proxmox_error(c, e, &seen)))
}

/// Gäste abrufen (läuft im Hintergrund).
fn fetch_guests(c: &Ctx, timeout: Duration) -> String {
    match c.protocol {
        Protocol::Proxmox => {
            let seen = Arc::new(Mutex::new(None));
            let client = match proxmox_client(c, seen.clone(), timeout) {
                Ok(x) => x,
                Err(e) => return json!({ "error": e }).to_string(),
            };
            match client.guests() {
                Ok(list) => {
                    let g: Vec<_> = list
                        .iter()
                        .filter(|g| !g.template)
                        .map(|g| {
                            let kind = match g.kind {
                                rj_proxmox::GuestKind::Qemu => ("qemu", "VM"),
                                rj_proxmox::GuestKind::Lxc => ("lxc", "CT"),
                            };
                            json!({
                                "id": format!("{}/{}/{}", g.node, kind.0, g.vmid),
                                "name": rj_core::charset::mark_visible(&g.name, false),
                                "state": g.status,
                                "detail": format!("{} · {} {}", g.node, kind.1, g.vmid),
                                "console": true,
                            })
                        })
                        .collect();
                    json!({ "guests": g }).to_string()
                }
                Err(e) => App::proxmox_error(c, e, &seen),
            }
        }
        _ => match hyperv_session(c).and_then(|mut s| rj_psrp::hyperv::list(&mut s).map_err(|e| e.to_string())) {
            Ok(list) => {
                let g: Vec<_> = list
                    .iter()
                    .map(|v| {
                        json!({
                            "id": v.id,
                            "name": rj_core::charset::mark_visible(&v.name, false),
                            "state": v.state.to_lowercase(),
                            "detail": v.console_host().unwrap_or(""),
                            "console": v.console_host().is_some(),
                            "console_host": v.console_host(),
                        })
                    })
                    .collect();
                json!({ "guests": g }).to_string()
            }
            Err(e) => json!({ "error": e }).to_string(),
        },
    }
}

fn run_action(c: &Ctx, guest: &str, action: &str, timeout: Duration) -> String {
    let r = match c.protocol {
        Protocol::Proxmox => {
            let a = match action {
                "start" => rj_proxmox::Action::Start,
                "shutdown" => rj_proxmox::Action::Shutdown,
                "stop" => rj_proxmox::Action::Stop,
                "reboot" => rj_proxmox::Action::Reboot,
                _ => return json!({ "error": "unbekannte Aktion" }).to_string(),
            };
            let Some(g) = guest_of(guest) else { return json!({ "error": "unbekannter Gast" }).to_string() };
            let seen = Arc::new(Mutex::new(None));
            match proxmox_client(c, seen.clone(), timeout).and_then(|cl| cl.action(&g, a).map_err(|e| App::proxmox_error(c, e, &seen))) {
                Ok(_) => Ok(()),
                Err(e) if e.starts_with('{') => return e,
                Err(e) => Err(e),
            }
        }
        _ => {
            let a = match action {
                "start" => rj_psrp::hyperv::HvAction::Start,
                "shutdown" => rj_psrp::hyperv::HvAction::Shutdown,
                "stop" => rj_psrp::hyperv::HvAction::TurnOff,
                "reboot" => rj_psrp::hyperv::HvAction::Restart,
                _ => return json!({ "error": "unbekannte Aktion" }).to_string(),
            };
            hyperv_session(c).and_then(|mut s| rj_psrp::hyperv::action(&mut s, guest, a).map_err(|e| e.to_string()))
        }
    };
    match r {
        Ok(()) => json!({ "ok": true }).to_string(),
        Err(e) => json!({ "error": e }).to_string(),
    }
}

/// Abfrage des Gast-Inventars über den Hypervisor (Hintergrund): Felder und Quelle.
pub(crate) type GuestFetch = Box<dyn FnOnce() -> Result<(Vec<(&'static str, String)>, &'static str), String> + Send>;

impl App {
    /// Bereitet die Inventarabfrage eines Gasts vor (Zugangsdaten des Hypervisors); `running` nur für Proxmox (Adressen).
    pub(crate) fn guest_inventory_task(&self, hv: Uuid, guest: &str, running: bool) -> Result<GuestFetch, String> {
        let c = self.hyper_ctx(hv)?;
        let timeout = Duration::from_secs(self.config.connection.connect_timeout_secs as u64);
        let guest = guest.to_string();
        Ok(Box::new(move || match c.protocol {
            Protocol::Proxmox => {
                let mut g = guest_of(&guest).ok_or("unbekannter Gast")?;
                if running {
                    g.status = "running".into();
                }
                let seen = Arc::new(Mutex::new(None));
                let i = proxmox_client(&c, seen.clone(), timeout).and_then(|cl| cl.guest_info(&g).map_err(|e| App::proxmox_error(&c, e, &seen)))?;
                let mut f: Vec<(&'static str, String)> = Vec::new();
                if let Some(n) = i.name {
                    f.push(("name", n));
                }
                if let Some(v) = i.vcpu {
                    f.push(("vcpu", v.to_string()));
                }
                if let Some(m) = i.mem_bytes {
                    f.push(("memory", crate::overview::inventory::format_bytes(m)));
                }
                if i.macs.len() == 1 {
                    f.push(("mac", i.macs[0].clone()));
                }
                if let Some(ip) = i.ips.iter().find(|a| a.parse::<std::net::Ipv4Addr>().is_ok()).or(i.ips.first()) {
                    f.push(("ip", ip.clone()));
                }
                Ok((f, "Proxmox"))
            }
            _ => {
                let mut s = hyperv_session(&c)?;
                let mut lines = Vec::new();
                let mut errors = Vec::new();
                s.run(&rj_psrp::inventory::hyperv_guest_inventory_script(&guest), |r| {
                    if r.stream == "error" {
                        errors.push(r.text.clone());
                    } else {
                        lines.push(r.text.clone());
                    }
                })
                .map_err(|e| e.to_string())?;
                let f = lines.iter().find_map(|l| crate::overview::remote::parse_windows(l, None)).ok_or_else(|| errors.first().cloned().unwrap_or_else(|| "Keine Antwort vom Hyper-V-Host".into()))?;
                Ok((f, "Hyper-V"))
            }
        }))
    }
}

fn hyperv_session(c: &Ctx) -> Result<rj_psrp::PsSession, String> {
    let transport = rj_core::PsrpTransport::effective(c.transport.clone(), None);
    let t = rj_psrp::PsTarget { host: c.host.clone(), username: c.creds.username.clone(), domain: c.creds.domain.clone(), transport };
    rj_psrp::PsSession::start(&t, &Secret::from_bytes(c.creds.password.0.clone())).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_from_credential() {
        let p = |u: &str| CredentialPayload { username: u.into(), domain: None, password: SecretBytes(b"x".to_vec()), private_key: None };
        assert!(matches!(proxmox_auth(&p("root@pam!rj")), rj_proxmox::Auth::Token { id, .. } if id == "root@pam!rj"));
        assert!(matches!(proxmox_auth(&p("admin@pve")), rj_proxmox::Auth::Password { username, realm, .. } if username == "admin" && realm == "pve"));
        assert!(matches!(proxmox_auth(&p("admin")), rj_proxmox::Auth::Password { realm, .. } if realm == "pam"));
        let g = guest_of("pve1/lxc/200").unwrap();
        assert_eq!((g.node.as_str(), g.kind, g.vmid), ("pve1", rj_proxmox::GuestKind::Lxc, 200));
        assert!(guest_of("pve1/storage/1").is_none());
    }

    /// Hyper-V-Konsole: RDP auf die Adresse des Gasts mit den Zugangsdaten
    /// des Hyper-V-Hosts (kein VMConnect).
    #[test]
    fn hyperv_console_is_rdp_to_guest() {
        use rj_kdbx::{CredentialData, KdfChoice, MasterKey, Vault};
        let dir = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&dir.path().join("t.kdbx"), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
        let e = v
            .add_credential(CredentialData { title: "Host".into(), username: "EXAMPLE\\hvadmin".into(), password: Secret::from_str_exact("geheim"), ..Default::default() })
            .unwrap();
        let mut idx = rj_core::Index::default();
        let mut t = rj_core::Target::new("hv01", Protocol::Hyperv, "hv01.example.test");
        t.settings.credentials.insert("default".into(), rj_core::CredentialRef { entry: e, last_title: None });
        let tid = t.id;
        idx.root.children.push(rj_core::Node::Target(t));
        let mut app = App::new(false);
        app.index = idx;
        app.vault = Some(v);
        app.sessions = crate::sessions::Manager::new(std::path::PathBuf::from("/nicht/vorhanden"), crate::sessions::Reconnect { auto: false, max: Duration::from_secs(60) });
        let r: serde_json::Value = serde_json::from_str(&app.guest_console(tid, "6f1c0000-0000-4000-8000-000000000001", "vm-01", "192.0.2.15", 1024, 768)).unwrap();
        let s = r["session"].as_u64().unwrap_or_else(|| panic!("{r}"));
        let spec = app.sessions.spec(s).unwrap();
        assert_eq!(spec.protocol, Protocol::Rdp);
        assert_eq!((spec.request.host.as_str(), spec.request.port), ("192.0.2.15", 3389));
        assert_eq!((spec.credentials.username.as_str(), spec.credentials.domain.as_deref()), ("hvadmin", Some("EXAMPLE")));
        // Ohne gemeldete Adresse keine Konsole.
        assert!(app.guest_console(tid, "x", "vm-02", "", 1024, 768).contains("error"));
    }
}
