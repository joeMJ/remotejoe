//! Tunnel vor dem Verbinden: SSH (auch über mehrere Sprungserver),
//! WireGuard im Benutzerbereich und vorhandene Systemtunnel (nur
//! Erreichbarkeitsprüfung).
//!
//! Ein Tunnel wird beim ersten Bedarf aufgebaut und nach der letzten Sitzung
//! abgebaut (Zähler). Ist er gestört, wird er beim nächsten Bedarf oder über
//! [`TunnelManager::check`] neu aufgebaut.

pub mod ssh;
pub mod wireguard;

use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rj_core::inherit::resolve_tunnel;
use rj_core::{Index, TunnelDef};
use rj_ssh::client::{HostKeyAsk, SshParams};
use std::sync::atomic::{AtomicBool, Ordering};
use rj_ssh::known_hosts::KnownHosts;
use uuid::Uuid;

use crate::wireguard::{WgConfig, WgTunnel};

#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("SSH-Tunnel: {0}")]
    Ssh(#[from] rj_ssh::client::SshError),
    #[error("WireGuard: {0}")]
    Wg(#[from] wireguard::WgError),
    #[error("Tunnel: {0}")]
    Io(#[from] std::io::Error),
    #[error("Systemtunnel: {host}:{port} nicht erreichbar")]
    Unreachable { host: String, port: u16 },
    #[error("Tunnelaufbau abgebrochen")]
    Cancelled,
}

/// Wartet auf `fut`, bis es fertig ist oder `cancel` gesetzt wird (geprüft
/// alle 20 ms). Bei Abbruch wird `fut` verworfen und damit ein halb
/// aufgebauter Tunnel geschlossen.
async fn until_cancelled<F: std::future::Future>(fut: F, cancel: &AtomicBool) -> Result<F::Output, TunnelError> {
    tokio::select! {
        r = fut => Ok(r),
        _ = async {
            while !cancel.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        } => Err(TunnelError::Cancelled),
    }
}

/// Fertig aufgelöste Tunnelbeschreibung (mit Zugangsdaten).
#[derive(Clone)]
pub enum TunnelSpec {
    Ssh { hops: Vec<SshParams> },
    Wireguard { config: WgConfig },
    System { probe_host: String, probe_port: u16 },
}

/// Zustand für die Anzeige.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelState {
    Down,
    Up { users: usize },
    Broken(String),
}

/// Adresse, über die das Ziel erreicht wird.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

/// Tunnel, die vor einem Ziel liegen müssen (Vererbung über den Ordnerbaum).
pub fn plan(index: &Index, target: Uuid) -> Option<&TunnelDef> {
    let (id, _) = resolve_tunnel(index, target).ok()??;
    index.tunnels.iter().find(|t| t.id == id)
}

/// Erreichbarkeitsprüfung (TCP-Verbindungsaufbau).
pub fn reachable(host: &str, port: u16, timeout: Duration) -> bool {
    (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .is_some_and(|a| TcpStream::connect_timeout(&a, timeout).is_ok())
}

enum Active {
    Ssh { chain: Arc<ssh::Chain>, forwards: HashMap<(String, u16), (ssh::Forward, usize)> },
    Wg { tunnel: WgTunnel, forwards: HashMap<(String, u16), (u16, usize)> },
}

impl Active {
    fn users(&self) -> usize {
        match self {
            Active::Ssh { forwards, .. } => forwards.values().map(|f| f.1).sum(),
            Active::Wg { forwards, .. } => forwards.values().map(|f| f.1).sum(),
        }
    }

    fn broken(&self) -> bool {
        match self {
            Active::Ssh { chain, forwards } => !chain.is_alive() || forwards.values().any(|f| f.0.is_broken()),
            Active::Wg { tunnel, .. } => tunnel.state() == wireguard::WgState::Stopped,
        }
    }
}

/// Verwaltung aller Tunnel.
pub struct TunnelManager {
    rt: tokio::runtime::Handle,
    known: KnownHosts,
    ask: HostKeyAsk,
    active: HashMap<Uuid, Active>,
    timeout: Duration,
}

impl TunnelManager {
    pub fn new(rt: tokio::runtime::Handle, known: KnownHosts, ask: HostKeyAsk) -> Self {
        TunnelManager { rt, known, ask, active: HashMap::new(), timeout: Duration::from_secs(15) }
    }

    /// Tunnel für `host:port` nutzen; liefert die lokale Adresse (bei
    /// Systemtunneln das Ziel selbst).
    pub fn acquire(&mut self, id: Uuid, spec: &TunnelSpec, host: &str, port: u16) -> Result<Endpoint, TunnelError> {
        self.acquire_cancellable(id, spec, host, port, &AtomicBool::new(false))
    }

    /// Wie [`acquire`](Self::acquire); wird `cancel` gesetzt, endet das Warten
    /// auf das Netz sofort mit [`TunnelError::Cancelled`] und die
    /// Tunnelverwaltung ist wieder frei.
    pub fn acquire_cancellable(&mut self, id: Uuid, spec: &TunnelSpec, host: &str, port: u16, cancel: &AtomicBool) -> Result<Endpoint, TunnelError> {
        let key = (host.to_string(), port);
        if self.active.get(&id).is_some_and(Active::broken) {
            // Gestört: neu aufbauen; bestehende Zähler bleiben erhalten.
            let old = self.active.remove(&id).expect("vorhanden");
            let counts: Vec<((String, u16), usize)> = match &old {
                Active::Ssh { forwards, .. } => forwards.iter().map(|(k, v)| (k.clone(), v.1)).collect(),
                Active::Wg { forwards, .. } => forwards.iter().map(|(k, v)| (k.clone(), v.1)).collect(),
            };
            drop(old);
            for (k, n) in counts {
                for _ in 0..n {
                    self.acquire_inner(id, spec, &k.0, k.1, cancel)?;
                }
            }
        }
        self.acquire_inner(id, spec, &key.0, key.1, cancel)
    }

    fn acquire_inner(&mut self, id: Uuid, spec: &TunnelSpec, host: &str, port: u16, cancel: &AtomicBool) -> Result<Endpoint, TunnelError> {
        let key = (host.to_string(), port);
        match spec {
            TunnelSpec::System { probe_host, probe_port } => {
                let timeout = self.timeout;
                let probe = async move { tokio::time::timeout(timeout, tokio::net::TcpStream::connect((probe_host.as_str(), *probe_port))).await };
                if !matches!(self.rt.block_on(until_cancelled(probe, cancel))?, Ok(Ok(_))) {
                    return Err(TunnelError::Unreachable { host: probe_host.clone(), port: *probe_port });
                }
                Ok(Endpoint { host: host.into(), port })
            }
            TunnelSpec::Ssh { hops } => {
                if !self.active.contains_key(&id) {
                    let chain = self.rt.block_on(until_cancelled(ssh::Chain::connect(hops, &self.known, &self.ask), cancel))??;
                    self.active.insert(id, Active::Ssh { chain: Arc::new(chain), forwards: HashMap::new() });
                }
                let Some(Active::Ssh { chain, forwards }) = self.active.get_mut(&id) else { unreachable!() };
                if let Some(f) = forwards.get_mut(&key) {
                    f.1 += 1;
                    return Ok(Endpoint { host: "127.0.0.1".into(), port: f.0.local_port });
                }
                let fw = self.rt.block_on(until_cancelled(ssh::Forward::start(chain.clone(), key.0.clone(), port), cancel))??;
                let local = fw.local_port;
                forwards.insert(key, (fw, 1));
                Ok(Endpoint { host: "127.0.0.1".into(), port: local })
            }
            TunnelSpec::Wireguard { config } => {
                if !self.active.contains_key(&id) {
                    let t = WgTunnel::start(config.clone(), SocketAddr::from(([0, 0, 0, 0], 0)))?;
                    self.active.insert(id, Active::Wg { tunnel: t, forwards: HashMap::new() });
                }
                let Some(Active::Wg { tunnel, forwards }) = self.active.get_mut(&id) else { unreachable!() };
                if let Some(f) = forwards.get_mut(&key) {
                    f.1 += 1;
                    return Ok(Endpoint { host: "127.0.0.1".into(), port: f.0 });
                }
                let remote: SocketAddr = (host, port)
                    .to_socket_addrs()?
                    .next()
                    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "Ziel im Tunnel nicht auflösbar"))?;
                let local = tunnel.forward(remote)?;
                forwards.insert(key, (local, 1));
                Ok(Endpoint { host: "127.0.0.1".into(), port: local })
            }
        }
    }

    /// Nutzung beenden; nach der letzten Sitzung wird der Tunnel abgebaut.
    pub fn release(&mut self, id: Uuid, host: &str, port: u16) {
        let key = (host.to_string(), port);
        let empty = match self.active.get_mut(&id) {
            Some(Active::Ssh { forwards, .. }) => {
                if let Some(f) = forwards.get_mut(&key) {
                    f.1 = f.1.saturating_sub(1);
                    if f.1 == 0 {
                        forwards.remove(&key);
                    }
                }
                forwards.is_empty()
            }
            Some(Active::Wg { forwards, .. }) => {
                if let Some(f) = forwards.get_mut(&key) {
                    f.1 = f.1.saturating_sub(1);
                    if f.1 == 0 {
                        forwards.remove(&key);
                    }
                }
                forwards.is_empty()
            }
            None => false,
        };
        if empty {
            if let Some(Active::Ssh { chain, .. }) = self.active.remove(&id) {
                // Von der letzten Stufe zur ersten trennen.
                for c in chain.connections().iter().rev() {
                    self.rt.block_on(c.disconnect());
                }
            }
        }
    }

    pub fn state(&self, id: Uuid) -> TunnelState {
        match self.active.get(&id) {
            None => TunnelState::Down,
            Some(a) if a.broken() => TunnelState::Broken("Verbindung unterbrochen".into()),
            Some(a) => TunnelState::Up { users: a.users() },
        }
    }

    /// Gestörte Tunnel mit Nutzern neu aufbauen. Liefert die neuen
    /// Endpunkte je (Tunnel, Ziel), damit Sitzungen neu verbinden können.
    pub fn check(&mut self, specs: &HashMap<Uuid, TunnelSpec>) -> Vec<(Uuid, String, u16, Result<Endpoint, String>)> {
        self.check_cancellable(specs, &AtomicBool::new(false))
    }

    /// Wie [`check`](Self::check), mit Abbruchmarke.
    pub fn check_cancellable(&mut self, specs: &HashMap<Uuid, TunnelSpec>, cancel: &AtomicBool) -> Vec<(Uuid, String, u16, Result<Endpoint, String>)> {
        let broken: Vec<Uuid> = self.active.iter().filter(|(_, a)| a.broken()).map(|(id, _)| *id).collect();
        let mut out = Vec::new();
        for id in broken {
            let Some(spec) = specs.get(&id) else { continue };
            let old = self.active.remove(&id).expect("vorhanden");
            let counts: Vec<((String, u16), usize)> = match &old {
                Active::Ssh { forwards, .. } => forwards.iter().map(|(k, v)| (k.clone(), v.1)).collect(),
                Active::Wg { forwards, .. } => forwards.iter().map(|(k, v)| (k.clone(), v.1)).collect(),
            };
            drop(old);
            for ((h, p), n) in counts {
                let mut last = Err("nicht aufgebaut".to_string());
                for _ in 0..n {
                    last = self.acquire_inner(id, spec, &h, p, cancel).map_err(|e| e.to_string());
                }
                out.push((id, h, p, last));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rj_core::{Folder, Node, Protocol, Target, TunnelKind, TunnelSetting};

    #[test]
    fn plan_inherits_and_none_disables() {
        let mut idx = Index::default();
        let tid = Uuid::new_v4();
        idx.tunnels.push(TunnelDef { id: tid, name: "Sprung".into(), kind: TunnelKind::System { probe_host: "192.0.2.1".into(), probe_port: 22 } });
        let mut f = Folder::new("Netz hinter Sprungserver");
        f.settings.tunnel = Some(TunnelSetting::Use(tid));
        let a = Target::new("a", Protocol::Ssh, "a.example.test");
        let mut b = Target::new("b", Protocol::Ssh, "b.example.test");
        b.settings.tunnel = Some(TunnelSetting::None);
        let (aid, bid) = (a.id, b.id);
        f.children.push(Node::Target(a));
        f.children.push(Node::Target(b));
        idx.root.children.push(Node::Folder(f));
        assert_eq!(plan(&idx, aid).map(|t| t.id), Some(tid), "Ordner → Ziel vererbt");
        assert!(plan(&idx, bid).is_none(), "tunnel = none am Ziel");
    }

    #[test]
    fn system_tunnel_reachability() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(reachable("127.0.0.1", port, Duration::from_secs(2)));
        drop(l);
        assert!(!reachable("127.0.0.1", port, Duration::from_millis(500)));
    }
}
