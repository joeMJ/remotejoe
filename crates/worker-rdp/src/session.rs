//! Sichere Hülle um die C-Anbindung: Verbindung, Bild, Eingabe und
//! Zwischenablage-Ablauf.

use std::collections::VecDeque;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::Mutex;

use rj_core::ClipboardDirection;

use crate::clip::{self, ClipError, FileDesc};
use crate::ffi;

/// Sicherheitsstufe der Verbindung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    #[default]
    Negotiate,
    Nla,
    Tls,
    Rdp,
}

/// Verbindungsparameter.
#[derive(Debug, Clone)]
pub struct RdpParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub domain: Option<String>,
    /// UTF-8, byte-genau; FreeRDP setzt für NLA nach UTF-16 um.
    pub password: rj_core::Secret,
    pub width: u32,
    pub height: u32,
    pub security: Security,
    pub ignore_certificate: bool,
    pub clipboard: ClipboardDirection,
    pub clipboard_limit: u64,
    pub keyboard_layout: u32,
    pub config_path: Option<String>,
    pub timeout_ms: u32,
    /// Grafikkanal (RDPGFX) anbieten; GNOME-Remote-Desktop verlangt ihn.
    pub gfx: bool,
}

/// Angaben zu einem Serverzertifikat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    pub host: String,
    pub port: u16,
    pub subject: String,
    pub issuer: String,
    pub fingerprint: String,
    /// Gesetzt, wenn sich das Zertifikat geändert hat.
    pub old_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertDecision {
    Accept,
    AcceptOnce,
    Reject,
}

/// Ereignisse für den Worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RdpEvent {
    /// Teilbild geändert (Bereich im Bildpuffer).
    Frame { x: u32, y: u32, w: u32, h: u32 },
    Resized { width: u32, height: u32 },
    ClipReady,
    ClipText(String),
    ClipFiles(Vec<(FileDesc, Vec<u8>)>),
    Notice(String),
    Log(String),
}

#[derive(Debug, thiserror::Error)]
pub enum RdpError {
    #[error("{0}")]
    Connect(String),
    #[error("FreeRDP konnte nicht angelegt werden")]
    Init,
    #[error("Zeichenkette enthält ein Nullzeichen")]
    Nul,
    #[error("Zwischenablage: {0}")]
    Clip(#[from] ClipError),
    #[error("Zwischenablage nicht verfügbar (Gegenstelle gibt sie nicht frei oder Richtung gesperrt)")]
    ClipUnavailable,
}

enum Raw {
    Ready,
    FormatList(Vec<(u32, String)>),
    DataRequest(u32),
    DataResponse(Option<Vec<u8>>),
    FileRequest { stream: u32, index: u32, flags: u32, position: u64, len: u32 },
    FileResponse { stream: u32, data: Option<Vec<u8>> },
    Resized(u32, u32),
}

#[derive(Default)]
struct Frame {
    buf: Vec<u8>,
    width: u32,
    height: u32,
    dirty: Option<(u32, u32, u32, u32)>,
}

type CertFn = Box<dyn Fn(&CertInfo) -> CertDecision + Send + Sync>;

struct Shared {
    raw: Mutex<VecDeque<Raw>>,
    frame: Mutex<Frame>,
    cert: CertFn,
}

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        // SAFETY: FreeRDP übergibt nullterminierte Zeichenketten.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

unsafe fn shared<'a>(user: *mut c_void) -> &'a Shared {
    // SAFETY: `user` zeigt auf das `Shared` der Sitzung, das länger lebt als die C-Seite.
    unsafe { &*(user as *const Shared) }
}

fn push(s: &Shared, r: Raw) {
    s.raw.lock().unwrap_or_else(|e| e.into_inner()).push_back(r);
}

unsafe extern "C" fn cb_frame(user: *mut c_void, data: *const u8, width: u32, height: u32, stride: u32, x: i32, y: i32, w: i32, h: i32) {
    let s = unsafe { shared(user) };
    let mut f = s.frame.lock().unwrap_or_else(|e| e.into_inner());
    let size = width as usize * height as usize * 4;
    if f.width != width || f.height != height || f.buf.len() != size {
        f.buf = vec![0; size];
        f.width = width;
        f.height = height;
        f.dirty = Some((0, 0, width, height));
    }
    let x0 = x.clamp(0, width as i32) as usize;
    let y0 = y.clamp(0, height as i32) as usize;
    let x1 = (x + w).clamp(0, width as i32) as usize;
    let y1 = (y + h).clamp(0, height as i32) as usize;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    // SAFETY: Der Puffer von FreeRDP umfasst `stride * height` Bytes.
    let src = unsafe { std::slice::from_raw_parts(data, stride as usize * height as usize) };
    let dst_stride = width as usize * 4;
    for row in y0..y1 {
        let s0 = row * stride as usize + x0 * 4;
        let d0 = row * dst_stride + x0 * 4;
        let n = (x1 - x0) * 4;
        f.buf[d0..d0 + n].copy_from_slice(&src[s0..s0 + n]);
    }
    let r = (x0 as u32, y0 as u32, x1 as u32, y1 as u32);
    f.dirty = Some(match f.dirty {
        None => r,
        Some((a, b, c, d)) => (a.min(r.0), b.min(r.1), c.max(r.2), d.max(r.3)),
    });
}

unsafe extern "C" fn cb_resize(user: *mut c_void, w: u32, h: u32) {
    push(unsafe { shared(user) }, Raw::Resized(w, h));
}

unsafe extern "C" fn cb_clip_ready(user: *mut c_void, _flags: u32) {
    push(unsafe { shared(user) }, Raw::Ready);
}

unsafe extern "C" fn cb_format_list(user: *mut c_void, n: u32, ids: *const u32, names: *const *const c_char) {
    let mut list = Vec::with_capacity(n as usize);
    for i in 0..n as usize {
        // SAFETY: C-Seite übergibt `n` Einträge.
        let (id, name) = unsafe { (*ids.add(i), cstr(*names.add(i))) };
        list.push((id, name));
    }
    push(unsafe { shared(user) }, Raw::FormatList(list));
}

unsafe extern "C" fn cb_data_request(user: *mut c_void, id: u32) {
    push(unsafe { shared(user) }, Raw::DataRequest(id));
}

unsafe fn bytes(data: *const u8, len: u32) -> Vec<u8> {
    if data.is_null() || len == 0 {
        Vec::new()
    } else {
        // SAFETY: C-Seite übergibt `len` gültige Bytes.
        unsafe { std::slice::from_raw_parts(data, len as usize) }.to_vec()
    }
}

unsafe extern "C" fn cb_data_response(user: *mut c_void, ok: i32, data: *const u8, len: u32) {
    let d = (ok != 0).then(|| unsafe { bytes(data, len) });
    push(unsafe { shared(user) }, Raw::DataResponse(d));
}

unsafe extern "C" fn cb_file_request(user: *mut c_void, stream: u32, index: u32, flags: u32, position: u64, len: u32) {
    push(unsafe { shared(user) }, Raw::FileRequest { stream, index, flags, position, len });
}

unsafe extern "C" fn cb_file_response(user: *mut c_void, stream: u32, ok: i32, data: *const u8, len: u32) {
    let d = (ok != 0).then(|| unsafe { bytes(data, len) });
    push(unsafe { shared(user) }, Raw::FileResponse { stream, data: d });
}

unsafe extern "C" fn cb_cert(
    user: *mut c_void,
    host: *const c_char,
    port: u16,
    subject: *const c_char,
    issuer: *const c_char,
    fingerprint: *const c_char,
    old: *const c_char,
) -> u32 {
    let s = unsafe { shared(user) };
    let info = unsafe {
        CertInfo {
            host: cstr(host),
            port,
            subject: cstr(subject),
            issuer: cstr(issuer),
            fingerprint: cstr(fingerprint),
            old_fingerprint: (!old.is_null()).then(|| cstr(old)),
        }
    };
    match (s.cert)(&info) {
        CertDecision::Accept => 1,
        CertDecision::AcceptOnce => 2,
        CertDecision::Reject => 0,
    }
}

enum Pending {
    Text,
    FileGroup,
}

struct Incoming {
    files: Vec<FileDesc>,
    index: usize,
    data: Vec<Vec<u8>>,
    stream: u32,
}

#[derive(Default)]
struct LocalOffer {
    text: Option<Vec<u8>>,
    group: Option<Vec<u8>>,
    files: Vec<Vec<u8>>,
}

/// Teilgröße beim Abholen von Dateiinhalten.
const FILE_CHUNK: u32 = 1024 * 1024;

/// RDP-Sitzung.
pub struct RdpSession {
    raw: *mut ffi::rj_rdp,
    shared: Box<Shared>,
    pub direction: ClipboardDirection,
    pub limit: u64,
    clip_ready: bool,
    pending: VecDeque<Pending>,
    incoming: Option<Incoming>,
    local: LocalOffer,
    next_stream: u32,
    finished_files: Option<Vec<(FileDesc, Vec<u8>)>>,
    /// Protokoll nur mit Länge und Prüfsumme.
    pub audit: bool,
}

// SAFETY: Die Sitzung wird nur von einem Faden benutzt; die C-Seite
// synchronisiert Kanalzugriffe selbst.
unsafe impl Send for RdpSession {}

impl RdpSession {
    pub fn new(cert: impl Fn(&CertInfo) -> CertDecision + Send + Sync + 'static) -> Result<Self, RdpError> {
        let shared = Box::new(Shared { raw: Mutex::new(VecDeque::new()), frame: Mutex::new(Frame::default()), cert: Box::new(cert) });
        let cb = ffi::rj_callbacks {
            user: &*shared as *const Shared as *mut c_void,
            on_frame: Some(cb_frame),
            on_desktop_resize: Some(cb_resize),
            on_clip_ready: Some(cb_clip_ready),
            on_clip_format_list: Some(cb_format_list),
            on_clip_data_request: Some(cb_data_request),
            on_clip_data_response: Some(cb_data_response),
            on_clip_file_request: Some(cb_file_request),
            on_clip_file_response: Some(cb_file_response),
            verify_certificate: Some(cb_cert),
        };
        // SAFETY: `cb` wird kopiert; `shared` lebt so lange wie die Sitzung.
        let raw = unsafe { ffi::rj_rdp_new(&cb) };
        if raw.is_null() {
            return Err(RdpError::Init);
        }
        Ok(RdpSession {
            raw,
            shared,
            direction: ClipboardDirection::Both,
            limit: 16 * 1024 * 1024,
            clip_ready: false,
            pending: VecDeque::new(),
            incoming: None,
            local: LocalOffer::default(),
            next_stream: 1,
            finished_files: None,
            audit: false,
        })
    }

    fn last_error(&self) -> String {
        // SAFETY: liefert einen Zeiger in die Sitzung.
        unsafe { cstr(ffi::rj_rdp_last_error(self.raw)) }
    }

    pub fn connect(&mut self, p: &RdpParams) -> Result<(), RdpError> {
        let c = |s: &str| CString::new(s).map_err(|_| RdpError::Nul);
        let host = c(&p.host)?;
        let user = c(&p.username)?;
        let domain = c(p.domain.as_deref().unwrap_or(""))?;
        let pw_str = p.password.as_str().ok_or_else(|| RdpError::Connect("Kennwort ist kein gültiges UTF-8".into()))?;
        let password = zeroizing_cstring(pw_str)?;
        let config = c(p.config_path.as_deref().unwrap_or(""))?;
        self.direction = p.clipboard;
        self.limit = p.clipboard_limit;
        let params = ffi::rj_params {
            host: host.as_ptr(),
            port: p.port,
            username: user.as_ptr(),
            domain: domain.as_ptr(),
            password: password.0.as_ptr() as *const c_char,
            width: p.width,
            height: p.height,
            security: match p.security {
                Security::Negotiate => ffi::rj_security_RJ_SEC_NEGOTIATE,
                Security::Nla => ffi::rj_security_RJ_SEC_NLA,
                Security::Tls => ffi::rj_security_RJ_SEC_TLS,
                Security::Rdp => ffi::rj_security_RJ_SEC_RDP,
            } as i32,
            ignore_certificate: p.ignore_certificate as i32,
            clipboard: (p.clipboard != ClipboardDirection::Off) as i32,
            keyboard_layout: p.keyboard_layout,
            config_path: config.as_ptr(),
            timeout_ms: p.timeout_ms,
            gfx: p.gfx as i32,
        };
        // SAFETY: alle Zeiger leben bis zum Ende des Aufrufs.
        let rc = unsafe { ffi::rj_rdp_connect(self.raw, &params) };
        if rc != 0 {
            return Err(RdpError::Connect(self.last_error()));
        }
        Ok(())
    }

    /// Verarbeitet Netzereignisse; `false`, wenn die Verbindung getrennt ist.
    pub fn poll(&mut self, timeout_ms: u32, events: &mut Vec<RdpEvent>) -> bool {
        // SAFETY: gültige Sitzung.
        let alive = unsafe { ffi::rj_rdp_poll(self.raw, timeout_ms) } == 0;
        self.process(events);
        if let Some((x0, y0, x1, y1)) = self.shared.frame.lock().unwrap_or_else(|e| e.into_inner()).dirty.take() {
            events.push(RdpEvent::Frame { x: x0, y: y0, w: x1 - x0, h: y1 - y0 });
        }
        if !alive {
            events.push(RdpEvent::Log(self.last_error()));
        }
        alive
    }

    /// Zugriff auf den Bildpuffer (BGRA32, Breite, Höhe).
    pub fn with_frame<R>(&self, f: impl FnOnce(&[u8], u32, u32) -> R) -> R {
        let fr = self.shared.frame.lock().unwrap_or_else(|e| e.into_inner());
        f(&fr.buf, fr.width, fr.height)
    }

    /// Grund des Servers für das Verbindungsende (0 = keiner).
    pub fn error_info(&self) -> u32 {
        // SAFETY: gültige Sitzung.
        unsafe { ffi::rj_rdp_error_info(self.raw) }
    }

    pub fn disconnect(&mut self) {
        // SAFETY: gültige Sitzung.
        unsafe { ffi::rj_rdp_disconnect(self.raw) };
    }

    /// Text als Unicode-Tastenereignisse senden (unabhängig von der Belegung).
    pub fn send_text(&mut self, text: &str) {
        for u in text.encode_utf16() {
            // SAFETY: gültige Sitzung.
            unsafe {
                ffi::rj_rdp_send_unicode(self.raw, u, 1);
                ffi::rj_rdp_send_unicode(self.raw, u, 0);
            }
        }
    }

    pub fn send_key(&mut self, keysym: u32, down: bool) -> bool {
        match crate::keymap::scancode(keysym) {
            Some(sc) => {
                // SAFETY: gültige Sitzung.
                unsafe { ffi::rj_rdp_send_scancode(self.raw, sc, down as i32) };
                true
            }
            None => false,
        }
    }

    pub fn send_mouse(&mut self, flags: u16, x: u16, y: u16) {
        // SAFETY: gültige Sitzung.
        unsafe { ffi::rj_rdp_send_mouse(self.raw, flags, x, y) };
    }

    /// Größenänderung über den Anzeige-Steuerkanal; `false`, wenn die
    /// Gegenstelle ihn nicht anbietet.
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        // SAFETY: gültige Sitzung.
        unsafe { ffi::rj_rdp_resize(self.raw, width, height) == 0 }
    }

    pub fn clipboard_ready(&self) -> bool {
        self.clip_ready
    }

    fn send_format_list(&mut self, formats: &[(u32, &str)]) -> Result<(), RdpError> {
        let ids: Vec<u32> = formats.iter().map(|f| f.0).collect();
        let names: Vec<CString> = formats.iter().map(|f| CString::new(f.1).map_err(|_| RdpError::Nul)).collect::<Result<_, _>>()?;
        let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
        // SAFETY: Felder leben bis zum Ende des Aufrufs.
        let rc = unsafe { ffi::rj_clip_send_format_list(self.raw, ids.len() as u32, ids.as_ptr(), ptrs.as_ptr()) };
        if rc != 0 {
            return Err(RdpError::ClipUnavailable);
        }
        Ok(())
    }

    fn check_send(&self, size: u64) -> Result<(), RdpError> {
        if !self.clip_ready || !clip::allows_host_to_target(self.direction) {
            return Err(RdpError::ClipUnavailable);
        }
        if size > self.limit {
            return Err(ClipError::TooLarge { size, limit: self.limit }.into());
        }
        Ok(())
    }

    /// Text in die Zwischenablage des Ziels anbieten.
    pub fn offer_text(&mut self, text: &str) -> Result<(), RdpError> {
        let enc = clip::encode_text(text);
        self.check_send(enc.len() as u64)?;
        self.local = LocalOffer { text: Some(enc), ..Default::default() };
        self.send_format_list(&[(clip::CF_UNICODETEXT, "")])
    }

    /// Dateien in die Zwischenablage des Ziels anbieten.
    pub fn offer_files(&mut self, files: Vec<(String, Vec<u8>)>) -> Result<(), RdpError> {
        let total: u64 = files.iter().map(|f| f.1.len() as u64).sum();
        self.check_send(total)?;
        let descs: Vec<FileDesc> = files.iter().map(|(n, d)| FileDesc { name: n.clone(), size: d.len() as u64, is_dir: false }).collect();
        let group = clip::encode_file_group(&descs)?;
        self.local = LocalOffer { text: None, group: Some(group), files: files.into_iter().map(|f| f.1).collect() };
        self.send_format_list(&[(clip::LOCAL_FILEGROUP_ID, clip::FILEGROUP_NAME)])
    }

    fn respond(&mut self, data: Option<&[u8]>) {
        let (ok, ptr, len) = match data {
            Some(d) => (1, d.as_ptr(), d.len() as u32),
            None => (0, std::ptr::null(), 0),
        };
        // SAFETY: Daten leben bis zum Ende des Aufrufs.
        unsafe { ffi::rj_clip_send_data_response(self.raw, ok, ptr, len) };
    }

    fn request_next_file_chunk(&mut self) {
        let Some(inc) = &mut self.incoming else { return };
        while inc.index < inc.files.len() && (inc.files[inc.index].is_dir || inc.data[inc.index].len() as u64 >= inc.files[inc.index].size) {
            inc.index += 1;
        }
        if inc.index >= inc.files.len() {
            let inc = self.incoming.take().expect("vorhanden");
            let _ = inc.stream;
            return self.finish_files(inc);
        }
        let have = inc.data[inc.index].len() as u64;
        let want = (inc.files[inc.index].size - have).min(FILE_CHUNK as u64) as u32;
        self.next_stream += 1;
        inc.stream = self.next_stream;
        // SAFETY: gültige Sitzung.
        unsafe { ffi::rj_clip_request_file(self.raw, inc.stream, inc.index as u32, clip::FILECONTENTS_RANGE, have, want) };
    }

    fn finish_files(&mut self, inc: Incoming) {
        let files = inc.files.into_iter().zip(inc.data).collect();
        self.finished_files = Some(files);
    }

    fn process(&mut self, events: &mut Vec<RdpEvent>) {
        let raws: Vec<Raw> = self.shared.raw.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect();
        for r in raws {
            match r {
                Raw::Ready => {
                    self.clip_ready = true;
                    events.push(RdpEvent::ClipReady);
                }
                Raw::Resized(w, h) => events.push(RdpEvent::Resized { width: w, height: h }),
                Raw::FormatList(list) => {
                    let ids: Vec<String> = list.iter().map(|(id, n)| if n.is_empty() { id.to_string() } else { format!("{id}:{n}") }).collect();
                    events.push(RdpEvent::Log(format!("Formate der Gegenstelle: {}", ids.join(", "))));
                    if !clip::allows_target_to_host(self.direction) {
                        continue;
                    }
                    if let Some((id, _)) = list.iter().find(|(_, n)| n == clip::FILEGROUP_NAME) {
                        self.pending.push_back(Pending::FileGroup);
                        // SAFETY: gültige Sitzung.
                        unsafe { ffi::rj_clip_request_data(self.raw, *id) };
                    } else if list.iter().any(|(id, _)| *id == clip::CF_UNICODETEXT) {
                        self.pending.push_back(Pending::Text);
                        // SAFETY: gültige Sitzung.
                        unsafe { ffi::rj_clip_request_data(self.raw, clip::CF_UNICODETEXT) };
                    }
                }
                Raw::DataRequest(id) => {
                    let allowed = clip::allows_host_to_target(self.direction);
                    let data = match id {
                        clip::CF_UNICODETEXT if allowed => self.local.text.clone(),
                        clip::LOCAL_FILEGROUP_ID if allowed => self.local.group.clone(),
                        _ => None,
                    };
                    if self.audit {
                        if let Some(d) = &data {
                            events.push(RdpEvent::Log(clip::audit_line("Host→Ziel", "Format", d)));
                        }
                    }
                    self.respond(data.as_deref());
                }
                Raw::DataResponse(data) => match (self.pending.pop_front(), data) {
                    (Some(Pending::Text), Some(d)) => {
                        if d.len() as u64 > self.limit {
                            events.push(RdpEvent::Notice(ClipError::TooLarge { size: d.len() as u64, limit: self.limit }.to_string()));
                            continue;
                        }
                        if self.audit {
                            events.push(RdpEvent::Log(clip::audit_line("Ziel→Host", "Text", &d)));
                        }
                        match clip::decode_text(&d) {
                            Ok(t) => events.push(RdpEvent::ClipText(t)),
                            Err(e) => events.push(RdpEvent::Notice(e.to_string())),
                        }
                    }
                    (Some(Pending::FileGroup), Some(d)) => match clip::decode_file_group(&d) {
                        Ok(files) => {
                            let total: u64 = files.iter().map(|f| f.size).sum();
                            if total > self.limit {
                                events.push(RdpEvent::Notice(ClipError::TooLarge { size: total, limit: self.limit }.to_string()));
                                continue;
                            }
                            let n = files.len();
                            self.incoming = Some(Incoming { files, index: 0, data: vec![Vec::new(); n], stream: 0 });
                            self.request_next_file_chunk();
                        }
                        Err(e) => events.push(RdpEvent::Notice(e.to_string())),
                    },
                    (Some(_), None) => events.push(RdpEvent::Notice("Die Gegenstelle hat die Zwischenablage nicht geliefert".into())),
                    (None, _) => {}
                },
                Raw::FileRequest { stream, index, flags, position, len } => {
                    let file = clip::allows_host_to_target(self.direction).then(|| self.local.files.get(index as usize)).flatten();
                    let reply: Option<Vec<u8>> = match file {
                        Some(d) if flags & clip::FILECONTENTS_SIZE != 0 => Some((d.len() as u64).to_le_bytes().to_vec()),
                        Some(d) if flags & clip::FILECONTENTS_RANGE != 0 => {
                            let start = (position as usize).min(d.len());
                            let end = (start + len as usize).min(d.len());
                            Some(d[start..end].to_vec())
                        }
                        _ => None,
                    };
                    let (ok, ptr, n) = match &reply {
                        Some(d) => (1, d.as_ptr(), d.len() as u32),
                        None => (0, std::ptr::null(), 0),
                    };
                    // SAFETY: Daten leben bis zum Ende des Aufrufs.
                    unsafe { ffi::rj_clip_send_file_response(self.raw, stream, ok, ptr, n) };
                }
                Raw::FileResponse { stream, data } => {
                    let Some(inc) = &mut self.incoming else { continue };
                    if inc.stream != stream {
                        continue;
                    }
                    match data {
                        Some(d) if !d.is_empty() => {
                            inc.data[inc.index].extend_from_slice(&d);
                            self.request_next_file_chunk();
                        }
                        _ => {
                            self.incoming = None;
                            events.push(RdpEvent::Notice("Dateiinhalt von der Gegenstelle nicht erhalten".into()));
                        }
                    }
                }
            }
            if let Some(files) = self.finished_files.take() {
                events.push(RdpEvent::ClipFiles(files));
            }
        }
    }
}

impl Drop for RdpSession {
    fn drop(&mut self) {
        // SAFETY: einmalige Freigabe.
        unsafe { ffi::rj_rdp_free(self.raw) };
    }
}

/// Nullterminierte Kopie, die beim Freigeben überschrieben wird.
struct ZeroCString(Vec<u8>);

impl Drop for ZeroCString {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

fn zeroizing_cstring(s: &str) -> Result<ZeroCString, RdpError> {
    if s.as_bytes().contains(&0) {
        return Err(RdpError::Nul);
    }
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    Ok(ZeroCString(v))
}
