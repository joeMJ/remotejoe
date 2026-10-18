//! WebKitGTK in einem `GtkOffscreenWindow`: kein sichtbares Fenster, das
//! Bild geht über den versiegelten geteilten Speicher an das Hauptfenster.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{cairo, gdk, glib};
use javascriptcore::ValueExt;
use rj_core::Secret;
use rj_ipc::shm::FrameBufferWriter;
use rj_ipc::worker::{Sender, WorkerLink};
use rj_ipc::*;
use rj_web::*;
use webkit2gtk::{
    CookieManagerExt, CookiePersistentStorage, HardwareAccelerationPolicy, HitTestResultExt, LoadEvent, OptionMenu, SettingsExt,
    WebContext, WebContextExt, WebView, WebViewExt, WebsiteDataManager, WebsiteDataManagerExt,
};

/// Anmeldung: Suche nach dem Kennwortfeld wiederholen (Abstand und Anzahl; zusammen 10 Sekunden).
const LOGIN_RETRY_EVERY: Duration = Duration::from_millis(500);
const LOGIN_RETRIES: u32 = 20;

/// Takt, in dem geänderte Bereiche in den Bildspeicher kopiert werden (höchstens 60 Bilder/s).
const RENDER_TICK: Duration = Duration::from_millis(16);
/// Größenänderungen erst nach dieser Pause übernehmen.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(120);
/// WebKit stellt Maus- und Tastenereignisse in getrennte Warteschlangen und
/// führt Skripte und Bearbeiten-Befehle sofort aus. Damit die Reihenfolge der
/// Eingaben erhalten bleibt, warten Tasten kurz nach einem Mausklick und
/// Bearbeiten-Befehle kurz nach der letzten Taste.
const KEY_AFTER_CLICK: Duration = Duration::from_millis(60);
const EDIT_AFTER_KEY: Duration = Duration::from_millis(120);
/// Höchstgröße des Textes, den die Oberfläche zum Einfügen anbietet.
const PASTE_LIMIT: usize = 16 * 1024 * 1024;

pub fn run() {
    let mut link = match WorkerLink::stdio("web", Duration::from_secs(2)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Kanal nicht verfügbar: {e}");
            std::process::exit(2);
        }
    };
    let out = link.sender.clone();
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
    if let Err(e) = gtk::init() {
        let _ = out.send(&FromWorker::State(SessionState::Failed { reason: format!("Grafikumgebung nicht verfügbar: {e}") }));
        std::process::exit(3);
    }
    let w = Worker::new(out);
    let w2 = w.clone();
    glib::timeout_add_local(Duration::from_millis(4), move || {
        w2.drain();
        while let Ok(m) = rx.try_recv() {
            if !w2.handle(m) {
                gtk::main_quit();
                return glib::ControlFlow::Break;
            }
        }
        glib::ControlFlow::Continue
    });
    let w3 = w.clone();
    glib::timeout_add_local(RENDER_TICK, move || {
        w3.tick();
        glib::ControlFlow::Continue
    });
    gtk::main();
}

/// Sichtbarer Teil der Sitzung: Fenster ohne Bildschirm und WebView.
struct Page {
    window: gtk::OffscreenWindow,
    view: WebView,
    tls: tls::Pending,
}

#[derive(Default)]
struct Frame {
    fb: Option<FrameBufferWriter>,
    /// Zwischenbild, in das der geänderte Bereich gezeichnet wird.
    scratch: Option<cairo::ImageSurface>,
    serial: u64,
    damage: Damage,
    /// Erstes Bild gemeldet (Messung).
    first: bool,
}

struct Inner {
    out: Sender,
    page: RefCell<Option<Rc<Page>>>,
    frame: RefCell<Frame>,
    flow: RefCell<LoginFlow>,
    creds: RefCell<Option<CredentialPayload>>,
    inject_pending: Cell<bool>,
    ignore_cert: Cell<bool>,
    mods: Cell<Modifiers>,
    buttons: Cell<u32>,
    pointer: Cell<(f64, f64)>,
    pending_size: Cell<Option<(i32, i32, Instant)>>,
    /// Von der Oberfläche angebotener Text zum Einfügen.
    offered: RefCell<String>,
    menus: RefCell<HashMap<u32, OptionMenu>>,
    next_menu: Cell<u32>,
    last_error: RefCell<Option<String>>,
    started: Instant,
    /// Zeitprotokoll der Sitzung (Seitenwechsel, Skriptläufe; nie Eingaben oder Kennwörter).
    trace_path: RefCell<Option<std::path::PathBuf>>,
    load_started: Cell<Option<Instant>>,
    /// Ein Anmeldeskript läuft (oder wartet auf das Formular): kein zweiter Lauf, kein doppeltes Abschicken.
    login_busy: Cell<bool>,
    /// Zurückgestellte Eingaben (Zeitpunkt, frühestens ausführen).
    deferred: RefCell<VecDeque<(Instant, Deferred)>>,
    last_click: Cell<Option<Instant>>,
    last_key: Cell<Option<Instant>>,
}

enum Deferred {
    Input(InputEvent),
    Edit(EditAction),
}

#[derive(Clone)]
struct Worker(Rc<Inner>);

impl Worker {
    fn new(out: Sender) -> Worker {
        Worker(Rc::new(Inner {
            out,
            page: RefCell::new(None),
            frame: RefCell::new(Frame::default()),
            flow: RefCell::new(LoginFlow::new(None)),
            creds: RefCell::new(None),
            inject_pending: Cell::new(false),
            ignore_cert: Cell::new(false),
            mods: Cell::new(Modifiers::default()),
            buttons: Cell::new(0),
            pointer: Cell::new((0.0, 0.0)),
            pending_size: Cell::new(None),
            offered: RefCell::new(String::new()),
            menus: RefCell::new(HashMap::new()),
            next_menu: Cell::new(1),
            last_error: RefCell::new(None),
            started: Instant::now(),
            trace_path: RefCell::new(None),
            load_started: Cell::new(None),
            login_busy: Cell::new(false),
            deferred: RefCell::new(VecDeque::new()),
            last_click: Cell::new(None),
            last_key: Cell::new(None),
        }))
    }

    /// Zeile ins Zeitprotokoll `protokoll.log` im Profilordner (nur Adresse ohne Anfrageteil).
    fn trace(&self, text: &str) {
        use std::io::Write;
        let Some(path) = self.0.trace_path.borrow().clone() else { return };
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "[+{:>8.3} s] {text}", self.0.started.elapsed().as_secs_f64());
        }
    }

    fn send(&self, m: FromWorker) {
        let _ = self.0.out.send(&m);
    }

    fn page(&self) -> Option<Rc<Page>> {
        self.0.page.borrow().clone()
    }

    /// Verarbeitet eine Nachricht; `false` beendet den Worker.
    fn handle(&self, m: ToWorker) -> bool {
        match m {
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
            ToWorker::Answer { question, accept } => {
                if let Some(p) = self.page() {
                    p.tls.answer(question, accept);
                }
            }
            ToWorker::Resize { width, height } => {
                let (w, h) = (width.clamp(64, 8192) as i32, height.clamp(64, 8192) as i32);
                self.0.pending_size.set(Some((w, h, Instant::now() + RESIZE_DEBOUNCE)));
            }
            ToWorker::Input(ev) => self.schedule_input(ev),
            ToWorker::Clipboard(ClipData::Text(t)) => {
                if t.len() <= PASTE_LIMIT {
                    *self.0.offered.borrow_mut() = t;
                }
            }
            ToWorker::Clipboard(ClipData::Files(_)) => {}
            // Terminal-Befehle gelten nur für den SSH-Worker.
            ToWorker::Term(_) => {}
            ToWorker::Custom(bytes) => match serde_json::from_slice::<WebCommand>(&bytes) {
                Ok(c) => self.command(c),
                Err(e) => self.send(FromWorker::Log(format!("Unbekannter Befehl an den Web-Worker: {e}"))),
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
        self.0.ignore_cert.set(opts.ignore_certificate);
        if let Some(old) = self.0.page.borrow_mut().take() {
            // SAFETY: das alte Fenster wird nicht mehr verwendet.
            unsafe { old.window.destroy() };
        }
        *self.0.frame.borrow_mut() = Frame::default();
        self.send(FromWorker::State(SessionState::Connecting));

        let view = new_view(&profile);
        let window = gtk::OffscreenWindow::new();
        let (w, h) = (r.width.clamp(64, 8192) as i32, r.height.clamp(64, 8192) as i32);
        view.set_size_request(w, h);
        window.set_default_size(w, h);
        window.add(&view);
        let mut tls = tls::Pending::default();
        tls.attach(&view, self.0.out.clone(), opts.ignore_certificate, &profile);
        self.connect_signals(&window, &view);
        window.show_all();
        view.set_can_focus(true);
        view.grab_focus();
        let page = Rc::new(Page { window, view: view.clone(), tls });
        *self.0.page.borrow_mut() = Some(page);
        self.focus(true);
        view.load_uri(&url.url);
        self.send(FromWorker::State(SessionState::Connected));
        self.send(FromWorker::Log(format!("Web-Sitzung geöffnet: {}", url.host)));
        Ok(())
    }

    fn connect_signals(&self, window: &gtk::OffscreenWindow, view: &WebView) {
        let me = self.clone();
        window.connect_damage_event(move |_, ev| {
            let a = ev.area();
            me.0.frame.borrow_mut().damage.add(a.x(), a.y(), a.width(), a.height());
            false
        });
        let me = self.clone();
        view.connect_load_changed(move |v, ev| {
            if ev == LoadEvent::Started {
                *me.0.last_error.borrow_mut() = None;
                me.0.load_started.set(Some(Instant::now()));
            }
            let shown = v.uri().map(|u| trace_uri(&u)).unwrap_or_default();
            let name = match ev {
                LoadEvent::Started => "Ladebeginn",
                LoadEvent::Redirected => "Weiterleitung",
                LoadEvent::Committed => "Antwort beginnt",
                _ => "Ladeende",
            };
            let dur = if ev == LoadEvent::Finished { me.0.load_started.get().map(|t| format!(" nach {} ms", t.elapsed().as_millis())).unwrap_or_default() } else { String::new() };
            me.trace(&format!("{name}{dur}: {shown}"));
            me.status(v);
            if ev == LoadEvent::Finished {
                let uri = v.uri().map(|u| u.to_string()).unwrap_or_default();
                let action = me.0.flow.borrow_mut().page_loaded(&uri);
                match action {
                    FlowAction::InjectLogin => me.inject(),
                    FlowAction::Notice(n) => me.send(FromWorker::Notice(n)),
                    FlowAction::CheckRejected => me.check_rejected(),
                    FlowAction::None => {}
                }
            }
        });
        let me = self.clone();
        view.connect_load_failed(move |v, _, uri, err| {
            // Abbruch durch eine neue Navigation ist kein Fehler.
            if !err.matches(webkit2gtk::NetworkError::Cancelled) {
                let host = check_url(uri).map(|u| u.host).unwrap_or_default();
                me.trace(&format!("Laden fehlgeschlagen: {host}: {}", err.message()));
                *me.0.last_error.borrow_mut() = Some(format!("{host}: {}", err.message()));
                me.status(v);
            }
            false
        });
        let me = self.clone();
        view.connect_title_notify(move |v| me.status(v));
        let me = self.clone();
        view.connect_estimated_load_progress_notify(move |v| me.status(v));
        // Neue Fenster (target=_blank, window.open) im selben Tab öffnen.
        view.connect_create(|v, action| {
            if let Some(u) = action.clone().request().and_then(|r| webkit2gtk::URIRequestExt::uri(&r)) {
                v.load_uri(&u);
            }
            None
        });
        view.connect_enter_fullscreen(|_| true);
        // Hinweisfenster (Tooltips) würden als eigene Fenster erscheinen.
        view.connect_query_tooltip(|v, _, _, _, _| {
            v.stop_signal_emission_by_name("query-tooltip");
            false
        });
        let me = self.clone();
        view.connect_context_menu(move |v, _menu, ev, hit| {
            let (x, y) = ev.coords().unwrap_or_default();
            let items = context_menu(v.can_go_back(), v.can_go_forward(), hit.context_is_editable(), hit.context_is_selection(), !me.0.offered.borrow().is_empty());
            me.send(FromWorker::Custom(WebEvent::Menu { id: 0, kind: "context".into(), x: x as i32, y: y as i32, items }.to_bytes()));
            true
        });
        let me = self.clone();
        view.connect_show_option_menu(move |_, menu, _, rect| {
            let id = me.0.next_menu.get();
            me.0.next_menu.set(id.wrapping_add(1).max(1));
            let mut items = Vec::new();
            for i in 0..webkit2gtk::OptionMenuExt::n_items(menu) {
                if let Some(mut it) = webkit2gtk::OptionMenuExt::item(menu, i) {
                    let label = it.label().map(|l| rj_core::charset::mark_visible(&l, false).into_owned()).unwrap_or_default();
                    items.push(MenuEntry { action: String::new(), label, enabled: it.is_enabled() && !it.is_group_label(), selected: it.is_selected() });
                }
            }
            me.0.menus.borrow_mut().insert(id, menu.clone());
            me.send(FromWorker::Custom(WebEvent::Menu { id, kind: "option".into(), x: rect.x(), y: rect.y() + rect.height(), items }.to_bytes()));
            true
        });
    }

    fn status(&self, v: &WebView) {
        let ev = WebEvent::Status {
            title: v.title().map(|t| rj_core::charset::mark_visible(&t, false).into_owned()).unwrap_or_default(),
            uri: v.uri().map(|u| u.to_string()).unwrap_or_default(),
            loading: v.is_loading(),
            progress: (v.estimated_load_progress() * 100.0).round().clamp(0.0, 100.0) as u8,
            error: self.0.last_error.borrow().clone(),
            back: v.can_go_back(),
            forward: v.can_go_forward(),
        };
        self.send(FromWorker::Custom(ev.to_bytes()));
    }

    /// Kopiert den geänderten Bereich in den geteilten Speicher.
    fn tick(&self) {
        let Some(page) = self.page() else { return };
        if let Some((w, h, at)) = self.0.pending_size.get() {
            if Instant::now() >= at {
                self.0.pending_size.set(None);
                page.view.set_size_request(w, h);
                page.window.set_default_size(w, h);
                page.window.resize(w, h);
                page.window.queue_resize();
            }
        }
        let (w, h) = (page.view.allocated_width(), page.view.allocated_height());
        if w < 1 || h < 1 {
            return;
        }
        let mut f = self.0.frame.borrow_mut();
        let size_changed = f.fb.as_ref().is_none_or(|b| b.width != w as u32 || b.height != h as u32);
        if size_changed {
            match FrameBufferWriter::create(w as u32, h as u32) {
                Ok(b) => match rj_ipc::fdpass::share(&b) {
                    Ok(handle) => {
                        self.send(FromWorker::SharedFrame { handle, width: w as u32, height: h as u32 });
                        f.fb = Some(b);
                        f.scratch = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).ok();
                        f.damage.add(0, 0, w, h);
                    }
                    Err(e) => {
                        self.send(FromWorker::Log(format!("Bildspeicher nicht übergebbar: {e}")));
                        return;
                    }
                },
                Err(e) => {
                    self.send(FromWorker::Log(format!("Bildspeicher nicht anlegbar: {e}")));
                    return;
                }
            }
        }
        let Some((x, y, dw, dh)) = f.damage.take(w, h) else { return };
        let Some(src) = page.window.surface() else { return };
        let Frame { fb, scratch, serial, first, .. } = &mut *f;
        let (Some(fb), Some(scratch)) = (fb.as_mut(), scratch.as_mut()) else { return };
        {
            let Ok(cr) = cairo::Context::new(&*scratch) else { return };
            cr.rectangle(x as f64, y as f64, dw as f64, dh as f64);
            cr.clip();
            // Deckender Hintergrund, dann die Seite darüber (Alpha immer 255).
            cr.set_source_rgb(1.0, 1.0, 1.0);
            let _ = cr.paint();
            if cr.set_source_surface(&src, 0.0, 0.0).is_ok() {
                let _ = cr.paint();
            }
        }
        scratch.flush();
        let stride = scratch.stride() as usize;
        let Ok(data) = scratch.data() else { return };
        let start = y as usize * stride + x as usize * 4;
        fb.blit(x as u32, y as u32, dw as u32, dh as u32, &data[start..], stride);
        *serial += 1;
        self.send(FromWorker::Frame(FrameUpdate { width: w as u32, height: h as u32, x: x as u32, y: y as u32, w: dw as u32, h: dh as u32, serial: *serial }));
        if !*first {
            *first = true;
            self.send(FromWorker::Log(format!("Erstes Bild nach {} ms", self.0.started.elapsed().as_millis())));
        }
    }

    fn command(&self, c: WebCommand) {
        let Some(page) = self.page() else { return };
        let v = &page.view;
        match c {
            WebCommand::Nav { action } => match action {
                NavAction::Back => v.go_back(),
                NavAction::Forward => v.go_forward(),
                NavAction::Reload => v.reload(),
                NavAction::Stop => v.stop_loading(),
            },
            WebCommand::Detect => {
                let me = self.clone();
                v.evaluate_javascript(rj_web::DETECT_SCRIPT, None, None, None::<&gtk::gio::Cancellable>, move |r| {
                    let text = match r {
                        Ok(v) => v.to_str().to_string(),
                        Err(_) => String::new(),
                    };
                    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    let list = |k: &str| -> Vec<rj_web::FieldCandidate> { parsed.get(k).and_then(|x| serde_json::from_value(x.clone()).ok()).unwrap_or_default() };
                    me.send(FromWorker::Custom(WebEvent::Fields { users: list("users"), passwords: list("passwords"), submits: list("submits") }.to_bytes()));
                });
            }
            WebCommand::Focus { on } => self.focus(on),
            WebCommand::Edit { action } => self.schedule_edit(action),
            WebCommand::Menu { id, index, action } => {
                if id == 0 {
                    let edit = match action.as_deref() {
                        Some("back") => return v.go_back(),
                        Some("forward") => return v.go_forward(),
                        Some("reload") => return v.reload(),
                        Some("copy") => EditAction::Copy,
                        Some("cut") => EditAction::Cut,
                        Some("paste") => EditAction::Paste,
                        Some("select_all") => EditAction::SelectAll,
                        _ => return,
                    };
                    self.schedule_edit(edit);
                } else if let Some(menu) = self.0.menus.borrow_mut().remove(&id) {
                    if let Some(i) = index {
                        webkit2gtk::OptionMenuExt::activate_item(&menu, i);
                    }
                    webkit2gtk::OptionMenuExt::close(&menu);
                }
            }
        }
    }

    /// Kopieren und Ausschneiden laufen über ein Skript in einer eigenen Welt
    /// und die Zwischenablage-Schicht des Hauptprogramms, Einfügen über den
    /// dort angebotenen Text; die Zwischenablage des Systems bleibt unberührt.
    fn edit(&self, action: EditAction) {
        let Some(page) = self.page() else { return };
        let v = page.view.clone();
        match action {
            EditAction::SelectAll => v.execute_editing_command("SelectAll"),
            EditAction::Paste => {
                let text = self.0.offered.borrow().clone();
                if !text.is_empty() {
                    v.execute_editing_command_with_argument("InsertText", &text);
                }
            }
            EditAction::Copy | EditAction::Cut => {
                let me = self.clone();
                let v2 = v.clone();
                v.evaluate_javascript(SELECTION_SCRIPT, Some("remotejoe"), None, None::<&gtk::gio::Cancellable>, move |r| {
                    let text = match r {
                        Ok(val) => val.to_str().to_string(),
                        Err(e) => {
                            me.send(FromWorker::Log(format!("Markierung nicht lesbar: {}", e.message())));
                            String::new()
                        }
                    };
                    if text.is_empty() {
                        return;
                    }
                    me.send(FromWorker::Clipboard(ClipData::Text(text)));
                    if action == EditAction::Cut {
                        v2.execute_editing_command("Delete");
                    }
                });
            }
        }
    }

    fn schedule_input(&self, ev: InputEvent) {
        let now = Instant::now();
        let at = match &ev {
            InputEvent::Key { .. } | InputEvent::Text(_) => {
                self.0.last_key.set(Some(now));
                self.0.last_click.get().map(|t| t + KEY_AFTER_CLICK).filter(|t| *t > now)
            }
            InputEvent::MouseButton { .. } => {
                self.0.last_click.set(Some(now));
                None
            }
            _ => None,
        };
        self.defer(at, Deferred::Input(ev));
    }

    fn schedule_edit(&self, a: EditAction) {
        let at = self.0.last_key.get().map(|t| t + EDIT_AFTER_KEY);
        self.defer(at, Deferred::Edit(a));
    }

    /// Reiht ein; ohne Wartezeit und leere Warteschlange: sofort ausführen.
    fn defer(&self, at: Option<Instant>, d: Deferred) {
        let now = Instant::now();
        let at = at.unwrap_or(now);
        if at <= now && self.0.deferred.borrow().is_empty() {
            return self.run_deferred(d);
        }
        let mut q = self.0.deferred.borrow_mut();
        // Die Reihenfolge bleibt erhalten: nie vor einem früher eingereihten Ereignis.
        let at = q.back().map(|(t, _)| (*t).max(at)).unwrap_or(at);
        q.push_back((at, d));
    }

    fn drain(&self) {
        loop {
            let next = {
                let mut q = self.0.deferred.borrow_mut();
                match q.front() {
                    Some((t, _)) if *t <= Instant::now() => q.pop_front().map(|(_, d)| d),
                    _ => None,
                }
            };
            match next {
                Some(d) => self.run_deferred(d),
                None => return,
            }
        }
    }

    fn run_deferred(&self, d: Deferred) {
        match d {
            Deferred::Input(ev) => self.input(ev),
            Deferred::Edit(a) => self.edit(a),
        }
    }

    fn input(&self, ev: InputEvent) {
        let Some(page) = self.page() else { return };
        match ev {
            InputEvent::MouseMove { x, y } => {
                self.0.pointer.set((x as f64, y as f64));
                events::motion(&page.view, x as f64, y as f64, self.state());
            }
            InputEvent::MouseButton { x, y, button, down } => {
                let (x, y) = (x as f64, y as f64);
                self.0.pointer.set((x, y));
                let n = match button {
                    MouseButton::Left => 1,
                    MouseButton::Middle => 2,
                    MouseButton::Right => 3,
                    MouseButton::WheelUp => return events::scroll(&page.view, x, y, gdk::ScrollDirection::Up, self.state()),
                    MouseButton::WheelDown => return events::scroll(&page.view, x, y, gdk::ScrollDirection::Down, self.state()),
                };
                let mask = button_mask(n);
                let state = self.state();
                if down {
                    self.0.buttons.set(self.0.buttons.get() | mask);
                } else {
                    self.0.buttons.set(self.0.buttons.get() & !mask);
                }
                events::button(&page.view, x, y, n, down, state);
            }
            InputEvent::Key { keysym, down } => {
                let mut m = self.0.mods.get();
                let is_mod = m.update(keysym, down);
                if !is_mod && down {
                    if let Some(a) = m.shortcut(keysym) {
                        self.0.mods.set(m);
                        return self.schedule_edit(a);
                    }
                }
                let state = self.state();
                self.0.mods.set(m);
                events::key(&page.window, keysym, down, state, is_mod);
            }
            InputEvent::Text(t) => {
                let state = self.state();
                for c in t.chars() {
                    // SAFETY: reine Umrechnung ohne Zeiger.
                    let keyval = unsafe { gdk::ffi::gdk_unicode_to_keyval(c as u32) };
                    events::key(&page.window, keyval, true, state, false);
                    events::key(&page.window, keyval, false, state, false);
                }
            }
            InputEvent::Bytes(_) => {}
        }
    }

    fn state(&self) -> gdk::ModifierType {
        let m = self.0.mods.get();
        let mut s = gdk::ModifierType::from_bits_truncate(self.0.buttons.get());
        if m.shift {
            s |= gdk::ModifierType::SHIFT_MASK;
        }
        if m.ctrl {
            s |= gdk::ModifierType::CONTROL_MASK;
        }
        if m.alt {
            s |= gdk::ModifierType::MOD1_MASK;
        }
        s
    }

    fn focus(&self, on: bool) {
        if let Some(p) = self.page() {
            if on {
                p.view.grab_focus();
            }
            events::focus(&p.window, on);
        }
    }

    fn inject(&self) {
        let Some(page) = self.page() else {
            self.0.inject_pending.set(true);
            return;
        };
        let creds = self.0.creds.borrow().clone();
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
        let user = match &c.domain {
            Some(d) if !d.is_empty() && !c.username.contains('@') && !c.username.contains('\\') => format!("{d}\\{}", c.username),
            _ => c.username.clone(),
        };
        let pw = Secret::from_bytes(c.password.0.clone());
        match login_script(&login, &user, &pw) {
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
        let me = self.clone();
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            let Some(page) = me.page() else { return };
            let me2 = me.clone();
            page.view.evaluate_javascript(&presence_script(), None, None, None::<&gtk::gio::Cancellable>, move |r| {
                let text = r.map(|v| v.to_str().to_string()).unwrap_or_default();
                me2.trace(&format!("Prüfung nach dem Abschicken: {text}"));
                if let FlowAction::Notice(n) = me2.0.flow.borrow_mut().rejected_check(&text) {
                    me2.send(FromWorker::Notice(n));
                }
            });
        });
    }

    /// Führt das Anmeldeskript aus. Seiten, die ihr Formular erst nach dem Laden aufbauen, brauchen
    /// Zeit: Wird kein Kennwortfeld gefunden, wird alle 500 ms erneut gesucht, höchstens 10 Sekunden.
    fn run_login(&self, script: zeroize::Zeroizing<String>, tries: u32) {
        let Some(page) = self.page() else {
            self.0.login_busy.set(false);
            return;
        };
        let me = self.clone();
        let again = script.clone();
        page.view.evaluate_javascript(&script, None, None, None::<&gtk::gio::Cancellable>, move |r| {
            let text = match r {
                Ok(v) => v.to_str().to_string(),
                Err(e) => e.message().to_string(),
            };
            if text.trim_matches('"') == RESULT_NO_PASSWORD_FIELD && tries < LOGIN_RETRIES {
                glib::timeout_add_local_once(LOGIN_RETRY_EVERY, move || me.run_login(again, tries + 1));
                return;
            }
            me.0.login_busy.set(false);
            // Ergebnisse sind feste Kennungen (rj:…) oder Meldungen ohne Eingabewerte.
            me.trace(&format!("Anmeldeskript nach {} Wiederholungen: {}", tries, text.chars().take(80).collect::<String>()));
            let action = me.0.flow.borrow_mut().script_result(&text);
            if let FlowAction::Notice(n) = action {
                me.send(FromWorker::Notice(n));
            }
        });
    }
}

/// Adresse für das Protokoll: ohne Anfrageteil und Anker (können Kennungen enthalten).
fn trace_uri(u: &str) -> String {
    u.split(['?', '#']).next().unwrap_or("").to_string()
}

fn button_mask(n: u32) -> u32 {
    match n {
        1 => gdk::ModifierType::BUTTON1_MASK.bits(),
        2 => gdk::ModifierType::BUTTON2_MASK.bits(),
        _ => gdk::ModifierType::BUTTON3_MASK.bits(),
    }
}

fn new_view(profile: &Path) -> WebView {
    let dir = profile.to_string_lossy().to_string();
    let manager = WebsiteDataManager::builder().base_cache_directory(dir.as_str()).base_data_directory(dir.as_str()).build();
    if let Some(cm) = manager.cookie_manager() {
        cm.set_persistent_storage(&profile.join("cookies").to_string_lossy(), CookiePersistentStorage::Text);
    }
    let ctx = WebContext::with_website_data_manager(&manager);
    ctx.set_automation_allowed(false);
    let view = WebView::with_context(&ctx);
    if let Some(s) = WebViewExt::settings(&view) {
        // Ohne Compositing zeichnet WebKit in das Fenster ohne Bildschirm.
        s.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
        s.set_enable_developer_extras(false);
        s.set_javascript_can_open_windows_automatically(false);
    }
    view
}

/// Synthetische GDK-Ereignisse für die WebView im Fenster ohne Bildschirm.
mod events {
    use std::sync::OnceLock;
    use std::time::Instant;

    use gtk::gdk;
    use gtk::glib::translate::{IntoGlib, ToGlibPtr};
    use gtk::prelude::*;

    fn now() -> u32 {
        static T0: OnceLock<Instant> = OnceLock::new();
        T0.get_or_init(Instant::now).elapsed().as_millis() as u32 + 1
    }

    fn device(w: &impl IsA<gtk::Widget>, pointer: bool) -> Option<gdk::Device> {
        let seat = w.display().default_seat()?;
        if pointer {
            seat.pointer()
        } else {
            seat.keyboard()
        }
    }

    /// Übergibt das Ereignis an GTK und gibt es frei.
    ///
    /// # Safety
    /// `ev` muss ein mit `gdk_event_new` angelegtes, vollständig gefülltes Ereignis sein.
    unsafe fn dispatch(ev: *mut gdk::ffi::GdkEvent, dev: Option<gdk::Device>) {
        if let Some(d) = dev {
            gdk::ffi::gdk_event_set_device(ev, d.to_glib_none().0);
            gdk::ffi::gdk_event_set_source_device(ev, d.to_glib_none().0);
        }
        // Das Fenster ohne Bildschirm hat kein X-Fenster; Abfragen von GTK, die
        // sich darauf beziehen, dürfen den Worker nicht beenden.
        gdk::ffi::gdk_error_trap_push();
        gtk::ffi::gtk_main_do_event(ev);
        gdk::ffi::gdk_error_trap_pop_ignored();
        gdk::ffi::gdk_event_free(ev);
    }

    pub fn motion(view: &webkit2gtk::WebView, x: f64, y: f64, state: gdk::ModifierType) {
        let Some(win) = view.window() else { return };
        // SAFETY: Felder eines frisch angelegten Bewegungsereignisses; das Fenster
        // erhält eine eigene Referenz, die gdk_event_free wieder abgibt.
        unsafe {
            let ev = gdk::ffi::gdk_event_new(gdk::ffi::GDK_MOTION_NOTIFY);
            let m = &mut *(ev as *mut gdk::ffi::GdkEventMotion);
            m.window = win.to_glib_full();
            m.time = now();
            m.x = x;
            m.y = y;
            m.x_root = x;
            m.y_root = y;
            m.state = state.into_glib();
            dispatch(ev, device(view, true));
        }
    }

    pub fn button(view: &webkit2gtk::WebView, x: f64, y: f64, n: u32, down: bool, state: gdk::ModifierType) {
        let Some(win) = view.window() else { return };
        // SAFETY: wie bei `motion`.
        unsafe {
            let ev = gdk::ffi::gdk_event_new(if down { gdk::ffi::GDK_BUTTON_PRESS } else { gdk::ffi::GDK_BUTTON_RELEASE });
            let b = &mut *(ev as *mut gdk::ffi::GdkEventButton);
            b.window = win.to_glib_full();
            b.time = now();
            b.x = x;
            b.y = y;
            b.x_root = x;
            b.y_root = y;
            b.button = n;
            b.state = state.into_glib();
            dispatch(ev, device(view, true));
        }
    }

    pub fn scroll(view: &webkit2gtk::WebView, x: f64, y: f64, dir: gdk::ScrollDirection, state: gdk::ModifierType) {
        let Some(win) = view.window() else { return };
        // SAFETY: wie bei `motion`.
        unsafe {
            let ev = gdk::ffi::gdk_event_new(gdk::ffi::GDK_SCROLL);
            let s = &mut *(ev as *mut gdk::ffi::GdkEventScroll);
            s.window = win.to_glib_full();
            s.time = now();
            s.x = x;
            s.y = y;
            s.x_root = x;
            s.y_root = y;
            s.direction = dir.into_glib();
            s.state = state.into_glib();
            dispatch(ev, device(view, true));
        }
    }

    pub fn key(window: &gtk::OffscreenWindow, keyval: u32, down: bool, state: gdk::ModifierType, is_modifier: bool) {
        let Some(win) = window.window() else { return };
        let (mut keycode, mut group) = (0u16, 0u8);
        if let Some(km) = gdk::Keymap::for_display(&window.display()) {
            if let Some(k) = km.entries_for_keyval(keyval).first() {
                keycode = k.keycode() as u16;
                group = k.group() as u8;
            }
        }
        // SAFETY: wie bei `motion`; `string` bleibt leer (NULL).
        unsafe {
            let ev = gdk::ffi::gdk_event_new(if down { gdk::ffi::GDK_KEY_PRESS } else { gdk::ffi::GDK_KEY_RELEASE });
            let k = &mut *(ev as *mut gdk::ffi::GdkEventKey);
            k.window = win.to_glib_full();
            k.time = now();
            k.state = state.into_glib();
            k.keyval = keyval;
            k.hardware_keycode = keycode;
            k.group = group;
            k.is_modifier = is_modifier as u32;
            dispatch(ev, device(window, false));
        }
    }

    pub fn focus(window: &gtk::OffscreenWindow, on: bool) {
        let Some(win) = window.window() else { return };
        // SAFETY: wie bei `motion`.
        unsafe {
            let ev = gdk::ffi::gdk_event_new(gdk::ffi::GDK_FOCUS_CHANGE);
            let f = &mut *(ev as *mut gdk::ffi::GdkEventFocus);
            f.window = win.to_glib_full();
            f.send_event = 1;
            f.in_ = on as i16;
            dispatch(ev, device(window, false));
        }
    }
}

mod tls {
    //! Zertifikatsfehler unter WebKitGTK: Rückfrage mit Warnung, angenommene
    //! Fingerabdrücke je Profil.
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use gtk::gio::prelude::*;
    use gtk::{gio, glib};
    use rj_ipc::worker::Sender;
    use rj_ipc::FromWorker;
    use rj_web::{AcceptedCerts, CertDecision};
    use webkit2gtk::{WebContextExt, WebViewExt};

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
        pub fn attach(&mut self, wk: &webkit2gtk::WebView, out: Sender, ignore: bool, profile: &Path) {
            let profile = profile.to_path_buf();
            let asks = self.asks.clone();
            let next = self.next.clone();
            wk.connect_load_failed_with_tls_errors(move |view, uri, cert, flags| {
                let host = glib::Uri::parse(uri, glib::UriFlags::NONE).ok().and_then(|u| u.host()).map(|h| h.to_string()).unwrap_or_default();
                // „Zertifikatsprüfung abschalten“ (Option des Ziels): ohne Rückfrage durchlassen.
                if ignore {
                    allow(view, cert, &host, uri);
                    return true;
                }
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

        pub fn answer(&self, id: u32, accept: bool) {
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
