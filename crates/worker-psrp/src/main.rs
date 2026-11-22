//! PowerShell-Worker: ein Prozess je Sitzung, lokaler `pwsh` als Kindprozess.
//! Eingaben werden zeilenweise gesammelt und als Skript ausgeführt; die
//! Ströme kommen farblich getrennt als Terminalausgabe und zusätzlich als
//! JSON (`FromWorker::Custom`).

use std::sync::mpsc;
use std::time::Duration;

use rj_core::{PsrpTransport, Secret};
use rj_ipc::worker::WorkerLink;
use rj_ipc::*;
use rj_psrp::{PsSession, PsTarget};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Options {
    psrp_transport: Option<PsrpTransport>,
}

fn color(stream: &str) -> &'static str {
    match stream {
        "error" => "\x1b[31m",
        "warning" => "\x1b[33m",
        "verbose" | "debug" => "\x1b[90m",
        "progress" => "\x1b[36m",
        "information" => "\x1b[32m",
        _ => "",
    }
}

fn main() {
    let mut link = match WorkerLink::stdio("psrp", Duration::from_secs(2)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Kanal nicht verfügbar: {e}");
            std::process::exit(2);
        }
    };
    let out = link.sender.clone();
    let send = |m: FromWorker| {
        let _ = out.send(&m);
    };
    let (tx, rx) = mpsc::channel::<ToWorker>();
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
    let mut req: Option<ConnectRequest> = None;
    let mut cred: Option<CredentialPayload> = None;
    let mut session: Option<PsSession> = None;
    let mut line = String::new();
    let prompt = "\r\nPS> ";
    while let Ok(m) = rx.recv() {
        match m {
            ToWorker::Shutdown => break,
            ToWorker::Ping => send(FromWorker::Pong),
            ToWorker::Connect(r) => req = Some(r),
            ToWorker::Credentials(c) => cred = Some(c),
            ToWorker::Input(_) if session.is_none() => {}
            ToWorker::Input(InputEvent::Bytes(b)) => {
                let text = String::from_utf8_lossy(&b).into_owned();
                for ch in text.chars() {
                    match ch {
                        '\r' | '\n' => {
                            let cmd = std::mem::take(&mut line);
                            send(FromWorker::Output(b"\r\n".to_vec()));
                            if cmd.trim().is_empty() {
                                send(FromWorker::Output(prompt.as_bytes().to_vec()));
                                continue;
                            }
                            let s = session.as_mut().expect("Sitzung vorhanden");
                            let r = s.run(&cmd, |rec| {
                                let txt = rec.text.replace('\n', "\r\n");
                                let reset = if color(&rec.stream).is_empty() { "" } else { "\x1b[0m" };
                                send(FromWorker::Output(format!("{}{txt}{reset}\r\n", color(&rec.stream)).into_bytes()));
                                send(FromWorker::Custom(serde_json::json!({ "stream": rec.stream, "text": rec.text }).to_string().into_bytes()));
                            });
                            if let Err(e) = r {
                                send(FromWorker::State(SessionState::Disconnected { reason: e.to_string() }));
                                session = None;
                                continue;
                            }
                            send(FromWorker::Output(prompt.as_bytes().to_vec()));
                        }
                        '\u{7f}' | '\u{8}' => {
                            if line.pop().is_some() {
                                send(FromWorker::Output(b"\x08 \x08".to_vec()));
                            }
                        }
                        c if !c.is_control() => {
                            line.push(c);
                            send(FromWorker::Output(c.to_string().into_bytes()));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if session.is_none() && req.is_some() && cred.is_some() {
            let (r, c) = (req.take().unwrap(), cred.take().unwrap());
            send(FromWorker::State(SessionState::Connecting));
            let o: Options = serde_json::from_str(&r.options_json).unwrap_or_default();
            let transport = PsrpTransport::effective(o.psrp_transport, Some(r.port));
            if let PsrpTransport::Winrm { verify_certificate: false, .. } = &transport {
                send(FromWorker::Notice("Zertifikatsprüfung für WinRM ist abgeschaltet.".into()));
            }
            let t = PsTarget { host: r.host.clone(), username: c.username.clone(), domain: c.domain.clone(), transport };
            match PsSession::start(&t, &Secret::from_bytes(c.password.0.clone())) {
                Ok(s) => {
                    session = Some(s);
                    send(FromWorker::State(SessionState::Connected));
                    send(FromWorker::Output(format!("PowerShell-Sitzung zu {}{prompt}", r.host).into_bytes()));
                }
                Err(e) => send(FromWorker::State(SessionState::Failed { reason: e.to_string() })),
            }
        }
    }
}
