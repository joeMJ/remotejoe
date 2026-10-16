//! Tests gegen TigerVNC (Xvnc) mit xterm und xclip auf der Anzeige.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use rj_core::{ClipboardDirection, Secret};
use rj_testkit::xvnc::{VncSecurity, Xvnc, PASSWORD};
use rj_vnc::client::*;

fn params(x: &Xvnc, password: &str) -> VncParams {
    VncParams {
        host: "127.0.0.1".into(),
        port: x.port,
        username: None,
        password: Some(Secret::from_str_exact(password)),
        shared: true,
        view_only: false,
        clipboard: ClipboardDirection::Both,
        clipboard_limit: 16 * 1024 * 1024,
        prefer_tls: true,
        known_certs: Some(KnownCerts::new(x.dir.path().join("known_certs"))),
        connect_timeout: Duration::from_secs(15),
        extended_clipboard: true,
    }
}

fn start(sec: VncSecurity) -> Option<Arc<Xvnc>> {
    match Xvnc::start(sec) {
        Ok(x) => Some(Arc::new(x)),
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {e}");
            None
        }
    }
}

fn accept_all() -> CertCheck {
    Arc::new(|_| true)
}

fn wait(c: &VncClient, ms: u64, mut f: impl FnMut(&VncEvent) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        match c.events.recv_timeout(Duration::from_millis(50)) {
            Ok(VncEvent::Closed(r)) => panic!("Verbindung beendet: {r}"),
            Ok(e) => {
                if f(&e) {
                    return true;
                }
            }
            Err(_) => {}
        }
    }
    false
}

#[test]
fn vnc_auth_frame_keys_clipboard() {
    let Some(x) = start(VncSecurity::VncAuth) else { return };
    // Falsches Kennwort.
    let bad = VncClient::connect(&params(&x, "falsch"), accept_all());
    assert!(matches!(bad, Err(VncError::AuthFailed(_))), "{:?}", bad.err());
    // NFD-Fassung des Umlauts: andere Bytes, abgelehnt.
    let nfd = PASSWORD.replace('ä', "a\u{0308}");
    assert!(VncClient::connect(&params(&x, &nfd), accept_all()).is_err());

    let c = VncClient::connect(&params(&x, PASSWORD), accept_all()).expect("Verbindung");
    assert_eq!(c.security, "VNC-Kennwort");
    assert!(wait(&c, 10_000, |e| matches!(e, VncEvent::Frame { .. })), "Bild kommt an");
    let nonzero = c.with_frame(|fb| {
        assert_eq!((fb.width, fb.height), (1024, 768));
        fb.data.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count()
    });
    assert!(nonzero > 1000, "xterm sichtbar ({nonzero} Bildpunkte)");

    // Tastatur.
    c.pointer(0, 100, 100).unwrap();
    c.type_text("Grüße ß äöü € 🚀\tTab\n").unwrap();
    wait(&c, 1500, |_| false);
    let typed = x.typed_text();
    eprintln!("MATRIX Tastatur: {typed:?}");
    assert_eq!(typed, "Grüße ß äöü € 🚀\tTab\n");

    // Zwischenablage-Matrix (erweiterte Zwischenablage, UTF-8).
    wait(&c, 500, |_| false);
    assert!(c.extended_clipboard(), "Gegenstelle bietet erweiterte Zwischenablage an");
    let mut big = String::new();
    let mut i = 0;
    while big.len() < 100 * 1024 {
        big.push_str(&format!("Zeile {i:05}: Grüße ß 🚀 äöü\tTab\n"));
        i += 1;
    }
    let cases: Vec<(&str, String)> = vec![
        ("Umlaute", "äöü ÄÖÜ".into()),
        ("ß", "Straße ẞ".into()),
        ("Emoji", "🚀👍🏽 €".into()),
        ("Tab", "a\tb\t\tc".into()),
        ("CRLF", "eins\r\nzwei\r\ndrei".into()),
        ("LF", "eins\nzwei".into()),
        ("100 kB", big),
    ];
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut failures = Vec::new();
    for (name, text) in &cases {
        c.send_clipboard(text).unwrap();
        wait(&c, 700, |_| false);
        let got = x.x_run("timeout 5 xclip -o -selection clipboard | base64 -w0", None).ok().and_then(|g| b64.decode(g.trim()).ok()).unwrap_or_default();
        let expected = text.replace("\r\n", "\n");
        let ok_ht = got == expected.as_bytes();
        eprintln!("MATRIX Host→Ziel {name}: {}", if ok_ht { "grün".into() } else { format!("abweichend ({} statt {} Bytes)", got.len(), expected.len()) });

        let x2 = x.clone();
        let data = text.clone().into_bytes();
        let h = std::thread::spawn(move || x2.x_run("timeout 8 xclip -i -selection clipboard -loops 2 -quiet", Some(&data)));
        let mut received = None;
        wait(&c, 8000, |e| {
            if let VncEvent::ClipText(t) = e {
                received = Some(t.clone());
                return true;
            }
            if let VncEvent::Notice(n) = e {
                eprintln!("Hinweis: {n}");
            }
            false
        });
        let _ = h.join();
        let ok_th = received.as_deref() == Some(expected.as_str());
        eprintln!("MATRIX Ziel→Host {name}: {}", if ok_th { "grün".into() } else { format!("abweichend: {:?}", received.map(|r| r.chars().take(30).collect::<String>())) });
        if !(ok_ht && ok_th) {
            failures.push(*name);
        }
    }
    assert!(failures.is_empty(), "Zwischenablage-Matrix: {failures:?}");

    // Größenänderung.
    assert!(c.resize(800, 600).unwrap(), "ExtendedDesktopSize angeboten");
    assert!(wait(&c, 5000, |e| matches!(e, VncEvent::Resized { width: 800, height: 600 })), "Größe geändert");
}

/// Ohne erweiterte Zwischenablage: nur Latin-1, sichtbare Warnung statt Zeichenverlust.
#[test]
fn legacy_clipboard_warns_instead_of_losing_characters() {
    let Some(x) = start(VncSecurity::VncAuth) else { return };
    let mut p = params(&x, PASSWORD);
    p.extended_clipboard = false;
    let c = VncClient::connect(&p, accept_all()).unwrap();
    wait(&c, 1000, |_| false);
    assert!(!c.extended_clipboard());
    let err = c.send_clipboard("Rakete 🚀").unwrap_err();
    assert!(err.contains("Latin-1") && err.contains('🚀'), "{err}");
    c.send_clipboard("Grüße").unwrap();
    wait(&c, 700, |_| false);
    assert_eq!(x.x_run("timeout 5 xclip -o -selection clipboard", None).unwrap(), "Grüße");
    // Ziel → Host: Latin-1 mit Hinweis.
    let x2 = x.clone();
    let h = std::thread::spawn(move || x2.x_run("printf 'Straße' | timeout 8 xclip -i -selection clipboard -loops 2 -quiet", None));
    let mut notice = false;
    let mut text = None;
    wait(&c, 8000, |e| {
        match e {
            VncEvent::Notice(n) if n.contains("Latin-1") => notice = true,
            VncEvent::ClipText(t) => text = Some(t.clone()),
            _ => {}
        }
        notice && text.is_some()
    });
    let _ = h.join();
    assert!(notice, "Warnung angezeigt");
    assert_eq!(text.as_deref(), Some("Straße"));
}

#[test]
fn vencrypt_x509_with_certificate_prompt() {
    let Some(x) = start(VncSecurity::X509Vnc) else { return };
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // Ablehnen.
    let a = asked.clone();
    let r = VncClient::connect(&params(&x, PASSWORD), Arc::new(move |_| {
        a.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        false
    }));
    assert!(matches!(r, Err(VncError::CertificateRejected)), "{:?}", r.err());
    // Annehmen und merken.
    let a = asked.clone();
    let c = VncClient::connect(&params(&x, PASSWORD), Arc::new(move |info| {
        assert!(info.old_fingerprint.is_none());
        a.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        true
    }))
    .expect("VeNCrypt");
    assert_eq!(c.security, "VeNCrypt X.509 + VNC-Kennwort");
    assert!(wait(&c, 10_000, |e| matches!(e, VncEvent::Frame { .. })));
    c.send_clipboard("über TLS 🚀").unwrap();
    wait(&c, 700, |_| false);
    assert_eq!(x.x_run("timeout 5 xclip -o -selection clipboard", None).unwrap(), "über TLS 🚀");
    drop(c);
    // Bekannt: keine weitere Frage.
    let before = asked.load(std::sync::atomic::Ordering::SeqCst);
    let _c = VncClient::connect(&params(&x, PASSWORD), Arc::new(|_| panic!("keine Frage erwartet"))).unwrap();
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), before);
}
