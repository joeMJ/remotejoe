//! Ende-zu-Ende über den Worker-Prozess und den Supervisor.

use std::time::{Duration, Instant};

use rj_ipc::*;
use rj_supervisor::*;
use rj_testkit::xvnc::{VncSecurity, Xvnc, PASSWORD};

#[test]
fn worker_connect_frame_clipboard_disconnect() {
    let mut x = match Xvnc::start(VncSecurity::VncAuth) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {e}");
            return;
        }
    };
    let mut sup = Supervisor::new();
    let t0 = Instant::now();
    let (id, _) = sup.spawn(WorkerSpec::new(env!("CARGO_BIN_EXE_rj-worker-vnc"))).unwrap();
    sup.send(id, &ToWorker::Connect(ConnectRequest {
        host: "127.0.0.1".into(),
        port: x.port,
        width: 1024,
        height: 768,
        options_json: "{}".into(),
        connect_timeout_secs: 15,
        keepalive_secs: 30,
    }))
    .unwrap();
    sup.send(id, &ToWorker::Credentials(CredentialPayload {
        username: String::new(),
        domain: None,
        password: SecretBytes(PASSWORD.as_bytes().to_vec()),
        private_key: None,
    }))
    .unwrap();
    let mut shm = None;
    let mut frames = 0;
    let mut connected = false;
    let until = |sup: &mut Supervisor, secs: u64, f: &mut dyn FnMut(&mut Supervisor, &FromWorker) -> bool| -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            for ev in sup.tick(Duration::from_millis(50)) {
                if let SupervisorEvent::Message { msg, .. } = ev {
                    if f(sup, &msg) {
                        return true;
                    }
                }
            }
        }
        false
    };
    assert!(until(&mut sup, 20, &mut |sup, m| {
        match m {
            FromWorker::State(SessionState::Connected) => connected = true,
            FromWorker::State(SessionState::Failed { reason }) => panic!("{reason}"),
            FromWorker::SharedFrame { handle, width, height } => {
                let h = sup.take_shared(id, *handle).expect("Bildpuffer übergeben");
                shm = Some(rj_ipc::shm::FrameBufferReader::open(h, *width, *height).unwrap())
            }
            FromWorker::Frame(_) => frames += 1,
            _ => {}
        }
        connected && frames > 0 && shm.is_some()
    }));
    eprintln!("Messung: verbunden mit erstem Bild im geteilten Speicher nach {} ms", t0.elapsed().as_millis());
    let px = shm.as_ref().unwrap().pixels().chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
    assert!(px > 1000, "Bild liegt im geteilten Speicher");
    sup.send(id, &ToWorker::Clipboard(ClipData::Text("über den Worker 🚀".into()))).unwrap();
    until(&mut sup, 1, &mut |_, _| false);
    assert_eq!(x.x_run("timeout 5 xclip -o -selection clipboard", None).unwrap(), "über den Worker 🚀");
    x.kill_server();
    assert!(until(&mut sup, 10, &mut |_, m| matches!(m, FromWorker::State(SessionState::Disconnected { .. }))), "Abbruch gemeldet");
    assert!(sup.is_running(id), "Worker lebt weiter");
    sup.shutdown();
}
