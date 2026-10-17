//! Terminal-Zustandsmaschine (Bildschirm, Farben, Alt-Bildschirm,
//! Zeichenbreiten, Cursor, Auswahl, OSC 52, Klammerung beim Einfügen).
//!
//! Grundlage ist `alacritty_terminal`; die Oberfläche zeichnet das Gitter
//! selbst.

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Osc52, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, Rgb};
use alacritty_terminal::Term;

/// Regel für OSC 52 (Zwischenablage durch das Ziel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Osc52Policy {
    /// Ziel darf die lokale Zwischenablage setzen (Standard: ja).
    pub allow_write: bool,
    /// Ziel darf die lokale Zwischenablage lesen; jede Anfrage wird dem
    /// Benutzer vorgelegt (Standard: ja, mit Nachfrage).
    pub allow_read_with_prompt: bool,
}

impl Default for Osc52Policy {
    fn default() -> Self {
        Osc52Policy { allow_write: true, allow_read_with_prompt: true }
    }
}

/// Ereignisse aus dem Terminal für die Oberfläche.
#[derive(Clone)]
pub enum TermEvent {
    /// Ziel setzt die Zwischenablage (OSC 52).
    ClipboardStore(String),
    /// Ziel möchte die Zwischenablage lesen; nach Zustimmung wird
    /// `respond(text)` an das Ziel gesendet.
    ClipboardLoad(Arc<dyn Fn(&str) -> String + Send + Sync>),
    /// Antwort des Terminals an das Ziel (z. B. Geräteabfragen).
    Reply(String),
    Title(String),
    Bell,
}

impl std::fmt::Debug for TermEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TermEvent::ClipboardStore(t) => write!(f, "ClipboardStore(<{} Bytes>)", t.len()),
            TermEvent::ClipboardLoad(_) => f.write_str("ClipboardLoad"),
            TermEvent::Reply(r) => write!(f, "Reply({r:?})"),
            TermEvent::Title(t) => write!(f, "Title({t:?})"),
            TermEvent::Bell => f.write_str("Bell"),
        }
    }
}

#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<TermEvent>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let ev = match event {
            Event::ClipboardStore(_, text) => TermEvent::ClipboardStore(text),
            Event::ClipboardLoad(_, f) => TermEvent::ClipboardLoad(f),
            Event::PtyWrite(s) => TermEvent::Reply(s),
            Event::Title(t) => TermEvent::Title(t),
            Event::Bell => TermEvent::Bell,
            _ => return,
        };
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(ev);
    }
}

#[derive(Clone, Copy)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Darstellung einer Zelle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellView {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    /// Zeichen belegt zwei Spalten.
    pub wide: bool,
    /// Platzhalter hinter einem breiten Zeichen.
    pub spacer: bool,
}

/// Terminal.
pub struct Terminal {
    term: Term<Listener>,
    config: Config,
    parser: Processor,
    events: Listener,
    scheme: Scheme,
}

/// Farben des Terminals: Vordergrund, Hintergrund, Cursor und die 16 ANSI-Farben
/// (Vorgabe: xterm auf Schwarz).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheme {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub ansi: [Rgb; 16],
}

impl Default for Scheme {
    fn default() -> Self {
        const BASE: [(u8, u8, u8); 16] = [
            (0, 0, 0), (205, 0, 0), (0, 205, 0), (205, 205, 0), (0, 0, 238), (205, 0, 205), (0, 205, 205), (229, 229, 229),
            (127, 127, 127), (255, 0, 0), (0, 255, 0), (255, 255, 0), (92, 92, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255),
        ];
        Scheme {
            fg: Rgb { r: 229, g: 229, b: 229 },
            bg: Rgb { r: 0, g: 0, b: 0 },
            cursor: Rgb { r: 229, g: 229, b: 229 },
            ansi: BASE.map(|(r, g, b)| Rgb { r, g, b }),
        }
    }
}

/// Farbe einer Zelle nach dem Schema (Zeichen 0–15 aus dem Schema, 16–255 wie bei xterm).
fn palette(c: Color, fg: bool, bold: bool, s: &Scheme) -> Rgb {
    let idx = |i: u8| -> Rgb {
        let i = i as usize;
        let (r, g, b) = if i < 16 {
            let c = s.ansi[i];
            (c.r, c.g, c.b)
        } else if i < 232 {
            let i = i - 16;
            let v = |x: usize| if x == 0 { 0 } else { (x * 40 + 55) as u8 };
            (v(i / 36), v((i / 6) % 6), v(i % 6))
        } else {
            let g = (8 + (i - 232) * 10) as u8;
            (g, g, g)
        };
        Rgb { r, g, b }
    };
    match c {
        Color::Spec(rgb) => rgb,
        Color::Indexed(i) => idx(if bold && i < 8 { i + 8 } else { i }),
        Color::Named(n) => match n {
            NamedColor::Foreground | NamedColor::BrightForeground => s.fg,
            NamedColor::Background => s.bg,
            NamedColor::Cursor => s.cursor,
            n => {
                let i = n as usize;
                if i < 16 {
                    idx(if bold && i < 8 { i as u8 + 8 } else { i as u8 })
                } else if fg {
                    s.fg
                } else {
                    s.bg
                }
            }
        },
    }
}

/// Zeilen im Rücklauf (Scrollback): Vorgabe, Mindest- und Höchstwert.
pub const HISTORY_DEFAULT: usize = 10_000;
pub const HISTORY_MIN: usize = 100;
pub const HISTORY_MAX: usize = 200_000;

impl Terminal {
    pub fn new(cols: usize, rows: usize, osc52: Osc52Policy) -> Self {
        Self::with_history(cols, rows, osc52, HISTORY_DEFAULT)
    }

    pub fn with_history(cols: usize, rows: usize, osc52: Osc52Policy, history: usize) -> Self {
        let events = Listener::default();
        let osc = match (osc52.allow_write, osc52.allow_read_with_prompt) {
            (true, true) => Osc52::CopyPaste,
            (true, false) => Osc52::OnlyCopy,
            (false, true) => Osc52::OnlyPaste,
            (false, false) => Osc52::Disabled,
        };
        let config = Config { osc52: osc, scrolling_history: history.clamp(HISTORY_MIN, HISTORY_MAX), ..Config::default() };
        let term = Term::new(config.clone(), &Size { cols, rows }, events.clone());
        Terminal { term, config, parser: Processor::new(), events, scheme: Scheme::default() }
    }

    /// Größe des Rücklaufs ändern (wirkt sofort; ältere Zeilen darüber hinaus fallen weg).
    pub fn set_history(&mut self, lines: usize) {
        self.config.scrolling_history = lines.clamp(HISTORY_MIN, HISTORY_MAX);
        self.term.set_options(self.config.clone());
    }

    /// Anzeige um `lines` Zeilen zurück (positiv) oder vor (negativ) verschieben.
    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_page(&mut self, back: bool) {
        self.term.scroll_display(if back { Scroll::PageUp } else { Scroll::PageDown });
    }

    pub fn scroll_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    /// Wie viele Zeilen die Anzeige hinter dem Ende liegt (0: am Ende).
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Zeilen im Rücklauf.
    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    /// Neues Farbschema; gilt für alle Zellen beim nächsten Zeichnen.
    pub fn set_scheme(&mut self, scheme: Scheme) {
        self.scheme = scheme;
    }

    pub fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    /// Verarbeitet Ausgabe des Ziels.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn take_events(&mut self) -> Vec<TermEvent> {
        std::mem::take(&mut *self.events.0.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.term.resize(Size { cols, rows });
    }

    pub fn columns(&self) -> usize {
        self.term.columns()
    }

    pub fn rows(&self) -> usize {
        self.term.screen_lines()
    }

    /// Zeile des Puffers für eine Zeile der Anzeige (berücksichtigt das Zurückblättern).
    fn line(&self, row: usize) -> Line {
        Line(row as i32 - self.display_offset() as i32)
    }

    pub fn cell(&self, row: usize, col: usize) -> CellView {
        let c = &self.term.grid()[self.line(row)][Column(col)];
        let bold = c.flags.contains(Flags::BOLD);
        CellView {
            ch: c.c,
            fg: palette(c.fg, true, bold, &self.scheme),
            bg: palette(c.bg, false, false, &self.scheme),
            bold,
            italic: c.flags.contains(Flags::ITALIC),
            underline: c.flags.contains(Flags::UNDERLINE),
            inverse: c.flags.contains(Flags::INVERSE),
            wide: c.flags.contains(Flags::WIDE_CHAR),
            spacer: c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
        }
    }

    /// Sichtbarer Text einer Zeile ohne Leerzeichen am Ende.
    pub fn line_text(&self, row: usize) -> String {
        let mut s = String::new();
        for col in 0..self.columns() {
            let c = &self.term.grid()[self.line(row)][Column(col)];
            if c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            s.push(c.c);
            if let Some(zw) = c.zerowidth() {
                s.extend(zw);
            }
        }
        s.trim_end_matches(' ').to_string()
    }

    pub fn screen_text(&self) -> Vec<String> {
        (0..self.rows()).map(|r| self.line_text(r)).collect()
    }

    /// Cursor (Zeile, Spalte).
    pub fn cursor(&self) -> (usize, usize) {
        let p = self.term.grid().cursor.point;
        (p.line.0.max(0) as usize, p.column.0)
    }

    pub fn alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    pub fn cursor_visible(&self) -> bool {
        self.term.mode().contains(TermMode::SHOW_CURSOR)
    }

    /// Auswahl von (Zeile, Spalte) bis (Zeile, Spalte) einschließlich.
    pub fn select(&mut self, start: (usize, usize), end: (usize, usize)) {
        let off = self.display_offset() as i32;
        let p = |(r, c): (usize, usize)| Point::new(Line(r as i32 - off), Column(c));
        let mut sel = Selection::new(SelectionType::Simple, p(start), Side::Left);
        sel.update(p(end), Side::Right);
        self.term.selection = Some(sel);
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// Text der Auswahl (Zeilenenden als LF).
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    /// Bytes zum Einfügen von `text`: Zeilenenden werden zu CR (wie eine
    /// Eingabetaste), bei aktivem Modus mit Klammerung `ESC[200~ … ESC[201~`.
    /// Steuerfolgen im eingefügten Text, die die Klammerung beenden könnten,
    /// werden entfernt.
    pub fn paste_bytes(&self, text: &str) -> Vec<u8> {
        let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.bracketed_paste() {
            let inner = normalized.replace("\x1b[201~", "").replace("\x1b[200~", "");
            let mut out = b"\x1b[200~".to_vec();
            out.extend_from_slice(inner.as_bytes());
            out.extend_from_slice(b"\x1b[201~");
            out
        } else {
            normalized.into_bytes()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_text() {
        let mut t = Terminal::new(20, 5, Osc52Policy::default());
        t.feed("\x1b[31mrot\x1b[0m \x1b[1;38;2;1;2;3mx\x1b[0m".as_bytes());
        assert_eq!(t.line_text(0), "rot x");
        assert_eq!(t.cell(0, 0).fg, Rgb { r: 205, g: 0, b: 0 });
        let x = t.cell(0, 4);
        assert!(x.bold);
        assert_eq!(x.fg, Rgb { r: 1, g: 2, b: 3 });
    }

    #[test]
    fn wide_chars_and_combining() {
        let mut t = Terminal::new(20, 3, Osc52Policy::default());
        t.feed("日本🚀a\u{0308}ß".as_bytes());
        assert!(t.cell(0, 0).wide);
        assert!(t.cell(0, 1).spacer);
        assert!(t.cell(0, 4).wide, "Emoji belegt zwei Spalten");
        assert_eq!(t.line_text(0), "日本🚀a\u{0308}ß");
        assert_eq!(t.cursor(), (0, 8));
    }

    #[test]
    fn alt_screen_and_cursor() {
        let mut t = Terminal::new(10, 3, Osc52Policy::default());
        t.feed(b"haupt");
        t.feed(b"\x1b[?1049h\x1b[Halt\x1b[?25l");
        assert!(t.alt_screen());
        assert!(!t.cursor_visible());
        assert_eq!(t.line_text(0), "alt");
        t.feed(b"\x1b[?1049l\x1b[?25h");
        assert!(!t.alt_screen());
        assert_eq!(t.line_text(0), "haupt");
    }

    #[test]
    fn selection() {
        let mut t = Terminal::new(20, 3, Osc52Policy::default());
        t.feed("Zeile ä🚀\r\nzweite\r\n".as_bytes());
        t.select((0, 0), (1, 5));
        assert_eq!(t.selection_text().unwrap(), "Zeile ä🚀\nzweite");
    }

    #[test]
    fn osc52_write_and_read() {
        let mut t = Terminal::new(20, 3, Osc52Policy::default());
        // "Grüße 🚀" Base64-kodiert.
        t.feed(b"\x1b]52;c;R3LDvMOfZSDwn5qA\x07");
        t.feed(b"\x1b]52;c;?\x07");
        let ev = t.take_events();
        assert!(matches!(&ev[0], TermEvent::ClipboardStore(s) if s == "Grüße 🚀"));
        let TermEvent::ClipboardLoad(f) = &ev[1] else { panic!("{ev:?}") };
        assert_eq!(f("ä"), "\x1b]52;c;w6Q=\x07");

        let mut off = Terminal::new(20, 3, Osc52Policy { allow_write: false, allow_read_with_prompt: false });
        off.feed(b"\x1b]52;c;R3LDvMOfZSDwn5qA\x07\x1b]52;c;?\x07");
        assert!(off.take_events().is_empty());
    }

    #[test]
    fn paste_bracketing_and_line_endings() {
        let mut t = Terminal::new(20, 3, Osc52Policy::default());
        assert_eq!(t.paste_bytes("a\r\nb\nc"), b"a\rb\rc");
        t.feed(b"\x1b[?2004h");
        assert!(t.bracketed_paste());
        assert_eq!(t.paste_bytes("x\x1b[201~rm -rf\n"), b"\x1b[200~xrm -rf\r\x1b[201~");
    }

    #[test]
    fn scheme_changes_named_and_default_colours() {
        let mut t = Terminal::new(20, 5, Osc52Policy::default());
        t.feed("\x1b[31mA\x1b[0mB\x1b[38;5;196mC\x1b[38;2;10;20;30mD".as_bytes());
        // Vorgabe: xterm.
        assert_eq!(t.cell(0, 0).fg, Rgb { r: 205, g: 0, b: 0 });
        assert_eq!(t.cell(0, 1).fg, Rgb { r: 229, g: 229, b: 229 });
        assert_eq!(t.cell(0, 1).bg, Rgb { r: 0, g: 0, b: 0 });
        let mut s = Scheme::default();
        s.fg = Rgb { r: 1, g: 2, b: 3 };
        s.bg = Rgb { r: 9, g: 8, b: 7 };
        s.ansi[1] = Rgb { r: 200, g: 100, b: 50 };
        t.set_scheme(s);
        // Rot (ANSI 1), Vorder- und Hintergrund folgen dem Schema.
        assert_eq!(t.cell(0, 0).fg, Rgb { r: 200, g: 100, b: 50 });
        assert_eq!(t.cell(0, 1).fg, Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(t.cell(0, 1).bg, Rgb { r: 9, g: 8, b: 7 });
        // 256-Farben-Würfel und 24-Bit-Farben bleiben unverändert.
        assert_eq!(t.cell(0, 2).fg, Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(t.cell(0, 3).fg, Rgb { r: 10, g: 20, b: 30 });
    }

    #[test]
    fn scrolling_back_shows_history_and_returns_to_the_end() {
        let mut t = Terminal::with_history(10, 3, Osc52Policy::default(), 1000);
        for i in 0..20 {
            t.feed(format!("zeile{i}\r\n").as_bytes());
        }
        assert_eq!(t.display_offset(), 0);
        let bottom = t.screen_text();
        assert!(t.history_size() >= 17);
        t.scroll(5);
        assert_eq!(t.display_offset(), 5);
        assert_ne!(t.screen_text(), bottom);
        assert_eq!(t.line_text(0), "zeile13", "{:?}", t.screen_text());
        t.scroll(-2);
        assert_eq!(t.display_offset(), 3);
        t.scroll_page(true);
        assert_eq!(t.display_offset(), 6);
        t.scroll_bottom();
        assert_eq!(t.display_offset(), 0);
        assert_eq!(t.screen_text(), bottom);
        // Auswahl folgt der Anzeige: markiert den Text, der gerade zu sehen ist.
        t.scroll(5);
        t.select((0, 0), (0, 6));
        assert_eq!(t.selection_text().as_deref(), Some("zeile13"));
    }

    #[test]
    fn history_size_can_be_changed_and_is_limited() {
        let mut t = Terminal::with_history(10, 3, Osc52Policy::default(), 5);
        for i in 0..400 {
            t.feed(format!("z{i}\r\n").as_bytes());
        }
        assert!(t.history_size() <= HISTORY_MIN, "Mindestwert gilt");
        t.set_history(300);
        for i in 0..400 {
            t.feed(format!("y{i}\r\n").as_bytes());
        }
        assert!(t.history_size() > HISTORY_MIN && t.history_size() <= 300);
    }
}
