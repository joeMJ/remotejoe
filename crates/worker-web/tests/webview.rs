//! Ende-zu-Ende mit echter WebView (WebKitGTK unter Xvfb) gegen lokale
//! Prüfseiten. Die Seite wird ohne sichtbares Fenster gerendert; geprüft wird
//! das Bild im geteilten Speicher. Nur mit Merkmal `webview`; ohne Xvfb wird
//! übersprungen.
#![cfg(feature = "webview")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use rj_ipc::shm::FrameBufferReader;
use rj_ipc::*;
use rj_supervisor::*;

/// Kennwort mit Umlaut, Euro, Anführungszeichen, Apostroph, Gegenschrägstrich und Emoji.
const PASSWORD: &str = "Prüf-\"Kennwort\"&ä€'\\ 🚀";
const LOGIN: &str = "<html><body><form method='post' action='/login'>\
<input id='user' name='user'><input id='pw' name='pw' type='password'>\
<button id='go' type='submit'>Anmelden</button></form></body></html>";

/// Prüfseite für Eingaben: grüner Hintergrund, Knopf, Eingabefeld, lange Seite.
const INPUT_PAGE: &str = "<html><head><meta charset='utf-8'><title>Prüfseite ä</title><style>\
body{margin:0;background:#00aa00;height:4000px}\
#b{position:absolute;left:20px;top:20px;width:200px;height:60px}\
#t{position:absolute;left:20px;top:120px;width:300px;height:30px;font-size:16px}\
</style></head><body>\
<button id='b' onclick=\"document.body.style.background='#0000ff';fetch('/ev?click')\" ondblclick=\"fetch('/ev?dbl')\">Knopf</button>\
<input id='t' oninput=\"fetch('/ev?text='+encodeURIComponent(this.value))\">\
<script>var sc=0;window.onscroll=function(){if(!sc){sc=1;setTimeout(function(){sc=0;fetch('/ev?scroll='+window.scrollY)},50)}};\
window.onresize=function(){fetch('/ev?w='+window.innerWidth)};fetch('/ev?w='+window.innerWidth);</script>\
</body></html>";

/// Seite mit Material-Design: Der Knopf wird erst nach echter (vertrauenswürdiger)
/// Eingabe in beide Felder freigegeben; per Skript gesetzte Werte zählen nicht.
const STRICT: &str = "<html><body><div><input id='u1' placeholder='Kennung'><input id='p1' type='password' placeholder='Passwort'>\
<button id='b1' disabled>Login</button></div><script>var tu=0,tp=0,u=document.getElementById('u1'),p=document.getElementById('p1'),b=document.getElementById('b1');\
u.addEventListener('input',function(e){if(e.isTrusted)tu=1;b.disabled=!(tu&&tp&&u.value&&p.value);});\
p.addEventListener('input',function(e){if(e.isTrusted)tp=1;b.disabled=!(tu&&tp&&u.value&&p.value);});\
b.onclick=function(){fetch('/login',{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:'user='+encodeURIComponent(u.value)+'&pw='+encodeURIComponent(p.value)});};</script></body></html>";

/// Seite ohne Formular: zufällige IDs bei jedem Laden; die Werte übernimmt die Seite erst
/// beim Verlassen des Feldes (blur), der Knopf schickt nur übernommene Werte.
const ZK_LIKE: &str = "<html><body><div id='box'></div><script>var r=Math.random().toString(36).slice(2,6),cu='',cp='';\
document.getElementById('box').innerHTML=\"<input id='\"+r+\"P31-real' placeholder='Benutzername'><input id='\"+r+\"P41-real' type='password' placeholder='Kennwort'>\
<button type='button' id='\"+r+\"Pg1' class='btn-login'>Anmelden</button>\";var u=document.getElementById(r+'P31-real'),p=document.getElementById(r+'P41-real');\
u.addEventListener('blur',function(){cu=u.value;});p.addEventListener('blur',function(){cp=p.value;});\
document.getElementById(r+'Pg1').onclick=function(){fetch('/login',{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:'user='+encodeURIComponent(cu)+'&pw='+encodeURIComponent(cp)});};</script></body></html>";

fn page_for(path: &str) -> String {
    match path {
        "/login" => "<html><body>angemeldet</body></html>".into(),
        "/eingabe" => INPUT_PAGE.into(),
        // Formular erst nach 1,5 s und in einer Webkomponente (Shadow-DOM), wie bei Oberflächen, die nachladen.
        "/spaet" => "<html><body><p>lade …</p><script>setTimeout(function(){var h=document.createElement('div');document.body.appendChild(h);\
var sr=h.attachShadow({mode:'open'});sr.innerHTML=\"<form method='post' action='/login'><label for='user'>Benutzername</label><input id='user' name='user'>\
<input id='pw' name='pw' type='password'><button id='go' type='submit'>Anmelden</button></form>\";},1500);</script></body></html>"
            .into(),
        // Wie die HomeMatic-Anmeldung: kein Absende-Knopf, „Anmelden“ ist ein div; das Absenden kopiert
        // den Benutzernamen erst in ein verstecktes Feld (form.requestSubmit() allein genügt nicht).
        "/homematic" => "<html><head><script>/* wie prototype.js 1.6: toJSON an Array und String */Array.prototype.toJSON=function(){return '[]';};String.prototype.toJSON=function(){return '\\\"x\\\"';};</script></head><body><form id='f' method='post' action='/login'>\
<input id='UserNameShow' type='text' name='tbUsernameShow'><input id='UserName' type='hidden' name='user'>\
<input id='Password' type='password' name='pw'><div id='btnLogin' onclick=\"FormSubmit()\">Anmelden </div></form>\
<script>function FormSubmit(){document.getElementById('UserName').value=document.getElementById('UserNameShow').value;document.getElementById('f').submit();}</script>\
</body></html>"
            .into(),
"/streng" => STRICT.into(),
        "/zk" => ZK_LIKE.into(),
        "/rot" => "<html><body style='margin:0;background:#ff0000' onclick=\"fetch('/ev?rot')\"></body></html>".into(),
        // Läuft ständig weiter (Farbwechsel), damit Bilder auch ohne Fokus kommen.
        "/blau" => "<html><body style='margin:0;background:#0000ff'><script>var n=0;setInterval(function(){n++;\
document.body.style.background=n%2?'#0000ee':'#0000ff'},100)</script></body></html>"
            .into(),
        p if p.starts_with("/ev") => String::new(),
        _ => LOGIN.into(),
    }
}

/// Kleiner HTTP-Server (ein Faden je Verbindung); meldet Pfad und Rumpf.
fn serve(l: TcpListener, tx: mpsc::Sender<(String, String)>, tls: Option<Arc<rustls::ServerConfig>>) {
    for s in l.incoming().flatten() {
        let tx = tx.clone();
        let tls = tls.clone();
        std::thread::spawn(move || match tls {
            None => handle(s, &tx),
            Some(cfg) => {
                let Ok(conn) = rustls::ServerConnection::new(cfg) else { return };
                handle(rustls::StreamOwned::new(conn, s), &tx);
            }
        });
    }
}

fn handle<S: Read + Write>(s: S, tx: &mpsc::Sender<(String, String)>) {
    let mut r = BufReader::new(s);
    let mut first = String::new();
    if r.read_line(&mut first).is_err() {
        return;
    }
    let mut len = 0;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap_or(0) == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; len];
    let _ = r.read_exact(&mut body);
    let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
    let page = page_for(&path);
    let s = r.get_mut();
    let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
    let _ = s.flush();
    let _ = tx.send((path, String::from_utf8_lossy(&body).into_owned()));
}

fn http_server() -> (u16, mpsc::Receiver<(String, String)>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || serve(l, tx, None));
    (port, rx)
}

fn form_decode(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), String::from_utf8(percent_decode(&v.replace('+', " "))).unwrap()))
        .collect()
}

fn percent_decode(v: &str) -> Vec<u8> {
    let b = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap(), 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// Eine Web-Sitzung über Supervisor und Worker-Prozess.
struct Sess {
    sup: Supervisor,
    id: WorkerId,
    shm: Option<FrameBufferReader>,
    size: (u32, u32),
    frames: usize,
    shared: usize,
    clip: Vec<String>,
    custom: Vec<serde_json::Value>,
    log: Vec<String>,
    questions: usize,
    accept: bool,
    started: Instant,
    first_frame: Option<Duration>,
}

fn worker_spec(display: &str, data: &std::path::Path) -> WorkerSpec {
    let mut spec = WorkerSpec::new(env!("CARGO_BIN_EXE_rj-worker-web"));
    // WebKit reserviert sehr viel Adressraum; eine RLIMIT_AS-Grenze bricht es.
    spec.memory_limit = None;
    spec.env = vec![
        ("DISPLAY".into(), display.to_string()),
        ("XDG_DATA_HOME".into(), data.display().to_string()),
        ("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS".into(), "1".into()),
        ("LIBGL_ALWAYS_SOFTWARE".into(), "1".into()),
        ("NO_AT_BRIDGE".into(), "1".into()),
    ];
    spec
}

impl Sess {
    fn start(display: &str, data: &std::path::Path, opts: serde_json::Value, width: u32, height: u32, accept: bool) -> Sess {
        let mut sup = Supervisor::new();
        let (id, _) = sup.spawn(worker_spec(display, data)).unwrap();
        let started = Instant::now();
        sup.send(id, &ToWorker::Connect(ConnectRequest {
            host: "127.0.0.1".into(),
            port: 0,
            width,
            height,
            options_json: opts.to_string(),
            connect_timeout_secs: 15,
            keepalive_secs: 30,
        }))
        .unwrap();
        sup.send(id, &ToWorker::Credentials(CredentialPayload {
            username: "admin".into(),
            domain: None,
            password: SecretBytes(PASSWORD.as_bytes().to_vec()),
            private_key: None,
        }))
        .unwrap();
        Sess { sup, id, shm: None, size: (0, 0), frames: 0, shared: 0, clip: Vec::new(), custom: Vec::new(), log: Vec::new(), questions: 0, accept, started, first_frame: None }
    }

    fn send(&mut self, m: ToWorker) {
        self.sup.send(self.id, &m).unwrap();
    }

    fn input(&mut self, ev: InputEvent) {
        self.send(ToWorker::Input(ev));
    }

    fn command(&mut self, json: serde_json::Value) {
        self.send(ToWorker::Custom(json.to_string().into_bytes()));
    }

    fn click(&mut self, x: i32, y: i32, button: MouseButton) {
        self.input(InputEvent::MouseMove { x, y });
        self.input(InputEvent::MouseButton { x, y, button, down: true });
        self.input(InputEvent::MouseButton { x, y, button, down: false });
    }

    fn key(&mut self, keysym: u32) {
        self.input(InputEvent::Key { keysym, down: true });
        self.input(InputEvent::Key { keysym, down: false });
    }

    fn ctrl(&mut self, keysym: u32) {
        self.input(InputEvent::Key { keysym: 0xffe3, down: true });
        self.key(keysym);
        self.input(InputEvent::Key { keysym: 0xffe3, down: false });
    }

    fn pump(&mut self, wait: Duration) {
        for ev in self.sup.tick(wait) {
            match ev {
                SupervisorEvent::Message { msg, .. } => match msg {
                    FromWorker::SharedFrame { handle, width, height } => {
                        let h = self.sup.take_shared(self.id, handle).expect("Bildpuffer übergeben");
                        self.shm = Some(FrameBufferReader::open(h, width, height).expect("Bildpuffer gültig"));
                        self.size = (width, height);
                        self.shared += 1;
                    }
                    FromWorker::Frame(_) => {
                        self.frames += 1;
                        if self.first_frame.is_none() {
                            self.first_frame = Some(self.started.elapsed());
                        }
                    }
                    FromWorker::Clipboard(ClipData::Text(t)) => self.clip.push(t),
                    FromWorker::Custom(b) => self.custom.push(serde_json::from_slice(&b).expect("JSON vom Worker")),
                    FromWorker::Question { id: q, text, .. } => {
                        self.questions += 1;
                        self.log.push(text);
                        let accept = self.accept;
                        self.send(ToWorker::Answer { question: q, accept });
                    }
                    FromWorker::State(SessionState::Failed { reason }) => panic!("Sitzung fehlgeschlagen: {reason}"),
                    other => self.log.push(format!("{other:?}")),
                },
                SupervisorEvent::Exited { reason, .. } => panic!("Worker beendet: {reason:?}; {:?}", self.log),
                other => self.log.push(format!("{other:?}")),
            }
        }
    }

    fn wait(&mut self, secs: u64, mut cond: impl FnMut(&Sess) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            self.pump(Duration::from_millis(20));
            if cond(self) {
                return true;
            }
        }
        false
    }

    /// Bildpunkt (B, G, R, A) aus dem geteilten Speicher.
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let Some(shm) = &self.shm else { return [0; 4] };
        let i = (y as usize * self.size.0 as usize + x as usize) * 4;
        shm.pixels()[i..i + 4].try_into().unwrap()
    }

    fn checksum(&self) -> u64 {
        self.shm.as_ref().map(|s| s.pixels().iter().step_by(97).map(|b| *b as u64).sum()).unwrap_or(0)
    }
}

impl Drop for Sess {
    fn drop(&mut self) {
        self.sup.shutdown();
    }
}

/// Wartet auf ein Ereignis der Prüfseite (`/ev?…`) mit Präfix; liefert den Rest.
fn wait_ev(s: &mut Sess, rx: &mpsc::Receiver<(String, String)>, prefix: &str, secs: u64) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        s.pump(Duration::from_millis(20));
        while let Ok((path, _)) = rx.try_recv() {
            if let Some(rest) = path.strip_prefix("/ev?").and_then(|q| q.strip_prefix(prefix)) {
                return Some(String::from_utf8(percent_decode(rest)).unwrap());
            }
        }
    }
    None
}

fn xvfb() -> Option<rj_testkit::Xvfb> {
    match rj_testkit::Xvfb::start("1280x1024x24") {
        Ok(x) => Some(x),
        Err(e) => {
            eprintln!("NICHT AUSGEFÜHRT: {}", e.0);
            None
        }
    }
}

/// Sichtbare Fenster auf der Anzeige (außer dem Wurzelfenster).
fn viewable_windows(display: &str) -> usize {
    let out = Command::new("xwininfo").args(["-root", "-children", "-display", display]).output();
    let Ok(out) = out else { return 0 };
    let text = String::from_utf8_lossy(&out.stdout);
    let ids: Vec<String> = text.lines().filter_map(|l| l.trim().strip_prefix("0x").map(|r| format!("0x{}", r.split_whitespace().next().unwrap_or("")))).collect();
    ids.iter()
        .filter(|id| {
            Command::new("xwininfo").args(["-id", id, "-display", display]).output().map(|o| String::from_utf8_lossy(&o.stdout).contains("IsViewable")).unwrap_or(false)
        })
        .count()
}

fn near(p: [u8; 4], bgr: [u8; 3]) -> bool {
    p[..3].iter().zip(bgr).all(|(a, b)| a.abs_diff(b) <= 6)
}

#[test]
#[ignore = "bekannter Fehler (zurückgestellt): Umschalt+Pfeil erweitert die Auswahl im Test nicht; mit --ignored ausführbar"]
fn offscreen_render_input_clipboard_resize() {
    let Some(x) = xvfb() else { return };
    let data = tempfile::tempdir().unwrap();
    let (port, rx) = http_server();
    let opts = serde_json::json!({"url": format!("http://127.0.0.1:{port}/eingabe"), "allow_http": true, "profile": "eingabe"});
    let mut s = Sess::start(&x.display, data.path(), opts, 800, 600, false);

    // 1. Gerendert in den Puffer: grüner Hintergrund (BGRA 00 AA 00).
    assert!(s.wait(60, |s| s.size == (800, 600) && near(s.pixel(600, 400), [0x00, 0xaa, 0x00])), "kein grünes Bild: {:?} {:?}", s.pixel(600, 400), s.log);
    let first = s.started.elapsed();
    eprintln!("Messung: erstes Bild nach {} ms, Seite sichtbar nach {} ms", s.first_frame.unwrap().as_millis(), first.as_millis());
    assert_eq!(viewable_windows(&x.display), 0, "der Worker darf kein sichtbares Fenster haben");
    assert_eq!(wait_ev(&mut s, &rx, "w=", 10).as_deref(), Some("800"));
    // Titel und Ladezustand kommen als Status.
    assert!(s.wait(10, |s| s.custom.iter().any(|c| c["ev"] == "status" && c["title"] == "Prüfseite ä" && c["loading"] == false)), "{:?}", s.custom);

    // 3. Klick auf den Knopf: Ereignis auf der Seite und blaues Bild.
    s.click(120, 50, MouseButton::Left);
    assert!(wait_ev(&mut s, &rx, "click", 10).is_some(), "Klick kam nicht an: {:?}", s.log);
    assert!(s.wait(10, |s| near(s.pixel(600, 400), [0xff, 0x00, 0x00])), "Bild nach Klick nicht blau: {:?}", s.pixel(600, 400));
    // Doppelklick.
    s.click(120, 50, MouseButton::Left);
    s.click(120, 50, MouseButton::Left);
    assert!(wait_ev(&mut s, &rx, "dbl", 10).is_some(), "Doppelklick kam nicht an");

    // Tastatur: Text mit Unicode, Rücktaste, Pfeiltaste.
    s.click(170, 135, MouseButton::Left);
    // Kurze Pause wie bei einer Person: der Fokuswechsel im Feld läuft asynchron.
    s.pump(Duration::from_millis(300));
    s.input(InputEvent::Text("aä€😀".into()));
    s.key(0xff08); // Rücktaste
    s.key(0xff51); // Pfeil links
    s.input(InputEvent::Text("X".into()));
    let mut last = None;
    while let Some(v) = wait_ev(&mut s, &rx, "text=", 5) {
        last = Some(v);
        if last.as_deref() == Some("aäX€") {
            break;
        }
    }
    assert_eq!(last.as_deref(), Some("aäX€"), "Eingabe im Feld");
    // Umschalt + Pfeil markiert, Strg+C kopiert die Markierung über den Kanal.
    s.key(0xff57); // Ende
    s.input(InputEvent::Key { keysym: 0xffe1, down: true });
    s.key(0xff51);
    s.key(0xff51);
    s.input(InputEvent::Key { keysym: 0xffe1, down: false });
    s.ctrl(0x63);
    assert!(s.wait(10, |s| !s.clip.is_empty()), "nichts kopiert");
    assert_eq!(s.clip.last().unwrap(), "X€");

    // 4. Zwischenablage Hauptprogramm → Seite (Strg+A, Strg+V).
    s.send(ToWorker::Clipboard(ClipData::Text("Zw-ü \"'\\".into())));
    s.ctrl(0x61);
    s.ctrl(0x76);
    let mut last = None;
    while let Some(v) = wait_ev(&mut s, &rx, "text=", 5) {
        last = Some(v);
        if last.as_deref() == Some("Zw-ü \"'\\") {
            break;
        }
    }
    assert_eq!(last.as_deref(), Some("Zw-ü \"'\\"), "Einfügen");
    // Kontextmenü: Rechtsklick ins Feld ⇒ Menü im Hauptfenster, Einfügen möglich.
    s.click(170, 135, MouseButton::Right);
    assert!(
        s.wait(10, |s| s.custom.iter().any(|c| c["ev"] == "menu" && c["kind"] == "context" && c["items"].as_array().unwrap().iter().any(|i| i["action"] == "paste" && i["enabled"] == true))),
        "kein Kontextmenü: {:?}",
        s.custom
    );
    s.command(serde_json::json!({"op": "menu", "id": 0, "action": "copy"}));
    let n = s.clip.len();
    s.ctrl(0x61);
    s.command(serde_json::json!({"op": "menu", "id": 0, "action": "copy"}));
    assert!(s.wait(10, |s| s.clip.len() > n), "Kopieren aus dem Kontextmenü");
    assert_eq!(s.clip.last().unwrap(), "Zw-ü \"'\\");

    // 3. Scrollen per Rad; Messung der Bildrate.
    s.input(InputEvent::MouseMove { x: 600, y: 400 });
    // 60 Raststufen im Abstand von 16 ms; gezählt werden die Bilder in dieser Zeit.
    let f0 = s.frames;
    let t0 = Instant::now();
    for i in 0..60u32 {
        s.input(InputEvent::MouseButton { x: 600, y: 400, button: MouseButton::WheelDown, down: true });
        let next = t0 + Duration::from_millis(16 * (i as u64 + 1));
        while Instant::now() < next {
            s.pump(next.saturating_duration_since(Instant::now()).min(Duration::from_millis(4)));
        }
    }
    let dt = t0.elapsed().as_secs_f64();
    eprintln!("Messung: {} Bilder in {:.2} s beim Scrollen = {:.1} Bilder/s", s.frames - f0, dt, (s.frames - f0) as f64 / dt);
    let y = wait_ev(&mut s, &rx, "scroll=", 10).expect("Scrollen kam nicht an");
    assert!(y.parse::<f64>().unwrap() > 0.0, "Seite gescrollt");

    // 5. Größenänderung: neue Renderbreite.
    let shared = s.shared;
    s.send(ToWorker::Resize { width: 640, height: 400 });
    assert!(s.wait(10, |s| s.shared > shared && s.size == (640, 400)), "neue Größe: {:?}", s.size);
    assert_eq!(wait_ev(&mut s, &rx, "w=", 10).as_deref(), Some("640"));
    assert!(s.wait(5, |s| s.frames > 0 && near(s.pixel(600, 350), [0xff, 0x00, 0x00])));

    // Navigation über die Kopfzeile: Neu laden ⇒ grüne Seite wieder.
    s.command(serde_json::json!({"op": "nav", "action": "reload"}));
    assert!(s.wait(20, |s| near(s.pixel(600, 350), [0x00, 0xaa, 0x00])), "neu laden");
    assert_eq!(viewable_windows(&x.display), 0);
    drop(s);
    drop(x);
}

#[test]
fn tiles_render_independently() {
    let Some(x) = xvfb() else { return };
    let data = tempfile::tempdir().unwrap();
    let (port, rx) = http_server();
    let a_opts = serde_json::json!({"url": format!("http://127.0.0.1:{port}/rot"), "allow_http": true, "profile": "rot"});
    let b_opts = serde_json::json!({"url": format!("http://127.0.0.1:{port}/blau"), "allow_http": true, "profile": "blau"});
    let mut a = Sess::start(&x.display, data.path(), a_opts, 500, 400, false);
    let mut b = Sess::start(&x.display, data.path(), b_opts, 320, 300, false);
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && !(near(a.pixel(100, 100), [0, 0, 0xff]) && b.pixel(100, 100)[0] >= 0xe0 && b.pixel(100, 100)[2] < 8) {
        a.pump(Duration::from_millis(20));
        b.pump(Duration::from_millis(20));
    }
    assert_eq!(a.size, (500, 400));
    assert_eq!(b.size, (320, 300));
    assert!(near(a.pixel(100, 100), [0, 0, 0xff]), "Kachel A rot: {:?}", a.pixel(100, 100));
    assert!(b.pixel(100, 100)[0] >= 0xe0 && b.pixel(100, 100)[2] < 8, "Kachel B blau: {:?}", b.pixel(100, 100));
    assert_ne!(a.checksum(), b.checksum());
    // A hat den Fokus, B nicht; B rendert trotzdem weiter.
    a.command(serde_json::json!({"op": "focus", "on": true}));
    b.command(serde_json::json!({"op": "focus", "on": false}));
    let fb = b.frames;
    b.wait(2, |_| false);
    assert!(b.frames >= fb + 5, "Kachel ohne Fokus rendert weiter ({} Bilder)", b.frames - fb);
    // Eingabe nur an A.
    a.click(50, 50, MouseButton::Left);
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && got.is_empty() {
        a.pump(Duration::from_millis(20));
        b.pump(Duration::from_millis(20));
        while let Ok((p, _)) = rx.try_recv() {
            if p.starts_with("/ev") {
                got.push(p);
            }
        }
    }
    assert_eq!(got, ["/ev?rot"]);
    assert_eq!(viewable_windows(&x.display), 0);
    drop(a);
    drop(b);
    drop(x);
}

/// Startet den Worker gegen `url` (Anmeldeseite) und wartet auf die Anmeldung.
fn login_run(display: &str, data: &std::path::Path, url: &str, accept: bool, ignore: bool, rx: &mpsc::Receiver<(String, String)>) -> (Option<String>, Sess) {
    login_run_with(display, data, url, accept, ignore, serde_json::json!({"username_selector": "#user", "password_selector": "#pw", "submit_selector": "#go"}), rx)
}

fn login_run_with(display: &str, data: &std::path::Path, url: &str, accept: bool, ignore: bool, login: serde_json::Value, rx: &mpsc::Receiver<(String, String)>) -> (Option<String>, Sess) {
    let opts = serde_json::json!({
        "url": url,
        "allow_http": true,
        "ignore_certificate": ignore,
        "profile": "6f1c0000-0000-4000-8000-0000000000aa",
        "login": login
    });
    let mut s = Sess::start(display, data, opts, 800, 600, accept);
    let deadline = Instant::now() + Duration::from_secs(if accept { 60 } else { 8 });
    let mut posted = None;
    while Instant::now() < deadline && posted.is_none() {
        s.pump(Duration::from_millis(50));
        while let Ok((path, body)) = rx.try_recv() {
            if path == "/login" {
                posted = Some(body);
            }
        }
    }
    (posted, s)
}

fn expect_login(posted: &Option<String>, s: &Sess) {
    let body = posted.as_ref().unwrap_or_else(|| panic!("keine Anmeldung gesendet; Meldungen: {:#?}", s.log));
    let fields = form_decode(body);
    assert_eq!(fields, [("user".to_string(), "admin".to_string()), ("pw".to_string(), PASSWORD.to_string())]);
}

#[test]
fn form_login_and_certificate_warning_offscreen() {
    let Some(x) = xvfb() else { return };
    let display = x.display.clone();
    let data = tempfile::tempdir().unwrap();

    // HTTP: Anmeldung über das eingespeiste Skript; das Bild kommt über den geteilten Speicher.
    let (port, rx) = http_server();
    let (posted, mut s) = login_run(&display, data.path(), &format!("http://127.0.0.1:{port}/"), true, false, &rx);
    expect_login(&posted, &s);
    assert_eq!(s.questions, 0);
    assert!(s.wait(10, |s| s.frames > 0 && s.shm.is_some()), "kein Bild im geteilten Speicher");
    drop(s);
    let profile = data.path().join("remotejoe/web-profile/6f1c0000-0000-4000-8000-0000000000aa");
    assert!(profile.is_dir());

    // HTTPS mit selbstsigniertem Zertifikat: erst Rückfrage, abgelehnt ⇒ keine Anmeldung.
    let pem = tempfile::tempdir().unwrap();
    let ok = Command::new("openssl")
        .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes", "-days", "2", "-subj", "/CN=web.example.test"])
        .arg("-keyout")
        .arg(pem.path().join("key.pem"))
        .arg("-out")
        .arg(pem.path().join("cert.pem"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("NICHT AUSGEFÜHRT (TLS-Teil): openssl fehlt");
        return;
    }
    use rustls::pki_types::pem::PemObject;
    let certs = rustls::pki_types::CertificateDer::pem_file_iter(pem.path().join("cert.pem")).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    let key = rustls::pki_types::PrivateKeyDer::from_pem_file(pem.path().join("key.pem")).unwrap();
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || serve(l, tx, Some(Arc::new(cfg))));
    let url = format!("https://127.0.0.1:{port}/");
    let (posted, s) = login_run(&display, data.path(), &url, false, false, &rx);
    assert!(posted.is_none(), "abgelehntes Zertifikat darf nicht zur Anmeldung führen");
    assert!(s.questions >= 1, "Rückfrage erwartet: {:#?}", s.log);
    assert!(s.log.iter().any(|t| t.contains("nicht vertrauenswürdig")), "{:#?}", s.log);
    drop(s);

    // Angenommen ⇒ Anmeldung; Fingerabdruck im Profil gespeichert.
    let (posted, s) = login_run(&display, data.path(), &url, true, false, &rx);
    expect_login(&posted, &s);
    assert_eq!(s.questions, 1);
    assert!(profile.join("zertifikate.json").is_file());
    drop(s);

    // Neuer Start mit demselben Profil: keine Rückfrage mehr.
    let (posted, s) = login_run(&display, data.path(), &url, true, false, &rx);
    expect_login(&posted, &s);
    assert_eq!(s.questions, 0);
    drop(s);

    // Option „Zertifikatsprüfung abschalten“: frisches Profil, keine Rückfrage, nichts gespeichert.
    let fresh = tempfile::tempdir().unwrap();
    let (posted, s) = login_run(&display, fresh.path(), &url, true, true, &rx);
    expect_login(&posted, &s);
    assert_eq!(s.questions, 0, "mit abgeschalteter Prüfung darf es keine Rückfrage geben: {:#?}", s.log);
    assert!(!fresh.path().join("remotejoe/web-profile/6f1c0000-0000-4000-8000-0000000000aa/zertifikate.json").exists());
    assert_eq!(viewable_windows(&display), 0);
    drop(s);
    drop(x);
}

#[test]
fn auto_login_and_field_detection_offscreen() {
    let Some(x) = xvfb() else { return };
    let data = tempfile::tempdir().unwrap();
    let (port, rx) = http_server();
    let url = format!("http://127.0.0.1:{port}/");

    // Ohne Selektoren erkennt das Anmeldeskript Benutzerfeld, Kennwortfeld und Knopf selbst.
    let (posted, s) = login_run_with(&x.display, data.path(), &url, true, false, serde_json::json!({"username_selector": null, "password_selector": null, "submit_selector": null}), &rx);
    expect_login(&posted, &s);
    drop(s);

    // „Felder erkennen“: der Worker meldet die sichtbaren Felder mit Selektor und Beschriftung.
    let opts = serde_json::json!({"url": url, "allow_http": true, "profile": "erkennen", "login": {}});
    let mut s = Sess::start(&x.display, data.path(), opts, 800, 600, true);
    assert!(s.wait(60, |s| s.custom.iter().any(|c| c["ev"] == "status" && c["loading"] == false)), "Seite nicht geladen: {:?}", s.log);
    s.command(serde_json::json!({"op": "detect"}));
    assert!(s.wait(20, |s| s.custom.iter().any(|c| c["ev"] == "fields")), "keine Felder gemeldet: {:?}", s.custom);
    let f = s.custom.iter().find(|c| c["ev"] == "fields").unwrap().clone();
    let sel = |k: &str| -> Vec<String> { f[k].as_array().unwrap().iter().map(|e| e["selector"].as_str().unwrap().to_string()).collect() };
    assert_eq!(sel("users"), ["#user"], "{f}");
    assert_eq!(sel("passwords"), ["#pw"], "{f}");
    assert!(sel("submits").contains(&"#go".to_string()), "{f}");
    // Werte der Felder werden nie gelesen.
    assert!(!f.to_string().contains(PASSWORD), "{f}");
}

#[test]
fn login_like_by_hand_on_strict_and_blur_pages() {
    let Some(x) = xvfb() else { return };
    let data = tempfile::tempdir().unwrap();
    let (port, rx) = http_server();
    let auto = serde_json::json!({"username_selector": null, "password_selector": null, "submit_selector": null});
    // Knopf erst nach echter Eingabe frei.
    let (posted, s) = login_run_with(&x.display, data.path(), &format!("http://127.0.0.1:{port}/streng"), true, false, auto.clone(), &rx);
    expect_login(&posted, &s);
    drop(s);
    // Werte erst beim Verlassen des Feldes übernommen, zufällige IDs, kein Formular.
    let (posted, s) = login_run_with(&x.display, data.path(), &format!("http://127.0.0.1:{port}/zk"), true, false, auto, &rx);
    expect_login(&posted, &s);
    drop(s);
}

#[test]
fn field_detection_avoids_random_ids() {
    let Some(x) = xvfb() else { return };
    let data = tempfile::tempdir().unwrap();
    let (port, _rx) = http_server();
    let opts = serde_json::json!({"url": format!("http://127.0.0.1:{port}/zk"), "allow_http": true, "profile": "erkennen-zk"});
    let mut s = Sess::start(&x.display, data.path(), opts, 800, 600, true);
    assert!(s.wait(60, |s| s.custom.iter().any(|c| c["ev"] == "status" && c["loading"] == false)), "Seite nicht geladen");
    s.command(serde_json::json!({"op": "detect"}));
    assert!(s.wait(20, |s| s.custom.iter().any(|c| c["ev"] == "fields")), "keine Felder gemeldet");
    let f = s.custom.iter().find(|c| c["ev"] == "fields").unwrap().clone();
    let sel = |k: &str| -> Vec<String> { f[k].as_array().unwrap().iter().map(|e| e["selector"].as_str().unwrap().to_string()).collect() };
    assert_eq!(sel("users"), [r#"input[placeholder="Benutzername"]"#], "{f}");
    assert_eq!(sel("passwords"), [r#"input[placeholder="Kennwort"]"#], "{f}");
    assert!(sel("submits").contains(&"button.btn-login".to_string()), "{f}");
}

#[test]
fn login_without_submit_button_clicks_the_login_div() {
    let Some(x) = xvfb() else { return };
    let data = tempfile::tempdir().unwrap();
    let (port, rx) = http_server();
    let url = format!("http://127.0.0.1:{port}/homematic");
    let (posted, s) = login_run_with(&x.display, data.path(), &url, true, false, serde_json::json!({"username_selector": null, "password_selector": null, "submit_selector": null}), &rx);
    let body = posted.unwrap_or_else(|| panic!("keine Anmeldung gesendet; Meldungen: {:#?}", s.log));
    assert_eq!(form_decode(&body), [("tbUsernameShow".to_string(), "admin".to_string()), ("user".to_string(), "admin".to_string()), ("pw".to_string(), PASSWORD.to_string())]);
    drop(s);
    // „Felder erkennen“ nennt den div als Absende-Knopf.
    let opts = serde_json::json!({"url": url, "allow_http": true, "profile": "erkennen-hm"});
    let mut s = Sess::start(&x.display, data.path(), opts, 800, 600, true);
    assert!(s.wait(60, |s| s.custom.iter().any(|c| c["ev"] == "status" && c["loading"] == false)), "Seite nicht geladen");
    s.command(serde_json::json!({"op": "detect"}));
    assert!(s.wait(20, |s| s.custom.iter().any(|c| c["ev"] == "fields")), "keine Felder gemeldet");
    let f = s.custom.iter().find(|c| c["ev"] == "fields").unwrap().clone();
    let subs: Vec<&str> = f["submits"].as_array().unwrap().iter().map(|e| e["selector"].as_str().unwrap()).collect();
    assert!(subs.contains(&"#btnLogin"), "{f}");
}

#[test]
fn late_form_in_a_web_component_is_found_after_waiting() {
    let Some(x) = xvfb() else { return };
    let data = tempfile::tempdir().unwrap();
    let (port, rx) = http_server();
    let url = format!("http://127.0.0.1:{port}/spaet");
    // Automatik: das Skript wartet, bis das Formular erscheint, und findet es im Shadow-DOM.
    let (posted, s) = login_run_with(&x.display, data.path(), &url, true, false, serde_json::json!({"username_selector": null, "password_selector": null, "submit_selector": null}), &rx);
    expect_login(&posted, &s);
    drop(s);
    // Auch eingetragene Selektoren werden in Webkomponenten gefunden.
    let (posted, s) = login_run_with(&x.display, data.path(), &url, true, false, serde_json::json!({"username_selector": "#user", "password_selector": "#pw", "submit_selector": "#go"}), &rx);
    expect_login(&posted, &s);
    drop(s);
    // „Felder erkennen“ nach dem Aufbau des Formulars.
    // Ohne Anmeldeoptionen: es wird nichts eingetragen oder abgeschickt (wie beim Probelauf im Programm).
    let opts = serde_json::json!({"url": url, "allow_http": true, "profile": "erkennen-spaet"});
    let mut s = Sess::start(&x.display, data.path(), opts, 800, 600, true);
    assert!(s.wait(60, |s| s.custom.iter().any(|c| c["ev"] == "status" && c["loading"] == false)), "Seite nicht geladen");
    // `pump` kehrt bei der ersten Meldung zurück: echte Wartezeit, bis das Formular aufgebaut ist.
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(3) {
        s.pump(Duration::from_millis(50));
    }
    s.command(serde_json::json!({"op": "detect"}));
    assert!(s.wait(20, |s| s.custom.iter().any(|c| c["ev"] == "fields")), "keine Felder gemeldet");
    let f = s.custom.iter().find(|c| c["ev"] == "fields").unwrap().clone();
    let sel = |k: &str| -> Vec<String> { f[k].as_array().unwrap().iter().map(|e| e["selector"].as_str().unwrap().to_string()).collect() };
    assert_eq!(sel("users"), ["#user"], "{f}");
    assert_eq!(sel("passwords"), ["#pw"], "{f}");
    assert!(sel("submits").contains(&"#go".to_string()), "{f}");
    assert!(f["users"][0]["label"].as_str().unwrap().contains("Benutzername"), "{f}");
}
