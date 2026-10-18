//! Durchsatz des Terminals bei großer Ausgabe (echter Worker, lokaler sshd): Zeit bis zum
//! letzten Schnappschuss, Zahl der Schnappschüsse je Sekunde und Rechenzeit der Oberfläche.
//! Der Lauf mit 50 MB (`RJ_MESSUNG_BYTES`) ist mit `--ignored` zu starten und gibt Zahlen aus.

use std::time::{Duration, Instant};

use rj_core::Protocol;
use rj_ipc::{ConnectRequest, CredentialPayload, SecretBytes};
use rj_testkit::sshd::{Sshd, PASSWORD, USER};
use rj_ui::sessions::{ConnectSpec, Manager, Reconnect, UiEvent};
use uuid::Uuid;

#[global_allocator]
static ALLOC: rj_ui::alloc::ZeroizingAllocator = rj_ui::alloc::ZeroizingAllocator;

/// Rechenzeit (Benutzer + System) eines Prozesses in Sekunden aus /proc (Linux).
fn cpu_secs(pid: &str) -> f64 {
    let Ok(s) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { return 0.0 };
    // Hinter der schließenden Klammer des Namens: Feld 14 und 15 (utime, stime) in Ticks.
    let rest = &s[s.rfind(')').map_or(0, |i| i + 2)..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    (ticks(11) + ticks(12)) / 100.0
}

fn worker_cpu() -> f64 {
    let Ok(dir) = std::fs::read_dir("/proc") else { return 0.0 };
    dir.flatten()
        .filter_map(|e| {
            let pid = e.file_name().to_string_lossy().to_string();
            let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
            comm.trim().starts_with("rj-worker-ssh").then(|| cpu_secs(&pid))
        })
        .sum()
}

struct Report {
    secs: f64,
    snapshots: u32,
    max_per_second: u32,
    ui_cpu: f64,
    worker_cpu: f64,
}

fn measure(bytes: u64) -> Option<Report> {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));
    std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
    let sshd = match Sshd::start() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {}", e.0);
            return None;
        }
    };
    rj_ssh::known_hosts::KnownHosts::default_location().unwrap().remember_openssh("127.0.0.1", sshd.port, &sshd.host_key_pub()).unwrap();
    let workers = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    let mut m = Manager::new(workers, Reconnect { auto: false, max: Duration::from_secs(60) });
    let s = m.open(ConnectSpec {
        target: Uuid::new_v4(),
        title: "ziel".into(),
        protocol: Protocol::Ssh,
        request: ConnectRequest { host: "127.0.0.1".into(), port: sshd.port, width: 200, height: 60, options_json: "{}".into(), connect_timeout_secs: 15, keepalive_secs: 30 },
        credentials: CredentialPayload { username: USER.into(), domain: None, password: SecretBytes(PASSWORD.as_bytes().to_vec()), private_key: None },
        named_credentials: Default::default(),
    });
    let rows = |m: &Manager| -> Vec<String> {
        m.terminal(s).map(|t| t.lines.iter().map(|l| l.iter().map(|r| r.0.as_str()).collect::<String>().trim_end().to_string()).collect()).unwrap_or_default()
    };
    let wait_for = |m: &mut Manager, what: &str, secs: u64, f: &dyn Fn(&Manager) -> bool| {
        let start = Instant::now();
        while !f(m) {
            m.poll(Duration::from_millis(20));
            assert!(start.elapsed() < Duration::from_secs(secs), "{what}: Zeitüberschreitung");
        }
    };
    wait_for(&mut m, "Eingabeaufforderung", 20, &|m| m.screen_text(s).is_some_and(|t| t.contains('$')));

    let cmd = format!("clear; yes 'Zeile mit etwas Text 0123456789 abcdefghijklmnopqrstuvwxyz ÄÖÜ' | head -c {bytes}; echo; echo ENDE-$((20+3))\r");
    let (cpu0, wcpu0) = (cpu_secs("self"), worker_cpu());
    let start = Instant::now();
    m.send_bytes(s, cmd.into_bytes());
    let mut snapshots = 0u32;
    let mut per_second = vec![0u32];
    let mut last_render = Instant::now();
    let mut dirty = false;
    loop {
        for e in m.poll(Duration::from_millis(5)) {
            if matches!(e, UiEvent::Terminal { .. }) {
                snapshots += 1;
                dirty = true;
                let sec = start.elapsed().as_secs() as usize;
                per_second.resize(sec + 1, 0);
                per_second[sec] += 1;
            }
        }
        // Die Oberfläche zeichnet höchstens etwa 30-mal je Sekunde: Darstellung als JSON holen.
        if dirty && last_render.elapsed() >= Duration::from_millis(33) {
            let _ = serde_json::to_string(&m.terminal(s));
            last_render = Instant::now();
            dirty = false;
        }
        if rows(&m).iter().any(|l| l == "ENDE-23") {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(600), "Zeitüberschreitung bei der Ausgabe");
    }
    let secs = start.elapsed().as_secs_f64();
    let report = Report { secs, snapshots, max_per_second: per_second.iter().copied().max().unwrap_or(0), ui_cpu: cpu_secs("self") - cpu0, worker_cpu: worker_cpu() - wcpu0 };
    m.disconnect(s);
    Some(report)
}

fn show(bytes: u64, r: &Report) {
    println!(
        "MESSUNG {} MB: {:.2} s ({:.1} MB/s), Terminal-Ereignisse {} (höchstens {} je Sekunde), Rechenzeit Oberfläche {:.2} s, Worker {:.2} s",
        bytes / 1_000_000,
        r.secs,
        bytes as f64 / 1e6 / r.secs,
        r.snapshots,
        r.max_per_second,
        r.ui_cpu,
        r.worker_cpu
    );
}

/// Kleiner Lauf: die Oberfläche bekommt höchstens etwa 30 Schnappschüsse je Sekunde.
#[test]
fn fast_output_reaches_the_ui_as_few_snapshots() {
    let bytes = 5_000_000;
    let Some(r) = measure(bytes) else { return };
    show(bytes, &r);
    assert!(r.max_per_second <= 40, "{} Schnappschüsse in einer Sekunde", r.max_per_second);
}

/// 50 MB: `cargo test --release -p remotejoe-ui --test terminal_leistung -- --ignored --nocapture`
#[test]
#[ignore = "Messung"]
fn messung_terminalausgabe() {
    let bytes = std::env::var("RJ_MESSUNG_BYTES").ok().and_then(|v| v.parse().ok()).unwrap_or(50_000_000);
    let Some(r) = measure(bytes) else { return };
    show(bytes, &r);
}
