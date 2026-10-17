//! NLA (CredSSP/NTLM) gegen den FreeRDP-Shadow-Server. Das Kennwort enthält
//! Umlaut, ß und Emoji; der NT-Hash wird aus den UTF-16-Codeeinheiten des
//! unveränderten Kennworts gebildet, so dass nur die byte-genaue Übergabe zur
//! Anmeldung führt.

use std::time::{Duration, Instant};

use md4::{Digest, Md4};
use rj_core::{ClipboardDirection, Secret};
use rj_rdp::session::*;
use rj_testkit::shadow::Shadow;

const USER: &str = "rjnla";
const PASSWORD: &str = "Prüf-Kennwort-ß-🚀";

fn nt_hash(pw: &str) -> String {
    let utf16: Vec<u8> = pw.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    Md4::digest(&utf16).iter().map(|b| format!("{b:02x}")).collect()
}

fn params(port: u16, password: &str) -> RdpParams {
    RdpParams {
        host: "127.0.0.1".into(),
        port,
        username: USER.into(),
        domain: None,
        password: Secret::from_str_exact(password),
        width: 800,
        height: 600,
        security: Security::Nla,
        ignore_certificate: false,
        clipboard: ClipboardDirection::Off,
        clipboard_limit: 0,
        keyboard_layout: 0x0407,
        config_path: Some(tempfile::tempdir().unwrap().keep().display().to_string()),
        timeout_ms: 15000,
        gfx: false,
    }
}

#[test]
fn nla_with_unicode_password() {
    let line = format!("{USER}::aad3b435b51404eeaad3b435b51404ee:{}:::", nt_hash(PASSWORD));
    let server = match Shadow::start(&line) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {e}");
            return;
        }
    };
    // Falsches Kennwort (NFD statt NFC) wird abgelehnt.
    let nfd = PASSWORD.replace('ü', "u\u{0308}");
    let mut bad = RdpSession::new(|_| CertDecision::AcceptOnce).unwrap();
    assert!(bad.connect(&params(server.port, &nfd)).is_err(), "NFD-Fassung darf nicht passen");

    let mut s = RdpSession::new(|_| CertDecision::AcceptOnce).unwrap();
    s.connect(&params(server.port, PASSWORD)).expect("NLA-Anmeldung mit Unicode-Kennwort");
    let mut frames = 0;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut ev = Vec::new();
    while frames == 0 && Instant::now() < deadline {
        ev.clear();
        assert!(s.poll(50, &mut ev), "getrennt: {ev:?}");
        frames += ev.iter().filter(|e| matches!(e, RdpEvent::Frame { .. })).count();
    }
    assert!(frames > 0, "Bild kommt an");
}
