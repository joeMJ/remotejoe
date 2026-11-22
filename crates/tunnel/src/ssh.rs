//! SSH-Tunnel: Verbindung über einen oder mehrere Sprungserver und lokale
//! Weiterleitung auf einen freien Hochport an 127.0.0.1.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rj_ssh::client::{HostKeyAsk, SshConnection, SshError, SshParams};
use rj_ssh::known_hosts::KnownHosts;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Kette angemeldeter Verbindungen (erste Stufe zuerst).
pub struct Chain {
    conns: Vec<Arc<SshConnection>>,
}

impl Chain {
    /// Baut die Kette auf: Stufe 1 direkt, jede weitere über die vorige.
    pub async fn connect(hops: &[SshParams], known: &KnownHosts, ask: &HostKeyAsk) -> Result<Chain, SshError> {
        let first = hops.first().ok_or_else(|| SshError::Connect("Tunnel".into(), "keine Stufe angegeben".into()))?;
        let mut conns = vec![Arc::new(SshConnection::connect(first, known.clone(), ask.clone()).await?)];
        for hop in &hops[1..] {
            let prev = conns.last().expect("mindestens eine Stufe");
            let stream = prev.direct_tcpip(&hop.host, hop.port).await?;
            conns.push(Arc::new(SshConnection::connect_stream(hop, stream, known.clone(), ask.clone()).await?));
        }
        Ok(Chain { conns })
    }

    pub fn last(&self) -> &Arc<SshConnection> {
        self.conns.last().expect("mindestens eine Stufe")
    }

    /// Lebt jede Stufe noch?
    pub fn is_alive(&self) -> bool {
        self.conns.iter().all(|c| !c.is_closed())
    }

    pub fn stages(&self) -> usize {
        self.conns.len()
    }

    pub fn connections(&self) -> &[Arc<SshConnection>] {
        &self.conns
    }
}

/// Lokale Weiterleitung zu `host:port` hinter der letzten Stufe.
pub struct Forward {
    pub local_port: u16,
    task: JoinHandle<()>,
    pub chain: Arc<Chain>,
    broken: Arc<AtomicBool>,
}

impl Forward {
    pub async fn start(chain: Arc<Chain>, host: String, port: u16) -> std::io::Result<Forward> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let local_port = listener.local_addr()?.port();
        let c = chain.clone();
        let broken = Arc::new(AtomicBool::new(false));
        let b2 = broken.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut tcp, _)) = listener.accept().await else { break };
                tcp.set_nodelay(true).ok();
                let last = c.last().clone();
                let (h, b3) = (host.clone(), b2.clone());
                tokio::spawn(async move {
                    match last.direct_tcpip(&h, port).await {
                        Ok(mut ch) => {
                            let _ = tokio::io::copy_bidirectional(&mut tcp, &mut ch).await;
                        }
                        Err(_) => b3.store(true, Ordering::Relaxed),
                    }
                });
            }
        });
        Ok(Forward { local_port, task, chain, broken })
    }

    /// Weiterleitung gestört (Kette getrennt oder Kanal verweigert)?
    pub fn is_broken(&self) -> bool {
        self.broken.load(Ordering::Relaxed) || !self.chain.is_alive()
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.task.abort();
    }
}
