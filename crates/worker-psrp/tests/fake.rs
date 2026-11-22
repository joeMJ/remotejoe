//! PowerShell-Sitzung gegen eine Attrappe von pwsh (kein echtes Windows).
#![cfg(unix)] // die Attrappe `fake-pwsh` ist ein Shell-Skript

use rj_core::{PsrpTransport, Secret, WinrmAuth};
use rj_psrp::*;

fn target() -> PsTarget {
    PsTarget {
        host: "win.example.test".into(),
        username: "admin".into(),
        domain: Some("EXAMPLE".into()),
        transport: PsrpTransport::Winrm { https: true, port: None, auth: WinrmAuth::Kerberos, verify_certificate: true },
    }
}

fn fake() -> String {
    format!("{}/tests/fake-pwsh", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn streams_are_separated_and_password_stays_off_the_command_line() {
    let pw = "Kennwort-ä 🚀 mit Leerzeichen ";
    std::env::set_var("RJ_PWSH", fake());
    std::env::set_var("RJ_FAKE_EXPECT", pw);
    let mut s = PsSession::start(&target(), &Secret::from_str_exact(pw)).expect("Sitzung");
    let cmdline = std::fs::read(format!("/proc/{}/cmdline", s.pid())).unwrap_or_default();
    assert!(!String::from_utf8_lossy(&cmdline).contains("Kennwort"), "Kennwort nicht auf der Befehlszeile");
    let mut recs = Vec::new();
    let ok = s.run("Get-Service | Where-Object Status -eq 'Running'", |r| recs.push((r.stream.clone(), r.text.clone()))).unwrap();
    assert!(ok);
    let streams: Vec<&str> = recs.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(streams, ["output", "warning", "error", "progress", "verbose"]);
    assert!(recs[0].1.contains("Get-Service"));
    assert_eq!(recs[2].1, "Fehler 🚀");
    // Zweiter Befehl in derselben Sitzung.
    let mut n = 0;
    assert!(s.run("Get-Date", |_| n += 1).unwrap());
    assert_eq!(n, 5);
    // Falsches Kennwort (NFD-Fassung) wird abgelehnt.
    let nfd = pw.replace('ä', "a\u{0308}");
    assert!(matches!(PsSession::start(&target(), &Secret::from_str_exact(&nfd)), Err(PsError::Session(_))));
}
