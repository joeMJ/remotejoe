//! Das Terminal läuft im SSH-Worker: Ausgabe → Schnappschuss → Anzeige, Scrollen,
//! Auswahl, Einfügen, Zwischenablage (OSC 52), Titel, Klingel, Terminal-Anfragen,
//! Makro-Abgriff und Isolation. Gegen einen lokalen OpenSSH-Server und den echten
//! Worker-Prozess; ohne `sshd` werden die Tests übersprungen.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rj_core::Protocol;
use rj_ipc::{ConnectRequest, CredentialPayload, SecretBytes};
use rj_testkit::sshd::{Sshd, PASSWORD, USER};
use rj_ui::sessions::{ConnectSpec, Manager, Reconnect, UiEvent};
use uuid::Uuid;

#[global_allocator]
static ALLOC: rj_ui::alloc::ZeroizingAllocator = rj_ui::alloc::ZeroizingAllocator;

static SHARED: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn env() {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    let d = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    std::env::set_var("XDG_DATA_HOME", d.path().join("data"));
    std::env::set_var("XDG_CONFIG_HOME", d.path().join("config"));
}

/// Ist der Worker mit den Testschaltern gebaut (`cargo test --workspace` tut das)?
fn worker_has_test_hooks() -> bool {
    let out = std::process::Command::new(workers().join("rj-worker-ssh")).arg("--build-info").output();
    out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("test-hooks=true"))
}

fn workers() -> std::path::PathBuf {
    std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_path_buf()
}

struct Rig {
    sshd: Sshd,
    m: Manager,
    events: Vec<UiEvent>,
}

impl Rig {
    fn start() -> Option<Rig> {
        env();
        let sshd = match Sshd::start() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ÜBERSPRUNGEN: {}", e.0);
                return None;
            }
        };
        let kh = rj_ssh::known_hosts::KnownHosts::default_location().unwrap();
        kh.remember_openssh("127.0.0.1", sshd.port, &sshd.host_key_pub()).unwrap();
        let m = Manager::new(workers(), Reconnect { auto: false, max: Duration::from_secs(60) });
        Some(Rig { sshd, m, events: Vec::new() })
    }

    fn spec(&self, options: serde_json::Value) -> ConnectSpec {
        ConnectSpec {
            target: Uuid::new_v4(),
            title: "ziel".into(),
            protocol: Protocol::Ssh,
            request: ConnectRequest { host: "127.0.0.1".into(), port: self.sshd.port, width: 80, height: 12, options_json: options.to_string(), connect_timeout_secs: 15, keepalive_secs: 30 },
            credentials: CredentialPayload { username: USER.into(), domain: None, password: SecretBytes(PASSWORD.as_bytes().to_vec()), private_key: None },
            named_credentials: Default::default(),
        }
    }

    /// Öffnet eine Sitzung und wartet auf die Eingabeaufforderung.
    fn open(&mut self, options: serde_json::Value) -> u64 {
        let spec = self.spec(options);
        let s = self.m.open(spec);
        self.wait("verbunden", |r| r.m.state(s) == Some("connected"));
        self.wait("Eingabeaufforderung", |r| r.m.screen_text(s).is_some_and(|t| t.contains('$')));
        s
    }

    fn pump(&mut self) {
        let ev = self.m.poll(Duration::from_millis(20));
        self.events.extend(ev);
    }

    fn wait(&mut self, what: &str, mut cond: impl FnMut(&Rig) -> bool) {
        let start = Instant::now();
        loop {
            self.pump();
            if cond(self) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "{what}: Zeitüberschreitung\n{:?}", self.events.iter().filter(|e| !matches!(e, UiEvent::Terminal { .. })).collect::<Vec<_>>());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Tippt eine Zeile; vorher löscht Strg-U Reste in der Eingabezeile (z. B. Antworten des Terminals).
    fn type_line(&mut self, s: u64, line: &str) {
        self.m.send_bytes(s, format!("\x15{line}\r").into_bytes());
    }

    fn rows(&self, s: u64) -> Vec<String> {
        let snap = self.m.terminal(s).expect("Terminal");
        snap.lines.iter().map(|l| l.iter().map(|r| r.0.as_str()).collect::<String>().trim_end().to_string()).collect()
    }

    fn screen_has(&self, s: u64, needle: &str) -> bool {
        self.rows(s).iter().any(|l| l.contains(needle))
    }

    fn has_event(&self, f: impl Fn(&UiEvent) -> bool) -> bool {
        self.events.iter().any(f)
    }
}

#[test]
fn output_scroll_selection_and_return_to_the_end() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut r) = Rig::start() else { return };
    let s = r.open(serde_json::json!({}));

    // Vor dem ersten Schnappschuss und danach: die Größe des Auftrags.
    let snap = r.m.terminal(s).unwrap();
    assert_eq!((snap.cols, snap.rows), (80, 12));

    // Ausgabe → Schnappschuss → terminal(): Zeile, Farbe, Doppelbreite.
    r.type_line(s, r#"printf 'RJ-%s \033[1;31mrot\033[0m \346\227\245\n' $((20+3))"#);
    r.wait("Ausgabe", |r| r.rows(s).iter().any(|l| l.starts_with("RJ-23 rot 日")));
    let snap = r.m.terminal(s).unwrap();
    let (row, runs) = snap.lines.iter().enumerate().find(|(_, l)| l.iter().any(|x| x.0.starts_with("RJ-23"))).unwrap();
    let red = runs.iter().find(|x| x.0 == "rot").expect("roter Lauf");
    assert_eq!((red.1.as_str(), red.3 & 1), ("#ff0000", 1), "fett, ANSI-Rot hell");
    assert!(runs.iter().any(|x| x.0 == "日" && x.3 & 8 != 0), "doppelt breit");

    // Auswahltext kommt aus dem letzten Schnappschuss (ohne Warten auf den Worker).
    assert_eq!(r.m.select(s, (row, 0), (row, 4)).as_deref(), Some("RJ-23"));
    assert_eq!(r.m.select(s, (row, 6), (row, 11)).as_deref(), Some("rot 日"));

    // Rücklauf und Scrollen: jeder Befehl ändert sofort die Anzeige.
    r.type_line(s, "seq 1 100");
    r.wait("Rücklauf", |r| r.m.terminal(s).is_some_and(|t| t.history > 80 && r.rows(s).iter().any(|l| l == "100")));
    let bottom = r.rows(s);
    r.m.scroll_terminal(s, 10);
    r.wait("Scrollen", |r| r.m.terminal(s).is_some_and(|t| t.scroll_offset == 10));
    assert_ne!(r.rows(s), bottom);
    assert!(!r.m.terminal(s).unwrap().cursor_visible, "beim Zurückblättern ohne Cursor");
    let scrolled_text = r.m.select(s, (0, 0), (0, 2)).unwrap();
    assert!(scrolled_text.trim().parse::<u32>().is_ok(), "Auswahl folgt der Anzeige: {scrolled_text:?}");
    r.m.scroll_terminal_page(s, true);
    r.wait("Seite zurück", |r| r.m.terminal(s).is_some_and(|t| t.scroll_offset > 10));
    // Eine Taste springt ans Ende zurück.
    r.m.key(s, 0, 0, "x", true);
    r.wait("Ende", |r| r.m.terminal(s).is_some_and(|t| t.scroll_offset == 0));
    assert!(r.m.terminal(s).unwrap().cursor_visible);

    // Größe ändern: Worker und Schnappschuss folgen.
    r.m.resize(s, 100, 15);
    r.wait("Größe", |r| r.m.terminal(s).is_some_and(|t| (t.cols, t.rows) == (100, 15)));
    r.m.disconnect(s);
}

#[test]
fn paste_is_bracketed_when_the_application_asks_for_it() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut r) = Rig::start() else { return };
    let s = r.open(serde_json::json!({}));
    // `cat -v` macht die Steuerfolgen sichtbar; zuerst setzt die Anwendung `?2004h`.
    r.type_line(s, r"clear; printf '\033[?2004h'; echo BEREIT-$((1+1)); cat -v");
    r.wait("Anwendung bereit", |r| r.rows(s).iter().any(|l| l == "BEREIT-2"));
    r.m.paste(s, "hi\nda");
    r.wait("Einfügen", |r| r.screen_has(s, "^[[200~hi") && r.screen_has(s, "da^[[201~"));
    r.m.disconnect(s);
}

#[test]
fn clipboard_title_bell_and_terminal_queries() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut r) = Rig::start() else { return };
    let s = r.open(serde_json::json!({ "osc52_read": true }));

    // OSC 52 Speichern ("Grüße 🚀"), Titel und Klingel.
    r.type_line(s, r"printf '\033]52;c;R3LDvMOfZSDwn5qA\007\033]0;Mein Titel\007\007'");
    r.wait("Ereignisse", |r| {
        r.has_event(|e| matches!(e, UiEvent::Clipboard { text, .. } if text == "Grüße 🚀"))
            && r.has_event(|e| matches!(e, UiEvent::Title { title, .. } if title == "Mein Titel"))
            && r.has_event(|e| matches!(e, UiEvent::Bell { .. }))
    });

    // Leseanfrage mit Zustimmung: Der Worker schreibt die Antwort ans Ziel (die Zeilenschaltung zeigt sie an).
    r.type_line(s, r"clear; printf '\033]52;c;?\007'; sleep 2");
    r.wait("Leseanfrage", |r| r.has_event(|e| matches!(e, UiEvent::ClipboardReadRequest { .. })));
    r.m.clipboard_read_answer(s, true, "ä");
    r.wait("Antwort", |r| r.screen_has(s, "^[]52;c;w6Q=^G"));
    r.wait("sleep fertig", |r| r.m.screen_text(s).is_some_and(|t| t.lines().last().is_some_and(|l| l.contains('$'))));

    // Leseanfrage mit Ablehnung: nichts erreicht das Ziel.
    r.events.clear();
    r.type_line(s, r"clear; printf '\033]52;c;?\007'; sleep 2; echo ENDE-1");
    r.wait("zweite Leseanfrage", |r| r.has_event(|e| matches!(e, UiEvent::ClipboardReadRequest { .. })));
    r.m.clipboard_read_answer(s, false, "geheim");
    r.wait("Ende 1", |r| r.rows(s).iter().any(|l| l == "ENDE-1"));
    assert!(!r.screen_has(s, "^[]52"), "keine Antwort bei Ablehnung: {:?}", r.rows(s));

    // Terminal-Anfrage (Cursorposition): der Worker antwortet selbst.
    r.type_line(s, r"clear; printf '\033[6n'; sleep 2");
    r.wait("Cursorposition", |r| r.screen_has(s, "^[[1;1R") || r.screen_has(s, "^[[2;1R"));
    r.m.disconnect(s);
}

#[test]
fn clipboard_limit_applies_to_the_target_clipboard() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut r) = Rig::start() else { return };
    let s = r.open(serde_json::json!({ "clipboard_limit": 4 }));
    r.type_line(s, r"printf '\033]52;c;R3LDvMOfZSDwn5qA\007'");
    r.wait("Limit", |r| r.has_event(|e| matches!(e, UiEvent::Notice { text, .. } if text.contains("Zwischenablage vom Ziel verworfen"))));
    assert!(!r.has_event(|e| matches!(e, UiEvent::Clipboard { .. })));
    r.m.disconnect(s);

    // Schreiben aus: keine Meldung, kein Ereignis.
    r.events.clear();
    let s2 = r.open(serde_json::json!({ "osc52_write": false }));
    r.type_line(s2, r"printf '\033]52;c;R3LDvMOfZSDwn5qA\007'; echo ENDE-2");
    r.wait("Ende 2", |r| r.rows(s2).iter().any(|l| l == "ENDE-2"));
    assert!(!r.has_event(|e| matches!(e, UiEvent::Clipboard { .. })));
    r.m.disconnect(s2);
}

#[test]
fn macro_tap_delivers_raw_output_only_while_attached() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut r) = Rig::start() else { return };
    let s = r.open(serde_json::json!({}));
    let (tx, rx) = std::sync::mpsc::channel();
    r.m.attach_macro_tap(s, tx);
    r.type_line(s, "echo TAP-$((1+1))");
    let mut got = Vec::new();
    r.wait("Abgriff", |_| {
        while let Ok(b) = rx.try_recv() {
            got.extend(b);
        }
        String::from_utf8_lossy(&got).contains("TAP-2")
    });
    r.m.detach_macro_tap(s);
    // Der Worker stellt den Abgriff ab; danach kommen keine Rohbytes mehr.
    r.type_line(s, "echo OHNE-$((1+1))");
    r.wait("Ausgabe ohne Abgriff", |r| r.rows(s).iter().any(|l| l == "OHNE-2"));
    std::thread::sleep(Duration::from_millis(200));
    r.pump();
    let late: Vec<u8> = rx.try_iter().flatten().collect();
    assert!(!String::from_utf8_lossy(&late).contains("OHNE-2"), "{:?}", String::from_utf8_lossy(&late));
    r.m.disconnect(s);
}

/// Ein Worker, der abstürzt, trifft nur seine Sitzung; Neuverbinden startet einen neuen Worker.
#[test]
fn a_crashing_worker_only_affects_its_own_session() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if !worker_has_test_hooks() {
        eprintln!("ÜBERSPRUNGEN: Worker ohne Testschalter gebaut (cargo test --workspace baut ihn mit)");
        return;
    }
    let Some(mut r) = Rig::start() else { return };
    let a = r.open(serde_json::json!({}));
    let b = r.open(serde_json::json!({}));
    // Der Testschalter im Worker (nur Testbauten, Umgebungsvariable `RJ_TEST_TERMINAL_PANIC`)
    // löst bei einer bestimmten Ausgabe eine Panik im Terminal des Workers aus.
    r.type_line(a, "printf 'RJ-PANIK-%s' AUSLOESER");
    r.wait("Absturz", |r| r.m.state(a) == Some("crashed") || r.m.state(a) == Some("failed"));
    assert_eq!(r.m.state(a), Some("crashed"));
    assert_eq!(r.m.state(b), Some("connected"), "die andere Sitzung läuft weiter");
    r.type_line(b, "echo NOCH-DA-$((40+2))");
    r.wait("andere Sitzung", |r| r.rows(b).iter().any(|l| l == "NOCH-DA-42"));
    // Neuverbinden startet einen neuen Worker.
    r.m.reconnect(a);
    r.wait("neu verbunden", |r| r.m.state(a) == Some("connected"));
    r.wait("Eingabeaufforderung", |r| r.m.screen_text(a).is_some_and(|t| t.contains('$')));
    r.type_line(a, "echo NEU-$((40+3))");
    r.wait("neuer Worker", |r| r.rows(a).iter().any(|l| l == "NEU-43"));
    r.m.disconnect(a);
    r.m.disconnect(b);
}

/// Ein hängender Emulator im Worker (Endlosschleife) wird vom Wächter des Workers beendet und
/// zeigt sich als Absturz dieser einen Sitzung.
#[test]
fn a_hanging_emulator_shows_up_as_a_crash_of_that_session() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if !worker_has_test_hooks() {
        eprintln!("ÜBERSPRUNGEN: Worker ohne Testschalter gebaut (cargo test --workspace baut ihn mit)");
        return;
    }
    // Die Frist gilt nur für Worker, die ab jetzt starten.
    std::env::set_var("RJ_TEST_WATCHDOG_MS", "800");
    let Some(mut r) = Rig::start() else { return };
    let a = r.open(serde_json::json!({}));
    let b = r.open(serde_json::json!({}));
    r.type_line(a, "printf 'RJ-HAENGER-%s' AUSLOESER");
    r.wait("Abbruch durch den Wächter", |r| r.m.state(a) == Some("crashed"));
    assert_eq!(r.m.state(b), Some("connected"));
    r.type_line(b, "echo LEBT-$((40+2))");
    r.wait("andere Sitzung", |r| r.rows(b).iter().any(|l| l == "LEBT-42"));
    r.m.disconnect(b);
    std::env::remove_var("RJ_TEST_WATCHDOG_MS");
}
