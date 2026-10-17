//! Tunnel gegen echte sshd-Instanzen (Testkit) und eine WireGuard-Schleife
//! zwischen zwei Instanzen im Benutzerbereich.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use rj_core::Secret;
use rj_ssh::client::{Auth, HostKeyAsk, SshConnection, SshParams};
use rj_ssh::known_hosts::KnownHosts;
use rj_testkit::sshd::{Sshd, PASSWORD, USER};
use rj_tunnel::wireguard::{public_key_b64, WgConfig, WgTunnel};
use rj_tunnel::*;
use uuid::Uuid;

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn params(port: u16) -> SshParams {
    SshParams {
        host: "127.0.0.1".into(),
        port,
        user: USER.into(),
        auth: vec![Auth::Password(Secret::from_str_exact(PASSWORD))],
        connect_timeout: Duration::from_secs(15),
        keepalive: Duration::from_secs(30),
        legacy: false,
    }
}

fn accept_all(rt: &tokio::runtime::Runtime) -> HostKeyAsk {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(rj_ssh::known_hosts::HostKeyQuestion, tokio::sync::oneshot::Sender<bool>)>();
    rt.spawn(async move {
        while let Some((_, r)) = rx.recv().await {
            let _ = r.send(true);
        }
    });
    tx
}

/// Anmeldung am Ziel über den lokalen Endpunkt, Befehl ausführen.
fn whoami_via(rt: &tokio::runtime::Runtime, ep: &Endpoint, known: &KnownHosts) -> String {
    let c = rt.block_on(SshConnection::connect(&params(ep.port), known.clone(), accept_all(rt))).expect("Anmeldung über den Tunnel");
    let (out, _, code) = rt.block_on(c.exec("id -un")).unwrap();
    assert_eq!(code, Some(0));
    String::from_utf8(out).unwrap().trim().to_string()
}

#[test]
fn ssh_jump_hosts_refcount_and_rebuild() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (a, b) = match (Sshd::start(), Sshd::start()) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("ÜBERSPRUNGEN: {e}");
            return;
        }
    };
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let known = KnownHosts::new(dir.path().join("known_hosts"));
    let mut m = TunnelManager::new(rt.handle().clone(), known.clone(), accept_all(&rt));

    // Eine Stufe: über A zu B.
    let one = Uuid::new_v4();
    let spec1 = TunnelSpec::Ssh { hops: vec![params(a.port)] };
    let ep = m.acquire(one, &spec1, "127.0.0.1", b.port).unwrap();
    assert_eq!(ep.host, "127.0.0.1");
    assert_ne!(ep.port, b.port, "lokaler Hochport");
    assert_eq!(whoami_via(&rt, &ep, &known), USER);

    // Zähler: zweite Nutzung, gleicher Port.
    let ep2 = m.acquire(one, &spec1, "127.0.0.1", b.port).unwrap();
    assert_eq!(ep, ep2);
    assert_eq!(m.state(one), TunnelState::Up { users: 2 });
    m.release(one, "127.0.0.1", b.port);
    assert_eq!(m.state(one), TunnelState::Up { users: 1 });
    m.release(one, "127.0.0.1", b.port);
    assert_eq!(m.state(one), TunnelState::Down, "nach der letzten Sitzung abgebaut");
    std::thread::sleep(Duration::from_millis(200));
    assert!(TcpStream::connect(("127.0.0.1", ep.port)).is_err(), "lokaler Port geschlossen");

    // Zwei Stufen: A → B → (über B) wieder A.
    let two = Uuid::new_v4();
    let spec2 = TunnelSpec::Ssh { hops: vec![params(a.port), params(b.port)] };
    let ep = m.acquire(two, &spec2, "127.0.0.1", a.port).unwrap();
    assert_eq!(whoami_via(&rt, &ep, &known), USER, "mehrstufiger Tunnel");

    // Wiederaufbau nach Abbruch.
    a.drop_sessions();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !matches!(m.state(two), TunnelState::Broken(_)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(matches!(m.state(two), TunnelState::Broken(_)), "Abbruch erkannt: {:?}", m.state(two));
    let specs = std::collections::HashMap::from([(two, spec2.clone())]);
    let rebuilt = m.check(&specs);
    assert_eq!(rebuilt.len(), 1);
    let ep = rebuilt[0].3.clone().expect("neu aufgebaut");
    assert_eq!(whoami_via(&rt, &ep, &known), USER, "nach Wiederaufbau");
    assert_eq!(m.state(two), TunnelState::Up { users: 1 });
}

fn keypair() -> ([u8; 32], String) {
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).unwrap();
    let p = public_key_b64(&k);
    (k, p)
}

fn wg_pair(wrong_key: bool) -> (WgTunnel, WgTunnel) {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let (ka, pa) = keypair();
    let (kb, pb) = keypair();
    let (_, other) = keypair();
    let server = WgConfig::parse(&format!(
        "[Interface]\nPrivateKey = {}\nAddress = 192.0.2.2/24\n[Peer]\nPublicKey = {pa}\n",
        b64.encode(kb)
    ))
    .unwrap();
    let srv = WgTunnel::start(server, SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
    srv.serve_echo(7).unwrap();
    let client = WgConfig::parse(&format!(
        "[Interface]\nPrivateKey = {}\nAddress = 192.0.2.1/24\n[Peer]\nPublicKey = {}\nEndpoint = 127.0.0.1:{}\nPersistentKeepalive = 1\n",
        b64.encode(ka),
        if wrong_key { other } else { pb },
        srv.udp_port()
    ))
    .unwrap();
    let cli = WgTunnel::start(client, SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
    (srv, cli)
}

fn echo_roundtrip(port: u16, data: &[u8], timeout: Duration) -> Option<Vec<u8>> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(timeout)).ok()?;
    s.write_all(data).ok()?;
    let mut got = vec![0u8; data.len()];
    let mut n = 0;
    while n < got.len() {
        match s.read(&mut got[n..]) {
            Ok(0) | Err(_) => return None,
            Ok(k) => n += k,
        }
    }
    Some(got)
}

#[test]
fn wireguard_loop_between_two_userspace_instances() {
    let (_srv, cli) = wg_pair(false);
    let port = cli.forward("192.0.2.2:7".parse().unwrap()).unwrap();
    let msg = "Grüße durch den Tunnel 🚀\n".repeat(4000);
    let got = echo_roundtrip(port, msg.as_bytes(), Duration::from_secs(15)).expect("Echo durch WireGuard");
    assert_eq!(got, msg.as_bytes(), "{} Bytes unverändert zurück", msg.len());
    assert_eq!(cli.state(), rj_tunnel::wireguard::WgState::Up);
    // Zweite Verbindung über denselben Tunnel.
    assert_eq!(echo_roundtrip(port, b"zweite", Duration::from_secs(5)).as_deref(), Some(&b"zweite"[..]));
}

#[test]
fn wireguard_wrong_key_gets_no_connection() {
    let (_srv, cli) = wg_pair(true);
    let port = cli.forward("192.0.2.2:7".parse().unwrap()).unwrap();
    assert!(echo_roundtrip(port, b"hallo", Duration::from_secs(3)).is_none());
    assert_ne!(cli.state(), rj_tunnel::wireguard::WgState::Up);
}

#[test]
fn manager_with_wireguard_and_system_tunnel() {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut m = TunnelManager::new(rt.handle().clone(), KnownHosts::new(dir.path().join("k")), accept_all(&rt));
    // WireGuard über den Manager (Konfiguration wie aus einem KDBX-Anhang).
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let (ka, pa) = keypair();
    let (kb, pb) = keypair();
    let srv = WgTunnel::start(
        WgConfig::parse(&format!("[Interface]\nPrivateKey = {}\nAddress = 192.0.2.2/24\n[Peer]\nPublicKey = {pa}\n", b64.encode(kb))).unwrap(),
        SocketAddr::from(([127, 0, 0, 1], 0)),
    )
    .unwrap();
    srv.serve_echo(22).unwrap();
    let cfg = WgConfig::parse(&format!(
        "[Interface]\nPrivateKey = {}\nAddress = 192.0.2.1/24\n[Peer]\nPublicKey = {pb}\nEndpoint = 127.0.0.1:{}\n",
        b64.encode(ka),
        srv.udp_port()
    ))
    .unwrap();
    let id = Uuid::new_v4();
    let ep = m.acquire(id, &TunnelSpec::Wireguard { config: cfg }, "192.0.2.2", 22).unwrap();
    assert_eq!(echo_roundtrip(ep.port, "ä🚀".as_bytes(), Duration::from_secs(10)).as_deref(), Some("ä🚀".as_bytes()));
    m.release(id, "192.0.2.2", 22);
    assert_eq!(m.state(id), TunnelState::Down);

    // Systemtunnel: nur Erreichbarkeit.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let p = l.local_addr().unwrap().port();
    let sys = TunnelSpec::System { probe_host: "127.0.0.1".into(), probe_port: p };
    let ep = m.acquire(Uuid::new_v4(), &sys, "ziel.example.test", 3389).unwrap();
    assert_eq!(ep, Endpoint { host: "ziel.example.test".into(), port: 3389 });
    drop(l);
    assert!(matches!(m.acquire(Uuid::new_v4(), &sys, "ziel.example.test", 3389), Err(TunnelError::Unreachable { .. })));
}

#[test]
fn cancelled_acquire_frees_the_manager_at_once() {
    // Langsame Gegenstelle: nimmt Verbindungen an, antwortet nie.
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let silent_port = silent.local_addr().unwrap().port();
    let _keep = std::thread::spawn(move || {
        let mut held = Vec::new();
        for c in silent.incoming().flatten() {
            held.push(c);
        }
    });
    let open = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let open_port = open.local_addr().unwrap().port();
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let m = std::sync::Arc::new(std::sync::Mutex::new(TunnelManager::new(rt.handle().clone(), KnownHosts::new(dir.path().join("k")), accept_all(&rt))));
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (m2, c2) = (m.clone(), cancel.clone());
    let slow = TunnelSpec::Ssh { hops: vec![params(silent_port)] };
    let t = std::thread::spawn(move || {
        let start = Instant::now();
        let r = m2.lock().unwrap().acquire_cancellable(Uuid::new_v4(), &slow, "192.0.2.10", 22, &c2);
        (r.map(|_| ()).map_err(|e| e.to_string()), start.elapsed())
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(!t.is_finished(), "Aufbau wartet auf die Gegenstelle");
    let cancelled_at = Instant::now();
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    // Sofort ein neuer Aufbau: wartet nicht auf das Zeitlimit von 15 s.
    let ep = m
        .lock()
        .unwrap()
        .acquire(Uuid::new_v4(), &TunnelSpec::System { probe_host: "127.0.0.1".into(), probe_port: open_port }, "192.0.2.11", 3389)
        .unwrap();
    let waited = cancelled_at.elapsed();
    assert_eq!(ep.port, 3389);
    let (r, total) = t.join().unwrap();
    assert_eq!(r, Err("Tunnelaufbau abgebrochen".to_string()));
    assert!(waited < Duration::from_secs(1), "neuer Aufbau wartete {waited:?}");
    assert!(total < Duration::from_secs(2), "Abbruch nach {total:?}");
    eprintln!("Abbruch wirksam nach {waited:?}");
}
