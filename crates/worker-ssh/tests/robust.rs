//! Das Terminal im Worker verträgt beliebige Ausgabe: keine Panik, begrenzter Speicher,
//! begrenzte Schnappschüsse und Ereignisse (Schutz vor bösartiger Terminalausgabe).

use std::time::Instant;

use rj_ipc::{FrameWriter, FromWorker, TermMsg};
use rj_ssh::host::{TermHost, MAX_CELLS, MAX_SIDE};
use rj_ssh::terminal::{Osc52Policy, Terminal, HISTORY_DEFAULT, MAX_ZEROWIDTH, MIN_COLUMNS, MIN_ROWS};

/// Kleiner Zufallsgenerator (xorshift), damit die Tests ohne Abhängigkeit wiederholbar sind.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// Belegter Arbeitsspeicher des Prozesses in Bytes (Linux; sonst 0).
fn rss() -> usize {
    std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<usize>().ok()).map_or(0, |p| p * 4096)
}

fn check_invariants(t: &Terminal, history: usize) {
    assert!(t.history_size() <= history, "Rücklauf {} über {history}", t.history_size());
    assert_eq!(t.screen_text().len(), t.rows());
    assert!(t.cursor().0 < t.rows().max(1) && t.cursor().1 <= t.columns());
    // Der Schnappschuss lässt sich in jedem Zustand bilden und ist stimmig.
    let v = t.view();
    assert_eq!(v.lines.len(), t.rows());
    assert_eq!(v.grid.len(), t.rows());
    assert!(v.grid.iter().all(|r| r.cells.len() == t.columns()));
}

#[test]
fn random_bytes_do_not_panic_and_history_stays_bounded() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut t = Terminal::new(80, 24, Osc52Policy::default());
    let mut host = TermHost::new(80, 24, Osc52Policy::default());
    let data = rng.bytes(1 << 20);
    let mut pos = 0;
    while pos < data.len() {
        let n = 1 + (rng.next() as usize % 3000);
        let end = (pos + n).min(data.len());
        t.feed(&data[pos..end]);
        host.feed(&data[pos..end]);
        pos = end;
    }
    check_invariants(&t, HISTORY_DEFAULT + 24);
    check_invariants(host.terminal(), HISTORY_DEFAULT + 24);
    // Mehrere Durchläufe mit Anteilen aus Steuerzeichen und Escape-Folgen.
    for round in 0..20 {
        let mut chunk = rng.bytes(50_000);
        for b in chunk.iter_mut() {
            if *b & 0x0f == 0 {
                *b = 0x1b;
            } else if *b & 0x0f == 1 {
                *b = b'[';
            } else if *b & 0x0f == 2 {
                *b = b';';
            }
        }
        t.feed(&chunk);
        host.feed(&chunk);
        if round % 5 == 0 {
            t.resize(rng.next() as usize % 200, rng.next() as usize % 60);
            t.scroll((rng.next() as i32) % 50);
            t.select((0, 0), (t.rows() - 1, t.columns() - 1));
            let _ = t.selection_text();
        }
    }
    check_invariants(&t, HISTORY_DEFAULT + 60);
}

#[test]
fn invalid_utf8_and_truncated_escape_sequences() {
    let mut t = Terminal::new(40, 10, Osc52Policy::default());
    let bad: &[&[u8]] = &[
        b"\xff\xfe\xfd", b"\xc0\x80", b"\xed\xa0\x80", b"\xf4\x90\x80\x80", b"\xe2\x82", b"\xf0\x9f\x9a", b"\xc3", b"a\xc3(b",
    ];
    for _ in 0..200 {
        for b in bad {
            t.feed(b);
        }
    }
    let truncated: &[&[u8]] = &[
        b"\x1b", b"\x1b[", b"\x1b[3", b"\x1b[38;2;", b"\x1b[?", b"\x1b]", b"\x1b]0;halber Titel", b"\x1b]52;c;", b"\x1b]8;;http://example.test/",
        b"\x1bP1$r", b"\x1b_Gf=24", b"\x1b^privat", b"\x1bX", b"\x1b(", b"\x1b#", b"\x1b%", b"\x1b[?2026h",
    ];
    for s in truncated {
        for cut in 0..=s.len() {
            let mut t = Terminal::new(40, 10, Osc52Policy::default());
            t.feed(&s[..cut]);
            t.feed(b"\x18text\x1b\\\x07\r\n\x1b[0m");
            t.feed("日本語 ok\r\n".as_bytes());
            check_invariants(&t, HISTORY_DEFAULT + 10);
        }
    }
    check_invariants(&t, HISTORY_DEFAULT + 10);
}

#[test]
fn very_long_lines_and_many_parameters() {
    let mut t = Terminal::new(80, 24, Osc52Policy::default());
    let before = rss();
    // Eine Zeile von 20 MB ohne Zeilenende: bricht um, der Rücklauf bleibt begrenzt.
    let chunk = vec![b'x'; 1 << 20];
    for _ in 0..20 {
        t.feed(&chunk);
    }
    check_invariants(&t, HISTORY_DEFAULT + 24);
    assert!(t.history_size() >= HISTORY_DEFAULT - 100, "Rücklauf läuft voll und wird dann begrenzt");

    // 10000 Parameter in einer Folge, auch bei Folgen, die Parameter auswerten.
    for fin in ["m", "H", "r", "t", "J", "K", "h", "l", "c", "n", "@", "A", "G", "X", "q"] {
        let params = "1;".repeat(10_000);
        t.feed(format!("\x1b[{params}1{fin}\x1b[0m\r\n").as_bytes());
        t.feed(format!("\x1b[?{params}1{fin}\x1b[0m\r\n").as_bytes());
    }
    // Riesige Zahlen.
    for fin in ["A", "B", "C", "D", "E", "F", "G", "H", "J", "K", "L", "M", "P", "S", "T", "X", "@", "b", "d", "r", "t"] {
        for n in ["99999999", "4294967295", "18446744073709551616", "-1"] {
            t.feed(format!("x\x1b[{n}{fin}\x1b[{n};{n}{fin}\x1b[0m").as_bytes());
        }
    }
    t.feed(b"\x1b[8;99999;99999t\x1b[18t\x1b[?1049h\x1b[99999;99999H\x1b[2J\x1b[?1049l");
    check_invariants(&t, HISTORY_DEFAULT + 24);
    if before > 0 {
        let grown = rss().saturating_sub(before);
        assert!(grown < 400 * 1024 * 1024, "Speicherzuwachs {} MiB", grown >> 20);
    }
}

#[test]
fn string_sequences_do_not_grow_without_bound() {
    let mut host = TermHost::new(80, 24, Osc52Policy::default());
    let before = rss();
    // 64 MiB innerhalb einer nie beendeten OSC-, DCS- und APC-Folge.
    let chunk = vec![b'a'; 1 << 20];
    for intro in [&b"\x1b]0;"[..], b"\x1b]52;c;", b"\x1bP", b"\x1b_G", b"\x1b]8;;"] {
        host.feed(intro);
        for _ in 0..64 {
            let out = host.feed(&chunk);
            assert!(out.msgs.len() < 10);
        }
        host.feed(b"\x07\x1b\\\x18\x1b\\");
    }
    if before > 0 {
        let grown = rss().saturating_sub(before);
        assert!(grown < 400 * 1024 * 1024, "Speicherzuwachs {} MiB", grown >> 20);
    }
    check_invariants(host.terminal(), HISTORY_DEFAULT + 24);
}

#[test]
fn event_floods_are_coalesced() {
    let mut host = TermHost::new(80, 24, Osc52Policy { allow_write: true, allow_read_with_prompt: true });
    let out = host.feed(&vec![7u8; 32 * 1024]);
    assert_eq!(out.msgs.len(), 1, "eine Klingel je Paket");
    let mut titles = Vec::new();
    for i in 0..2000 {
        titles.extend_from_slice(format!("\x1b]0;Titel {i}\x07").as_bytes());
    }
    let out = host.feed(&titles);
    assert_eq!(out.msgs.len(), 8, "die letzten Titel bleiben");
    assert!(format!("{:?}", out.msgs[7]).contains("Titel 1999"));
    let long_title = format!("\x1b]0;{}\x07", "ä".repeat(1 << 20));
    let out = host.feed(long_title.as_bytes());
    assert!(format!("{:?}", out.msgs).len() < 20_000, "Titel wird gekürzt");
    let loads: Vec<u8> = b"\x1b]52;c;?\x07".repeat(5000);
    let out = host.feed(&loads);
    assert!(out.msgs.len() <= 4, "{}", out.msgs.len());
    let replies: Vec<u8> = b"\x1b[6n".repeat(5000);
    let out = host.feed(&replies);
    assert!(out.to_target.len() <= 5000 * 12);
}

#[test]
fn combining_marks_cannot_inflate_a_snapshot() {
    let mut host = TermHost::new(80, 24, Osc52Policy::default());
    // 5 MB kombinierende Zeichen auf einer Zelle.
    let marks = "a".to_string() + &"\u{0301}".repeat(1 << 21);
    host.feed(marks.as_bytes());
    let v = host.terminal().view();
    assert!(v.grid[0].cells[0].chars().count() <= 1 + MAX_ZEROWIDTH);
    assert!(v.tail[0].chars().count() <= 1000);
    assert!(v.approx_size() < 64 * 1024, "{}", v.approx_size());
}

#[test]
fn absurd_sizes_are_clamped() {
    let mut host = TermHost::new(0, 0, Osc52Policy::default());
    assert_eq!((host.terminal().columns(), host.terminal().rows()), (MIN_COLUMNS, MIN_ROWS));
    host.reset(100_000, 100_000, Osc52Policy::default());
    let (c, r) = (host.terminal().columns(), host.terminal().rows());
    assert!(c <= MAX_SIDE && r <= MAX_SIDE && c * r <= MAX_CELLS, "{c}×{r}");
    let out = host.resize(usize::MAX, usize::MAX, Instant::now());
    let Some(TermMsg::Snapshot(v)) = out.msgs.first() else { panic!() };
    assert!((v.cols as usize) <= MAX_SIDE && (v.rows as usize) <= MAX_SIDE && (v.cols * v.rows) as usize <= MAX_CELLS);
    host.feed(b"text\r\n");
    let _ = host.resize(3, 2, Instant::now());
}

/// Größe eines Schnappschusses im Rahmen des Kanals.
fn frame_size(m: TermMsg) -> usize {
    let mut w = FrameWriter::new(Vec::new());
    w.send(0, &FromWorker::Term(m)).unwrap();
    w.get_mut().len()
}

/// Schlechtester Fall: jede Zelle mit eigener Farbe und eigenem Zeichen (jeder Lauf hat eine Zelle).
fn colourful(cols: usize, rows: usize) -> TermHost {
    let mut host = TermHost::new(cols, rows, Osc52Policy::default());
    let mut s = String::new();
    for r in 0..rows {
        for c in 0..cols {
            s.push_str(&format!("\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m{}", (c * 7) % 256, (r * 11) % 256, (c + r) % 256, (c * 3) % 256, (r * 5) % 256, (c * r) % 256, if (c + r) % 2 == 0 { 'a' } else { 'b' }));
        }
        s.push_str("\x1b[0m\r\n");
    }
    host.feed(s.as_bytes());
    host
}

#[test]
fn snapshot_size_stays_small_and_is_reduced_when_too_large() {
    let t0 = Instant::now();
    // 200 × 60, schlechtester Fall: einige hundert KiB.
    let mut host = colourful(200, 60);
    let n = frame_size(host.poll(t0).unwrap());
    println!("SCHNAPPSCHUSS 200x60 (schlechtester Fall): {n} Bytes");
    assert!(n < 300 * 1024, "200×60: {n} Bytes");
    // 400 × 120.
    let mut host = colourful(400, 120);
    let n = frame_size(host.poll(t0).unwrap());
    println!("SCHNAPPSCHUSS 400x120 (schlechtester Fall): {n} Bytes");
    assert!(n < 1024 * 1024, "400×120: {n} Bytes");
    assert!(n < rj_ipc::MAX_FRAME as usize / 32);
    // Zu groß: die Zellentexte entfallen, Zeilen und Maße bleiben.
    let mut host = colourful(400, 120);
    host.set_snapshot_limit(100 * 1024);
    let Some(TermMsg::Snapshot(v)) = host.poll(t0) else { panic!() };
    assert_eq!((v.cols, v.rows), (400, 120));
    assert_eq!(v.grid.len(), 120);
    assert!(v.grid.iter().all(|r| r.cells.len() == 400 && r.cells.iter().all(|c| c.is_empty())));
    assert!(!v.lines[0].is_empty());
}

/// Regressionstest: ein Terminal mit einer Spalte stürzte bei doppelt breiten Zeichen ab.
#[test]
fn tiny_terminals_survive_wide_characters() {
    for (cols, rows) in [(1, 1), (1, 5), (2, 1), (3, 3)] {
        let mut t = Terminal::new(cols, rows, Osc52Policy::default());
        t.feed("日本語🚀 a\u{301}\r\n日".as_bytes());
        t.resize(1, 1);
        t.feed("日本".as_bytes());
        check_invariants(&t, HISTORY_DEFAULT + rows);
    }
}
