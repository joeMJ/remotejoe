//! Tests gegen einen lokal gestarteten OpenSSH-Server (Testkit).
//! Ohne `sshd` oder ohne Root-Rechte werden die Tests mit Hinweis übersprungen.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use rj_core::Secret;
use rj_ssh::client::*;
use rj_ssh::known_hosts::{HostKeyStatus, KnownHosts};
use rj_ssh::terminal::{Osc52Policy, TermEvent, Terminal};
use rj_testkit::sshd::{Sshd, PASSWORD, USER};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

static SHARED: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Fix {
    sshd: Sshd,
    known: KnownHosts,
    _dir: tempfile::TempDir,
    rt: Runtime,
    questions: Arc<AtomicUsize>,
}

fn fixture() -> Option<Fix> {
    let sshd = match Sshd::start() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {e}");
            return None;
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let known = KnownHosts::new(dir.path().join("known_hosts"));
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    Some(Fix { sshd, known, _dir: dir, rt, questions: Arc::new(AtomicUsize::new(0)) })
}

impl Fix {
    fn params(&self, auth: Vec<Auth>) -> SshParams {
        SshParams {
            host: "127.0.0.1".into(),
            port: self.sshd.port,
            user: USER.into(),
            auth,
            connect_timeout: Duration::from_secs(15),
            keepalive: Duration::from_secs(30),
        }
    }

    /// Fragt Host-Schlüssel automatisch mit `accept` ab und zählt die Fragen.
    fn asker(&self, accept: bool) -> (HostKeyAsk, Arc<std::sync::Mutex<Vec<HostKeyStatus>>>) {
        let (tx, mut rx) = mpsc::unbounded_channel::<(rj_ssh::known_hosts::HostKeyQuestion, tokio::sync::oneshot::Sender<bool>)>();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let q = self.questions.clone();
        self.rt.spawn(async move {
            while let Some((question, reply)) = rx.recv().await {
                q.fetch_add(1, Ordering::SeqCst);
                s2.lock().unwrap().push(question.status.clone());
                let _ = reply.send(accept);
            }
        });
        (tx, seen)
    }

    fn connect(&self, auth: Vec<Auth>) -> Result<SshConnection, SshError> {
        let (ask, _) = self.asker(true);
        self.rt.block_on(SshConnection::connect(&self.params(auth), self.known.clone(), ask))
    }

    fn pw() -> Vec<Auth> {
        vec![Auth::Password(Secret::from_str_exact(PASSWORD))]
    }
}

/// Liest Shell-Ausgabe in das Terminal, bis `needle` in der Rohausgabe auftaucht.
fn read_until(rt: &Runtime, shell: &mut Shell, term: &mut Terminal, raw: &mut Vec<u8>, needle: &str, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if String::from_utf8_lossy(raw).contains(needle) {
            return true;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        match rt.block_on(async { tokio::time::timeout(left, shell.events.recv()).await }) {
            Ok(Some(ShellEvent::Data(d))) => {
                term.feed(&d);
                raw.extend_from_slice(&d);
            }
            Ok(Some(ShellEvent::Exit(_))) => {}
            _ => return false,
        }
    }
}

/// Wartet, bis die bash die Klammerung beim Einfügen eingeschaltet hat.
fn wait_bracketed(rt: &Runtime, shell: &mut Shell, term: &mut Terminal, raw: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !term.bracketed_paste() && Instant::now() < deadline {
        let _ = read_until(rt, shell, term, raw, "\x1b[?2004h", 1);
    }
    assert!(term.bracketed_paste(), "bash schaltet Klammerung beim Einfügen ein");
}

fn exec(f: &Fix, c: &SshConnection, cmd: &str) -> String {
    let (out, err, code) = f.rt.block_on(c.exec(cmd)).unwrap();
    assert_eq!(code, Some(0), "{cmd}: {}", String::from_utf8_lossy(&err));
    String::from_utf8(out).unwrap()
}

/// Startet eine Shell und wartet auf eine Markierung.
fn shell(f: &Fix, c: &SshConnection, cols: u32, rows: u32) -> (Shell, Terminal, Vec<u8>) {
    let mut sh = f.rt.block_on(c.open_shell("xterm-256color", cols, rows)).unwrap();
    let mut term = Terminal::new(cols as usize, rows as usize, Osc52Policy::default());
    let mut raw = Vec::new();
    f.rt.block_on(sh.write(b"PS1='RJ-PROMPT''> '; echo RJ-$((40+2))\r")).unwrap();
    assert!(read_until(&f.rt, &mut sh, &mut term, &mut raw, "RJ-42", 15), "Shell antwortet nicht");
    assert!(read_until(&f.rt, &mut sh, &mut term, &mut raw, "RJ-PROMPT> ", 15), "keine Eingabeaufforderung");
    (sh, term, raw)
}

#[test]
fn password_login_command_and_host_key() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = fixture() else { return };
    let (ask, seen) = f.asker(true);
    let c = f.rt.block_on(SshConnection::connect(&f.params(Fix::pw()), f.known.clone(), ask)).unwrap();
    assert_eq!(exec(&f, &c, "id -un").trim(), USER);
    assert_eq!(seen.lock().unwrap().as_slice(), &[HostKeyStatus::Unknown], "neuer Schlüssel wird abgefragt");
    // Zweite Verbindung: Schlüssel bekannt, keine Frage.
    let (ask, seen2) = f.asker(true);
    f.rt.block_on(SshConnection::connect(&f.params(Fix::pw()), f.known.clone(), ask)).unwrap();
    assert!(seen2.lock().unwrap().is_empty());
    // Abgelehnter unbekannter Schlüssel: keine Verbindung.
    let other = tempfile::tempdir().unwrap();
    let (ask, _) = f.asker(false);
    let r = f.rt.block_on(SshConnection::connect(&f.params(Fix::pw()), KnownHosts::new(other.path().join("k")), ask));
    assert!(matches!(r, Err(SshError::HostKeyRejected)), "{:?}", r.err());
}

#[test]
fn changed_host_key_is_reported() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = fixture() else { return };
    // Gefälschter bekannter Schlüssel für denselben Host und Port.
    let fake = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFake";
    let fake = if russh::keys::PublicKey::from_openssh(fake).is_ok() {
        fake.to_string()
    } else {
        // Gültigen fremden Schlüssel erzeugen.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("k");
        rj_testkit::run("ssh-keygen", &["-q", "-t", "ed25519", "-N", "", "-f", p.to_str().unwrap()], None).unwrap();
        std::fs::read_to_string(p.with_extension("pub")).unwrap()
    };
    let mut parts = fake.split_whitespace();
    let (alg, b64) = (parts.next().unwrap(), parts.next().unwrap());
    std::fs::write(f.known.path(), format!("127.0.0.1:{} {alg} {b64}\n", f.sshd.port)).unwrap();
    let (ask, seen) = f.asker(false);
    let r = f.rt.block_on(SshConnection::connect(&f.params(Fix::pw()), f.known.clone(), ask));
    assert!(matches!(r, Err(SshError::HostKeyRejected)));
    assert!(matches!(seen.lock().unwrap()[0], HostKeyStatus::Changed { .. }));
}

#[test]
fn key_logins() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = fixture() else { return };
    for kind in ["ed25519", "rsa"] {
        let pem = std::fs::read(f.sshd.client_key(kind)).unwrap();
        let c = f.connect(vec![Auth::Key { pem: Secret::from_bytes(pem), passphrase: None }]).unwrap();
        assert_eq!(exec(&f, &c, "id -un").trim(), USER, "{kind}");
    }
    let c = f.connect(vec![Auth::KeyboardInteractive(Secret::from_str_exact(PASSWORD))]).unwrap();
    assert_eq!(exec(&f, &c, "id -un").trim(), USER, "tastatur-interaktiv");
    let bad = f.connect(vec![Auth::Password(Secret::from_str_exact("falsch"))]);
    assert!(matches!(bad, Err(SshError::AuthFailed(_))));
    // NFC statt NFD: andere Bytes, Anmeldung muss scheitern.
    let nfd = PASSWORD.replace('ü', "u\u{0308}");
    let bad = f.connect(vec![Auth::Password(Secret::from_str_exact(&nfd))]);
    assert!(matches!(bad, Err(SshError::AuthFailed(_))), "Kennwort wird nicht normalisiert");
}

#[test]
fn shell_resize_and_reconnect() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = fixture() else { return };
    let c = f.connect(Fix::pw()).unwrap();
    let (mut sh, mut term, mut raw) = shell(&f, &c, 80, 24);
    f.rt.block_on(sh.resize(100, 30)).unwrap();
    term.resize(100, 30);
    raw.clear();
    f.rt.block_on(sh.write(b"stty size; echo RJ-$((1+1))\r")).unwrap();
    assert!(read_until(&f.rt, &mut sh, &mut term, &mut raw, "RJ-2", 10));
    assert!(read_until(&f.rt, &mut sh, &mut term, &mut raw, "RJ-PROMPT> ", 10));
    assert!(String::from_utf8_lossy(&raw).contains("30 100"), "neue Fenstergröße beim Ziel");
    wait_bracketed(&f.rt, &mut sh, &mut term, &mut raw);

    // Verbindung bricht ab.
    f.sshd.drop_sessions();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut closed = false;
    while Instant::now() < deadline && !closed {
        match f.rt.block_on(async { tokio::time::timeout(Duration::from_secs(1), sh.events.recv()).await }) {
            Ok(Some(ShellEvent::Closed)) | Ok(None) => closed = true,
            _ => {}
        }
    }
    assert!(closed, "Abbruch wird erkannt");
    // Neu verbinden.
    let c2 = f.connect(Fix::pw()).unwrap();
    let _ = shell(&f, &c2, 80, 24);
}

#[test]
fn sudo_macro_success_and_failure() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = fixture() else { return };
    let c = f.connect(Fix::pw()).unwrap();
    let m = rj_tasks::macros::Macro::sudo_validate();
    for (pw, ok) in [(PASSWORD, true), ("falsches-Kennwort", false)] {
        let (mut sh, _, _) = shell(&f, &c, 80, 24);
        let creds = move |name: &str| (name == "password").then(|| Secret::from_str_exact(pw));
        let handle = f.rt.handle().clone();
        let run = {
            let mut io = BlockingShell { rt: handle, shell: &mut sh };
            m.run(&mut io, &creds)
        };
        assert_eq!(run.result.is_ok(), ok, "{:?}\n{}", run.result, run.log.join("\n"));
        assert!(!run.log.join("\n").contains(pw), "Kennwort nie im Protokoll");
    }
}

/// Zwischenablage-Matrix. Host→Ziel über Einfügen in `cat`, Ziel→Host über
/// OSC 52 (byte-genau) und Auswahl im Terminal.
#[test]
fn clipboard_matrix() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = fixture() else { return };
    let c = f.connect(Fix::pw()).unwrap();
    let mut big = String::new();
    let mut i = 0;
    while big.len() < 100 * 1024 {
        big.push_str(&format!("Zeile {i:05}: Grüße ß 🚀 äöü ÄÖÜ\tTab\n"));
        i += 1;
    }
    let cases: Vec<(&str, String)> = vec![
        ("Umlaute", "äöü ÄÖÜ\n".into()),
        ("ß", "Straße ẞ\n".into()),
        ("Emoji", "🚀👍🏽 €\n".into()),
        ("Tab", "a\tb\t\tc\n".into()),
        ("LF", "eins\nzwei\ndrei\n".into()),
        ("CRLF", "eins\r\nzwei\r\n".into()),
        ("100 kB", big),
    ];
    let b64 = base64::engine::general_purpose::STANDARD;
    let (mut sh, mut term, mut raw) = shell(&f, &c, 200, 50);
    for (n, (name, text)) in cases.iter().enumerate() {
        // Host → Ziel: Einfügen; das Terminal überträgt Zeilenenden als CR,
        // die Gegenstelle (tty) macht daraus LF.
        let file = format!("/tmp/rj-clip-{}-{n}", f.sshd.port);
        raw.clear();
        f.rt.block_on(sh.write(format!("stty -echo; cat > {file}; echo RJ-$((500+{n}))\r").as_bytes())).unwrap();
        // Ausgabe einlesen, bis bash die Klammerung für den Befehl abschaltet.
        let deadline = Instant::now() + Duration::from_secs(5);
        while term.bracketed_paste() && Instant::now() < deadline {
            read_until(&f.rt, &mut sh, &mut term, &mut raw, "\u{0}nie", 0);
            let _ = read_until(&f.rt, &mut sh, &mut term, &mut raw, "\x1b[?2004l", 1);
        }
        std::thread::sleep(Duration::from_millis(200));
        let data = term.paste_bytes(text);
        assert!(!term.bracketed_paste() || data.starts_with(b"\x1b[200~"));
        for chunk in data.chunks(8192) {
            f.rt.block_on(sh.write(chunk)).unwrap();
        }
        f.rt.block_on(sh.write(b"\x04")).unwrap();
        let mark = format!("RJ-{}", 1000 + n);
        f.rt.block_on(sh.write(format!("stty echo; echo RJ-$((1000+{n}))\r").as_bytes())).unwrap();
        if !read_until(&f.rt, &mut sh, &mut term, &mut raw, &mark, 30) {
            let size = exec(&f, &c, &format!("wc -c < {file}; ps -o stat,cmd -u {USER}"));
            panic!("{name}: Einfügen endet nicht; beim Ziel: {size}");
        }
        let got = b64.decode(exec(&f, &c, &format!("base64 -w0 {file}")).trim()).unwrap();
        let expected = text.replace("\r\n", "\n");
        assert_eq!(String::from_utf8_lossy(&got), expected, "Host→Ziel {name}");

        // Ziel → Host über OSC 52: Datei mit genau `text` anlegen und senden.
        // Quelldatei in Teilen anlegen (ein Befehlsargument darf höchstens 128 kB lang sein).
        let encoded = b64.encode(text.as_bytes());
        exec(&f, &c, &format!(": > {file}.b64"));
        for part in encoded.as_bytes().chunks(60_000) {
            exec(&f, &c, &format!("printf %s {} >> {file}.b64", std::str::from_utf8(part).unwrap()));
        }
        exec(&f, &c, &format!("base64 -d {file}.b64 > {file}.src"));
        term.take_events();
        f.rt.block_on(sh.write(format!("printf '\\033]52;c;%s\\a' \"$(base64 -w0 {file}.src)\"; echo; echo RJ-$((2000+{n}))\r").as_bytes())).unwrap();
        assert!(read_until(&f.rt, &mut sh, &mut term, &mut raw, &format!("RJ-{}", 2000 + n), 30), "{name}: OSC 52");
        let stored: Vec<String> = term
            .take_events()
            .into_iter()
            .filter_map(|e| if let TermEvent::ClipboardStore(s) = e { Some(s) } else { None })
            .collect();
        assert_eq!(stored.last().map(|s| s.as_bytes()), Some(text.as_bytes()), "Ziel→Host {name} byte-genau");
        exec(&f, &c, &format!("rm -f {file} {file}.src {file}.b64"));
    }

    // Ziel → Host über Auswahl auf dem Bildschirm.
    raw.clear();
    f.rt.block_on(sh.write("clear; printf 'Grüße ß 🚀\\n'; echo RJ-$((3000+1))\r".as_bytes())).unwrap();
    assert!(read_until(&f.rt, &mut sh, &mut term, &mut raw, "RJ-3001", 10));
    assert!(read_until(&f.rt, &mut sh, &mut term, &mut raw, "RJ-PROMPT> ", 10));
    let row = term.screen_text().iter().position(|l| l.starts_with("Grüße")).expect("Zeile sichtbar");
    term.select((row, 0), (row, term.columns() - 1));
    assert_eq!(term.selection_text().unwrap().trim_end(), "Grüße ß 🚀");

    // Klammerung beim Einfügen in die interaktive bash.
    wait_bracketed(&f.rt, &mut sh, &mut term, &mut raw);
    f.rt.block_on(sh.write(&term.paste_bytes("echo 'eingefügt-🚀'"))).unwrap();
    f.rt.block_on(sh.write(b"\r")).unwrap();
    assert!(read_until(&f.rt, &mut sh, &mut term, &mut raw, "eingefügt-🚀\r\n", 10));
}

/// Ende-zu-Ende über den Worker-Prozess und den Supervisor.
#[test]
fn worker_process_end_to_end() {
    use rj_ipc::*;
    use rj_supervisor::*;
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = fixture() else { return };
    let mut sup = Supervisor::new();
    let (id, _) = sup.spawn(WorkerSpec::new(env!("CARGO_BIN_EXE_rj-worker-ssh"))).unwrap();
    let connect = |sup: &mut Supervisor| {
        sup.send(id, &ToWorker::Connect(ConnectRequest {
            host: "127.0.0.1".into(),
            port: f.sshd.port,
            width: 80,
            height: 24,
            options_json: serde_json::json!({ "known_hosts": f.known.path() }).to_string(),
            connect_timeout_secs: 15,
            keepalive_secs: 30,
        }))
        .unwrap();
        sup.send(id, &ToWorker::Credentials(CredentialPayload {
            username: USER.into(),
            domain: None,
            password: SecretBytes(PASSWORD.as_bytes().to_vec()),
            private_key: None,
        }))
        .unwrap();
    };
    let wait = |sup: &mut Supervisor, pred: &mut dyn FnMut(&FromWorker, &mut Supervisor) -> bool| -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            for ev in sup.tick(Duration::from_millis(50)) {
                if let SupervisorEvent::Message { msg, .. } = ev {
                    if pred(&msg, sup) {
                        return true;
                    }
                }
            }
        }
        false
    };
    connect(&mut sup);
    let mut questions = 0;
    assert!(wait(&mut sup, &mut |m, sup| {
        if let FromWorker::Question { id: q, .. } = m {
            questions += 1;
            sup.send(id, &ToWorker::Answer { question: *q, accept: true }).unwrap();
        }
        matches!(m, FromWorker::State(SessionState::Connected))
    }));
    assert_eq!(questions, 1, "Host-Schlüssel wird abgefragt");
    sup.send(id, &ToWorker::Input(InputEvent::Bytes(b"echo RJ-$((20+3))\r".to_vec()))).unwrap();
    let mut out = Vec::new();
    assert!(wait(&mut sup, &mut |m, _| {
        if let FromWorker::Output(d) = m {
            out.extend_from_slice(d);
        }
        String::from_utf8_lossy(&out).contains("RJ-23")
    }));
    f.sshd.drop_sessions();
    assert!(wait(&mut sup, &mut |m, _| matches!(m, FromWorker::State(SessionState::Disconnected { .. }))));
    assert!(sup.is_running(id), "Worker bleibt nach Verbindungsabbruch am Leben");
    connect(&mut sup);
    assert!(wait(&mut sup, &mut |m, _| matches!(m, FromWorker::State(SessionState::Connected))), "Neu verbinden");
    sup.shutdown();
}

/// apt im Trockenlauf über SSH mit sudo (Kennwort per Makro-Engine).
#[test]
fn apt_dry_run_over_ssh() {
    let _g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = fixture() else { return };
    let c = f.connect(Fix::pw()).unwrap();
    let (mut sh, _, _) = shell(&f, &c, 200, 50);
    struct Log(Vec<String>);
    impl rj_tasks::apt::Progress for Log {
        fn phase(&mut self, p: &str) {
            self.0.push(format!("Phase {p}"));
        }
        fn log(&mut self, l: &str) {
            self.0.push(l.to_string());
        }
    }
    let mut log = Log(Vec::new());
    let creds = |n: &str| (n == "password").then(|| Secret::from_str_exact(PASSWORD));
    let handle = f.rt.handle().clone();
    let r = {
        let mut io = BlockingShell { rt: handle, shell: &mut sh };
        rj_tasks::apt::run(&mut io, &creds, false, false, &mut log)
    };
    eprintln!("apt-Protokoll:\n{}", log.0.join("\n"));
    let report = r.expect("Trockenlauf läuft durch");
    eprintln!("APT {} Aktualisierungen, Neustart nötig: {}", report.updates.len(), report.reboot_required);
    assert!(!report.upgraded);
    assert!(!log.0.join("\n").contains(PASSWORD));
}
