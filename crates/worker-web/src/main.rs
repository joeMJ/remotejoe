//! Web-Worker: WebView ohne eigenes sichtbares Fenster. Unter Linux rendert
//! WebKitGTK in ein `GtkOffscreenWindow`; nach jeder Änderung („damage“)
//! wird der geänderte Bereich in den versiegelten Bildspeicher kopiert, den
//! das Hauptfenster wie bei RDP und VNC im Tab der Sitzung anzeigt. Eingaben
//! aus dem Hauptfenster werden als GDK-Ereignisse an die WebView gegeben.
//! Eigenes Profil je Ziel, Formular-Anmeldung per Skript.

#[cfg(target_os = "linux")]
mod offscreen;

#[cfg(target_os = "linux")]
fn main() {
    offscreen::run();
}

/// Unter Windows ist das Rendern ohne Fenster (WebView2 mit
/// Composition-Controller) noch nicht umgesetzt; der Worker meldet das,
/// statt ein eigenes Fenster zu öffnen.
#[cfg(not(target_os = "linux"))]
fn main() {
    use std::time::Duration;

    use rj_ipc::worker::WorkerLink;
    use rj_ipc::*;

    let mut link = match WorkerLink::stdio("web", Duration::from_secs(2)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Kanal nicht verfügbar: {e}");
            std::process::exit(2);
        }
    };
    loop {
        match link.recv() {
            Ok(ToWorker::Connect(_)) => {
                let _ = link.sender.send(&FromWorker::State(SessionState::Failed {
                    reason: "Web-Ziele werden unter Windows noch nicht im Tab angezeigt.".into(),
                }));
            }
            Ok(ToWorker::Ping) => {
                let _ = link.sender.send(&FromWorker::Pong);
            }
            Ok(ToWorker::Shutdown) | Err(_) => return,
            Ok(_) => {}
        }
    }
}
