//! RDP-Worker: ein Prozess je Sitzung.

#[cfg(rj_no_freerdp)]
fn main() {
    eprintln!("Dieser RDP-Worker wurde ohne FreeRDP gebaut.");
    std::process::exit(2);
}

#[cfg(not(rj_no_freerdp))]
fn main() {
    imp::main()
}

#[cfg(not(rj_no_freerdp))]
mod imp {
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use rj_core::{ClipboardDirection, Secret};
    use rj_ipc::shm::FrameBufferWriter;
    use rj_ipc::worker::{Sender, WorkerLink};
    use rj_ipc::*;
    use rj_rdp::keymap::mouse;
    use rj_rdp::session::*;
    use serde::Deserialize;

    /// Optionen aus dem Index (JSON), ergänzt um Verbindungsdetails.
    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    struct Options {
        clipboard: Option<ClipboardDirection>,
        clipboard_limit: Option<u64>,
        ignore_certificate: bool,
        security: Security,
        keyboard_layout: Option<String>,
        config_path: Option<String>,
        clipboard_audit: bool,
        gfx: rj_core::model::GfxMode,
    }

    fn layout_id(s: &str) -> u32 {
        match s {
            "de" | "de-DE" => 0x0407,
            "de-CH" => 0x0807,
            "de-AT" => 0x0C07,
            "en-US" | "us" => 0x0409,
            "en-GB" | "gb" => 0x0809,
            "fr" | "fr-FR" => 0x040C,
            _ => u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(0x0407),
        }
    }

    /// Anmeldeprüfung (`--auth-check`): Angaben als eine JSON-Zeile auf stdin (Rechner, Port, Benutzer, Domäne,
    /// Kennwort, Sicherheit, Zertifikatsablage); nur anmelden (NLA), keine Sitzung. Ausgabe: eine JSON-Zeile mit
    /// `result` (`ok`, `auth`, `locked`, `expired`, `unreachable`, `cert`, `other`) und `message`. Unbekannte oder
    /// geänderte Zertifikate werden nicht angenommen (`cert`): sonst gingen die Zugangsdaten an einen fremden Server.
    #[derive(Deserialize)]
    struct AuthCheck {
        host: String,
        port: u16,
        username: String,
        #[serde(default)]
        domain: Option<String>,
        password: String,
        #[serde(default)]
        security: Security,
        #[serde(default)]
        config_path: Option<String>,
    }

    fn auth_check() {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        let req: AuthCheck = match serde_json::from_str(line.trim()) {
            Ok(r) => r,
            Err(e) => {
                println!("{}", serde_json::json!({ "result": "other", "message": format!("Angaben unlesbar: {e}") }));
                return;
            }
        };
        let cert_rejected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = cert_rejected.clone();
        let mut s = match RdpSession::new(move |_| {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            CertDecision::Reject
        }) {
            Ok(s) => s,
            Err(e) => {
                println!("{}", serde_json::json!({ "result": "other", "message": e.to_string() }));
                return;
            }
        };
        let p = RdpParams {
            host: req.host,
            port: req.port,
            username: req.username,
            domain: req.domain.filter(|d| !d.is_empty()),
            password: Secret::from_str_exact(&req.password),
            width: 800,
            height: 600,
            security: req.security,
            ignore_certificate: false,
            clipboard: rj_core::ClipboardDirection::Off,
            clipboard_limit: 0,
            keyboard_layout: 0x0407,
            config_path: req.config_path.or_else(|| rj_core::config::data_dir().map(|d| d.join("freerdp").display().to_string())),
            timeout_ms: 10000,
            gfx: false,
        };
        let out = match s.authenticate(&p) {
            Ok(()) => serde_json::json!({ "result": "ok", "message": "" }),
            Err(e) => {
                let msg = e.to_string();
                let result = if cert_rejected.load(std::sync::atomic::Ordering::SeqCst) { "cert" } else { classify_error(&msg) };
                serde_json::json!({ "result": result, "message": msg })
            }
        };
        println!("{out}");
    }

    pub fn main() {
        // FreeRDP-Protokoll: nur Warnungen und Fehler.
        if std::env::var_os("WLOG_LEVEL").is_none() {
            std::env::set_var("WLOG_LEVEL", "WARN");
        }
        if std::env::args().any(|a| a == "--auth-check") {
            auth_check();
            return;
        }
        let mut link = match WorkerLink::stdio("rdp", Duration::from_secs(2)) {
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

    struct Worker {
        out: Sender,
        answers: Arc<Mutex<Receiver<(u32, bool)>>>,
        session: Option<RdpSession>,
        fb: Option<FrameBufferWriter>,
        serial: u64,
        last: Option<(ConnectRequest, CredentialPayload)>,
        connected_at: Option<Instant>,
        clip_notice_sent: bool,
        /// Text, der vor der Freigabe der Zwischenablage angeboten wurde; geht nach "bereit" hinaus.
        clip_wait: Option<String>,
    }

    impl Worker {
        fn send(&self, m: FromWorker) {
            let _ = self.out.send(&m);
        }

        fn connect(&mut self, req: ConnectRequest, cred: CredentialPayload) {
            self.send(FromWorker::State(SessionState::Connecting));
            let opts: Options = serde_json::from_str(&req.options_json).unwrap_or_default();
            let (out0, answers0) = (self.out.clone(), self.answers.clone());
            let make_session = || {
                let out = out0.clone();
                let answers = answers0.clone();
                let next = Arc::new(Mutex::new(1u32));
                RdpSession::new(move |c: &CertInfo| {
                let mut n = next.lock().unwrap_or_else(|e| e.into_inner());
                let id = *n;
                *n += 1;
                let text = match &c.old_fingerprint {
                    Some(_) => format!("WARNUNG: Das Zertifikat von {}:{} hat sich geändert.", c.host, c.port),
                    None => format!("Das Zertifikat von {}:{} ist nicht vertrauenswürdig oder unbekannt.", c.host, c.port),
                };
                let mut detail = format!("Inhaber: {}\nAussteller: {}\nFingerabdruck: {}", c.subject, c.issuer, c.fingerprint);
                if let Some(old) = &c.old_fingerprint {
                    detail.push_str(&format!("\nbisher: {old}"));
                }
                let info = rj_ipc::msg::SecurityInfo {
                    kind: "certificate".into(),
                    host: c.host.clone(),
                    port: c.port,
                    key_type: "X.509".into(),
                    fingerprint: c.fingerprint.clone(),
                    previous: c.old_fingerprint.clone(),
                };
                let _ = out.send(&FromWorker::Question { id, text, detail, info });
                let rx = answers.lock().unwrap_or_else(|e| e.into_inner());
                let deadline = Instant::now() + Duration::from_secs(300);
                loop {
                    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok((q, accept)) if q == id => {
                            return if accept { CertDecision::Accept } else { CertDecision::Reject };
                        }
                        Ok(_) => continue,
                        Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => return CertDecision::Reject,
                    }
                }
                })
            };
            // `Auto`: erst ohne Grafikkanal; scheitert der Aufbau, einmal mit. Vorgabe ist `On`.
            let mut gfx_on = opts.gfx == rj_core::model::GfxMode::On;
            loop {
            let mut session = match make_session() {
                Ok(s) => s,
                Err(e) => return self.send(FromWorker::State(SessionState::Failed { reason: e.to_string() })),
            };
            session.audit = opts.clipboard_audit;
            let params = RdpParams {
                host: req.host.clone(),
                port: req.port,
                username: cred.username.clone(),
                domain: cred.domain.clone(),
                password: Secret::from_bytes(cred.password.0.clone()),
                width: req.width.max(200),
                height: req.height.max(200),
                security: opts.security,
                ignore_certificate: opts.ignore_certificate,
                clipboard: opts.clipboard.unwrap_or_default(),
                clipboard_limit: opts.clipboard_limit.unwrap_or(16 * 1024 * 1024),
                keyboard_layout: opts.keyboard_layout.as_deref().map(layout_id).unwrap_or(0x0407),
                config_path: opts.config_path.clone().or_else(|| rj_core::config::data_dir().map(|d| d.join("freerdp").display().to_string())),
                timeout_ms: req.connect_timeout_secs.max(1) * 1000,
                gfx: gfx_on,
            };
            if params.ignore_certificate {
                self.send(FromWorker::Notice("Zertifikatsprüfung ist für dieses Ziel abgeschaltet.".into()));
            }
            match session.connect(&params) {
                Ok(()) => {
                    self.session = Some(session);
                    self.connected_at = Some(Instant::now());
                    self.clip_notice_sent = params.clipboard == ClipboardDirection::Off;
                    self.last = Some((req, cred));
                    self.send(FromWorker::State(SessionState::Connected));
                    return;
                }
                Err(e) => {
                    let text = e.to_string();
                    if opts.gfx == rj_core::model::GfxMode::Auto && !gfx_on && text.contains("ERRCONNECT_CONNECT_TRANSPORT_FAILED") {
                        gfx_on = true;
                        self.send(FromWorker::Notice("Der Server hat den Aufbau ohne Grafikkanal abgebrochen. Neuer Versuch mit Grafikkanal (GFX).".into()));
                        continue;
                    }
                    self.send(FromWorker::State(SessionState::Failed { reason: text }));
                    return;
                }
            }
            }
        }

        fn handle(&mut self, m: ToWorker, pending: &mut (Option<ConnectRequest>, Option<CredentialPayload>)) -> bool {
            match m {
                ToWorker::Connect(r) => {
                    if let Some(s) = self.session.as_mut() {
                        s.disconnect();
                    }
                    self.session = None;
                    pending.0 = Some(r);
                }
                ToWorker::Credentials(c) => pending.1 = Some(c),
                ToWorker::Shutdown => {
                    if let Some(s) = self.session.as_mut() {
                        s.disconnect();
                    }
                    return false;
                }
                ToWorker::Ping => self.send(FromWorker::Pong),
                ToWorker::Answer { .. } | ToWorker::Custom(_) | ToWorker::Term(_) => {}
                ToWorker::Input(ev) => {
                    let Some(s) = self.session.as_mut() else { return true };
                    match ev {
                        InputEvent::Text(t) => s.send_text(&t),
                        InputEvent::Key { keysym, down } => {
                            if std::env::var_os("RJ_KEYLOG").is_some() {
                                eprintln!("[remotejoe-rdp] Taste Keysym 0x{keysym:x} → Scancode {:?} {}", rj_rdp::keymap::scancode(keysym).map(|c| format!("0x{c:x}")), if down { "gedrückt" } else { "losgelassen" });
                            }
                            if !s.send_key(keysym, down) && down {
                                if let Some(c) = char::from_u32(keysym).filter(|c| !c.is_control()) {
                                    s.send_text(&c.to_string());
                                }
                            }
                        }
                        InputEvent::Scancodes(seq) => {
                            for (sc, down) in seq {
                                if std::env::var_os("RJ_KEYLOG").is_some() {
                                    eprintln!("[remotejoe-rdp] Tastenfolge Scancode 0x{sc:x} {}", if down { "gedrückt" } else { "losgelassen" });
                                }
                                s.send_scancode(sc, down);
                            }
                        }
                        InputEvent::Bytes(_) => {}
                        InputEvent::MouseMove { x, y } => s.send_mouse(mouse::MOVE, x.max(0) as u16, y.max(0) as u16),
                        InputEvent::MouseButton { x, y, button, down } => {
                            let (x, y) = (x.max(0) as u16, y.max(0) as u16);
                            let flags = match button {
                                MouseButton::Left => mouse::BUTTON1,
                                MouseButton::Right => mouse::BUTTON2,
                                MouseButton::Middle => mouse::BUTTON3,
                                MouseButton::WheelUp => mouse::WHEEL | 0x0078,
                                MouseButton::WheelDown => mouse::WHEEL | mouse::WHEEL_NEGATIVE | 0x0088,
                            };
                            let is_wheel = matches!(button, MouseButton::WheelUp | MouseButton::WheelDown);
                            if is_wheel {
                                if down {
                                    s.send_mouse(flags, x, y);
                                }
                            } else {
                                s.send_mouse(flags | if down { mouse::DOWN } else { 0 }, x, y);
                            }
                        }
                    }
                }
                ToWorker::Resize { width, height } => {
                    // Keine laufende Sitzung (abgemeldet, getrennt): nur die Größe für das nächste
                    // Verbinden merken, nicht selbst neu verbinden.
                    if self.session.is_none() {
                        if let Some((req, _)) = self.last.as_mut() {
                            req.width = width;
                            req.height = height;
                        }
                        return true;
                    }
                    let handled = self.session.as_mut().is_some_and(|s| s.resize(width, height));
                    if !handled {
                        if let Some((mut req, cred)) = self.last.clone() {
                            // Kleine Änderungen (z. B. Kopfzeile des Tabs nach dem Verbinden): nicht neu
                            // verbinden, die Oberfläche skaliert das Bild. Jede Neuverbindung ist eine neue
                            // Anmeldung; mitten in der Anmeldung abgebrochen, meldet Windows die Sitzung ab.
                            if rj_rdp::small_resize((req.width, req.height), (width, height)) {
                                return true;
                            }
                            self.send(FromWorker::Notice("Die Gegenstelle unterstützt keine Größenänderung im laufenden Betrieb; es wird neu verbunden.".into()));
                            req.width = width;
                            req.height = height;
                            if let Some(s) = self.session.as_mut() {
                                s.disconnect();
                            }
                            self.session = None;
                            self.fb = None;
                            self.connect(req, cred);
                        }
                    }
                }
                ToWorker::Clipboard(data) => {
                    let Some(s) = self.session.as_mut() else { return true };
                    if !s.clipboard_ready() {
                        // Kanal noch nicht bereit: merken statt melden.
                        if let ClipData::Text(t) = data {
                            self.clip_wait = Some(t);
                        }
                        return true;
                    }
                    let r = match data {
                        ClipData::Text(t) => s.offer_text(&t),
                        ClipData::Files(files) => s.offer_files(files.into_iter().map(|f| (f.name, f.data)).collect()),
                    };
                    // Gesperrte Richtung ist Absicht des Nutzers, keine Störung: keine Meldung.
                    if let Err(e) = r {
                        if !matches!(e, rj_rdp::session::RdpError::ClipUnavailable) {
                            self.send(FromWorker::Notice(e.to_string()));
                        }
                    }
                }
            }
            if self.session.is_none() && pending.0.is_some() && pending.1.is_some() {
                let (r, c) = (pending.0.take().unwrap(), pending.1.take().unwrap());
                self.connect(r, c);
            }
            true
        }

        fn frame(&mut self, x: u32, y: u32, w: u32, h: u32) {
            let Some(s) = self.session.as_ref() else { return };
            let fb = &mut self.fb;
            let out = &self.out;
            let mut new_fb = None;
            s.with_frame(|buf, width, height| {
                if fb.as_ref().is_none_or(|f| f.width != width || f.height != height) {
                    match FrameBufferWriter::create(width, height) {
                        Ok(f) => match rj_ipc::fdpass::share(&f) {
                            Ok(h) => {
                                new_fb = Some(h);
                                *fb = Some(f);
                            }
                            Err(e) => {
                                let _ = out.send(&FromWorker::Log(format!("Bildspeicher nicht übergebbar: {e}")));
                                return;
                            }
                        },
                        Err(e) => {
                            let _ = out.send(&FromWorker::Log(format!("Bildspeicher nicht anlegbar: {e}")));
                            return;
                        }
                    }
                }
                if let Some(f) = fb.as_mut() {
                    let stride = width as usize * 4;
                    let start = y as usize * stride + x as usize * 4;
                    f.blit(x, y, w, h, &buf[start..], stride);
                }
            });
            let Some(f) = self.fb.as_ref() else { return };
            if let Some(handle) = new_fb {
                self.send(FromWorker::SharedFrame { handle, width: f.width, height: f.height });
            }
            self.serial += 1;
            self.send(FromWorker::Frame(FrameUpdate { width: f.width, height: f.height, x, y, w, h, serial: self.serial }));
        }
    }

    fn run(out: Sender, rx: Receiver<ToWorker>, answers: Arc<Mutex<Receiver<(u32, bool)>>>) {
        let mut w = Worker { out, answers, session: None, fb: None, serial: 0, last: None, connected_at: None, clip_notice_sent: false, clip_wait: None };
        let mut pending = (None, None);
        let mut events = Vec::new();
        loop {
            // Nachrichten des Hauptprozesses.
            let first = if w.session.is_some() { rx.try_recv().ok() } else { rx.recv_timeout(Duration::from_millis(100)).ok() };
            let mut next = first;
            while let Some(m) = next {
                if !w.handle(m, &mut pending) {
                    return;
                }
                next = rx.try_recv().ok();
            }
            let Some(s) = w.session.as_mut() else { continue };
            events.clear();
            let alive = s.poll(20, &mut events);
            for ev in events.drain(..) {
                match ev {
                    RdpEvent::Frame { x, y, w: fw, h } => w.frame(x, y, fw, h),
                    RdpEvent::Resized { .. } => {}
                    RdpEvent::Pointer(p) => w.send(FromWorker::Pointer(p)),
                    RdpEvent::ClipReady => {
                        if let (Some(t), Some(s)) = (w.clip_wait.take(), w.session.as_mut()) {
                            let _ = s.offer_text(&t);
                        }
                    }
                    RdpEvent::ClipText(t) => w.send(FromWorker::Clipboard(ClipData::Text(t))),
                    RdpEvent::ClipFiles(files) => w.send(FromWorker::Clipboard(ClipData::Files(
                        files.into_iter().filter(|(d, _)| !d.is_dir).map(|(d, data)| ClipFile { name: d.name, data }).collect(),
                    ))),
                    RdpEvent::Notice(n) => w.send(FromWorker::Notice(n)),
                    RdpEvent::Log(l) => {
                        if !l.is_empty() {
                            w.send(FromWorker::Log(l));
                        }
                    }
                }
            }
            if !w.clip_notice_sent && w.connected_at.is_some_and(|t| t.elapsed() > Duration::from_secs(10)) {
                w.clip_notice_sent = true;
                if !w.session.as_ref().is_some_and(|s| s.clipboard_ready()) {
                    w.send(FromWorker::Notice("Die Gegenstelle gibt die Zwischenablage nicht frei.".into()));
                }
            }
            if !alive {
                let info = w.session.as_ref().map(|s| s.error_info()).unwrap_or(0);
                let since = w.connected_at.map(|t| t.elapsed());
                w.session = None;
                w.fb = None;
                w.connected_at = None;
                // Abmelden, Trennen in Windows usw.: gewollt, kein automatisches Neuverbinden.
                let state = match rj_rdp::deliberate_end(info).filter(|_| rj_rdp::counts_as_deliberate(since)) {
                    Some(reason) => SessionState::Ended { reason: reason.into() },
                    None => SessionState::Disconnected { reason: "Verbindung getrennt".into() },
                };
                w.send(FromWorker::State(state));
            }
        }
    }
}
