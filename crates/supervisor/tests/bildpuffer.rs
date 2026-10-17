//! Bildpuffer eines bösartigen Workers: Verkürzen nach der Übergabe
//! scheitert, unversiegelter Speicher wird abgelehnt.

#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use rj_ipc::shm::FrameBufferReader;
use rj_ipc::FromWorker;
use rj_supervisor::*;

fn spec(mode: &str) -> WorkerSpec {
    let mut s = WorkerSpec::new(env!("CARGO_BIN_EXE_rj-test-worker"));
    s.args = vec!["--mode".into(), mode.into(), "--after-ms".into(), "50".into()];
    s
}

fn wait_for(sup: &mut Supervisor, mut f: impl FnMut(&mut Supervisor, WorkerId, &FromWorker) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        for ev in sup.tick(Duration::from_millis(20)) {
            if let SupervisorEvent::Message { id, msg } = ev {
                if f(sup, id, &msg) {
                    return;
                }
            }
        }
    }
    panic!("Zeitüberschreitung");
}

#[test]
fn malicious_worker_cannot_shrink_shared_frame() {
    let mut sup = Supervisor::new();
    sup.spawn(spec("shrink")).unwrap();
    let mut reader = None;
    wait_for(&mut sup, |sup, id, m| {
        if let FromWorker::SharedFrame { handle, width, height } = m {
            let h = sup.take_shared(id, *handle).expect("Deskriptor übergeben");
            reader = Some(FrameBufferReader::open(h, *width, *height).expect("versiegelt angenommen"));
        }
        reader.is_some()
    });
    let mut log = String::new();
    wait_for(&mut sup, |_, _, m| {
        if let FromWorker::Log(l) = m {
            log = l.clone();
            return true;
        }
        false
    });
    assert!(log.starts_with("verkürzen abgelehnt"), "{log}");
    // Ganzer Puffer lesbar, kein SIGBUS.
    let px = reader.as_ref().unwrap().pixels();
    assert_eq!(px.iter().map(|b| *b as u64).sum::<u64>(), 0);
    assert_eq!(px.len(), 64 * 64 * 4);
    sup.shutdown();
}

#[test]
fn unsealed_memory_is_rejected() {
    let mut sup = Supervisor::new();
    sup.spawn(spec("unsealed")).unwrap();
    let mut result = None;
    wait_for(&mut sup, |sup, id, m| {
        if let FromWorker::SharedFrame { handle, width, height } = m {
            let h = sup.take_shared(id, *handle).expect("Deskriptor übergeben");
            result = Some(FrameBufferReader::open(h, *width, *height).map(|_| ()).map_err(|e| e.kind()));
        }
        result.is_some()
    });
    assert_eq!(result, Some(Err(std::io::ErrorKind::PermissionDenied)));
    sup.shutdown();
}
