//! Terminal-Zustandsmaschine im Worker: füttert das [`Terminal`] mit der Ausgabe
//! des Ziels, beantwortet Terminal-Anfragen selbst und liefert der Oberfläche
//! zusammengefasste Schnappschüsse.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rj_ipc::{TermCmd, TermMsg, TermNotice, TermView};

use crate::terminal::{Osc52Policy, Scheme, TermEvent, Terminal, HISTORY_DEFAULT, MIN_COLUMNS, MIN_ROWS};

/// Kürzester Abstand zwischen zwei Schnappschüssen bei laufender Ausgabe.
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(33);

/// So viele unbeantwortete Leseanfragen (OSC 52) bleiben höchstens offen.
const MAX_PENDING_LOADS: usize = 16;

/// Größte Zeilen-/Spaltenzahl und Zellenzahl des Terminals (die Oberfläche lehnt mehr ab).
pub const MAX_SIDE: usize = 1000;
pub const MAX_CELLS: usize = 250_000;

/// Ab dieser Größe wird ein Schnappschuss verkleinert (Zellentexte entfallen); weit unter `MAX_FRAME`.
pub const SNAPSHOT_LIMIT: usize = 8 * 1024 * 1024;

/// Längster Titel (Zeichen), der an die Oberfläche geht.
const MAX_TITLE_CHARS: usize = 4096;

/// Größte Zwischenablage des Ziels, die an die Oberfläche geht; größere Texte passen nicht in einen Rahmen.
const MAX_CLIPBOARD_BYTES: usize = 32 * 1024 * 1024;

/// Spalten und Zeilen auf den zulässigen Bereich begrenzen.
fn clamp_size(cols: usize, rows: usize) -> (usize, usize) {
    let cols = cols.clamp(MIN_COLUMNS, MAX_SIDE);
    let rows = rows.clamp(MIN_ROWS, MAX_SIDE).min((MAX_CELLS / cols).max(MIN_ROWS));
    (cols, rows)
}

type LoadFn = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// Ergebnis einer Verarbeitung.
#[derive(Debug, Default)]
pub struct HostOut {
    /// Bytes, die der Worker ans Ziel schreibt (Antworten, eingefügter Text).
    pub to_target: Vec<u8>,
    /// Nachrichten an die Oberfläche.
    pub msgs: Vec<TermMsg>,
}

/// Wächter gegen einen Hänger des Emulators: Läuft eine Verarbeitung des Terminals
/// länger als die Frist, wird der Worker beendet. Der Herzschlag des Workers läuft
/// in einem eigenen Faden und würde einen Hänger nicht anzeigen; so zeigt sich der
/// Hänger als Absturz dieser einen Sitzung.
#[derive(Clone)]
pub struct Watchdog {
    start: Instant,
    /// Beginn der laufenden Verarbeitung in Millisekunden seit `start` plus eins; 0: untätig.
    busy_since: Arc<AtomicU64>,
}

/// Markiert die Dauer einer Verarbeitung; endet mit dem Wert.
pub struct WatchGuard(Arc<AtomicU64>);

impl Drop for WatchGuard {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Release);
    }
}

impl Watchdog {
    pub fn new() -> Self {
        Watchdog { start: Instant::now(), busy_since: Arc::new(AtomicU64::new(0)) }
    }

    pub fn enter(&self) -> WatchGuard {
        self.busy_since.store(self.start.elapsed().as_millis() as u64 + 1, Ordering::Release);
        WatchGuard(self.busy_since.clone())
    }

    /// Startet den Wächter; `trip` wird einmal aufgerufen, wenn eine Verarbeitung länger als `limit` dauert.
    pub fn spawn(&self, limit: Duration, trip: impl FnOnce() + Send + 'static) {
        let me = self.clone();
        let step = (limit / 4).clamp(Duration::from_millis(10), Duration::from_millis(500));
        let _ = std::thread::Builder::new().name("waechter".into()).spawn(move || loop {
            std::thread::sleep(step);
            let since = me.busy_since.load(Ordering::Acquire);
            if since != 0 && me.start.elapsed().as_millis() as u64 + 1 > since + limit.as_millis() as u64 {
                trip();
                return;
            }
        });
    }
}

impl Default for Watchdog {
    fn default() -> Self {
        Self::new()
    }
}

/// Frist, nach der der Wächter den Worker beendet.
pub fn watchdog_limit() -> Duration {
    #[cfg(feature = "test-hooks")]
    if let Some(ms) = std::env::var("RJ_TEST_WATCHDOG_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
        return Duration::from_millis(ms);
    }
    Duration::from_secs(10)
}

/// Testschalter: bestimmte Ausgabe löst Panik oder Hänger im Terminal des Workers aus.
#[cfg(feature = "test-hooks")]
fn test_hooks(bytes: &[u8]) {
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    if has(b"RJ-PANIK-AUSLOESER") {
        panic!("Testschalter: Panik im Terminal");
    }
    if has(b"RJ-HAENGER-AUSLOESER") {
        loop {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

pub struct TermHost {
    watch: Watchdog,
    term: Terminal,
    scheme: Scheme,
    history: usize,
    osc52: Osc52Policy,
    tap: bool,
    snapshot_limit: usize,
    dirty: bool,
    last_sent: Option<Instant>,
    loads: VecDeque<(u32, LoadFn)>,
    next_load: u32,
}

impl Default for TermHost {
    /// Vor dem Verbindungsauftrag: noch nichts zu senden (kein Schnappschuss in falscher Größe).
    fn default() -> Self {
        let mut host = Self::new(80, 24, Osc52Policy::default());
        host.dirty = false;
        host
    }
}

impl TermHost {
    pub fn new(cols: usize, rows: usize, osc52: Osc52Policy) -> Self {
        let (cols, rows) = clamp_size(cols, rows);
        TermHost {
            watch: Watchdog::new(),
            term: Terminal::with_history(cols, rows, osc52, HISTORY_DEFAULT),
            scheme: Scheme::default(),
            history: HISTORY_DEFAULT,
            osc52,
            tap: false,
            snapshot_limit: SNAPSHOT_LIMIT,
            dirty: true,
            last_sent: None,
            loads: VecDeque::new(),
            next_load: 1,
        }
    }

    /// Neues Terminal in der Größe des Verbindungsauftrags; Farbschema, Rücklauf
    /// und die Regel für OSC 52 bleiben erhalten bzw. kommen aus dem Auftrag.
    pub fn reset(&mut self, cols: usize, rows: usize, osc52: Osc52Policy) {
        let (cols, rows) = clamp_size(cols, rows);
        self.osc52 = osc52;
        self.term = Terminal::with_history(cols, rows, osc52, self.history);
        self.term.set_scheme(self.scheme.clone());
        self.loads.clear();
        self.dirty = true;
    }

    pub fn watchdog(&self) -> &Watchdog {
        &self.watch
    }

    /// Soll die Rohausgabe zusätzlich an die Oberfläche gehen (Makro-Abgriff)?
    pub fn tap(&self) -> bool {
        self.tap
    }

    pub fn terminal(&self) -> &Terminal {
        &self.term
    }

    /// Ausgabe des Ziels verarbeiten; Schnappschüsse kommen von [`Self::poll`].
    pub fn feed(&mut self, bytes: &[u8]) -> HostOut {
        let _g = self.watch.enter();
        #[cfg(feature = "test-hooks")]
        test_hooks(bytes);
        self.term.feed(bytes);
        self.dirty = true;
        let mut out = HostOut::default();
        self.collect_events(&mut out);
        out
    }

    pub fn resize(&mut self, cols: usize, rows: usize, now: Instant) -> HostOut {
        let _g = self.watch.enter();
        let (cols, rows) = clamp_size(cols, rows);
        self.term.resize(cols, rows);
        let mut out = HostOut::default();
        self.flush(&mut out, now);
        out
    }

    pub fn command(&mut self, cmd: TermCmd, now: Instant) -> HostOut {
        let _g = self.watch.enter();
        let mut out = HostOut::default();
        let offset = self.term.display_offset();
        match cmd {
            TermCmd::Scroll(n) => {
                self.term.scroll(n);
                self.flush_if(self.term.display_offset() != offset, &mut out, now);
            }
            TermCmd::ScrollPage { back } => {
                self.term.scroll_page(back);
                self.flush_if(self.term.display_offset() != offset, &mut out, now);
            }
            TermCmd::ScrollBottom => {
                self.term.scroll_bottom();
                self.flush_if(self.term.display_offset() != offset, &mut out, now);
            }
            TermCmd::SetScheme(s) => {
                self.scheme = Scheme::from(&s);
                self.term.set_scheme(self.scheme.clone());
                self.flush(&mut out, now);
            }
            TermCmd::SetHistory(n) => {
                self.history = n as usize;
                self.term.set_history(self.history);
                self.drop_option_events();
                self.flush(&mut out, now);
            }
            TermCmd::Tap(on) => self.tap = on,
            TermCmd::Paste(text) => {
                self.term.scroll_bottom();
                out.to_target = self.term.paste_bytes(&text);
                self.flush_if(self.term.display_offset() != offset, &mut out, now);
            }
            TermCmd::SetOsc52(rule) => {
                self.osc52 = rule.into();
                self.term.set_osc52(self.osc52);
                self.drop_option_events();
            }
            TermCmd::ClipboardAnswer { id, text } => {
                if let Some(pos) = self.loads.iter().position(|(i, _)| *i == id) {
                    let (_, f) = self.loads.remove(pos).expect("vorhanden");
                    if let Some(t) = text {
                        out.to_target = f(&t).into_bytes();
                    }
                }
            }
        }
        out
    }

    /// Schnappschuss, wenn sich etwas geändert hat und der Mindestabstand
    /// zum vorigen verstrichen ist.
    pub fn poll(&mut self, now: Instant) -> Option<TermMsg> {
        let _g = self.watch.enter();
        if !self.dirty || self.wait(now).is_some_and(|w| !w.is_zero()) {
            return None;
        }
        self.dirty = false;
        self.last_sent = Some(now);
        Some(TermMsg::Snapshot(self.view()))
    }

    /// Wie lange bis zum nächsten fälligen Schnappschuss; `None`, wenn nichts ansteht.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        if !self.dirty {
            return None;
        }
        Some(match self.last_sent {
            Some(t) => (t + SNAPSHOT_INTERVAL).saturating_duration_since(now),
            None => Duration::ZERO,
        })
    }

    /// Das Setzen von Optionen meldet den Titel erneut; das ist keine neue Meldung des Ziels.
    fn drop_option_events(&mut self) {
        let _ = self.term.take_events();
    }

    fn flush_if(&mut self, cond: bool, out: &mut HostOut, now: Instant) {
        if cond {
            self.flush(out, now);
        }
    }

    fn flush(&mut self, out: &mut HostOut, now: Instant) {
        self.dirty = false;
        self.last_sent = Some(now);
        out.msgs.push(TermMsg::Snapshot(self.view()));
    }

    /// Schnappschuss; ist er zu groß, entfallen die Zellentexte (Auswahl liefert dann nichts), die Zeilen bleiben.
    fn view(&self) -> TermView {
        let mut v = self.term.view();
        if v.approx_size() > self.snapshot_limit {
            for row in &mut v.grid {
                row.cells.iter_mut().for_each(String::clear);
                row.lead_prev = None;
            }
        }
        v
    }

    /// Grenze für die Größe eines Schnappschusses setzen (Tests).
    pub fn set_snapshot_limit(&mut self, bytes: usize) {
        self.snapshot_limit = bytes;
    }

    /// Ereignisse des Terminals; bei einer Flut in einem Paket bleibt je Art nur das Nötige
    /// (eine Klingel, die letzten Titel und Zwischenablagen, wenige Leseanfragen), in der alten Reihenfolge.
    fn collect_events(&mut self, out: &mut HostOut) {
        const MAX_LOADS_PER_FEED: usize = 4;
        const KEEP_TITLES: usize = 8;
        const KEEP_STORES: usize = 2;
        let mut loads = 0;
        let mut notices: Vec<TermNotice> = Vec::new();
        for e in self.term.take_events() {
            match e {
                TermEvent::ClipboardStore(t) => {
                    if t.len() > MAX_CLIPBOARD_BYTES {
                        continue;
                    }
                    notices.push(TermNotice::ClipboardStore(t));
                }
                TermEvent::ClipboardLoad(f) => {
                    if loads >= MAX_LOADS_PER_FEED {
                        continue;
                    }
                    loads += 1;
                    let id = self.next_load;
                    self.next_load = self.next_load.wrapping_add(1);
                    if self.loads.len() >= MAX_PENDING_LOADS {
                        self.loads.pop_front();
                    }
                    self.loads.push_back((id, f));
                    notices.push(TermNotice::ClipboardLoad { id });
                }
                TermEvent::Reply(r) => out.to_target.extend_from_slice(r.as_bytes()),
                TermEvent::Title(t) => {
                    let t: String = t.chars().take(MAX_TITLE_CHARS).collect();
                    notices.push(TermNotice::Title(t));
                }
                TermEvent::Bell => {
                    if !notices.contains(&TermNotice::Bell) {
                        notices.push(TermNotice::Bell);
                    }
                }
            }
        }
        // Die ältesten Titel und Zwischenablagen entfallen, wenn es zu viele sind.
        let count = |notices: &[TermNotice], f: fn(&TermNotice) -> bool| notices.iter().filter(|n| f(n)).count();
        let is_title: fn(&TermNotice) -> bool = |n| matches!(n, TermNotice::Title(_));
        let is_store: fn(&TermNotice) -> bool = |n| matches!(n, TermNotice::ClipboardStore(_));
        let mut drop_titles = count(&notices, is_title).saturating_sub(KEEP_TITLES);
        let mut drop_stores = count(&notices, is_store).saturating_sub(KEEP_STORES);
        notices.retain(|n| {
            let drop = if is_title(n) && drop_titles > 0 {
                drop_titles -= 1;
                true
            } else if is_store(n) && drop_stores > 0 {
                drop_stores -= 1;
                true
            } else {
                false
            };
            !drop
        });
        out.msgs.extend(notices.into_iter().map(TermMsg::Event));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::Rgb;
    use rj_ipc::{Osc52Rule, TermScheme, TermView};

    fn hex(c: [u8; 3]) -> String {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    }

    /// Die Darstellung, wie die Oberfläche sie bisher aus einem eigenen Terminal
    /// gebildet hat (Läufe als Text, Vordergrund, Hintergrund, Kennzeichen).
    fn reference_lines(t: &Terminal) -> Vec<Vec<(String, String, String, u8)>> {
        let h = |c: Rgb| format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b);
        let mut lines = Vec::new();
        for r in 0..t.rows() {
            let mut runs: Vec<(String, String, String, u8)> = Vec::new();
            for c in 0..t.columns() {
                let cell = t.cell(r, c);
                if cell.spacer {
                    continue;
                }
                let (mut fg, mut bg) = (h(cell.fg), h(cell.bg));
                if cell.inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let flags = cell.bold as u8 | (cell.italic as u8) << 1 | (cell.underline as u8) << 2 | (cell.wide as u8) << 3;
                match runs.last_mut() {
                    Some(last) if last.1 == fg && last.2 == bg && last.3 == flags && !cell.wide => last.0.push(cell.ch),
                    _ => runs.push((cell.ch.to_string(), fg, bg, flags)),
                }
            }
            lines.push(runs);
        }
        lines
    }

    fn assert_same(v: &TermView, t: &Terminal, what: &str) {
        let got: Vec<Vec<(String, String, String, u8)>> =
            v.lines.iter().map(|l| l.iter().map(|r| (r.text.clone(), hex(r.fg), hex(r.bg), r.flags)).collect()).collect();
        assert_eq!(got, reference_lines(t), "{what}: Zeilen");
        assert_eq!((v.cols as usize, v.rows as usize), (t.columns(), t.rows()), "{what}");
        assert_eq!(v.cursor, (t.cursor().0 as u32, t.cursor().1 as u32), "{what}: Cursor");
        assert_eq!(v.cursor_visible, t.cursor_visible() && t.display_offset() == 0, "{what}");
        assert_eq!(v.alt_screen, t.alt_screen(), "{what}");
        assert_eq!(v.scroll_offset as usize, t.display_offset(), "{what}");
        assert_eq!(v.history as usize, t.history_size(), "{what}");
        let sc = t.scheme();
        assert_eq!((v.bg, v.fg, v.cursor_color), ([sc.bg.r, sc.bg.g, sc.bg.b], [sc.fg.r, sc.fg.g, sc.fg.b], [sc.cursor.r, sc.cursor.g, sc.cursor.b]), "{what}");
        assert_eq!(v.grid.len(), t.rows(), "{what}");
        let text: Vec<String> = v.grid.iter().map(|r| r.cells.concat().trim_end().to_string()).collect();
        assert_eq!(text, t.screen_text(), "{what}: Zellen");
        let screen: Vec<String> = t.screen_text().iter().map(|l| l.trim_end().to_string()).collect();
        let tail = screen.iter().rposition(|l| !l.is_empty()).map(|last| screen[last.saturating_sub(4)..=last].to_vec()).unwrap_or_default();
        assert_eq!(v.tail, tail, "{what}: Ende");
    }

    fn snapshot(h: &mut TermHost, now: Instant) -> TermView {
        match h.poll(now) {
            Some(TermMsg::Snapshot(v)) => v,
            other => panic!("kein Schnappschuss: {other:?}"),
        }
    }

    #[test]
    fn snapshot_equals_a_directly_fed_terminal() {
        let cases: &[(&str, &str)] = &[
            ("Farben", "\x1b[31mrot\x1b[0m \x1b[1;38;2;1;2;3mx\x1b[0m \x1b[7minvers\x1b[0m \x1b[3;4mkursiv\x1b[0m\r\n\x1b[48;5;21mblau\x1b[0m"),
            ("Doppelbreite", "日本語 abc\r\n日本"),
            ("Emoji", "🚀 start 🚀\r\na\u{308}ß"),
            ("Farbwechsel", "\x1b[31ma\x1b[32mb\x1b[33mc\x1b[0md"),
            ("CRLF und Löschen", "eins\r\nzwei\r\ndrei\x1b[2K\x1b[1;1H\x1b[Jneu"),
            ("Scrollbereich", "\x1b[2;4r\x1b[2;1Hb\r\nc\r\nd\r\ne\r\nf\x1b[r"),
            ("Alt-Bildschirm", "haupt\x1b[?1049h\x1b[Halt\x1b[?25l"),
            ("Rücklauf", &(0..40).map(|i| format!("zeile{i}\r\n")).collect::<String>()),
        ];
        for (what, input) in cases {
            let mut host = TermHost::new(20, 6, Osc52Policy::default());
            let mut direct = Terminal::new(20, 6, Osc52Policy::default());
            // In ungleichen Stücken füttern: das Ergebnis hängt nicht von der Aufteilung ab.
            for chunk in input.as_bytes().chunks(7) {
                assert!(host.feed(chunk).msgs.is_empty());
            }
            direct.feed(input.as_bytes());
            let v = snapshot(&mut host, Instant::now());
            assert_same(&v, &direct, what);
        }
    }

    #[test]
    fn scrolling_changes_the_snapshot_immediately() {
        let t0 = Instant::now();
        let mut host = TermHost::new(10, 3, Osc52Policy::default());
        let mut direct = Terminal::new(10, 3, Osc52Policy::default());
        let input: String = (0..20).map(|i| format!("zeile{i}\r\n")).collect();
        host.feed(input.as_bytes());
        direct.feed(input.as_bytes());
        snapshot(&mut host, t0);
        for (cmd, f) in [
            (TermCmd::Scroll(5), Box::new(|t: &mut Terminal| t.scroll(5)) as Box<dyn Fn(&mut Terminal)>),
            (TermCmd::ScrollPage { back: true }, Box::new(|t: &mut Terminal| t.scroll_page(true))),
            (TermCmd::Scroll(-2), Box::new(|t: &mut Terminal| t.scroll(-2))),
        ] {
            let out = host.command(cmd, t0);
            f(&mut direct);
            let Some(TermMsg::Snapshot(v)) = out.msgs.first() else { panic!("{out:?}") };
            assert!(v.scroll_offset > 0);
            assert_same(v, &direct, "Scroll");
        }
        let out = host.command(TermCmd::ScrollBottom, t0);
        direct.scroll_bottom();
        let Some(TermMsg::Snapshot(v)) = out.msgs.first() else { panic!() };
        assert_eq!(v.scroll_offset, 0);
        assert_same(v, &direct, "Ende");
        // Schon am Ende: keine Änderung, kein Schnappschuss.
        assert!(host.command(TermCmd::ScrollBottom, t0).msgs.is_empty());
    }

    #[test]
    fn scheme_history_and_resize() {
        let t0 = Instant::now();
        let mut host = TermHost::new(10, 3, Osc52Policy::default());
        host.feed(b"\x1b[31mA\x1b[0mB");
        let mut s = Scheme::default();
        s.fg = Rgb { r: 1, g: 2, b: 3 };
        s.ansi[1] = Rgb { r: 200, g: 100, b: 50 };
        let ts = TermScheme::from(&s);
        let out = host.command(TermCmd::SetScheme(ts.clone()), t0);
        let Some(TermMsg::Snapshot(v)) = out.msgs.first() else { panic!() };
        assert_eq!(v.lines[0][0].fg, [200, 100, 50]);
        assert_eq!(v.lines[0][1].fg, [1, 2, 3]);
        assert_eq!(v.fg, [1, 2, 3]);
        // Nach dem Neuaufbau (Verbindungsauftrag) bleiben Schema und Rücklauf erhalten.
        host.command(TermCmd::SetHistory(300), t0);
        host.reset(30, 8, Osc52Policy::default());
        host.feed(b"\x1b[31mA");
        let v = snapshot(&mut host, t0 + SNAPSHOT_INTERVAL);
        assert_eq!((v.cols, v.rows), (30, 8));
        assert_eq!(v.lines[0][0].fg, [200, 100, 50]);
        let out = host.resize(12, 4, t0 + SNAPSHOT_INTERVAL);
        let Some(TermMsg::Snapshot(v)) = out.msgs.first() else { panic!() };
        assert_eq!((v.cols, v.rows), (12, 4));
        assert_eq!(v.grid[0].cells.len(), 12);
    }

    #[test]
    fn terminal_queries_are_answered_by_the_worker() {
        let mut host = TermHost::default();
        let out = host.feed(b"abc\x1b[6n");
        assert_eq!(out.to_target, b"\x1b[1;4R");
        assert!(out.msgs.is_empty());
        let out = host.feed(b"\x1b[c");
        assert!(!out.to_target.is_empty());
    }

    #[test]
    fn osc52_title_and_bell_events() {
        let t0 = Instant::now();
        let mut host = TermHost::default();
        // Schreiben (Vorgabe erlaubt), Titel, Klingel, Leseanfrage.
        let out = host.feed(b"\x1b]52;c;R3LDvMOfZSDwn5qA\x07\x1b]0;Mein Titel\x07\x07\x1b]52;c;?\x07");
        assert!(matches!(&out.msgs[0], TermMsg::Event(TermNotice::ClipboardStore(s)) if s == "Grüße 🚀"));
        assert!(matches!(&out.msgs[1], TermMsg::Event(TermNotice::Title(s)) if s == "Mein Titel"));
        assert!(matches!(&out.msgs[2], TermMsg::Event(TermNotice::Bell)));
        let TermMsg::Event(TermNotice::ClipboardLoad { id }) = out.msgs[3].clone() else { panic!("{:?}", out.msgs) };
        assert!(out.to_target.is_empty());
        // Zustimmung: Antwort geht ans Ziel; eine zweite Antwort auf dieselbe Kennung bleibt wirkungslos.
        let out = host.command(TermCmd::ClipboardAnswer { id, text: Some("ä".into()) }, t0);
        assert_eq!(out.to_target, b"\x1b]52;c;w6Q=\x07");
        assert!(host.command(TermCmd::ClipboardAnswer { id, text: Some("ä".into()) }, t0).to_target.is_empty());
        // Ablehnung: nichts geht ans Ziel.
        let out = host.feed(b"\x1b]52;c;?\x07");
        let TermMsg::Event(TermNotice::ClipboardLoad { id: id2 }) = out.msgs[0].clone() else { panic!() };
        assert_ne!(id, id2);
        assert!(host.command(TermCmd::ClipboardAnswer { id: id2, text: None }, t0).to_target.is_empty());
        assert!(host.command(TermCmd::ClipboardAnswer { id: id2, text: Some("x".into()) }, t0).to_target.is_empty());
        // Regel: Schreiben aus.
        host.command(TermCmd::SetOsc52(Osc52Rule { allow_write: false, allow_read_with_prompt: false }), t0);
        assert!(host.feed(b"\x1b]52;c;R3LDvMOfZSDwn5qA\x07\x1b]52;c;?\x07").msgs.is_empty());
    }

    #[test]
    fn unanswered_clipboard_requests_stay_bounded() {
        let t0 = Instant::now();
        let mut host = TermHost::default();
        let mut ids = Vec::new();
        for _ in 0..100 {
            if let Some(TermMsg::Event(TermNotice::ClipboardLoad { id })) = host.feed(b"\x1b]52;c;?\x07").msgs.first() {
                ids.push(*id);
            }
        }
        assert_eq!(ids.len(), 100);
        assert!(host.loads.len() <= MAX_PENDING_LOADS);
        // Die ältesten sind verworfen.
        assert!(host.command(TermCmd::ClipboardAnswer { id: ids[0], text: Some("x".into()) }, t0).to_target.is_empty());
        assert!(!host.command(TermCmd::ClipboardAnswer { id: ids[99], text: Some("x".into()) }, t0).to_target.is_empty());
    }

    #[test]
    fn paste_is_bracketed_when_the_application_asks_for_it() {
        let t0 = Instant::now();
        let mut host = TermHost::default();
        let out = host.command(TermCmd::Paste("a\r\nb\nc".into()), t0);
        assert_eq!(out.to_target, b"a\rb\rc");
        host.feed(b"\x1b[?2004h");
        let out = host.command(TermCmd::Paste("x\x1b[201~rm -rf\n".into()), t0);
        assert_eq!(out.to_target, b"\x1b[200~xrm -rf\r\x1b[201~");
    }

    #[test]
    fn tap_switch() {
        let t0 = Instant::now();
        let mut host = TermHost::default();
        assert!(!host.tap());
        host.command(TermCmd::Tap(true), t0);
        assert!(host.tap());
        host.command(TermCmd::Tap(false), t0);
        assert!(!host.tap());
    }

    #[test]
    fn fast_output_is_summarised() {
        // 1 MiB schnelle Ausgabe in Stücken, wie sie vom Netz kommen.
        let line = "Zeile mit etwas \x1b[32mFarbe\x1b[0m und Text 0123456789 abcdefghij\r\n";
        let data: Vec<u8> = line.bytes().cycle().take(1 << 20).collect();
        let mut host = TermHost::new(120, 40, Osc52Policy::default());
        let start = Instant::now();
        let mut snapshots = 0u32;
        let mut chunks = 0u32;
        for chunk in data.chunks(4096) {
            host.feed(chunk);
            chunks += 1;
            if host.poll(Instant::now()).is_some() {
                snapshots += 1;
            }
        }
        // Am Ende wartet höchstens noch ein Schnappschuss.
        assert!(host.wait(Instant::now()).is_some());
        let secs = start.elapsed().as_secs_f64().max(SNAPSHOT_INTERVAL.as_secs_f64());
        let rate = snapshots as f64 / secs;
        println!("1 MiB in {chunks} Paketen: {snapshots} Schnappschüsse, {rate:.1} je Sekunde");
        assert!(snapshots < chunks, "Pakete wurden nicht zusammengefasst: {snapshots} von {chunks}");
        assert!(rate < 100.0, "{rate} Schnappschüsse je Sekunde");
        assert!(u128::from(snapshots) <= start.elapsed().as_millis() / SNAPSHOT_INTERVAL.as_millis() + 2, "mehr als einer je Abstand");
        // Nach Ablauf des Abstands kommt der letzte.
        let later = Instant::now() + SNAPSHOT_INTERVAL;
        assert!(host.poll(later).is_some());
        assert!(host.wait(later).is_none());
    }

    #[test]
    fn nothing_is_sent_without_change() {
        let t0 = Instant::now();
        let mut host = TermHost::default();
        assert!(host.poll(t0).is_none(), "vor dem Verbindungsauftrag kommt nichts");
        host.reset(80, 24, Osc52Policy::default());
        assert!(host.poll(t0).is_some(), "der erste Schnappschuss kommt sofort");
        assert!(host.poll(t0 + Duration::from_secs(1)).is_none());
        host.feed(b"x");
        assert!(host.poll(t0 + Duration::from_millis(1)).is_none(), "zu früh");
        assert_eq!(host.wait(t0 + Duration::from_millis(1)), Some(SNAPSHOT_INTERVAL - Duration::from_millis(1)));
        assert!(host.poll(t0 + SNAPSHOT_INTERVAL).is_some());
    }

    #[test]
    fn watchdog_trips_only_on_a_long_running_step() {
        use std::sync::atomic::AtomicBool;
        let tripped = Arc::new(AtomicBool::new(false));
        let w = Watchdog::new();
        let t = tripped.clone();
        w.spawn(Duration::from_millis(150), move || t.store(true, Ordering::SeqCst));
        // Viele kurze Schritte lösen nichts aus.
        for _ in 0..30 {
            let _g = w.enter();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!tripped.load(Ordering::SeqCst));
        // Untätig lösen auch nichts aus.
        std::thread::sleep(Duration::from_millis(400));
        assert!(!tripped.load(Ordering::SeqCst));
        // Ein hängender Schritt löst aus.
        let _g = w.enter();
        let start = Instant::now();
        while !tripped.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(tripped.load(Ordering::SeqCst));
    }
}
