//! Windows: WebView2 (über `wry`) als Kindfenster im Tab-Bereich des Hauptfensters.
//!
//! Anders als unter Linux (WebKitGTK ohne Fenster, Bild im geteilten Speicher) zeichnet WebView2 die Seite
//! selbst. Der Worker bekommt vom Hauptfenster die Kennung des Tab-Bereichs (`WebCommand::Embed`) und setzt die
//! Seite als Kindfenster hinein; Größe, Sichtbarkeit und Tastenliste folgen per Befehl. Eingaben gehen nativ an die
//! Seite; nur Tasten, die der Anwendung gehören (Kurzbefehle des Hauptfensters), meldet der Worker zurück.
//!
//! Formular-Anmeldung, Zertifikatsfrage und Profil je Ziel verwenden dieselbe Logik wie unter Linux (`rj_web`).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::num::NonZeroIsize;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use rj_core::Secret;
use rj_ipc::worker::{Sender, WorkerLink};
use rj_ipc::*;
use rj_web::*;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    take_pwstr, AcceleratorKeyPressedEventHandler, BytesReceivedChangedEventHandler, DocumentTitleChangedEventHandler, DownloadStartingEventHandler,
    FocusChangedEventHandler, HistoryChangedEventHandler, MoveFocusRequestedEventHandler, NavigationCompletedEventHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, ServerCertificateErrorDetectedEventHandler, SourceChangedEventHandler, StateChangedEventHandler,
};
use windows::core::{Interface, BOOL, HSTRING, PWSTR};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use wry::dpi::{PhysicalPosition, PhysicalSize, Position, Size};
use wry::{Rect, WebContext, WebView, WebViewBuilder, WebViewExtWindows};

/// Anmeldung: Suche nach dem Kennwortfeld wiederholen (Abstand und Anzahl; zusammen 10 Sekunden).
const LOGIN_RETRY_EVERY: Duration = Duration::from_millis(500);
const LOGIN_RETRIES: u32 = 20;
/// Fortschrittsmeldungen eines Downloads höchstens in diesem Abstand.
const DOWNLOAD_TICK: Duration = Duration::from_millis(300);
/// Nur diese Dateiendung wird nach dem Download sofort mit dem zugeordneten Programm geöffnet (Citrix-Startdatei).
const AUTO_OPEN_EXT: &str = "ica";

/// Eingang des Worker-Threads: Nachrichten des Hauptprogramms und eigene Zeitgeber.
enum Msg {
    Host(ToWorker),
    RetryLogin(zeroize::Zeroizing<String>, u32),
    CheckRejected,
    /// Ergebnisse von Skripten (die Rückrufe von wry müssen `Send` sein und reichen den Text deshalb hierher weiter).
    Detected(String),
    Rejected(String),
    /// Ergebnis der Suche in der Seite.
    Found(String),
    LoginResult { text: String, script: zeroize::Zeroizing<String>, tries: u32 },
    /// Anmeldung in Schritten: Schritt-Skript (erneut) ausführen, sein Ergebnis, Eingabe getippt.
    RunStep(Step, u32),
    StepResult { step: Step, text: String, tries: u32 },
    Typed(FlowAction),
    /// Ein „Verbindung abgebrochen“ nach dem Laden zeigen, falls kein Download gestartet ist.
    PendingError,
    /// Diagnose: gerenderte Seite als Bild speichern (nur mit `RJ_WEB_SHOTDIR`).
    DebugShot(&'static str),
}

#[derive(Clone)]
struct Queue {
    q: Arc<Mutex<VecDeque<Msg>>>,
    tid: u32,
}

impl Queue {
    fn push(&self, m: Msg) {
        self.q.lock().unwrap().push_back(m);
        // SAFETY: nur eine Nachricht an den Thread; schlägt fehl, solange der Thread noch keine Nachrichtenschleife hat
        // (dann holt der Worker die Warteschlange beim ersten Durchlauf nach).
        unsafe { PostThreadMessageW(self.tid, WM_APP, 0, 0) };
    }

    fn push_after(&self, d: Duration, m: Msg) {
        let me = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(d);
            me.push(m);
        });
    }

    fn pop(&self) -> Option<Msg> {
        self.q.lock().unwrap().pop_front()
    }
}

/// Elternfenster, das einem anderen Prozess (Hauptfenster) oder diesem Worker (unsichtbar) gehört.
struct Parent(isize);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let h = Win32WindowHandle::new(NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?);
        // SAFETY: Das Fenster lebt, solange die Seite eingebettet ist; der Aufrufer löst die Seite vorher.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(h)) })
    }
}

pub fn run() {
    let mut link = match WorkerLink::stdio("web", Duration::from_secs(2)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Kanal nicht verfügbar: {e}");
            std::process::exit(2);
        }
    };
    // Ohne diese Angabe würde Windows die Seite auf hochauflösenden Bildschirmen unscharf hochrechnen.
    // SAFETY: einmaliger Aufruf vor dem ersten Fenster.
    unsafe { windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(-4isize as _) };
    let out = link.sender.clone();
    let queue = Queue { q: Arc::new(Mutex::new(VecDeque::new())), tid: unsafe { GetCurrentThreadId() } };
    let reader = queue.clone();
    std::thread::spawn(move || loop {
        match link.recv() {
            Ok(m) => reader.push(Msg::Host(m)),
            Err(_) => {
                reader.push(Msg::Host(ToWorker::Shutdown));
                break;
            }
        }
    });
    let w = Worker::new(out, queue.clone());
    // Nachrichten nachholen, die vor dem ersten Nachrichtenlauf eintrafen.
    unsafe { PostThreadMessageW(queue.tid, WM_APP, 0, 0) };
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: gewöhnliche Nachrichtenschleife des Threads, der die WebView besitzt.
        let r = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if r <= 0 {
            break;
        }
        if msg.message == WM_APP && msg.hwnd.is_null() {
            while let Some(m) = queue.pop() {
                if !w.handle(m) {
                    w.close();
                    return;
                }
            }
            continue;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    w.close();
}

/// Das Ziel der Sitzung, sobald `ToWorker::Connect` da ist.
struct Target {
    url: WebUrl,
    profile: PathBuf,
    opts: WebOptions,
}

struct Inner {
    out: Sender,
    queue: Queue,
    target: RefCell<Option<Target>>,
    /// Kennung des Elternfensters (0: noch keines).
    embed: Cell<isize>,
    /// Unsichtbares Elternfenster (Felderkennung), wird beim Beenden zerstört.
    own_parent: Cell<isize>,
    bounds: Cell<(i32, i32, i32, i32)>,
    scale: Cell<u32>,
    visible: Cell<bool>,
    keys: RefCell<Vec<ReservedKey>>,
    view: RefCell<Option<Rc<WebView>>>,
    ctx: RefCell<Option<WebContext>>,
    flow: RefCell<LoginFlow>,
    creds: RefCell<Option<CredentialPayload>>,
    inject_pending: Cell<bool>,
    login_busy: Cell<bool>,
    // Anzeigezustand für `WebEvent::Status`
    title: RefCell<String>,
    uri: RefCell<String>,
    loading: Cell<bool>,
    error: RefCell<Option<String>>,
    /// Abbruch einer Navigation, die zum Download wurde (kein Fehler): erst melden, wenn kein Download folgt.
    pending_error: RefCell<Option<String>>,
    started: Instant,
    load_started: Cell<Option<Instant>>,
    trace_path: RefCell<Option<PathBuf>>,
    asks: RefCell<HashMap<u32, CertAsk>>,
    next_ask: Cell<u32>,
    next_download: Cell<u32>,
}

/// Zurückgestellte Zertifikatsentscheidung (Rückruf wartet auf die Antwort des Nutzers).
struct CertAsk {
    args: ICoreWebView2ServerCertificateErrorDetectedEventArgs,
    deferral: ICoreWebView2Deferral,
    host: String,
    der: Vec<u8>,
    profile: PathBuf,
}

#[derive(Clone)]
struct Worker(Rc<Inner>);

impl Worker {
    fn new(out: Sender, queue: Queue) -> Worker {
        Worker(Rc::new(Inner {
            out,
            queue,
            target: RefCell::new(None),
            embed: Cell::new(0),
            own_parent: Cell::new(0),
            bounds: Cell::new((0, 0, 800, 600)),
            scale: Cell::new(0),
            visible: Cell::new(true),
            keys: RefCell::new(Vec::new()),
            view: RefCell::new(None),
            ctx: RefCell::new(None),
            flow: RefCell::new(LoginFlow::new(None)),
            creds: RefCell::new(None),
            inject_pending: Cell::new(false),
            login_busy: Cell::new(false),
            title: RefCell::new(String::new()),
            uri: RefCell::new(String::new()),
            loading: Cell::new(false),
            error: RefCell::new(None),
            pending_error: RefCell::new(None),
            started: Instant::now(),
            load_started: Cell::new(None),
            trace_path: RefCell::new(None),
            asks: RefCell::new(HashMap::new()),
            next_ask: Cell::new(0),
            next_download: Cell::new(0),
        }))
    }

    fn send(&self, m: FromWorker) {
        let _ = self.0.out.send(&m);
    }

    fn event(&self, e: WebEvent) {
        self.send(FromWorker::Custom(e.to_bytes()));
    }

    fn view(&self) -> Option<Rc<WebView>> {
        self.0.view.borrow().clone()
    }

    /// Zeile ins Zeitprotokoll `protokoll.log` im Profilordner (nur Adresse ohne Anfrageteil).
    fn trace(&self, text: &str) {
        use std::io::Write;
        let Some(path) = self.0.trace_path.borrow().clone() else { return };
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "[+{:>8.3} s] {text}", self.0.started.elapsed().as_secs_f64());
        }
    }

    /// Verarbeitet eine Nachricht; `false` beendet den Worker.
    fn handle(&self, m: Msg) -> bool {
        match m {
            Msg::RetryLogin(script, tries) => self.run_login(script, tries),
            Msg::CheckRejected => self.check_rejected(),
            Msg::DebugShot(tag) => self.debug_capture(tag),
            Msg::PendingError => {
                if let Some(e) = self.0.pending_error.borrow_mut().take() {
                    *self.0.error.borrow_mut() = Some(e);
                    self.status();
                }
            }
            Msg::Detected(r) => {
                // WebView2 liefert das Ergebnis als JSON-Wert; das Skript gibt selbst JSON-Text zurück.
                let text: String = serde_json::from_str(&r).unwrap_or_default();
                let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let list = |k: &str| -> Vec<FieldCandidate> { parsed.get(k).and_then(|x| serde_json::from_value(x.clone()).ok()).unwrap_or_default() };
                self.event(WebEvent::Fields { users: list("users"), passwords: list("passwords"), submits: list("submits") });
            }
            Msg::Found(r) => {
                let text: String = serde_json::from_str(&r).unwrap_or_default();
                self.event(WebEvent::Found { found: text == "1" });
            }
            Msg::Rejected(r) => {
                let text: String = serde_json::from_str(&r).unwrap_or(r);
                self.trace(&format!("Prüfung nach dem Abschicken: {text}"));
                let action = self.0.flow.borrow_mut().rejected_check(&text);
                match action {
                    FlowAction::Notice(n) => self.send(FromWorker::Notice(n)),
                    // Hinweis steht noch: später erneut nachsehen (Anmeldung von Hand?).
                    FlowAction::CheckRejected => self.0.queue.push_after(Duration::from_secs(2), Msg::CheckRejected),
                    _ => {}
                }
            }
            Msg::RunStep(step, tries) => self.run_step(step, tries),
            Msg::StepResult { step, text, tries } => {
                let text: String = serde_json::from_str(&text).unwrap_or(text);
                if let Some((every, max)) = step_retry(&text) {
                    if tries < max {
                        self.0.queue.push_after(every, Msg::RunStep(step, tries + 1));
                        return true;
                    }
                }
                // Ergebnisse sind feste Kennungen (rj:…) ohne Eingabewerte.
                self.trace(&format!("Schritt {step:?} nach {tries} Wiederholungen: {}", text.chars().take(80).collect::<String>()));
                let next = self.0.flow.borrow_mut().step_result(step, &text);
                self.drive(next);
            }
            Msg::Typed(action) => {
                let next = self.0.flow.borrow_mut().typed(&action);
                self.drive(next);
            }
            Msg::LoginResult { text, script, tries } => {
                let text: String = serde_json::from_str(&text).unwrap_or(text);
                if text == RESULT_NO_PASSWORD_FIELD && tries < LOGIN_RETRIES {
                    self.0.queue.push_after(LOGIN_RETRY_EVERY, Msg::RetryLogin(script, tries + 1));
                    return true;
                }
                self.0.login_busy.set(false);
                // Ergebnisse sind feste Kennungen (rj:…) oder Meldungen ohne Eingabewerte.
                self.trace(&format!("Anmeldeskript nach {tries} Wiederholungen: {}", text.chars().take(80).collect::<String>()));
                if let FlowAction::Notice(n) = self.0.flow.borrow_mut().script_result(&text) {
                    self.send(FromWorker::Notice(n));
                }
            }
            Msg::Host(m) => match m {
                ToWorker::Shutdown => return false,
                ToWorker::Ping => self.send(FromWorker::Pong),
                ToWorker::Credentials(c) => {
                    *self.0.creds.borrow_mut() = Some(c);
                    if self.0.inject_pending.get() {
                        self.inject();
                    }
                }
                ToWorker::Connect(r) => {
                    if let Err(e) = self.open(r) {
                        self.send(FromWorker::State(SessionState::Failed { reason: e }));
                    }
                }
                ToWorker::Answer { question, accept } => self.answer(question, accept),
                // Größe, Eingaben und Zwischenablage regelt WebView2 selbst (Befehle Bounds/Embed, Systemzwischenablage).
                ToWorker::Resize { .. } | ToWorker::Input(_) | ToWorker::Clipboard(_) | ToWorker::Term(_) => {}
                ToWorker::Custom(bytes) => match serde_json::from_slice::<WebCommand>(&bytes) {
                    Ok(c) => self.command(c),
                    Err(e) => self.send(FromWorker::Log(format!("Unbekannter Befehl an den Web-Worker: {e}"))),
                },
            },
        }
        true
    }

    fn open(&self, r: ConnectRequest) -> Result<(), String> {
        let opts: WebOptions = if r.options_json.trim().is_empty() { WebOptions::default() } else { serde_json::from_str(&r.options_json).map_err(|e| format!("Optionen: {e}"))? };
        let url = start_url(&opts, &r.host, r.port).map_err(|e| e.to_string())?;
        if !url.https && !opts.allow_http {
            self.send(FromWorker::Notice("Achtung: unverschlüsselte Verbindung (HTTP).".into()));
        }
        let base = rj_core::config::data_dir().ok_or("kein Datenverzeichnis")?;
        let key = opts.profile.clone().unwrap_or_else(|| format!("{}-{}", url.host, url.port));
        let profile = profile_dir(&base, &key);
        std::fs::create_dir_all(&profile).map_err(|e| format!("Profil: {e}"))?;
        let trace = profile.join("protokoll.log");
        // Das Protokoll ist nur für die letzten Sitzungen gedacht.
        if std::fs::metadata(&trace).is_ok_and(|m| m.len() > 200_000) {
            let _ = std::fs::remove_file(&trace);
        }
        *self.0.trace_path.borrow_mut() = Some(trace);
        self.trace(&format!("—— Sitzung geöffnet: {} ——", url.host));
        *self.0.flow.borrow_mut() = LoginFlow::new(opts.login.clone());
        self.close_view();
        let headless = opts.headless;
        *self.0.target.borrow_mut() = Some(Target { url, profile, opts });
        self.send(FromWorker::State(SessionState::Connecting));
        if headless && self.0.embed.get() == 0 {
            let h = create_hidden_parent();
            self.0.own_parent.set(h);
            self.0.embed.set(h);
            self.0.bounds.set((0, 0, r.width.clamp(64, 4096) as i32, r.height.clamp(64, 4096) as i32));
        }
        self.try_create();
        Ok(())
    }

    /// Legt die Seite an, sobald Ziel und Elternfenster bekannt sind (in beliebiger Reihenfolge eingetroffen).
    fn try_create(&self) {
        if self.0.view.borrow().is_some() || self.0.embed.get() == 0 {
            return;
        }
        let (url, profile, ignore) = match self.0.target.borrow().as_ref() {
            Some(t) => (t.url.url.clone(), t.profile.clone(), t.opts.ignore_certificate),
            None => return,
        };
        let (x, y, w, h) = self.0.bounds.get();
        let mut ctx = WebContext::new(Some(profile.clone()));
        let built = WebViewBuilder::new_with_web_context(&mut ctx)
            .with_bounds(Rect {
                position: Position::Physical(PhysicalPosition::new(x, y)),
                size: Size::Physical(PhysicalSize::new(w.max(1) as u32, h.max(1) as u32)),
            })
            .with_devtools(false)
            .with_visible(self.0.visible.get())
            .with_url(&url)
            .build_as_child(&Parent(self.0.embed.get()));
        let view = match built {
            Ok(v) => Rc::new(v),
            Err(e) => {
                let hint = e.to_string();
                self.send(FromWorker::State(SessionState::Failed {
                    reason: format!("WebView2 konnte nicht gestartet werden (ist die WebView2-Laufzeit von Microsoft installiert?): {hint}"),
                }));
                return;
            }
        };
        *self.0.ctx.borrow_mut() = Some(ctx);
        self.apply_scale(&view);
        self.connect_events(&view, &profile, ignore);
        *self.0.view.borrow_mut() = Some(view);
        self.send(FromWorker::State(SessionState::Connected));
        let host = self.0.target.borrow().as_ref().map(|t| t.url.host.clone()).unwrap_or_default();
        self.send(FromWorker::Log(format!("Web-Sitzung geöffnet: {host}")));
        self.trace(&format!("Seite angelegt nach {} ms", self.0.started.elapsed().as_millis()));
    }

    /// Seite und Elternfenster lösen (neues Ziel, Beenden).
    fn close_view(&self) {
        self.0.view.borrow_mut().take();
        self.0.ctx.borrow_mut().take();
        self.0.asks.borrow_mut().clear();
    }

    fn close(&self) {
        self.close_view();
        let h = self.0.own_parent.replace(0);
        if h != 0 {
            // SAFETY: eigenes Fenster dieses Prozesses.
            unsafe { DestroyWindow(h as _) };
        }
    }

    fn apply_scale(&self, view: &WebView) {
        let pct = self.0.scale.get();
        if pct == 0 {
            return;
        }
        if let Ok(c3) = view.controller().cast::<ICoreWebView2Controller3>() {
            // SAFETY: COM-Aufrufe auf dem Thread, der die WebView besitzt.
            unsafe {
                let _ = c3.SetShouldDetectMonitorScaleChanges(false);
                let _ = c3.SetRasterizationScale(pct as f64 / 100.0);
            }
        }
    }

    fn connect_events(&self, view: &WebView, profile: &Path, ignore_cert: bool) {
        let ctl = view.controller();
        let core = view.webview();
        // Einstellungen: keine Entwicklerwerkzeuge, keine Angebote zum Speichern von Kennwörtern oder Formulardaten im Profil
        // (die Zugangsdaten liegen in der Schlüsseldatenbank).
        if let Ok(s) = unsafe { core.Settings() } {
            unsafe {
                let _ = s.SetAreDevToolsEnabled(false);
            }
            if let Ok(s4) = s.cast::<ICoreWebView2Settings4>() {
                unsafe {
                    let _ = s4.SetIsPasswordAutosaveEnabled(false);
                    let _ = s4.SetIsGeneralAutofillEnabled(false);
                }
            }
        }
        let mut tok = Default::default();

        // Navigation: Ladebeginn
        let me = self.clone();
        let h = NavigationStartingEventHandler::create(Box::new(move |_, args| {
            let uri = args.and_then(|a| uri_of(|p| unsafe { a.Uri(p) })).unwrap_or_default();
            me.0.loading.set(true);
            *me.0.error.borrow_mut() = None;
            me.0.load_started.set(Some(Instant::now()));
            me.trace(&format!("Ladebeginn: {}", trace_uri(&uri)));
            me.status();
            Ok(())
        }));
        unsafe { let _ = core.add_NavigationStarting(&h, &mut tok); };

        // Navigation: Ladeende oder Fehler
        let me = self.clone();
        let h = NavigationCompletedEventHandler::create(Box::new(move |sender, args| {
            let uri = sender.as_ref().and_then(|s| uri_of(|p| unsafe { s.Source(p) })).unwrap_or_default();
            let mut ok = BOOL(0);
            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS(0);
            if let Some(a) = args {
                unsafe {
                    let _ = a.IsSuccess(&mut ok);
                    let _ = a.WebErrorStatus(&mut status);
                }
            }
            me.0.loading.set(false);
            let dur = me.0.load_started.get().map(|t| format!(" nach {} ms", t.elapsed().as_millis())).unwrap_or_default();
            if !ok.as_bool() && status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                // Schlüssel des Dienstes: Rechner, bei anderem Port als 443 mit Port.
            let host = check_url(&uri).map(|u| AcceptedCerts::key(&u.host, u.port)).unwrap_or_default();
                let why = web_error_text(status);
                me.trace(&format!("Laden fehlgeschlagen: {host}: {why}"));
                let text = format!("{host}: {why}");
                if status == COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED {
                    // So endet auch eine Navigation, die zum Download wird: kurz abwarten, ob einer startet.
                    *me.0.pending_error.borrow_mut() = Some(text);
                    me.0.queue.push_after(Duration::from_millis(900), Msg::PendingError);
                } else {
                    *me.0.error.borrow_mut() = Some(text);
                }
                me.status();
                return Ok(());
            }
            me.trace(&format!("Ladeende{dur}: {}", trace_uri(&uri)));
            me.status();
            me.0.queue.push_after(Duration::from_millis(1500), Msg::DebugShot("geladen"));
            if ok.as_bool() {
                let action = me.0.flow.borrow_mut().page_loaded(&uri);
                match action {
                    FlowAction::InjectLogin => me.inject(),
                    FlowAction::Notice(n) => me.send(FromWorker::Notice(n)),
                    FlowAction::CheckRejected => me.0.queue.push_after(Duration::from_secs(2), Msg::CheckRejected),
                    // Schritte und Eingaben beginnen erst über `inject` (`LoginFlow::begin`).
                    FlowAction::None | FlowAction::Script(_) | FlowAction::Type(..) | FlowAction::PressEnter => {}
                }
            }
            Ok(())
        }));
        unsafe { let _ = core.add_NavigationCompleted(&h, &mut tok); };

        // Titel, Adresse, Zurück/Vor
        let me = self.clone();
        let h = DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(s) = sender {
                *me.0.title.borrow_mut() = uri_of(|p| unsafe { s.DocumentTitle(p) }).unwrap_or_default();
            }
            me.status();
            Ok(())
        }));
        unsafe { let _ = core.add_DocumentTitleChanged(&h, &mut tok); };
        let me = self.clone();
        let h = SourceChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(s) = sender {
                *me.0.uri.borrow_mut() = uri_of(|p| unsafe { s.Source(p) }).unwrap_or_default();
            }
            me.status();
            Ok(())
        }));
        unsafe { let _ = core.add_SourceChanged(&h, &mut tok); };
        let me = self.clone();
        let h = HistoryChangedEventHandler::create(Box::new(move |_, _| {
            me.status();
            Ok(())
        }));
        unsafe { let _ = core.add_HistoryChanged(&h, &mut tok); };

        // Neue Fenster (target=_blank, window.open) im selben Tab öffnen, wie unter Linux.
        let h = NewWindowRequestedEventHandler::create(Box::new(move |sender, args| {
            if let (Some(s), Some(a)) = (sender, args) {
                if let Some(u) = uri_of(|p| unsafe { a.Uri(p) }) {
                    unsafe {
                        let _ = a.SetHandled(true);
                        let _ = s.Navigate(&HSTRING::from(u));
                    }
                }
            }
            Ok(())
        }));
        unsafe { let _ = core.add_NewWindowRequested(&h, &mut tok); };

        // Tastatur: Kurzbefehle der Anwendung gehen nicht an die Seite, sondern ans Hauptfenster.
        let me = self.clone();
        let h = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let (mut kind, mut vk) = (COREWEBVIEW2_KEY_EVENT_KIND(0), 0u32);
            unsafe {
                let _ = args.KeyEventKind(&mut kind);
                let _ = args.VirtualKey(&mut vk);
            }
            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                return Ok(());
            }
            // Tatsächlicher Tastenzustand: Die Tasten gehen an den Browser-Prozess von WebView2, nicht an diesen
            // Faden; GetKeyState kennt Strg/Alt/Umschalt hier nicht (Strg+Bild↓ kam sonst nicht an).
            let down = |k: u16| unsafe { GetAsyncKeyState(k as i32) } < 0;
            let key = ReservedKey { vk, ctrl: down(VK_CONTROL), alt: down(VK_MENU), shift: down(VK_SHIFT) };
            if is_reserved(&me.0.keys.borrow(), &key) {
                unsafe {
                    let _ = args.SetHandled(true);
                }
                me.event(WebEvent::Key { vk, ctrl: key.ctrl, alt: key.alt, shift: key.shift });
            }
            Ok(())
        }));
        unsafe { let _ = ctl.add_AcceleratorKeyPressed(&h, &mut tok); };

        // Fokus: Die Seite hat ihn bekommen (Klick) oder gibt ihn mit Tab am Ende ab.
        let me = self.clone();
        let h = FocusChangedEventHandler::create(Box::new(move |_, _| {
            me.event(WebEvent::Focus { on: true, forward: true });
            Ok(())
        }));
        unsafe { let _ = ctl.add_GotFocus(&h, &mut tok); };
        let me = self.clone();
        let h = MoveFocusRequestedEventHandler::create(Box::new(move |_, args| {
            if let Some(a) = args {
                let mut reason = COREWEBVIEW2_MOVE_FOCUS_REASON(0);
                unsafe {
                    let _ = a.Reason(&mut reason);
                    let _ = a.SetHandled(true);
                }
                me.event(WebEvent::Focus { on: false, forward: reason != COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS });
            }
            Ok(())
        }));
        unsafe { let _ = ctl.add_MoveFocusRequested(&h, &mut tok); };

        self.connect_certificates(view, profile, ignore_cert);
        self.connect_downloads(view);
    }

    /// Zertifikatsfehler: gültig oder bekannt: durchlassen; sonst Rückruf anhalten und den Nutzer fragen.
    fn connect_certificates(&self, view: &WebView, profile: &Path, ignore: bool) {
        let Ok(w14) = view.webview().cast::<ICoreWebView2_14>() else {
            self.send(FromWorker::Log("Zertifikatsfragen nicht verfügbar (WebView2-Laufzeit zu alt)".into()));
            return;
        };
        let me = self.clone();
        let profile = profile.to_path_buf();
        let h = ServerCertificateErrorDetectedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let always = COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW;
            // „Zertifikatsprüfung abschalten“ (Option des Ziels): ohne Rückfrage durchlassen.
            if ignore {
                unsafe {
                    let _ = args.SetAction(always);
                }
                return Ok(());
            }
            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS(0);
            let uri = uri_of(|p| unsafe { args.RequestUri(p) }).unwrap_or_default();
            unsafe {
                let _ = args.ErrorStatus(&mut status);
            }
            // Schlüssel je Rechner und Port wie unter Linux (443 und 4343 eines APs haben verschiedene Zertifikate).
            let (host, port) = check_url(&uri).map(|u| (u.host, u.port)).unwrap_or_default();
            let key = AcceptedCerts::key(&host, port);
            let der = unsafe { args.ServerCertificate() }.ok().and_then(|c| certificate_der(&c)).unwrap_or_default();
            let known = AcceptedCerts::load(&profile);
            match known.decide(&key, &der, &[web_error_text(status)]) {
                CertDecision::Valid | CertDecision::Accepted => unsafe {
                    let _ = args.SetAction(always);
                },
                CertDecision::Ask { text, detail, fingerprint, previous } => {
                    let Ok(deferral) = (unsafe { args.GetDeferral() }) else {
                        unsafe {
                            let _ = args.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL);
                        }
                        return Ok(());
                    };
                    let id = me.0.next_ask.get().wrapping_add(1).max(1);
                    me.0.next_ask.set(id);
                    me.0.asks.borrow_mut().insert(id, CertAsk { args: args.clone(), deferral, host: key, der, profile: profile.clone() });
                    let info = rj_ipc::msg::SecurityInfo { kind: "certificate".into(), host, port, key_type: "X.509".into(), fingerprint, previous };
                    me.send(FromWorker::Question { id, text, detail, info });
                }
            }
            Ok(())
        }));
        let mut tok = Default::default();
        unsafe { let _ = w14.add_ServerCertificateErrorDetected(&h, &mut tok); };
    }

    /// Antwort auf die Zertifikatsfrage.
    fn answer(&self, id: u32, accept: bool) {
        let Some(a) = self.0.asks.borrow_mut().remove(&id) else { return };
        if accept {
            let mut known = AcceptedCerts::load(&a.profile);
            known.accept(&a.host, &a.der);
            let _ = known.save(&a.profile);
        }
        unsafe {
            let _ = a.args.SetAction(if accept { COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW } else { COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL });
            let _ = a.deferral.Complete();
        }
    }

    /// Downloads: in den Zielordner speichern, Fortschritt und Ergebnis melden; .ica-Dateien danach öffnen.
    fn connect_downloads(&self, view: &WebView) {
        // Alte Startdateien (Anmeldetickets) von früheren Läufen entfernen.
        for f in rj_web::stale_launch_files(&download_dir(), std::time::Duration::from_secs(300)) {
            let _ = std::fs::remove_file(f);
        }
        let Ok(w4) = view.webview().cast::<ICoreWebView2_4>() else { return };
        let me = self.clone();
        let h = DownloadStartingEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let suggested = uri_of(|p| unsafe { args.ResultFilePath(p) }).unwrap_or_default();
            let name = sanitize_file_name(Path::new(&suggested).file_name().and_then(|n| n.to_str()).unwrap_or("download"));
            let dir = download_dir();
            let id = me.0.next_download.get().wrapping_add(1).max(1);
            me.0.next_download.set(id);
            // Die Navigation war ein Download: kein Ladefehler.
            me.0.pending_error.borrow_mut().take();
            *me.0.error.borrow_mut() = None;
            me.status();
            let (dir_s, path) = match std::fs::create_dir_all(&dir).map(|_| unique_path(&dir, &name)) {
                Ok(p) => (dir.to_string_lossy().into_owned(), p),
                Err(e) => {
                    unsafe {
                        let _ = args.SetCancel(true);
                    }
                    me.download_event(id, "failed", &name, &dir.to_string_lossy(), 0, 0, Some(format!("Zielordner nicht anlegbar: {e}")), false);
                    return Ok(());
                }
            };
            let final_name = path.file_name().and_then(|n| n.to_str()).unwrap_or(&name).to_string();
            unsafe {
                let _ = args.SetResultFilePath(&HSTRING::from(path.to_string_lossy().as_ref()));
                // Keine eigene Download-Oberfläche von WebView2: die Meldungen laufen über remotejoe.
                let _ = args.SetHandled(true);
            }
            me.download_event(id, "started", &final_name, &dir_s, 0, 0, None, false);
            if let Ok(op) = unsafe { args.DownloadOperation() } {
                me.watch_download(id, op, final_name, dir_s, path);
            }
            Ok(())
        }));
        let mut tok = Default::default();
        unsafe { let _ = w4.add_DownloadStarting(&h, &mut tok); };
    }

    fn watch_download(&self, id: u32, op: ICoreWebView2DownloadOperation, name: String, dir: String, path: PathBuf) {
        let last = Rc::new(Cell::new(Instant::now() - DOWNLOAD_TICK));
        let (me, n, d, l) = (self.clone(), name.clone(), dir.clone(), last.clone());
        let h = BytesReceivedChangedEventHandler::create(Box::new(move |sender, _| {
            if l.get().elapsed() >= DOWNLOAD_TICK {
                l.set(Instant::now());
                if let Some(s) = sender {
                    let (r, t) = progress_of(&s);
                    me.download_event(id, "progress", &n, &d, r, t, None, false);
                }
            }
            Ok(())
        }));
        let mut tok = Default::default();
        unsafe { let _ = op.add_BytesReceivedChanged(&h, &mut tok); };
        let me = self.clone();
        let h = StateChangedEventHandler::create(Box::new(move |sender, _| {
            let Some(s) = sender else { return Ok(()) };
            let mut state = COREWEBVIEW2_DOWNLOAD_STATE(0);
            unsafe {
                let _ = s.State(&mut state);
            }
            let (r, t) = progress_of(&s);
            if state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
                let opened = if is_auto_open(&name) { me.open_downloaded(&path) } else { Ok(false) };
                // Geöffnete Startdatei nach einer Minute löschen: das Programm (z. B. Citrix) hat sie dann gelesen,
                // und das Anmeldeticket darin bleibt nicht liegen.
                if opened == Ok(true) {
                    let p = path.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(60));
                        let _ = std::fs::remove_file(p);
                    });
                }
                match opened {
                    Ok(o) => me.download_event(id, "done", &name, &dir, r.max(t), t, None, o),
                    Err(e) => me.download_event(id, "done", &name, &dir, r.max(t), t, Some(e), false),
                }
            } else if state == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED {
                let mut why = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON(0);
                unsafe {
                    let _ = s.InterruptReason(&mut why);
                }
                me.download_event(id, "failed", &name, &dir, r, t, Some(format!("Download abgebrochen (Grund {})", why.0)), false);
            }
            Ok(())
        }));
        unsafe { let _ = op.add_StateChanged(&h, &mut tok); };
    }

    #[allow(clippy::too_many_arguments)]
    fn download_event(&self, id: u32, state: &str, name: &str, dir: &str, received: u64, total: u64, error: Option<String>, opened: bool) {
        self.trace(&format!("Download {id} {state}: {name}"));
        self.event(WebEvent::Download { id, state: state.into(), name: name.into(), dir: dir.into(), received, total, error, opened });
    }

    /// Öffnet eine heruntergeladene Startdatei mit dem zugeordneten Programm (Umgebungsvariable `RJ_ICA_OPENER`
    /// nennt für Tests ein anderes Programm). `Ok(false)`: nichts zu öffnen.
    fn open_downloaded(&self, path: &Path) -> Result<bool, String> {
        if let Some(opener) = std::env::var_os("RJ_ICA_OPENER").filter(|o| !o.is_empty()) {
            return std::process::Command::new(opener).arg(path).spawn().map(|_| true).map_err(|e| format!("Programm nicht startbar: {e}"));
        }
        let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        // Ohne zugeordnetes Programm würde Windows „Wie soll diese Datei geöffnet werden?“ mit dem Store zeigen:
        // vorher prüfen und nur den Hinweis melden.
        let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        if !has_open_command(&ext) {
            return Err("Dem Dateityp ist kein Programm zugeordnet (z. B. Citrix Workspace installieren)".into());
        }
        // SAFETY: nullterminierte Zeichenketten leben über den Aufruf.
        let r = unsafe { windows_sys::Win32::UI::Shell::ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), wide.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        if (r as usize) > 32 {
            Ok(true)
        } else if (r as usize) == 31 {
            Err("Dem Dateityp ist kein Programm zugeordnet (z. B. Citrix Workspace installieren)".into())
        } else {
            Err(format!("Datei nicht öffenbar (Code {})", r as usize))
        }
    }

    /// Diagnose für Prüfläufe: Die gerenderte Seite (nicht das Fenster) als PNG in den Ordner `RJ_WEB_SHOTDIR`.
    /// Ohne die Umgebungsvariable geschieht nichts.
    fn debug_capture(&self, tag: &str) {
        use windows::Win32::System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};
        let Some(dir) = std::env::var_os("RJ_WEB_SHOTDIR") else { return };
        let Some(v) = self.view() else { return };
        let Some(stream) = (unsafe { windows::Win32::UI::Shell::SHCreateMemStream(None) }) else { return };
        let path = PathBuf::from(dir).join(format!("{}-{tag}-{}.png", std::process::id(), self.0.started.elapsed().as_secs()));
        let s2 = stream.clone();
        let h = webview2_com::CapturePreviewCompletedHandler::create(Box::new(move |_| {
            // SAFETY: Datenstrom im Speicher, nur in diesem Rückruf gelesen.
            unsafe {
                let mut stat = STATSTG::default();
                s2.Stat(&mut stat, STATFLAG_NONAME)?;
                let len = stat.cbSize as usize;
                s2.Seek(0, STREAM_SEEK_SET, None)?;
                let mut buf = vec![0u8; len];
                let mut read = 0u32;
                s2.Read(buf.as_mut_ptr() as *mut _, len as u32, Some(&mut read)).ok()?;
                let _ = std::fs::write(&path, &buf[..read as usize]);
            }
            Ok(())
        }));
        unsafe {
            let _ = v.webview().CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &h);
        }
    }

    fn status(&self) {
        let (back, forward) = match self.view() {
            Some(v) => {
                let (mut b, mut f) = (BOOL(0), BOOL(0));
                unsafe {
                    let _ = v.webview().CanGoBack(&mut b);
                    let _ = v.webview().CanGoForward(&mut f);
                }
                (b.as_bool(), f.as_bool())
            }
            None => (false, false),
        };
        let loading = self.0.loading.get();
        self.event(WebEvent::Status {
            title: rj_core::charset::mark_visible(&self.0.title.borrow(), false).into_owned(),
            uri: self.0.uri.borrow().clone(),
            loading,
            progress: if loading { 0 } else { 100 },
            error: self.0.error.borrow().clone(),
            back,
            forward,
        });
    }

    fn command(&self, c: WebCommand) {
        match c {
            WebCommand::Embed { hwnd } => {
                // Das Hauptfenster meldet die Kennung bei jedem Anzeigen erneut; nur bei einem Wechsel umsetzen
                // (Umsetzen in dasselbe Elternfenster lässt die Seite leer oder veraltet stehen).
                let old = self.0.embed.replace(hwnd as isize);
                if old == hwnd as isize && self.view().is_some() {
                    return;
                }
                match self.view() {
                    Some(v) if hwnd != 0 => {
                        if let Err(e) = v.reparent(hwnd as isize) {
                            self.send(FromWorker::Log(format!("Seite nicht umsetzbar: {e}")));
                        }
                    }
                    _ => self.try_create(),
                }
            }
            WebCommand::Bounds { x, y, w, h, scale } => {
                self.0.bounds.set((x, y, w.max(1), h.max(1)));
                if scale > 0 && scale != self.0.scale.get() {
                    self.0.scale.set(scale);
                    if let Some(v) = self.view() {
                        self.apply_scale(&v);
                    }
                }
                if let Some(v) = self.view() {
                    let _ = v.set_bounds(Rect {
                        position: Position::Physical(PhysicalPosition::new(x, y)),
                        size: Size::Physical(PhysicalSize::new(w.max(1) as u32, h.max(1) as u32)),
                    });
                }
            }
            WebCommand::Visible { on } => {
                self.0.visible.set(on);
                if let Some(v) = self.view() {
                    let _ = v.set_visible(on);
                }
                if on {
                    self.0.queue.push_after(Duration::from_millis(800), Msg::DebugShot("gezeigt"));
                }
            }
            WebCommand::Keys { keys } => *self.0.keys.borrow_mut() = keys,
            WebCommand::Focus { on } => {
                if let Some(v) = self.view() {
                    let _ = if on { v.focus() } else { v.focus_parent() };
                }
            }
            WebCommand::Nav { action } => {
                let Some(v) = self.view() else { return };
                let core = v.webview();
                unsafe {
                    let _ = match action {
                        NavAction::Back => core.GoBack(),
                        NavAction::Forward => core.GoForward(),
                        NavAction::Reload => core.Reload(),
                        NavAction::Stop => core.Stop(),
                    };
                }
            }
            WebCommand::Edit { action } => {
                // Kopieren, Ausschneiden und Alles auswählen laufen als Bearbeitungsbefehl der Seite; Einfügen übernimmt
                // die Seite selbst (Strg+V mit der Systemzwischenablage).
                let cmd = match action {
                    EditAction::Copy => "copy",
                    EditAction::Cut => "cut",
                    EditAction::SelectAll => "selectAll",
                    EditAction::Paste => return,
                };
                if let Some(v) = self.view() {
                    let _ = v.evaluate_script(&format!("document.execCommand('{cmd}')"));
                }
            }
            WebCommand::Detect => {
                let Some(v) = self.view() else { return };
                let q = self.0.queue.clone();
                let _ = v.evaluate_script_with_callback(DETECT_SCRIPT, move |r| q.push(Msg::Detected(r)));
            }
            WebCommand::Find { text, backwards, restart } => {
                let Some(v) = self.view() else { return };
                let q = self.0.queue.clone();
                let _ = v.evaluate_script_with_callback(&rj_web::find_script(&text, backwards, restart), move |r| q.push(Msg::Found(r)));
            }
            // Das Menü der Seite zeigt WebView2 selbst (Kontextmenü, Auswahllisten).
            WebCommand::Menu { .. } => {}
        }
    }

    fn inject(&self) {
        let (Some(_), creds) = (self.view(), self.0.creds.borrow().clone()) else {
            self.0.inject_pending.set(true);
            return;
        };
        let Some(c) = creds else {
            self.0.inject_pending.set(true);
            return;
        };
        self.0.inject_pending.set(false);
        // Ohne Zugangsdaten gibt es nichts einzutragen (z. B. beim Erkennen der Felder).
        if c.username.is_empty() && c.password.0.is_empty() {
            return;
        }
        let Some(login) = self.0.flow.borrow().login().cloned() else { return };
        if self.0.login_busy.replace(true) {
            self.trace("Anmeldeskript läuft bereits: zweiter Lauf übersprungen");
            return;
        }
        // Anmeldung wie von Hand (Schritte mit echter Eingabe); eigenes Skript: bisheriger Weg.
        let first = self.0.flow.borrow_mut().begin();
        match first {
            FlowAction::InjectLogin => self.legacy_login(&login),
            other => {
                self.trace("Anmeldung in Schritten gestartet");
                self.drive(other);
            }
        }
    }

    /// Benutzername (mit Domäne) und Kennwort der Sitzung.
    fn login_strings(&self) -> Option<(String, zeroize::Zeroizing<String>)> {
        let c = self.0.creds.borrow().clone()?;
        let user = match &c.domain {
            Some(d) if !d.is_empty() && !c.username.contains('@') && !c.username.contains('\\') => format!("{d}\\{}", c.username),
            _ => c.username.clone(),
        };
        let pw = zeroize::Zeroizing::new(String::from_utf8_lossy(&c.password.0).to_string());
        Some((user, pw))
    }

    /// Nächste Aktion der Anmeldung ausführen.
    fn drive(&self, action: FlowAction) {
        match action {
            FlowAction::InjectLogin => {
                let Some(login) = self.0.flow.borrow().login().cloned() else { return self.0.login_busy.set(false) };
                self.send(FromWorker::Notice("Eingabe wie von Hand kam auf dieser Seite nicht an; es wird der bisherige Weg versucht.".into()));
                self.legacy_login(&login);
            }
            FlowAction::Script(step) => self.run_step(step, 0),
            FlowAction::Type(field, mode) => {
                let Some((user, pw)) = self.login_strings() else { return self.0.login_busy.set(false) };
                let text = match field {
                    Field::User => zeroize::Zeroizing::new(user),
                    Field::Password => pw,
                };
                // Kurz warten, damit die Seite die Eingabe verarbeitet hat; schlägt das Tippen fehl,
                // fällt die Längenprüfung danach auf (zweiter Durchgang Taste für Taste, dann alter Weg).
                self.type_text(&text, mode, Msg::Typed(action));
            }
            FlowAction::PressEnter => {
                let enter = |kind: &str, text: &str| {
                    zeroize::Zeroizing::new(format!(
                        "{{\"type\":\"{kind}\",\"key\":\"Enter\",\"code\":\"Enter\",\"windowsVirtualKeyCode\":13,\"nativeVirtualKeyCode\":13{text}}}"
                    ))
                };
                self.cdp("Input.dispatchKeyEvent", &enter("keyDown", ",\"text\":\"\\r\",\"unmodifiedText\":\"\\r\""), None);
                self.cdp("Input.dispatchKeyEvent", &enter("keyUp", ""), Some(Msg::Typed(action)));
            }
            FlowAction::Notice(n) => {
                self.0.login_busy.set(false);
                self.send(FromWorker::Notice(n));
            }
            FlowAction::CheckRejected => {
                self.0.login_busy.set(false);
                self.0.queue.push_after(Duration::from_secs(2), Msg::CheckRejected);
            }
            FlowAction::None => self.0.login_busy.set(false),
        }
    }

    /// Text als echte Eingabe in das fokussierte Feld (DevTools-Protokoll von WebView2): eingefügt wie von einer
    /// Eingabehilfe (`Input.insertText`) oder Taste für Taste (`Input.dispatchKeyEvent`). `then` folgt 150 ms nach
    /// der letzten Antwort. Der Text steht nur in überschriebenem Speicher, nie im Skript der Seite.
    fn type_text(&self, text: &str, mode: TypeMode, then: Msg) {
        let then = Some(then);
        match mode {
            TypeMode::Insert => {
                let mut p = zeroize::Zeroizing::new(String::from("{\"text\":"));
                json_quote(&mut p, text);
                p.push('}');
                self.cdp("Input.insertText", &p, then);
            }
            TypeMode::Keys => {
                let chars: Vec<char> = text.chars().collect();
                if chars.is_empty() {
                    if let Some(m) = then {
                        self.0.queue.push(m);
                    }
                    return;
                }
                let mut then = then;
                for (i, c) in chars.iter().enumerate() {
                    let mut ch = zeroize::Zeroizing::new(String::new());
                    json_quote(&mut ch, c.encode_utf8(&mut [0u8; 4]));
                    let mut down = zeroize::Zeroizing::new(String::from("{\"type\":\"keyDown\",\"text\":"));
                    down.push_str(&ch);
                    down.push_str(",\"unmodifiedText\":");
                    down.push_str(&ch);
                    down.push_str(",\"key\":");
                    down.push_str(&ch);
                    down.push('}');
                    self.cdp("Input.dispatchKeyEvent", &down, None);
                    let mut up = zeroize::Zeroizing::new(String::from("{\"type\":\"keyUp\",\"key\":"));
                    up.push_str(&ch);
                    up.push('}');
                    let last = i + 1 == chars.len();
                    self.cdp("Input.dispatchKeyEvent", &up, if last { then.take() } else { None });
                }
            }
        }
    }

    /// Ruft eine Methode des DevTools-Protokolls auf; `done` kommt 150 ms nach der Antwort in die Warteschlange
    /// (auch bei einem Fehler: die Längenprüfung danach zeigt, ob die Eingabe ankam).
    fn cdp(&self, method: &str, params: &zeroize::Zeroizing<String>, done: Option<Msg>) {
        let q = self.0.queue.clone();
        let Some(v) = self.view() else {
            if let Some(m) = done {
                q.push(m);
            }
            return;
        };
        let core = v.webview();
        let m: Vec<u16> = method.encode_utf16().chain(Some(0)).collect();
        let p = zeroize::Zeroizing::new(params.encode_utf16().chain(Some(0)).collect::<Vec<u16>>());
        let pending = std::rc::Rc::new(RefCell::new(done));
        let in_handler = pending.clone();
        let h = webview2_com::CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |_, _| {
            if let Some(m) = in_handler.borrow_mut().take() {
                q.push_after(Duration::from_millis(150), m);
            }
            Ok(())
        }));
        // SAFETY: nullterminierte Zeichenketten leben über den Aufruf; WebView2 kopiert die Parameter.
        let ok = unsafe { core.CallDevToolsProtocolMethod(windows::core::PCWSTR(m.as_ptr()), windows::core::PCWSTR(p.as_ptr()), &h) }.is_ok();
        if !ok {
            if let Some(m) = pending.borrow_mut().take() {
                self.0.queue.push(m);
            }
        }
    }

    /// Schritt-Skript ausführen; „noch nicht so weit“ wiederholt `Msg::StepResult` (`step_retry`).
    fn run_step(&self, step: Step, tries: u32) {
        let Some(v) = self.view() else { return self.0.login_busy.set(false) };
        let Some(login) = self.0.flow.borrow().login().cloned() else { return self.0.login_busy.set(false) };
        let (ulen, plen) = self.login_strings().map(|(u, p)| (u.chars().count(), p.chars().count())).unwrap_or((0, 0));
        let script = step_script(&login, step, ulen, plen);
        let q = self.0.queue.clone();
        if v.evaluate_script_with_callback(&script, move |text| q.push(Msg::StepResult { step, text, tries })).is_err() {
            self.0.login_busy.set(false);
        }
    }

    /// Bisheriger Weg: Werte per Skript eintragen (eigene Skripte, Rückfall).
    fn legacy_login(&self, login: &FormLogin) {
        let Some((user, pw)) = self.login_strings() else { return self.0.login_busy.set(false) };
        let pw = Secret::from_bytes(pw.as_bytes().to_vec());
        match login_script(login, &user, &pw) {
            Ok(script) => {
                self.trace("Anmeldeskript gestartet");
                self.run_login(script, 0)
            }
            Err(e) => {
                self.0.login_busy.set(false);
                self.send(FromWorker::Notice(e.to_string()))
            }
        }
    }

    /// Zwei Sekunden nach dem Laden prüfen, ob nach dem Abschicken wieder ein Kennwortfeld da ist.
    fn check_rejected(&self) {
        let Some(v) = self.view() else { return };
        let q = self.0.queue.clone();
        let _ = v.evaluate_script_with_callback(&presence_script(), move |r| q.push(Msg::Rejected(r)));
    }

    /// Führt das Anmeldeskript aus. Seiten, die ihr Formular erst nach dem Laden aufbauen, brauchen
    /// Zeit: Wird kein Kennwortfeld gefunden, wird alle 500 ms erneut gesucht, höchstens 10 Sekunden.
    fn run_login(&self, script: zeroize::Zeroizing<String>, tries: u32) {
        let Some(v) = self.view() else {
            self.0.login_busy.set(false);
            return;
        };
        let q = self.0.queue.clone();
        let again = script.clone();
        let started = v.evaluate_script_with_callback(&script, move |text| q.push(Msg::LoginResult { text, script: again.clone(), tries }));
        if started.is_err() {
            self.0.login_busy.set(false);
        }
    }
}

/// Text aus einem COM-Aufruf, der eine zugewiesene Zeichenkette zurückgibt (wird freigegeben).
fn uri_of(f: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> Option<String> {
    let mut p = PWSTR::null();
    f(&mut p).ok()?;
    Some(take_pwstr(p))
}

/// Adresse für das Protokoll: ohne Anfrageteil und Anker (können Kennungen enthalten).
fn trace_uri(u: &str) -> String {
    u.split(['?', '#']).next().unwrap_or("").to_string()
}

/// Lesbare Ursache eines Ladefehlers.
fn web_error_text(s: COREWEBVIEW2_WEB_ERROR_STATUS) -> String {
    let t = match s {
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT => "Name passt nicht",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED => "abgelaufen",
        COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS => "Clientzertifikat fehlerhaft",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED => "widerrufen",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID => "ungültig (z. B. unbekannte Zertifizierungsstelle)",
        COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => "Server nicht erreichbar",
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => "Zeitüberschreitung",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET => "Verbindung abgebrochen",
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => "keine Netzwerkverbindung",
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT => "Verbindung nicht möglich",
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => "Name nicht auflösbar",
        COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED => "Weiterleitung fehlgeschlagen",
        _ => return format!("Fehler {}", s.0),
    };
    t.to_string()
}

/// DER-Bytes eines Zertifikats (WebView2 liefert PEM).
fn certificate_der(c: &ICoreWebView2Certificate) -> Option<Vec<u8>> {
    use base64::Engine;
    let text = uri_of(|p| unsafe { c.ToPemEncoding(p) })?;
    let body: String = text.lines().filter(|l| !l.starts_with("-----")).collect::<Vec<_>>().concat();
    let body: String = body.chars().filter(|ch| !ch.is_whitespace()).collect();
    Some(base64::engine::general_purpose::STANDARD.decode(body.as_bytes()).unwrap_or_else(|_| text.into_bytes()))
}

fn progress_of(op: &ICoreWebView2DownloadOperation) -> (u64, u64) {
    let (mut r, mut t) = (0i64, 0i64);
    unsafe {
        let _ = op.BytesReceived(&mut r);
        let _ = op.TotalBytesToReceive(&mut t);
    }
    (r.max(0) as u64, t.max(0) as u64)
}

/// Zielordner der Downloads: `downloads` im Datenverzeichnis des Programms.
fn download_dir() -> PathBuf {
    rj_core::config::data_dir().unwrap_or_else(std::env::temp_dir).join("downloads")
}

/// Dateiname ohne Pfad und ohne Zeichen, die unter Windows nicht erlaubt sind; keine reservierten Gerätenamen.
fn sanitize_file_name(name: &str) -> String {
    // Auch Formatzeichen (Umkehr der Schreibrichtung u. ä.) ersetzen: ein Name mit U+202E kann sonst eine andere Endung vortäuschen.
    let hidden = |c: char| matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}');
    let cleaned: String = name.chars().map(|c| if c.is_control() || hidden(c) || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c == ' ').to_string();
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    if cleaned.is_empty() || reserved {
        format!("download_{cleaned}")
    } else {
        cleaned.chars().take(150).collect()
    }
}

/// Freier Dateiname im Ordner: bei Namensgleichheit „name (1).ext“.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (1..10_000).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Tasten, die immer dem Hauptprogramm gehören, auch wenn es sie (noch) nicht als Kurzbefehl meldet:
/// Strg+Umschalt+W schließt den Tab, nie die Seite (Strg+W bleibt bei der Seite).
const ALWAYS_RESERVED: [ReservedKey; 1] = [ReservedKey { vk: 0x57, ctrl: true, alt: false, shift: true }];

/// Geht die Taste ans Hauptprogramm statt an die Seite?
fn is_reserved(keys: &[ReservedKey], key: &ReservedKey) -> bool {
    ALWAYS_RESERVED.contains(key) || keys.contains(key)
}

/// Hängt `s` als JSON-Zeichenkette (mit Anführungszeichen) an `out` an, ohne Zwischenkopie (Kennwörter).
fn json_quote(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn is_auto_open(name: &str) -> bool {
    Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(AUTO_OPEN_EXT))
}

/// Hat die Endung (z. B. `.ica`) ein Programm zum Öffnen? Unbekannte Typen zählen nicht (sonst „Öffnen mit“).
fn has_open_command(ext: &str) -> bool {
    use windows_sys::Win32::UI::Shell::{AssocQueryStringW, ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_COMMAND};
    if ext.len() < 2 {
        return false;
    }
    let assoc: Vec<u16> = ext.encode_utf16().chain(Some(0)).collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let mut len = 0u32;
    // SAFETY: nullterminierte Zeichenketten; ohne Ausgabepuffer liefert der Aufruf nur die Länge.
    let hr = unsafe { AssocQueryStringW(ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_COMMAND, assoc.as_ptr(), verb.as_ptr(), std::ptr::null_mut(), &mut len) };
    // S_FALSE (1) = Länge geliefert.
    (hr == 0 || hr == 1) && len > 1
}

/// Unsichtbares Elternfenster für Sitzungen ohne Oberfläche (Felderkennung).
fn create_hidden_parent() -> isize {
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    // SAFETY: gewöhnlicher Aufruf; das Fenster wird nie gezeigt und beim Beenden zerstört.
    unsafe { CreateWindowExW(0, class.as_ptr(), std::ptr::null(), WS_POPUP, 0, 0, 1024, 768, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null()) as isize }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), ".._.._etc_passwd".trim_matches('.'));
        assert_eq!(sanitize_file_name("a:b*c?.ica"), "a_b_c_.ica");
        assert_eq!(sanitize_file_name("CON.txt"), "download_CON.txt");
        assert_eq!(sanitize_file_name("COM1"), "download_COM1");
        assert_eq!(sanitize_file_name("  .. "), "download_");
        assert_eq!(sanitize_file_name("Zugang\u{202e}.ica").chars().filter(|c| *c == '\u{202e}').count(), 0, "Steuerzeichen entfernt");
        assert!(sanitize_file_name(&"x".repeat(400)).len() <= 150);
    }

    #[test]
    fn unique_names_do_not_overwrite() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(d.path(), "a.ica"), d.path().join("a.ica"));
        std::fs::write(d.path().join("a.ica"), b"1").unwrap();
        assert_eq!(unique_path(d.path(), "a.ica"), d.path().join("a (1).ica"));
        std::fs::write(d.path().join("a (1).ica"), b"1").unwrap();
        assert_eq!(unique_path(d.path(), "a.ica"), d.path().join("a (2).ica"));
        std::fs::write(d.path().join("rohdatei"), b"1").unwrap();
        assert_eq!(unique_path(d.path(), "rohdatei"), d.path().join("rohdatei (1)"));
    }

    #[test]
    fn ctrl_shift_w_always_goes_to_the_main_program() {
        let ctrl_shift_w = ReservedKey { vk: 0x57, ctrl: true, shift: true, ..Default::default() };
        assert!(is_reserved(&[], &ctrl_shift_w), "auch ohne gemeldete Kurzbefehle");
        assert!(!is_reserved(&[], &ReservedKey { vk: 0x57, ..Default::default() }), "W allein gehört der Seite");
        assert!(!is_reserved(&[], &ReservedKey { vk: 0x57, ctrl: true, ..Default::default() }), "Strg+W gehört der Seite");
        let ctrl_pgdn = ReservedKey { vk: 0x22, ctrl: true, ..Default::default() };
        assert!(!is_reserved(&[], &ctrl_pgdn));
        assert!(is_reserved(&[ctrl_pgdn], &ctrl_pgdn));
    }

    #[test]
    fn json_quote_round_trips_special_characters() {
        for s in ["einfach", "mit \"Anführung\" und \\ Schrägstrich", "Zeile\nneu\tTab\r", "Steuer\u{1}zeichen", "Ünïcödé €😀", "\u{2028}"] {
            let mut out = String::new();
            json_quote(&mut out, s);
            assert_eq!(serde_json::from_str::<String>(&out).unwrap(), s, "{out}");
        }
    }

    #[test]
    fn only_ica_opens_automatically() {
        assert!(is_auto_open("start.ica") && is_auto_open("START.ICA"));
        assert!(!is_auto_open("start.ica.exe") && !is_auto_open("start.rdp") && !is_auto_open("ica") && !is_auto_open("start.icax"));
    }
}
