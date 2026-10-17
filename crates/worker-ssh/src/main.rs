//! SSH-Worker: ein Prozess je Sitzung. Kanal zum Hauptprozess über die
//! Standardkanäle (siehe `rj_ipc`).

use std::collections::HashMap;
use std::time::Duration;

use rj_core::Secret;
use rj_ipc::worker::WorkerLink;
use rj_ipc::{ConnectRequest, CredentialPayload, FromWorker, InputEvent, SessionState, ToWorker};
use rj_ssh::client::{Auth, Shell, ShellEvent, SshConnection, SshParams};
use rj_ssh::known_hosts::KnownHosts;
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};

/// Optionen aus dem Index (JSON).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Options {
    term: Option<String>,
    known_hosts: Option<String>,
    /// Ziel hinter einem Tunnel: Host-Schlüssel wird unter diesem Namen (`host:port`) geprüft.
    host_key_alias: Option<String>,
}

fn main() {
    let mut link = match WorkerLink::stdio("ssh", Duration::from_secs(2)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Kanal nicht verfügbar: {e}");
            std::process::exit(2);
        }
    };
    let sender = link.sender.clone();
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("Laufzeit");
    let (tx, rx) = mpsc::unbounded_channel::<ToWorker>();
    // Lesefaden: Nachrichten des Hauptprozesses in die Laufzeit geben.
    std::thread::spawn(move || loop {
        match link.recv() {
            Ok(m) => {
                if tx.send(m).is_err() {
                    break;
                }
            }
            Err(_) => {
                let _ = tx.send(ToWorker::Shutdown);
                break;
            }
        }
    });
    rt.block_on(run(rx, sender));
}

async fn run(mut rx: mpsc::UnboundedReceiver<ToWorker>, out: rj_ipc::worker::Sender) {
    let send = |m: FromWorker| {
        let _ = out.send(&m);
    };
    let mut request: Option<ConnectRequest> = None;
    let mut creds: Option<CredentialPayload> = None;
    let mut conn: Option<SshConnection> = None;
    let mut shell: Option<Shell> = None;
    let (ask_tx, mut ask_rx) = mpsc::unbounded_channel();
    let mut pending: HashMap<u32, oneshot::Sender<bool>> = HashMap::new();
    let mut next_q = 1u32;
    let mut connecting: Option<tokio::task::JoinHandle<Result<(SshConnection, Shell), String>>> = None;

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    ToWorker::Connect(r) => {
                        request = Some(r);
                        shell = None;
                        conn = None;
                    }
                    ToWorker::Credentials(c) => creds = Some(c),
                    ToWorker::Answer { question, accept } => {
                        if let Some(tx) = pending.remove(&question) {
                            let _ = tx.send(accept);
                        }
                    }
                    ToWorker::Input(InputEvent::Bytes(b)) => {
                        if let Some(s) = &shell { let _ = s.write(&b).await; }
                    }
                    ToWorker::Input(InputEvent::Text(t)) => {
                        if let Some(s) = &shell { let _ = s.write(t.as_bytes()).await; }
                    }
                    ToWorker::Input(_) => {}
                    ToWorker::Resize { width, height } => {
                        if let Some(s) = &shell { let _ = s.resize(width, height).await; }
                    }
                    ToWorker::Ping => send(FromWorker::Pong),
                    ToWorker::Shutdown => {
                        if let Some(s) = &shell { s.close().await; }
                        if let Some(c) = &conn { c.disconnect().await; }
                        break;
                    }
                    ToWorker::Clipboard(_) | ToWorker::Custom(_) => {}
                }
                // Verbindungsaufbau starten, sobald Auftrag und Zugangsdaten da sind.
                if connecting.is_none() && shell.is_none() && request.is_some() && creds.is_some() {
                    if let (Some(r), Some(c)) = (request.take(), creds.take()) {
                        send(FromWorker::State(SessionState::Connecting));
                        let ask = ask_tx.clone();
                        connecting = Some(tokio::spawn(async move { establish(r, c, ask).await }));
                    }
                }
            }
            q = ask_rx.recv() => {
                if let Some((q, reply)) = q {
                    let id = next_q;
                    next_q += 1;
                    pending.insert(id, reply);
                    send(FromWorker::Question { id, text: q.text(), detail: q.detail(), info: q.info() });
                }
            }
            res = async { connecting.as_mut().unwrap().await }, if connecting.is_some() => {
                connecting = None;
                match res {
                    Ok(Ok((c, s))) => {
                        conn = Some(c);
                        shell = Some(s);
                        send(FromWorker::State(SessionState::Connected));
                    }
                    Ok(Err(e)) => send(FromWorker::State(SessionState::Failed { reason: e })),
                    Err(e) => send(FromWorker::State(SessionState::Failed { reason: e.to_string() })),
                }
            }
            ev = async { shell.as_mut().unwrap().events.recv().await }, if shell.is_some() => {
                match ev {
                    Some(ShellEvent::Data(d)) => send(FromWorker::Output(d)),
                    Some(ShellEvent::Exit(code)) => send(FromWorker::Log(format!("Shell beendet mit Code {code}"))),
                    Some(ShellEvent::Closed) | None => {
                        shell = None;
                        conn = None;
                        send(FromWorker::State(SessionState::Disconnected { reason: "Verbindung beendet".into() }));
                    }
                }
            }
        }
    }
}

async fn establish(
    r: ConnectRequest,
    c: CredentialPayload,
    ask: rj_ssh::client::HostKeyAsk,
) -> Result<(SshConnection, Shell), String> {
    let opts: Options = serde_json::from_str(&r.options_json).unwrap_or_default();
    let known = opts
        .known_hosts
        .map(KnownHosts::new)
        .or_else(KnownHosts::default_location)
        .ok_or("Kein Ort für bekannte Host-Schlüssel")?;
    let mut auth = Vec::new();
    if let Some(k) = &c.private_key {
        auth.push(Auth::Key { pem: Secret::from_bytes(k.0.clone()), passphrase: None });
    }
    if !c.password.0.is_empty() {
        let pw = Secret::from_bytes(c.password.0.clone());
        auth.push(Auth::Password(pw.clone()));
        auth.push(Auth::KeyboardInteractive(pw));
    }
    let params = SshParams {
        host: r.host,
        port: r.port,
        user: c.username.clone(),
        auth,
        connect_timeout: Duration::from_secs(r.connect_timeout_secs.max(1) as u64),
        keepalive: Duration::from_secs(r.keepalive_secs.max(1) as u64),
    };
    let conn = match opts.host_key_alias.as_deref().and_then(|a| a.rsplit_once(':')).and_then(|(h, p)| Some((h.to_string(), p.parse::<u16>().ok()?))) {
        Some((alias_host, alias_port)) => {
            let stream = tokio::time::timeout(params.connect_timeout, tokio::net::TcpStream::connect((params.host.as_str(), params.port)))
                .await
                .map_err(|_| "Zeitüberschreitung beim Verbindungsaufbau".to_string())?
                .map_err(|e| e.to_string())?;
            let alias = SshParams { host: alias_host, port: alias_port, ..params.clone() };
            SshConnection::connect_stream(&alias, stream, known, ask).await.map_err(|e| e.to_string())?
        }
        None => SshConnection::connect(&params, known, ask).await.map_err(|e| e.to_string())?,
    };
    let term = opts.term.unwrap_or_else(|| "xterm-256color".into());
    let shell = conn.open_shell(&term, r.width, r.height).await.map_err(|e| e.to_string())?;
    Ok((conn, shell))
}
