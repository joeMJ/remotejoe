//! VNC-Worker: ein Prozess je Sitzung.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rj_core::{ClipboardDirection, Secret};
use rj_ipc::shm::FrameBufferWriter;
use rj_ipc::worker::{Sender, WorkerLink};
use rj_ipc::*;
use rj_vnc::client::*;
use serde::Deserialize;

/// Optionen aus dem Index (JSON).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Options {
    clipboard: Option<ClipboardDirection>,
    clipboard_limit: Option<u32>,
    view_only: bool,
    prefer_tls: Option<bool>,
    known_certs: Option<String>,
    extended_clipboard: Option<bool>,
}

fn main() {
    let mut link = match WorkerLink::stdio("vnc", Duration::from_secs(2)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Kanal nicht verfügbar: {e}");
            std::process::exit(2);
        }
    };
    let out = link.sender.clone();
    let (msg_tx, msg_rx) = mpsc::channel::<ToWorker>();
    let (ans_tx, ans_rx) = mpsc::channel::<(u32, bool)>();
    std::thread::spawn(move || loop {
        match link.recv() {
            Ok(ToWorker::Answer { question, accept }) => {
                let _ = ans_tx.send((question, accept));
            }
            Ok(m) => {
                if msg_tx.send(m).is_err() {
                    break;
                }
            }
            Err(_) => {
                let _ = msg_tx.send(ToWorker::Shutdown);
                break;
            }
        }
    });
    run(out, msg_rx, Arc::new(Mutex::new(ans_rx)));
}

fn cert_check(out: Sender, answers: Arc<Mutex<Receiver<(u32, bool)>>>) -> CertCheck {
    let next = Arc::new(Mutex::new(1u32));
    Arc::new(move |c: &CertInfo| {
        let id = {
            let mut n = next.lock().unwrap_or_else(|e| e.into_inner());
            *n += 1;
            *n
        };
        let text = match &c.old_fingerprint {
            Some(_) => format!("WARNUNG: Das Zertifikat von {}:{} hat sich geändert.", c.host, c.port),
            None => format!("Das Zertifikat von {}:{} ist unbekannt.", c.host, c.port),
        };
        let mut detail = format!("Fingerabdruck (SHA-256): {}", c.fingerprint);
        if let Some(o) = &c.old_fingerprint {
            detail.push_str(&format!("\nbisher: {o}"));
        }
        let _ = out.send(&FromWorker::Question { id, text, detail });
        let rx = answers.lock().unwrap_or_else(|e| e.into_inner());
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok((q, a)) if q == id => return a,
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => return false,
            }
        }
    })
}

fn run(out: Sender, rx: Receiver<ToWorker>, answers: Arc<Mutex<Receiver<(u32, bool)>>>) {
    let send = |m: FromWorker| {
        let _ = out.send(&m);
    };
    let mut client: Option<VncClient> = None;
    let mut fb: Option<FrameBufferWriter> = None;
    let mut serial = 0u64;
    let mut pending: (Option<ConnectRequest>, Option<CredentialPayload>) = (None, None);
    let mut buttons = 0u8;
    let mut view_only = false;
    loop {
        let msg = rx.recv_timeout(Duration::from_millis(15));
        match msg {
            Ok(ToWorker::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(ToWorker::Connect(r)) => {
                client = None;
                pending.0 = Some(r);
            }
            Ok(ToWorker::Credentials(c)) => pending.1 = Some(c),
            Ok(ToWorker::Ping) => send(FromWorker::Pong),
            Ok(ToWorker::Input(ev)) => {
                if let Some(c) = client.as_ref().filter(|_| !view_only) {
                    let _ = match ev {
                        InputEvent::Text(t) => c.type_text(&t),
                        InputEvent::Key { keysym, down } => c.key(keysym, down),
                        InputEvent::Bytes(_) => Ok(()),
                        InputEvent::MouseMove { x, y } => c.pointer(buttons, x.max(0) as u16, y.max(0) as u16),
                        InputEvent::MouseButton { x, y, button, down } => {
                            let bit = match button {
                                MouseButton::Left => 1,
                                MouseButton::Middle => 2,
                                MouseButton::Right => 4,
                                MouseButton::WheelUp => 8,
                                MouseButton::WheelDown => 16,
                            };
                            let (x, y) = (x.max(0) as u16, y.max(0) as u16);
                            if bit >= 8 {
                                // Mausrad: Drücken und Loslassen.
                                c.pointer(buttons | bit, x, y).and_then(|_| c.pointer(buttons, x, y))
                            } else {
                                if down {
                                    buttons |= bit;
                                } else {
                                    buttons &= !bit;
                                }
                                c.pointer(buttons, x, y)
                            }
                        }
                    };
                }
            }
            Ok(ToWorker::Resize { width, height }) => {
                if let Some(c) = &client {
                    if !c.resize(width, height).unwrap_or(false) {
                        send(FromWorker::Notice("Die Gegenstelle unterstützt keine Größenänderung; das Bild wird skaliert.".into()));
                    }
                }
            }
            Ok(ToWorker::Clipboard(ClipData::Text(t))) => {
                if let Some(c) = &client {
                    if let Err(e) = c.send_clipboard(&t) {
                        send(FromWorker::Notice(e));
                    }
                }
            }
            Ok(ToWorker::Clipboard(ClipData::Files(_))) => {
                send(FromWorker::Notice("VNC überträgt keine Dateien über die Zwischenablage.".into()));
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
        }

        if client.is_none() && pending.0.is_some() && pending.1.is_some() {
            let (r, cred) = (pending.0.take().unwrap(), pending.1.take().unwrap());
            send(FromWorker::State(SessionState::Connecting));
            let o: Options = serde_json::from_str(&r.options_json).unwrap_or_default();
            view_only = o.view_only;
            let p = VncParams {
                host: r.host.clone(),
                port: r.port,
                username: (!cred.username.is_empty()).then(|| cred.username.clone()),
                password: (!cred.password.0.is_empty()).then(|| Secret::from_bytes(cred.password.0.clone())),
                shared: true,
                view_only: o.view_only,
                clipboard: o.clipboard.unwrap_or_default(),
                clipboard_limit: o.clipboard_limit.unwrap_or(16 * 1024 * 1024),
                prefer_tls: o.prefer_tls.unwrap_or(true),
                known_certs: o
                    .known_certs
                    .map(KnownCerts::new)
                    .or_else(|| rj_core::config::data_dir().map(|d| KnownCerts::new(d.join("vnc_certs")))),
                connect_timeout: Duration::from_secs(r.connect_timeout_secs.max(1) as u64),
                extended_clipboard: o.extended_clipboard.unwrap_or(true),
            };
            match VncClient::connect(&p, cert_check(out.clone(), answers.clone())) {
                Ok(c) => {
                    send(FromWorker::Log(format!("Verbunden mit „{}“ ({})", c.name, c.security)));
                    client = Some(c);
                    send(FromWorker::State(SessionState::Connected));
                }
                Err(e) => send(FromWorker::State(SessionState::Failed { reason: e.to_string() })),
            }
        }

        let Some(c) = client.as_ref() else { continue };
        let mut closed = None;
        let mut dirty: Option<(u32, u32, u32, u32)> = None;
        while let Ok(ev) = c.events.try_recv() {
            match ev {
                VncEvent::Frame { x, y, w, h } => {
                    dirty = Some(match dirty {
                        None => (x, y, x + w, y + h),
                        Some((a, b, cc, d)) => (a.min(x), b.min(y), cc.max(x + w), d.max(y + h)),
                    })
                }
                VncEvent::Resized { width, height } => dirty = Some((0, 0, width, height)),
                VncEvent::ClipText(t) => send(FromWorker::Clipboard(ClipData::Text(t))),
                VncEvent::Notice(n) => send(FromWorker::Notice(n)),
                VncEvent::Bell => {}
                VncEvent::Closed(r) => closed = Some(r),
            }
        }
        if let Some((x0, y0, x1, y1)) = dirty {
            c.with_frame(|f| {
                if fb.as_ref().is_none_or(|b| b.width != f.width || b.height != f.height) {
                    match FrameBufferWriter::create(f.width, f.height) {
                        Ok(b) => {
                            send(FromWorker::SharedFrame { path: b.path().display().to_string(), width: f.width, height: f.height });
                            fb = Some(b);
                        }
                        Err(e) => send(FromWorker::Log(format!("Bildspeicher nicht anlegbar: {e}"))),
                    }
                }
                if let Some(b) = fb.as_mut() {
                    let (x1, y1) = (x1.min(f.width), y1.min(f.height));
                    if x1 > x0 && y1 > y0 {
                        let stride = f.width as usize * 4;
                        let start = y0 as usize * stride + x0 as usize * 4;
                        b.blit(x0, y0, x1 - x0, y1 - y0, &f.data[start..], stride);
                        serial += 1;
                        send(FromWorker::Frame(FrameUpdate { width: f.width, height: f.height, x: x0, y: y0, w: x1 - x0, h: y1 - y0, serial }));
                    }
                }
            });
        }
        if let Some(r) = closed {
            client = None;
            fb = None;
            send(FromWorker::State(SessionState::Disconnected { reason: r }));
        }
    }
}
