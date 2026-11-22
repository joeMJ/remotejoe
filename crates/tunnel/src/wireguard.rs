//! WireGuard im Benutzerbereich: `boringtun` für das Protokoll, `smoltcp`
//! als TCP/IP-Stapel im Prozess. Keine Netzwerkschnittstelle, keine
//! Sonderrechte. TCP-Verbindungen zu Zielen im Tunnel werden über einen
//! lokalen Port auf 127.0.0.1 angeboten.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant as StdInstant};

use base64::Engine as _;
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{self, ChecksumCapabilities, Device, DeviceCapabilities, Medium};
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, Ipv4Address};

#[derive(Debug, thiserror::Error)]
pub enum WgError {
    #[error("WireGuard-Konfiguration: {0}")]
    Config(String),
    #[error("Netz: {0}")]
    Io(#[from] std::io::Error),
    #[error("WireGuard-Tunnel ist beendet")]
    Stopped,
}

/// Konfiguration im Format von `wg-quick` (Anhang eines KDBX-Eintrags).
#[derive(Clone)]
pub struct WgConfig {
    pub private_key: [u8; 32],
    pub address: Ipv4Addr,
    pub prefix: u8,
    pub peer_public: [u8; 32],
    pub preshared: Option<[u8; 32]>,
    pub endpoint: Option<SocketAddr>,
    pub keepalive: Option<u16>,
    pub listen_port: Option<u16>,
}

impl Drop for WgConfig {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.private_key);
        if let Some(p) = self.preshared.as_mut() {
            zeroize::Zeroize::zeroize(p);
        }
    }
}

impl std::fmt::Debug for WgConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WgConfig {{ address: {}/{}, endpoint: {:?} }}", self.address, self.prefix, self.endpoint)
    }
}

fn key(v: &str, what: &str) -> Result<[u8; 32], WgError> {
    let b = base64::engine::general_purpose::STANDARD.decode(v.trim()).map_err(|_| WgError::Config(format!("{what} ist kein Base64")))?;
    b.try_into().map_err(|_| WgError::Config(format!("{what} hat nicht 32 Bytes")))
}

impl WgConfig {
    /// Liest eine Konfiguration (`[Interface]`, `[Peer]`). Unterstützt wird
    /// genau ein Peer und eine IPv4-Adresse.
    pub fn parse(text: &str) -> Result<WgConfig, WgError> {
        let mut section = String::new();
        let (mut pk, mut addr, mut peer, mut psk, mut ep, mut ka, mut lp) = (None, None, None, None, None, None, None);
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                section = line.to_ascii_lowercase();
                continue;
            }
            let (k, v) = line.split_once('=').ok_or_else(|| WgError::Config(format!("Zeile {}: kein Schlüssel=Wert", n + 1)))?;
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            match (section.as_str(), k.as_str()) {
                ("[interface]", "privatekey") => pk = Some(key(v, "PrivateKey")?),
                ("[interface]", "address") => {
                    let first = v.split(',').next().unwrap_or("").trim();
                    let (ip, pre) = first.split_once('/').unwrap_or((first, "32"));
                    let ip: Ipv4Addr = ip.parse().map_err(|_| WgError::Config(format!("Adresse {first} ist keine IPv4-Adresse")))?;
                    addr = Some((ip, pre.parse::<u8>().map_err(|_| WgError::Config("Präfix ungültig".into()))?));
                }
                ("[interface]", "listenport") => lp = v.parse().ok(),
                ("[peer]", "publickey") => peer = Some(key(v, "PublicKey")?),
                ("[peer]", "presharedkey") => psk = Some(key(v, "PresharedKey")?),
                ("[peer]", "endpoint") => {
                    use std::net::ToSocketAddrs;
                    ep = Some(v.to_socket_addrs().map_err(|e| WgError::Config(format!("Endpoint {v}: {e}")))?.next().ok_or_else(|| WgError::Config(format!("Endpoint {v} nicht auflösbar")))?);
                }
                ("[peer]", "persistentkeepalive") => ka = v.parse().ok(),
                _ => {}
            }
        }
        let (address, prefix) = addr.ok_or_else(|| WgError::Config("Address fehlt".into()))?;
        Ok(WgConfig {
            private_key: pk.ok_or_else(|| WgError::Config("PrivateKey fehlt".into()))?,
            address,
            prefix,
            peer_public: peer.ok_or_else(|| WgError::Config("PublicKey des Peers fehlt".into()))?,
            preshared: psk,
            endpoint: ep,
            keepalive: ka,
            listen_port: lp,
        })
    }
}

/// Öffentlicher Schlüssel zu einem privaten (Base64).
pub fn public_key_b64(private: &[u8; 32]) -> String {
    let p = PublicKey::from(&StaticSecret::from(*private));
    base64::engine::general_purpose::STANDARD.encode(p.as_bytes())
}

#[derive(Default)]
struct QueueDevice {
    rx: VecDeque<Vec<u8>>,
    tx: VecDeque<Vec<u8>>,
}

struct RxTok(Vec<u8>);
struct TxTok<'a>(&'a mut VecDeque<Vec<u8>>);

impl phy::RxToken for RxTok {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}

impl phy::TxToken for TxTok<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut b = vec![0; len];
        let r = f(&mut b);
        self.0.push_back(b);
        r
    }
}

impl Device for QueueDevice {
    type RxToken<'a> = RxTok;
    type TxToken<'a> = TxTok<'a>;

    fn receive(&mut self, _t: Instant) -> Option<(RxTok, TxTok<'_>)> {
        let p = self.rx.pop_front()?;
        Some((RxTok(p), TxTok(&mut self.tx)))
    }

    fn transmit(&mut self, _t: Instant) -> Option<TxTok<'_>> {
        Some(TxTok(&mut self.tx))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut c = DeviceCapabilities::default();
        c.medium = Medium::Ip;
        c.max_transmission_unit = 1420;
        c.checksum = ChecksumCapabilities::default();
        c
    }
}

enum Cmd {
    Forward { stream: TcpStream, remote: SocketAddr },
    Echo { port: u16 },
}

struct Bridge {
    stream: TcpStream,
    handle: SocketHandle,
    closing: bool,
}

/// Zustand des Tunnels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WgState {
    Starting,
    Up,
    Stopped,
}

/// Laufender WireGuard-Tunnel.
pub struct WgTunnel {
    cmds: Sender<Cmd>,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<WgState>>,
    udp_port: u16,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WgTunnel {
    /// Startet den Tunnel. `bind` ist die lokale UDP-Adresse (Port 0 = frei).
    pub fn start(cfg: WgConfig, bind: SocketAddr) -> Result<WgTunnel, WgError> {
        let bind = match cfg.listen_port {
            Some(p) if bind.port() == 0 => SocketAddr::new(bind.ip(), p),
            _ => bind,
        };
        let udp = UdpSocket::bind(bind)?;
        udp.set_nonblocking(true)?;
        let udp_port = udp.local_addr()?.port();
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(WgState::Starting));
        let tunn = Tunn::new(StaticSecret::from(cfg.private_key), PublicKey::from(cfg.peer_public), cfg.preshared, cfg.keepalive, rand_index(), None);
        let (s2, st2) = (stop.clone(), state.clone());
        let thread = std::thread::Builder::new().name("wireguard".into()).spawn(move || {
            WgEngine::new(cfg, udp, tunn, rx, s2, st2.clone()).run();
            *st2.lock().unwrap_or_else(|e| e.into_inner()) = WgState::Stopped;
        })?;
        Ok(WgTunnel { cmds: tx, stop, state, udp_port, thread: Some(thread) })
    }

    pub fn udp_port(&self) -> u16 {
        self.udp_port
    }

    pub fn state(&self) -> WgState {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lokaler Port auf 127.0.0.1, der zu `remote` im Tunnel weiterleitet.
    pub fn forward(&self, remote: SocketAddr) -> Result<u16, WgError> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let cmds = self.cmds.clone();
        let stop = self.stop.clone();
        std::thread::Builder::new().name("wg-annahme".into()).spawn(move || {
            for s in listener.incoming() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(s) = s else { continue };
                if cmds.send(Cmd::Forward { stream: s, remote }).is_err() {
                    break;
                }
            }
        })?;
        Ok(port)
    }

    /// Nur für Tests: TCP-Echo-Dienst im Tunnel auf `port`.
    pub fn serve_echo(&self, port: u16) -> Result<(), WgError> {
        self.cmds.send(Cmd::Echo { port }).map_err(|_| WgError::Stopped)
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WgTunnel {
    fn drop(&mut self) {
        self.stop();
    }
}

fn rand_index() -> u32 {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    u32::from_le_bytes(b) >> 8
}

struct WgEngine {
    udp: UdpSocket,
    tunn: Tunn,
    peer: Option<SocketAddr>,
    iface: Interface,
    dev: QueueDevice,
    sockets: SocketSet<'static>,
    cmds: Receiver<Cmd>,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<WgState>>,
    bridges: Vec<Bridge>,
    echoes: Vec<(u16, SocketHandle)>,
    next_port: u16,
    start: StdInstant,
}

impl WgEngine {
    fn new(cfg: WgConfig, udp: UdpSocket, tunn: Tunn, cmds: Receiver<Cmd>, stop: Arc<AtomicBool>, state: Arc<Mutex<WgState>>) -> Self {
        let mut dev = QueueDevice::default();
        let start = StdInstant::now();
        let mut iface = Interface::new(Config::new(HardwareAddress::Ip), &mut dev, Instant::from_millis(0));
        let a = cfg.address.octets();
        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::new(IpAddress::v4(a[0], a[1], a[2], a[3]), cfg.prefix));
        });
        // Alles, was nicht im eigenen Netz liegt, geht in den Tunnel.
        let gw = Ipv4Address::new(a[0], a[1], a[2], if a[3] == 254 { 253 } else { 254 });
        let _ = iface.routes_mut().add_default_ipv4_route(gw);
        WgEngine {
            udp,
            tunn,
            peer: cfg.endpoint,
            iface,
            dev,
            sockets: SocketSet::new(Vec::new()),
            cmds,
            stop,
            state,
            bridges: Vec::new(),
            echoes: Vec::new(),
            next_port: 40000,
            start,
        }
    }

    fn now(&self) -> Instant {
        Instant::from_millis(self.start.elapsed().as_millis() as i64)
    }

    fn send_udp(&self, data: &[u8]) {
        if let Some(p) = self.peer {
            let _ = self.udp.send_to(data, p);
        }
    }

    fn tcp_socket() -> tcp::Socket<'static> {
        tcp::Socket::new(tcp::SocketBuffer::new(vec![0; 256 * 1024]), tcp::SocketBuffer::new(vec![0; 256 * 1024]))
    }

    fn run(mut self) {
        let mut buf = vec![0u8; 65536];
        let mut out = vec![0u8; 65536];
        let mut last_timer = StdInstant::now();
        while !self.stop.load(Ordering::Relaxed) {
            let mut busy = false;
            // Kommandos.
            while let Ok(c) = self.cmds.try_recv() {
                match c {
                    Cmd::Forward { stream, remote } => {
                        let _ = stream.set_nonblocking(true);
                        let mut s = Self::tcp_socket();
                        self.next_port = self.next_port.wrapping_add(1).max(40000);
                        let ip = match remote.ip() {
                            IpAddr::V4(v) => IpAddress::Ipv4(v),
                            IpAddr::V6(v) => IpAddress::Ipv6(v),
                        };
                        if s.connect(self.iface.context(), (ip, remote.port()), self.next_port).is_ok() {
                            let handle = self.sockets.add(s);
                            self.bridges.push(Bridge { stream, handle, closing: false });
                        }
                    }
                    Cmd::Echo { port } => {
                        // Mehrere lauschende Sockets, damit Folgeverbindungen nicht warten.
                        for _ in 0..4 {
                            let mut s = Self::tcp_socket();
                            let _ = s.listen(port);
                            let h = self.sockets.add(s);
                            self.echoes.push((port, h));
                        }
                    }
                }
            }
            // Eingehende UDP-Pakete.
            loop {
                match self.udp.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        busy = true;
                        let mut first = true;
                        loop {
                            let input: &[u8] = if first { &buf[..n] } else { &[] };
                            first = false;
                            match self.tunn.decapsulate(Some(src.ip()), input, &mut out) {
                                TunnResult::WriteToNetwork(p) => {
                                    let _ = self.udp.send_to(p, src);
                                    self.peer = Some(src);
                                    continue;
                                }
                                TunnResult::WriteToTunnelV4(p, _) | TunnResult::WriteToTunnelV6(p, _) => {
                                    self.peer = Some(src);
                                    self.dev.rx.push_back(p.to_vec());
                                    *self.state.lock().unwrap_or_else(|e| e.into_inner()) = WgState::Up;
                                }
                                TunnResult::Done => {
                                    // Handshake abgeschlossen (z. B. Keepalive) zählt als erreichbar.
                                    if self.tunn.time_since_last_handshake().is_some() {
                                        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = WgState::Up;
                                        self.peer = Some(src);
                                    }
                                }
                                TunnResult::Err(_) => {}
                            }
                            break;
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
            let now = self.now();
            self.iface.poll(now, &mut self.dev, &mut self.sockets);
            // Ausgehende IP-Pakete verschlüsseln.
            while let Some(p) = self.dev.tx.pop_front() {
                busy = true;
                if let TunnResult::WriteToNetwork(d) = self.tunn.encapsulate(&p, &mut out) {
                    let d = d.to_vec();
                    self.send_udp(&d);
                }
            }
            if last_timer.elapsed() >= Duration::from_millis(100) {
                last_timer = StdInstant::now();
                if let TunnResult::WriteToNetwork(d) = self.tunn.update_timers(&mut out) {
                    let d = d.to_vec();
                    self.send_udp(&d);
                }
            }
            busy |= self.pump_bridges(&mut buf);
            self.pump_echoes(&mut buf);
            if !busy {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    fn pump_bridges(&mut self, buf: &mut [u8]) -> bool {
        let mut busy = false;
        let mut i = 0;
        while i < self.bridges.len() {
            let b = &mut self.bridges[i];
            let s = self.sockets.get_mut::<tcp::Socket>(b.handle);
            if !b.closing && s.can_send() {
                let room = s.send_capacity() - s.send_queue();
                if room > 0 {
                    let want = room.min(buf.len());
                    match b.stream.read(&mut buf[..want]) {
                        Ok(0) => {
                            s.close();
                            b.closing = true;
                        }
                        Ok(n) => {
                            let _ = s.send_slice(&buf[..n]);
                            busy = true;
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                        Err(_) => {
                            s.abort();
                            b.closing = true;
                        }
                    }
                }
            }
            while s.can_recv() {
                match s.recv_slice(buf) {
                    Ok(n) if n > 0 => {
                        busy = true;
                        let _ = b.stream.set_nonblocking(false);
                        let ok = b.stream.write_all(&buf[..n]).is_ok();
                        let _ = b.stream.set_nonblocking(true);
                        if !ok {
                            s.abort();
                        }
                    }
                    _ => break,
                }
            }
            let finished = matches!(s.state(), tcp::State::Closed | tcp::State::TimeWait) || (!s.may_recv() && !s.may_send() && b.closing);
            if finished || (!s.is_active() && !matches!(s.state(), tcp::State::SynSent)) {
                let _ = b.stream.shutdown(std::net::Shutdown::Both);
                let h = b.handle;
                self.bridges.remove(i);
                self.sockets.remove(h);
                continue;
            }
            i += 1;
        }
        busy
    }

    fn pump_echoes(&mut self, buf: &mut [u8]) {
        for (port, h) in &mut self.echoes {
            let s = self.sockets.get_mut::<tcp::Socket>(*h);
            if s.can_recv() && s.can_send() {
                let n = s.recv_slice(buf).unwrap_or(0);
                let _ = s.send_slice(&buf[..n]);
            }
            if matches!(s.state(), tcp::State::CloseWait) {
                s.close();
            }
            if matches!(s.state(), tcp::State::Closed | tcp::State::TimeWait) {
                s.abort();
                let _ = s.listen(*port);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wg_quick() {
        let k = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        let text = format!(
            "# Kommentar\n[Interface]\nPrivateKey = {k}\nAddress = 192.0.2.10/24, fd00::1/64\nListenPort = 51820\n\n[Peer]\nPublicKey={k}\nEndpoint = 127.0.0.1:51821\nAllowedIPs = 0.0.0.0/0\nPersistentKeepalive = 25\n"
        );
        let c = WgConfig::parse(&text).unwrap();
        assert_eq!(c.address, Ipv4Addr::new(192, 0, 2, 10));
        assert_eq!(c.prefix, 24);
        assert_eq!(c.keepalive, Some(25));
        assert_eq!(c.listen_port, Some(51820));
        assert_eq!(c.endpoint.unwrap().port(), 51821);
        assert!(format!("{c:?}").find(&k).is_none(), "Schlüssel nie in der Ausgabe");
        assert!(WgConfig::parse("[Interface]\nAddress = 192.0.2.1/24\n").is_err());
        assert!(WgConfig::parse(&format!("[Interface]\nPrivateKey = abc\nAddress = 192.0.2.1\n[Peer]\nPublicKey = {k}\n")).is_err());
    }
}
