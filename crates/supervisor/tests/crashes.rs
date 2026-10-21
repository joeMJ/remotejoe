//! 25 Worker, 5 davon sterben auf unterschiedliche Weise; die übrigen 20
//! laufen weiter.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rj_ipc::{FromWorker, ToWorker};
use rj_supervisor::*;

fn spec(mode: &str) -> WorkerSpec {
    let mut s = WorkerSpec::new(env!("CARGO_BIN_EXE_rj-test-worker"));
    s.args = vec!["--mode".into(), mode.into(), "--after-ms".into(), "400".into()];
    // Großzügig: Unter Windows hält die Fehlerberichterstattung (WER) einen abgebrochenen Worker bei einem
    // vollständigen Bericht bis etwa 1,45 s nach dem Abbruch am Leben (gemessen 2026-10-21); bei 1,5 s Frist
    // galt er dann gelegentlich als „reagiert nicht mehr“ statt „abgebrochen“.
    s.heartbeat_timeout = Duration::from_millis(3000);
    s.memory_limit = Some(200 * 1024 * 1024);
    s
}

#[cfg(target_os = "linux")]
fn zombie_children() -> usize {
    let me = std::process::id().to_string();
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("stat")).ok())
        .filter(|stat| {
            // Feld 3 = Zustand, Feld 4 = Elternprozess (nach dem Namen in Klammern).
            let rest = stat.rsplit_once(')').map(|(_, r)| r).unwrap_or("");
            let f: Vec<&str> = rest.split_whitespace().collect();
            f.len() > 2 && f[0] == "Z" && f[1] == me
        })
        .count()
}

#[test]
fn twenty_five_workers_five_die() {
    let mut sup = Supervisor::new();
    let modes = ["panic", "abort", "segv", "hang", "oom"];
    let mut expected: HashMap<WorkerId, &str> = HashMap::new();
    let mut healthy = Vec::new();
    for i in 0..25 {
        if i % 5 == 0 {
            let mode = modes[i / 5];
            let (id, _) = sup.spawn(spec(mode)).unwrap();
            expected.insert(id, mode);
        } else {
            let (id, _) = sup.spawn(spec("normal")).unwrap();
            healthy.push(id);
        }
    }

    let mut started = 0;
    let mut exits: HashMap<WorkerId, (ExitReason, std::time::SystemTime)> = HashMap::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    while exits.len() < 5 && Instant::now() < deadline {
        for ev in sup.tick(Duration::from_millis(50)) {
            match ev {
                SupervisorEvent::Started { .. } => started += 1,
                SupervisorEvent::Exited { id, reason, at } => {
                    eprintln!("Worker {id} ({}) beendet: {} um {:?}", expected.get(&id).unwrap_or(&"?"), reason.describe(), at);
                    exits.insert(id, (reason, at));
                }
                _ => {}
            }
        }
    }
    assert_eq!(started, 25);
    assert_eq!(exits.len(), 5, "genau fünf Worker sterben: {exits:?}");
    for (id, mode) in &expected {
        let (reason, _) = &exits[id];
        let ok = match *mode {
            "panic" => *reason == ExitReason::Panic,
            "abort" => *reason == ExitReason::Abort,
            "segv" => *reason == ExitReason::Segfault,
            "hang" => *reason == ExitReason::Hung,
            "oom" => matches!(reason, ExitReason::MemoryLimit { .. }),
            _ => false,
        };
        assert!(ok, "{mode}: {reason:?}");
    }

    // Die übrigen 20 antworten weiter.
    assert_eq!(sup.running_count(), 20);
    for id in &healthy {
        sup.send(*id, &ToWorker::Ping).unwrap();
    }
    let mut pongs = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while pongs < 20 && Instant::now() < deadline {
        for ev in sup.tick(Duration::from_millis(50)) {
            if let SupervisorEvent::Message { msg: FromWorker::Pong, .. } = ev {
                pongs += 1;
            }
            assert!(!matches!(ev, SupervisorEvent::Exited { .. }), "{ev:?}");
        }
    }
    assert_eq!(pongs, 20);

    #[cfg(target_os = "linux")]
    assert_eq!(zombie_children(), 0, "keine Zombie-Prozesse");

    // Sauberes Ende: alle Kanäle schließen, alle Prozesse sind abgeräumt.
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    let pids: Vec<u32> = healthy.iter().filter_map(|id| sup.pid(*id)).collect();
    sup.shutdown();
    assert_eq!(sup.running_count(), 0);
    #[cfg(target_os = "linux")]
    {
        assert_eq!(zombie_children(), 0);
        for pid in pids {
            assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists(), "Prozess {pid} lebt noch");
        }
    }
}

#[test]
fn restart_with_backoff() {
    let mut sup = Supervisor::new();
    let mut s = spec("panic");
    s.args[3] = "100".into();
    s.restart = RestartPolicy { enabled: true, base: Duration::from_millis(200), max: Duration::from_millis(800), stable_after: Duration::from_secs(30) };
    let (id, first_pid) = sup.spawn(s).unwrap();
    let mut delays = Vec::new();
    let mut pids = vec![first_pid];
    let deadline = Instant::now() + Duration::from_secs(15);
    while delays.len() < 4 && Instant::now() < deadline {
        for ev in sup.tick(Duration::from_millis(20)) {
            match ev {
                SupervisorEvent::RestartScheduled { id: i, delay, .. } if i == id => delays.push(delay),
                SupervisorEvent::Started { pid, .. } if !pids.contains(&pid) => pids.push(pid),
                _ => {}
            }
        }
    }
    assert_eq!(
        delays,
        [200, 400, 800, 800].map(Duration::from_millis),
        "Rückstau verdoppelt sich bis zur Obergrenze"
    );
    assert!(pids.len() >= 4, "Worker wurde neu gestartet: {pids:?}");
    sup.stop(id);
    sup.tick(Duration::from_millis(100));
    assert!(!sup.is_running(id));
}

#[test]
fn stop_is_not_a_crash() {
    let mut sup = Supervisor::new();
    let (id, _) = sup.spawn(spec("normal")).unwrap();
    sup.tick(Duration::from_millis(200));
    sup.stop(id);
    let mut reason = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while reason.is_none() && Instant::now() < deadline {
        for ev in sup.tick(Duration::from_millis(50)) {
            if let SupervisorEvent::Exited { reason: r, .. } = ev {
                reason = Some(r);
            }
        }
    }
    assert_eq!(reason, Some(ExitReason::Stopped));
}
