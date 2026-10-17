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
use crate::sessions::ConnectSpec;

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

    fn proxmox(&self, c: &Ctx, seen: Arc<Mutex<Option<String>>>) -> Result<rj_proxmox::Client, String> {
        let key = format!("{}:{}", c.host, c.port);
        let accepted = known().get(&key).cloned();
        let check: rj_proxmox::CertCheck = Arc::new(move |fp: &str| {
            *seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(fp.to_string());
            accepted.as_deref() == Some(fp)
        });
        let host = if c.host.contains(':') && !c.host.starts_with('[') { format!("[{}]", c.host) } else { c.host.clone() };
        rj_proxmox::Client::new(
            &format!("https://{host}:{}", c.port),
            proxmox_auth(&c.creds),
            check,
            Duration::from_secs(self.config.connection.connect_timeout_secs as u64),
        )
        .map_err(|e| e.to_string())
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
    /// `{"certificate":{…}}` oder `{"error":"…"}`.
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
        match c.protocol {
            Protocol::Proxmox => {
                let seen = Arc::new(Mutex::new(None));
                let client = match self.proxmox(&c, seen.clone()) {
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
                    Err(e) => Self::proxmox_error(&c, e, &seen),
                }
            }
            _ => match self.hyperv_session(&c).and_then(|mut s| rj_psrp::hyperv::list(&mut s).map_err(|e| e.to_string())) {
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

    fn hyperv_session(&self, c: &Ctx) -> Result<rj_psrp::PsSession, String> {
        let transport = c.transport.clone().unwrap_or(rj_core::PsrpTransport::Winrm { https: true, port: None, auth: rj_core::WinrmAuth::Ntlm, verify_certificate: true });
        let t = rj_psrp::PsTarget { host: c.host.clone(), username: c.creds.username.clone(), domain: c.creds.domain.clone(), transport };
        rj_psrp::PsSession::start(&t, &Secret::from_bytes(c.creds.password.0.clone())).map_err(|e| e.to_string())
    }

    /// Aktion `start`, `shutdown`, `stop` (hart) oder `reboot`.
    pub fn guest_action(&mut self, target: Uuid, guest: &str, action: &str) -> String {
        if self.demo {
            return json!({ "ok": true }).to_string();
        }
        let c = match self.hyper_ctx(target) {
            Ok(c) => c,
            Err(e) => return json!({ "error": e }).to_string(),
        };
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
                match self.proxmox(&c, seen.clone()).and_then(|cl| cl.action(&g, a).map_err(|e| Self::proxmox_error(&c, e, &seen))) {
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
                self.hyperv_session(&c).and_then(|mut s| rj_psrp::hyperv::action(&mut s, guest, a).map_err(|e| e.to_string()))
            }
        };
        match r {
            Ok(()) => json!({ "ok": true }).to_string(),
            Err(e) => json!({ "error": e }).to_string(),
        }
    }

    /// Konsole eines Gasts öffnen; `console_host` (Hyper-V) aus der Liste.
    pub fn guest_console(&mut self, target: Uuid, guest: &str, title: &str, console_host: &str, width: u32, height: u32) -> String {
        let c = match self.hyper_ctx(target) {
            Ok(c) => c,
            Err(e) => return json!({ "error": e }).to_string(),
        };
        let title = format!("{} – {}", c.name, title);
        let request = |host: String, port: u16, options: String| ConnectRequest {
            host,
            port,
            width: width.max(200),
            height: height.max(200),
            options_json: options,
            connect_timeout_secs: self.config.connection.connect_timeout_secs,
            keepalive_secs: self.config.connection.keepalive_secs,
        };
        let (spec, bridge) = match c.protocol {
            Protocol::Proxmox => {
                let Some(g) = guest_of(guest) else { return json!({ "error": "unbekannter Gast" }).to_string() };
                let seen = Arc::new(Mutex::new(None));
                let bridge = match self.proxmox(&c, seen.clone()).and_then(|cl| cl.console(&g).map_err(|e| Self::proxmox_error(&c, e, &seen))) {
                    Ok(b) => b,
                    Err(e) if e.starts_with('{') => return e,
                    Err(e) => return json!({ "error": e }).to_string(),
                };
                let creds = CredentialPayload { username: String::new(), domain: None, password: SecretBytes(bridge.password.as_bytes().to_vec()), private_key: None };
                let opts = json!({ "prefer_tls": false, "clipboard_limit": self.config.connection.clipboard_limit }).to_string();
                (ConnectSpec { target, title: title.clone(), protocol: Protocol::Vnc, request: request("127.0.0.1".into(), bridge.port, opts), credentials: creds }, Some(bridge))
            }
            _ => {
                if console_host.is_empty() {
                    return json!({ "error": "Der Gast meldet keine Adresse (Integrationsdienste prüfen)" }).to_string();
                }
                let opts = json!({ "clipboard_limit": self.config.connection.clipboard_limit }).to_string();
                (ConnectSpec { target, title: title.clone(), protocol: Protocol::Rdp, request: request(console_host.to_string(), 3389, opts), credentials: c.creds.clone() }, None)
            }
        };
        let s = self.sessions.open(spec);
        if let Some(b) = bridge {
            self.console_bridges.insert(s, b);
        }
        self.workspace.open(s, &title);
        json!({ "session": s }).to_string()
    }
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
}
