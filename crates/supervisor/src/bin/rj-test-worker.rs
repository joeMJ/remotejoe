//! Prüf-Worker für den Supervisor. Verhalten per Argument:
//! `normal`, `panic`, `abort`, `segv`, `hang`, `oom`; Auslösung nach
//! `--after-ms`. Herzschlag alle `--heartbeat-ms`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rj_ipc::{FrameReader, FrameWriter, FromWorker, ToWorker, PROTOCOL_VERSION};

fn arg(name: &str, default: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| default.to_string())
}

fn main() {
    let mode = arg("--mode", "normal");
    let after = Duration::from_millis(arg("--after-ms", "300").parse().unwrap_or(300));
    let beat = Duration::from_millis(arg("--heartbeat-ms", "100").parse().unwrap_or(100));

    let out = Arc::new(Mutex::new(FrameWriter::new(std::io::stdout())));
    let send = {
        let out = out.clone();
        move |m: &FromWorker| out.lock().unwrap().send(0, m).is_ok()
    };
    send(&FromWorker::Hello { pid: std::process::id(), kind: "test".into(), protocol_version: PROTOCOL_VERSION });

    let beating = Arc::new(AtomicBool::new(true));
    {
        let beating = beating.clone();
        let send = send.clone();
        thread::spawn(move || {
            let mut seq = 0;
            loop {
                thread::sleep(beat);
                if beating.load(Ordering::Relaxed) {
                    seq += 1;
                    if !send(&FromWorker::Heartbeat { seq }) {
                        std::process::exit(0);
                    }
                }
            }
        });
    }

    if mode != "normal" {
        let beating = beating.clone();
        #[cfg_attr(not(unix), allow(unused_variables))]
        let send2 = send.clone();
        thread::spawn(move || {
            thread::sleep(after);
            match mode.as_str() {
                "panic" => {
                    // Panik im Hauptfaden-Ersatz: Prozess endet mit Code 101.
                    std::panic::set_hook(Box::new(|_| {}));
                    let r = std::panic::catch_unwind(|| panic!("absichtliche Panik"));
                    if r.is_err() {
                        std::process::exit(101);
                    }
                }
                "abort" => std::process::abort(),
                "segv" => {
                    #[cfg(unix)]
                    // SAFETY: Absichtlicher Absturz für den Test.
                    unsafe {
                        // Die Laufzeit fängt SIGSEGV für Stapelüberläufe ab; zurücksetzen.
                        libc::signal(libc::SIGSEGV, libc::SIG_DFL);
                        libc::raise(libc::SIGSEGV);
                    }
                }
                "hang" => {
                    beating.store(false, Ordering::Relaxed);
                }
                // Bösartiger Worker: Bildpuffer übergeben, dann verkürzen.
                #[cfg(unix)]
                "shrink" => {
                    use std::os::fd::{AsRawFd, AsFd};
                    let fb = Box::leak(Box::new(rj_ipc::shm::FrameBufferWriter::create(64, 64).expect("Bildpuffer")));
                    let handle = rj_ipc::fdpass::share(fb).expect("übergeben");
                    send2(&FromWorker::SharedFrame { handle, width: 64, height: 64 });
                    thread::sleep(Duration::from_millis(100));
                    // SAFETY: ftruncate auf den eigenen Deskriptor.
                    let rc = unsafe { libc::ftruncate(fb.handle().as_raw_fd(), 0) };
                    let err = std::io::Error::last_os_error();
                    send2(&FromWorker::Log(if rc == 0 { "verkürzt".into() } else { format!("verkürzen abgelehnt: {err}") }));
                    let _ = fb.handle().as_fd();
                }
                // Bösartiger Worker: unversiegelten Speicher übergeben.
                #[cfg(unix)]
                "unsealed" => {
                    use std::os::fd::AsFd;
                    let f = tempfile_in_memory();
                    let handle = rj_ipc::fdpass::share_fd(f.as_fd()).expect("übergeben");
                    send2(&FromWorker::SharedFrame { handle, width: 64, height: 64 });
                    std::mem::forget(f);
                }
                "oom" => {
                    let mut hog: Vec<Vec<u8>> = Vec::new();
                    loop {
                        hog.push(vec![1u8; 8 * 1024 * 1024]);
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                _ => {}
            }
        });
    }

    let mut r = FrameReader::new(std::io::stdin());
    loop {
        match r.recv::<ToWorker>() {
            Ok((_, ToWorker::Ping)) => {
                send(&FromWorker::Pong);
            }
            Ok((_, ToWorker::Shutdown)) | Err(_) => std::process::exit(0),
            Ok(_) => {}
        }
    }
}

/// Unversiegelter anonymer Speicher (für den Test „unsealed“).
#[cfg(unix)]
fn tempfile_in_memory() -> std::fs::File {
    use std::os::fd::FromRawFd;
    // SAFETY: memfd_create mit festem Namen; der Deskriptor gehört der Datei.
    let f = unsafe { std::fs::File::from_raw_fd(libc::memfd_create(c"unversiegelt".as_ptr(), libc::MFD_CLOEXEC)) };
    f.set_len(64 * 64 * 4).expect("Größe");
    f
}
