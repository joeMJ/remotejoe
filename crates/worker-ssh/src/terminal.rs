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
use rj_ipc::{GridRow, Osc52Rule, Rgb3, TermRun, TermScheme, TermView};

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

impl From<Osc52Rule> for Osc52Policy {
    fn from(r: Osc52Rule) -> Self {
        Osc52Policy { allow_write: r.allow_write, allow_read_with_prompt: r.allow_read_with_prompt }
    }
}

impl From<Osc52Policy> for Osc52Rule {
    fn from(p: Osc52Policy) -> Self {
        Osc52Rule { allow_write: p.allow_write, allow_read_with_prompt: p.allow_read_with_prompt }
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

fn to_rgb3(c: Rgb) -> Rgb3 {
    [c.r, c.g, c.b]
}

fn from_rgb3(c: Rgb3) -> Rgb {
    Rgb { r: c[0], g: c[1], b: c[2] }
}

impl From<&Scheme> for TermScheme {
    fn from(s: &Scheme) -> Self {
        TermScheme { fg: to_rgb3(s.fg), bg: to_rgb3(s.bg), cursor: to_rgb3(s.cursor), ansi: s.ansi.map(to_rgb3) }
    }
}

impl From<&TermScheme> for Scheme {
    fn from(s: &TermScheme) -> Self {
        Scheme { fg: from_rgb3(s.fg), bg: from_rgb3(s.bg), cursor: from_rgb3(s.cursor), ansi: s.ansi.map(from_rgb3) }
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

/// So viele kombinierende Zeichen je Zelle und so viele Zeichen je Zeile des Endes
/// gehen höchstens in einen Schnappschuss (Schutz vor gezielt aufgeblähten Zellen).
pub const MAX_ZEROWIDTH: usize = 8;
pub const MAX_TAIL_CHARS: usize = 1000;

/// Zeilen im Rücklauf (Scrollback): Vorgabe, Mindest- und Höchstwert.
pub const HISTORY_DEFAULT: usize = 10_000;
pub const HISTORY_MIN: usize = 100;
pub const HISTORY_MAX: usize = 200_000;

/// Kleinste Größe des Terminals. Bei nur einer Spalte stürzt `alacritty_terminal` beim
/// Setzen eines doppelt breiten Zeichens ab (Index außerhalb der Zeile).
pub const MIN_COLUMNS: usize = 2;
pub const MIN_ROWS: usize = 1;

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
        let (cols, rows) = (cols.max(MIN_COLUMNS), rows.max(MIN_ROWS));
        let term = Term::new(config.clone(), &Size { cols, rows }, events.clone());
        Terminal { term, config, parser: Processor::new(), events, scheme: Scheme::default() }
    }

    /// Regel für OSC 52 ändern.
    pub fn set_osc52(&mut self, osc52: Osc52Policy) {
        self.config.osc52 = match (osc52.allow_write, osc52.allow_read_with_prompt) {
            (true, true) => Osc52::CopyPaste,
            (true, false) => Osc52::OnlyCopy,
            (false, true) => Osc52::OnlyPaste,
            (false, false) => Osc52::Disabled,
        };
        self.term.set_options(self.config.clone());
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
        self.term.resize(Size { cols: cols.max(MIN_COLUMNS), rows: rows.max(MIN_ROWS) });
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

    /// Schnappschuss für die Oberfläche: Läufe gleicher Darstellung, Zellen für
    /// die Textauswahl und die letzten Zeilen für den Makrostart.
    pub fn view(&self) -> TermView {
        let (cols, rows) = (self.columns(), self.rows());
        let mut lines = Vec::with_capacity(rows);
        for r in 0..rows {
            let mut runs: Vec<TermRun> = Vec::new();
            for c in 0..cols {
                let cell = self.cell(r, c);
                if cell.spacer {
                    continue;
                }
                let (mut fg, mut bg) = (to_rgb3(cell.fg), to_rgb3(cell.bg));
                if cell.inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let flags = cell.bold as u8 | (cell.italic as u8) << 1 | (cell.underline as u8) << 2 | (cell.wide as u8) << 3;
                match runs.last_mut() {
                    Some(last) if last.fg == fg && last.bg == bg && last.flags == flags && !cell.wide => last.text.push(cell.ch),
                    _ => runs.push(TermRun { text: cell.ch.to_string(), fg, bg, flags }),
                }
            }
            lines.push(runs);
        }
        let sc = &self.scheme;
        let (cur_row, cur_col) = self.cursor();
        TermView {
            cols: cols as u32,
            rows: rows as u32,
            cursor: (cur_row as u32, cur_col as u32),
            cursor_visible: self.cursor_visible() && self.display_offset() == 0,
            alt_screen: self.alt_screen(),
            scroll_offset: self.display_offset() as u32,
            history: self.history_size() as u32,
            bg: to_rgb3(sc.bg),
            fg: to_rgb3(sc.fg),
            cursor_color: to_rgb3(sc.cursor),
            lines,
            grid: self.grid_rows(),
            tail: self.tail_text(),
        }
    }

    /// Zellen der angezeigten Zeilen für [`selection_text_from_grid`].
    pub fn grid_rows(&self) -> Vec<GridRow> {
        let cols = self.columns();
        let top = self.term.topmost_line();
        (0..self.rows())
            .map(|r| {
                let line = self.line(r);
                let row = &self.term.grid()[line];
                let mut cells = Vec::with_capacity(cols);
                let mut len = 0;
                for col in 0..cols {
                    let c = &row[Column(col)];
                    let mut text = String::new();
                    if !c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                        text.push(c.c);
                        if let Some(zw) = c.zerowidth() {
                            text.extend(zw.iter().take(MAX_ZEROWIDTH));
                        }
                    }
                    if c.c != ' ' || c.zerowidth().is_some_and(|z| !z.is_empty()) {
                        len = col + 1;
                    }
                    cells.push(text);
                }
                let last = cols.checked_sub(1).map(|l| &row[Column(l)]);
                let wrapped = last.is_some_and(|c| c.flags.contains(Flags::WRAPLINE));
                let lead_spacer = last.is_some_and(|c| c.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER));
                let lead_prev = (lead_spacer && cols >= 2 && line.0 - 1 >= top.0).then(|| self.term.grid()[line - 1i32][Column(0)].c);
                GridRow { cells, len: if wrapped { cols as u32 } else { len as u32 }, wrapped, lead_spacer, lead_prev }
            })
            .collect()
    }

    /// Die letzten Zeilen bis zur letzten nichtleeren (höchstens fünf), ohne Leerraum am Ende.
    pub fn tail_text(&self) -> Vec<String> {
        let lines: Vec<String> = self.screen_text().into_iter().map(|l| l.trim_end().chars().take(MAX_TAIL_CHARS).collect()).collect();
        match lines.iter().rposition(|l| !l.is_empty()) {
            Some(last) => lines[last.saturating_sub(4)..=last].to_vec(),
            None => Vec::new(),
        }
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

/// Regel für OSC 52 aus den SSH-Optionen des Ziels (`osc52_write`, Vorgabe an;
/// `osc52_read`, Vorgabe aus).
pub fn osc52_policy(options_json: &str) -> Osc52Policy {
    let o: rj_core::model::SshOptions = serde_json::from_str(options_json).unwrap_or_else(|_| serde_json::from_str("{}").expect("leere Optionen sind gültig"));
    Osc52Policy { allow_write: o.osc52_write, allow_read_with_prompt: o.osc52_read }
}

/// Text einer Auswahl aus den Zellen eines Schnappschusses; liefert dasselbe wie
/// [`Terminal::select`] mit [`Terminal::selection_text`] (Zeilenenden als LF,
/// Leerraum am Zeilenende entfällt, Umbruchzeilen werden verbunden, doppelt
/// breite Zeichen einmal). `start` und `end` sind (Zeile, Spalte) einschließlich;
/// eine rückwärts gezogene Auswahl verhält sich wie im Terminal (beide
/// Randzellen entfallen). Tabulatorsprünge folgen den Stopps im Abstand von 8.
pub fn selection_text_from_grid(grid: &[GridRow], start: (usize, usize), end: (usize, usize)) -> Option<String> {
    let rows = grid.len();
    let cols = grid.first()?.cells.len();
    if cols == 0 {
        return None;
    }
    let clamp = |(r, c): (usize, usize)| (r.min(rows - 1) as i64, c.min(cols - 1));
    // Anfang links der Startzelle, Ende rechts der Endzelle.
    let (mut a, mut b) = (clamp(start), clamp(end));
    let (mut a_right, mut b_right) = (false, true);
    if a > b {
        std::mem::swap(&mut a, &mut b);
        std::mem::swap(&mut a_right, &mut b_right);
    }
    // Leer: zwei benachbarte Zellen mit Rechts → Links.
    if a_right && !b_right && a.0 == b.0 && a.1 + 1 == b.1 {
        return None;
    }
    let (mut s, mut e) = (a, b);
    if !b_right && s != e {
        if e.1 == 0 {
            e = (e.0 - 1, cols - 1);
        } else {
            e.1 -= 1;
        }
    }
    if a_right && s != e {
        s.1 += 1;
        if s.1 == cols {
            s = (s.0 + 1, 0);
        }
    }
    let mut res = String::new();
    for line in s.0..=e.0 {
        let Some(row) = usize::try_from(line).ok().and_then(|l| grid.get(l)) else { continue };
        let from = if line == s.0 { s.1 } else { 0 };
        let to = if line == e.0 { e.1 } else { cols - 1 };
        res += &line_to_string(row, from, to, line == e.0, cols);
    }
    if res.ends_with('\n') {
        res.pop();
    }
    Some(res)
}

fn line_to_string(row: &GridRow, mut from: usize, to: usize, include_wrapped_wide: bool, cols: usize) -> String {
    let mut text = String::new();
    let len = (row.len as usize).min(to + 1);
    // Ist der Platzhalter eines doppelt breiten Zeichens gewählt, gehört das Zeichen dazu.
    if from > 0 && row.cells[from].is_empty() && !(from == cols - 1 && row.lead_spacer) {
        from -= 1;
    }
    let mut tab_mode = false;
    for col in from..len {
        let c = row.cells[col].chars().next().unwrap_or(' ');
        if tab_mode {
            if col % 8 == 0 || c != ' ' {
                tab_mode = false;
            } else {
                continue;
            }
        }
        if c == '\t' {
            tab_mode = true;
        }
        text.push_str(&row.cells[col]);
    }
    if to >= cols - 1 && (len == 0 || !row.wrapped) {
        text.push('\n');
    }
    if len == cols && cols >= 2 && row.lead_spacer && include_wrapped_wide {
        text.extend(row.lead_prev);
    }
    text
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

    /// Für jede Auswahl (Anfang, Ende) liefert die Auswahl aus den Zellen exakt
    /// den Text des Terminals.
    fn assert_selections_agree(t: &mut Terminal, what: &str) {
        let (rows, cols) = (t.rows(), t.columns());
        let grid = t.grid_rows();
        for r0 in 0..rows {
            for c0 in 0..cols {
                for r1 in 0..rows {
                    for c1 in 0..cols {
                        t.select((r0, c0), (r1, c1));
                        let want = t.selection_text();
                        let got = selection_text_from_grid(&grid, (r0, c0), (r1, c1));
                        assert_eq!(got, want, "{what}: ({r0},{c0}) bis ({r1},{c1}), Zeilen {:?}", t.screen_text());
                    }
                }
            }
        }
    }

    #[test]
    fn grid_selection_equals_terminal_selection() {
        let cases: &[(&str, &str)] = &[
            ("mehrzeilig", "erste Zeile\r\nzweite\r\n\r\ndritte"),
            ("Leerzeichen am Ende", "ab   \r\n  cd  \r\n\x1b[41m  \x1b[0m"),
            ("Doppelbreite", "日本語\r\na日b本\r\n日本語日本語日"),
            ("Umbruch", "0123456789ABCDEFGHIJ0123456789xyz\r\nende"),
            ("Doppelbreite am Zeilenende", "12345678日本語\r\n123456789日本"),
            ("Emoji und Kombizeichen", "🚀a\u{308}b🚀\r\nßäö 🚀🚀 x"),
            ("Tabulator", "a\tb\r\n\tc\t\td\r\nxx\t"),
            ("Farben", "\x1b[31mrot\x1b[0m \x1b[1;42mfett\x1b[0m x"),
        ];
        for (what, input) in cases {
            let mut t = Terminal::new(10, 5, Osc52Policy::default());
            t.feed(input.as_bytes());
            assert_selections_agree(&mut t, what);
        }
    }

    #[test]
    fn grid_selection_follows_the_scrolled_display() {
        let mut t = Terminal::with_history(8, 4, Osc52Policy::default(), 1000);
        for i in 0..30 {
            t.feed(format!("z{i} 日\r\n").as_bytes());
        }
        t.scroll(7);
        assert_selections_agree(&mut t, "zurückgeblättert");
        t.select((0, 0), (1, 3));
        assert_eq!(selection_text_from_grid(&t.grid_rows(), (0, 0), (1, 3)), t.selection_text());
    }

    #[test]
    fn grid_selection_examples() {
        let mut t = Terminal::new(20, 3, Osc52Policy::default());
        t.feed("Zeile ä🚀\r\nzweite\r\n".as_bytes());
        let g = t.grid_rows();
        assert_eq!(selection_text_from_grid(&g, (0, 0), (1, 5)).as_deref(), Some("Zeile ä🚀\nzweite"));
        assert_eq!(selection_text_from_grid(&g, (1, 0), (1, 2)).as_deref(), Some("zwe"));
        assert_eq!(selection_text_from_grid(&[], (0, 0), (0, 0)), None);
        // Rückwärts gezogen entfallen beide Randzellen, wie im Terminal.
        assert_eq!(selection_text_from_grid(&g, (1, 4), (1, 0)).as_deref(), Some("wei"));
    }

    #[test]
    fn view_content() {
        let mut t = Terminal::new(12, 4, Osc52Policy::default());
        t.feed("\x1b[32mgrün\x1b[0m 日本\r\n$ ls  ".as_bytes());
        let v = t.view();
        assert_eq!((v.cols, v.rows, v.cursor), (12, 4, (1, 6)));
        assert_eq!(v.lines[0][0].text, "grün");
        assert_eq!(v.lines[0][0].fg, [0, 205, 0]);
        assert_eq!(v.grid.len(), 4);
        assert_eq!(v.grid[0].cells.len(), 12);
        assert_eq!(v.grid[0].cells[5], "日");
        assert_eq!(v.grid[0].cells[6], "");
        assert_eq!(v.tail, ["grün 日本", "$ ls"]);
        assert_eq!(Terminal::new(5, 3, Osc52Policy::default()).view().tail, Vec::<String>::new());
    }
}
