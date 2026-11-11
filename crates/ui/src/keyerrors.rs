//! Einstellung „Bei Schlüsselfehlern trotzdem fortfahren“ (Einstellungen → Sicherheit): Unbekannte oder geänderte
//! Host-Schlüssel und Zertifikate werden dann ohne Rückfrage nur für diese eine Verbindung angenommen – nichts wird
//! gespeichert. Jeder solche Fall wird gemeldet (rot in der Fußzeile); die Meldungen sammelt diese Stelle, auch aus
//! Hintergrundaufgaben, und `App::poll` gibt sie an die Oberfläche weiter.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ACCEPT: AtomicBool = AtomicBool::new(false);
static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Einstellung übernehmen (beim Start und beim Ändern).
pub fn set(on: bool) {
    ACCEPT.store(on, Ordering::SeqCst);
}

/// Schlüsselfehler ohne Rückfrage übergehen?
pub fn accept() -> bool {
    ACCEPT.load(Ordering::SeqCst)
}

/// Übergangenen Schlüsselfehler melden (aus jedem Faden).
pub fn warn(text: String) {
    WARNINGS.lock().unwrap_or_else(|e| e.into_inner()).push(text);
}

/// Gesammelte Meldungen abholen.
pub fn take() -> Vec<String> {
    std::mem::take(&mut *WARNINGS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Text für die Fußzeile: was übergangen wurde, mit Fingerabdruck.
pub fn text(kind: &str, host: &str, port: u16, key_type: &str, fingerprint: &str, changed: bool) -> String {
    let what = match (kind, changed) {
        ("certificate", true) => "Zertifikat geändert",
        ("certificate", false) => "Zertifikat unbekannt",
        (_, true) => "Host-Schlüssel geändert",
        (_, false) => "Host-Schlüssel unbekannt",
    };
    let key = [key_type, fingerprint].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
    format!("Schlüsselfehler bei {host}:{port} ({what}, {key}) – trotzdem fortgefahren, nur für diese Verbindung")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_are_collected_once() {
        // Eigener Zustand nur über take(): vorhandene Meldungen anderer Tests stören nicht.
        let _ = take();
        warn(text("hostkey", "192.0.2.5", 22, "ssh-ed25519", "SHA256:abc", true));
        let w = take();
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("192.0.2.5:22") && w[0].contains("Host-Schlüssel geändert") && w[0].contains("SHA256:abc"));
        assert!(take().is_empty());
        assert!(text("certificate", "h", 443, "X.509", "aa:bb", false).contains("Zertifikat unbekannt"));
    }
}
