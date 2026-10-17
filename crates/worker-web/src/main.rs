//! Web-Worker: eigenes Fenster mit WebView (Linux: WebKitGTK, Windows:
//! WebView2), eigenes Profil je Ziel, Formular-Anmeldung per Skript.

use std::time::Duration;

use rj_core::Secret;
use rj_ipc::worker::{Sender, WorkerLink};
use rj_ipc::*;
use rj_web::*;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::{WebContext, WebView, WebViewBuilder};

#[derive(Debug)]
enum UserEvent {
    Msg(ToWorker),
    PageLoaded(String),
    ScriptResult(String),
}

fn main() {
    let mut link = match WorkerLink::stdio("web", Duration::from_secs(2)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Kanal nicht verfügbar: {e}");
            std::process::exit(2);
        }
    };
    let out = link.sender.clone();
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || loop {
        match link.recv() {
            Ok(m) => {
                if proxy.send_event(UserEvent::Msg(m)).is_err() {
                    break;
                }
            }
            Err(_) => {
                let _ = proxy.send_event(UserEvent::Msg(ToWorker::Shutdown));
                break;
            }
        }
    });
    let proxy = event_loop.create_proxy();
    let mut st = State { out, proxy, view: None, window: None, context: None, flow: LoginFlow::new(None), creds: None, inject_pending: false };
    #[cfg(target_os = "linux")]
    let mut tls = tls::Pending::default();
    event_loop.run(move |event, target, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Msg(m)) => match m {
                ToWorker::Shutdown => *flow = ControlFlow::Exit,
                ToWorker::Ping => st.send(FromWorker::Pong),
                ToWorker::Credentials(c) => {
                    st.creds = Some(c);
                    if st.inject_pending {
                        st.inject();
                    }
                }
                ToWorker::Connect(r) => {
                    if let Err(e) = st.open(r, target) {
                        st.send(FromWorker::State(SessionState::Failed { reason: e }));
                    }
                    #[cfg(target_os = "linux")]
                    if let Some(v) = &st.view {
                        tls.attach(v, st.out.clone());
                    }
                }
                #[cfg(target_os = "linux")]
                ToWorker::Answer { question, accept } => tls.answer(question, accept),
                _ => {}
            },
            Event::UserEvent(UserEvent::PageLoaded(url)) => match st.flow.page_loaded(&url) {
                FlowAction::InjectLogin => st.inject(),
                FlowAction::Notice(n) => st.send(FromWorker::Notice(n)),
                FlowAction::None => {}
            },
            Event::UserEvent(UserEvent::ScriptResult(r)) => {
                if let FlowAction::Notice(n) = st.flow.script_result(&r) {
                    st.send(FromWorker::Notice(n));
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                st.send(FromWorker::State(SessionState::Disconnected { reason: "Fenster geschlossen".into() }));
                *flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

struct State {
    out: Sender,
    proxy: EventLoopProxy<UserEvent>,
    view: Option<WebView>,
    window: Option<tao::window::Window>,
    context: Option<WebContext>,
    flow: LoginFlow,
    creds: Option<CredentialPayload>,
    inject_pending: bool,
}

impl State {
    fn send(&self, m: FromWorker) {
        let _ = self.out.send(&m);
    }

    fn open(&mut self, r: ConnectRequest, target: &tao::event_loop::EventLoopWindowTarget<UserEvent>) -> Result<(), String> {
        let opts: WebOptions = if r.options_json.trim().is_empty() { WebOptions::default() } else { serde_json::from_str(&r.options_json).map_err(|e| format!("Optionen: {e}"))? };
        let url = start_url(&opts, &r.host, r.port).map_err(|e| e.to_string())?;
        if !url.https && !opts.allow_http {
            self.send(FromWorker::Notice("Achtung: unverschlüsselte Verbindung (HTTP).".into()));
        }
        let base = rj_core::config::data_dir().ok_or("kein Datenverzeichnis")?;
        let key = opts.profile.clone().unwrap_or_else(|| format!("{}-{}", url.host, url.port));
        let profile = profile_dir(&base, &key);
        std::fs::create_dir_all(&profile).map_err(|e| format!("Profil: {e}"))?;
        self.flow = LoginFlow::new(opts.login.clone());
        self.view = None;
        self.send(FromWorker::State(SessionState::Connecting));
        let window = WindowBuilder::new()
            .with_title(format!("remotejoe – {}", url.host))
            .with_inner_size(tao::dpi::LogicalSize::new(r.width.max(640), r.height.max(480)))
            .build(target)
            .map_err(|e| e.to_string())?;
        let mut ctx = WebContext::new(Some(profile));
        let proxy = self.proxy.clone();
        let builder = WebViewBuilder::new_with_web_context(&mut ctx)
            .with_url(&url.url)
            .with_on_page_load_handler(move |ev, u| {
                if matches!(ev, wry::PageLoadEvent::Finished) {
                    let _ = proxy.send_event(UserEvent::PageLoaded(u));
                }
            });
        #[cfg(target_os = "linux")]
        let view = {
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            builder.build_gtk(window.default_vbox().ok_or("kein GTK-Container")?)
        };
        #[cfg(not(target_os = "linux"))]
        let view = builder.build(&window);
        self.view = Some(view.map_err(|e| e.to_string())?);
        self.window = Some(window);
        self.context = Some(ctx);
        self.send(FromWorker::State(SessionState::Connected));
        self.send(FromWorker::Log(format!("Web-Sitzung geöffnet: {}", url.host)));
        Ok(())
    }

    fn inject(&mut self) {
        let (Some(view), Some(c)) = (&self.view, &self.creds) else {
            self.inject_pending = true;
            return;
        };
        self.inject_pending = false;
        let Some(login) = self.flow_login() else { return };
        let user = match &c.domain {
            Some(d) if !d.is_empty() && !c.username.contains('@') && !c.username.contains('\\') => format!("{d}\\{}", c.username),
            _ => c.username.clone(),
        };
        let pw = Secret::from_bytes(c.password.0.clone());
        match login_script(&login, &user, &pw) {
            Ok(script) => {
                let proxy = self.proxy.clone();
                let _ = view.evaluate_script_with_callback(&script, move |r| {
                    let _ = proxy.send_event(UserEvent::ScriptResult(r));
                });
            }
            Err(e) => self.send(FromWorker::Notice(e.to_string())),
        }
    }

    fn flow_login(&self) -> Option<FormLogin> {
        self.flow.login().cloned()
    }
}

#[cfg(target_os = "linux")]
mod tls {
    //! Zertifikatsfehler unter WebKitGTK: Rückfrage mit Warnung, angenommene
    //! Fingerabdrücke je Profil.
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::rc::Rc;

    use gio::prelude::*;
    use rj_ipc::worker::Sender;
    use rj_ipc::FromWorker;
    use rj_web::{AcceptedCerts, CertDecision};
    use webkit2gtk::{WebContextExt, WebViewExt, WebsiteDataManagerExt};
    use wry::WebViewExtUnix;

    struct Ask {
        view: webkit2gtk::WebView,
        cert: gio::TlsCertificate,
        host: String,
        uri: String,
        profile: PathBuf,
    }

    #[derive(Default)]
    pub struct Pending {
        asks: Rc<RefCell<HashMap<u32, Ask>>>,
        next: Rc<RefCell<u32>>,
    }

    fn problems(f: gio::TlsCertificateFlags) -> Vec<String> {
        let mut v = Vec::new();
        for (flag, text) in [
            (gio::TlsCertificateFlags::UNKNOWN_CA, "unbekannte Zertifizierungsstelle"),
            (gio::TlsCertificateFlags::BAD_IDENTITY, "Name passt nicht"),
            (gio::TlsCertificateFlags::NOT_ACTIVATED, "noch nicht gültig"),
            (gio::TlsCertificateFlags::EXPIRED, "abgelaufen"),
            (gio::TlsCertificateFlags::REVOKED, "widerrufen"),
            (gio::TlsCertificateFlags::INSECURE, "unsicherer Algorithmus"),
            (gio::TlsCertificateFlags::GENERIC_ERROR, "allgemeiner Fehler"),
        ] {
            if f.contains(flag) {
                v.push(text.to_string());
            }
        }
        if v.is_empty() {
            v.push("ungültig".into());
        }
        v
    }

    fn allow(view: &webkit2gtk::WebView, cert: &gio::TlsCertificate, host: &str, uri: &str) {
        if let Some(ctx) = view.context() {
            ctx.allow_tls_certificate_for_host(cert, host);
        }
        view.load_uri(uri);
    }

    impl Pending {
        pub fn attach(&mut self, v: &wry::WebView, out: Sender) {
            let wk = v.webview();
            let profile = wk
                .website_data_manager()
                .and_then(|m| m.base_data_directory())
                .map(|s| PathBuf::from(s.as_str()))
                .unwrap_or_default();
            let asks = self.asks.clone();
            let next = self.next.clone();
            wk.connect_load_failed_with_tls_errors(move |view, uri, cert, flags| {
                let host = glib::Uri::parse(uri, glib::UriFlags::NONE).ok().and_then(|u| u.host()).map(|h| h.to_string()).unwrap_or_default();
                let der = cert.certificate().map(|b| b.to_vec()).unwrap_or_default();
                let known = AcceptedCerts::load(&profile);
                match known.decide(&host, &der, &problems(flags)) {
                    CertDecision::Valid | CertDecision::Accepted => allow(view, cert, &host, uri),
                    CertDecision::Ask { text, detail, fingerprint, previous } => {
                        let id = {
                            let mut n = next.borrow_mut();
                            *n += 1;
                            *n
                        };
                        asks.borrow_mut().insert(id, Ask { view: view.clone(), cert: cert.clone(), host: host.clone(), uri: uri.to_string(), profile: profile.clone() });
                        let info = rj_ipc::msg::SecurityInfo { kind: "certificate".into(), host: host.clone(), port: 0, key_type: "X.509".into(), fingerprint, previous };
                        let _ = out.send(&FromWorker::Question { id, text, detail, info });
                        view.load_html("<html><body style='font-family:sans-serif'><h2>Zertifikat nicht vertrauenswürdig</h2><p>Bitte die Rückfrage in remotejoe beantworten.</p></body></html>", None);
                    }
                }
                true
            });
        }

        pub fn answer(&mut self, id: u32, accept: bool) {
            let Some(a) = self.asks.borrow_mut().remove(&id) else { return };
            if !accept {
                a.view.load_html("<html><body style='font-family:sans-serif'><h2>Verbindung abgelehnt</h2><p>Das Zertifikat wurde nicht angenommen.</p></body></html>", None);
                return;
            }
            let der = a.cert.certificate().map(|b| b.to_vec()).unwrap_or_default();
            let mut known = AcceptedCerts::load(&a.profile);
            known.accept(&a.host, &der);
            let _ = known.save(&a.profile);
            allow(&a.view, &a.cert, &a.host, &a.uri);
        }
    }
}
