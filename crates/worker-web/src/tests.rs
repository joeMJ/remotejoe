use super::*;

#[test]
fn urls_are_checked() {
    let u = check_url("https://web.example.test:8443/anmeldung?x=1").unwrap();
    assert_eq!((u.https, u.host.as_str(), u.port), (true, "web.example.test", 8443));
    assert_eq!(check_url("http://[2001:db8::1]/").unwrap().port, 80);
    assert_eq!(check_url("https://admin:geheim@web.example.test/"), Err(WebError::CredentialsInUrl));
    assert!(matches!(check_url("javascript:alert(1)"), Err(WebError::Url(_))));
    assert!(matches!(check_url("file:///etc/passwd"), Err(WebError::Url(_))));
    let o = WebOptions::default();
    assert_eq!(start_url(&o, "192.0.2.5", 443).unwrap().url, "https://192.0.2.5/");
    assert_eq!(start_url(&o, "2001:db8::1", 8006).unwrap().url, "https://[2001:db8::1]:8006/");
}

#[test]
fn profiles_are_separate_per_target() {
    let base = Path::new("/daten");
    let a = profile_dir(base, "6f1c0000-0000-4000-8000-000000000001");
    let b = profile_dir(base, "6f1c0000-0000-4000-8000-000000000002");
    assert_ne!(a, b);
    assert_eq!(profile_dir(base, "../../x"), base.join("web-profile").join("______x"));
}

#[test]
fn script_escapes_password_and_never_touches_url() {
    let l = FormLogin { username_selector: Some("#user".into()), password_selector: Some("input[name='pw']".into()), ..Default::default() };
    let pw = "a\"b'c\\d</script>\u{2028}e\u{308}🚀 ";
    let s = login_script(&l, "admin", &Secret::from_str_exact(pw)).unwrap();
    assert!(s.contains(r#"p="a\"b'c\\d</script>\u2028e"#), "{}", s.as_str());
    assert!(s.contains("\u{308}🚀 \""), "Kennwort unverändert (kein NFC, nicht getrimmt)");
    assert!(!s.contains("location") && !s.contains("href"), "keine Navigation mit Kennwort");
    assert!(login_script(&FormLogin::default(), "a", &Secret::from_str_exact("x")).is_err());
    assert_eq!(login_script(&l, "a", &Secret::from_bytes(vec![0xff])), Err(WebError::PasswordNotUtf8));
}

#[test]
fn flow_limits_attempts() {
    let mut f = LoginFlow::new(Some(FormLogin { password_selector: Some("#pw".into()), url_contains: Some("/login".into()), ..Default::default() }));
    assert_eq!(f.page_loaded("https://web.example.test/start"), FlowAction::None);
    for _ in 0..3 {
        assert_eq!(f.page_loaded("https://web.example.test/login"), FlowAction::InjectLogin);
    }
    assert!(matches!(f.page_loaded("https://web.example.test/login"), FlowAction::Notice(_)));
    assert_eq!(f.page_loaded("https://web.example.test/login"), FlowAction::None);
    assert!(matches!(f.script_result("\"rj:kein-kennwortfeld\""), FlowAction::Notice(m) if m.contains("Kennwortfeld")));
    assert!(matches!(f.script_result("x\u{202e}y"), FlowAction::Notice(m) if m.contains("⟦U+202E⟧")));
    let mut none = LoginFlow::new(None);
    assert_eq!(none.page_loaded("https://web.example.test/login"), FlowAction::None);
}

#[test]
fn certificates_warn_and_remember() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = AcceptedCerts::load(dir.path());
    let cert = b"zertifikat-a";
    assert_eq!(c.decide("web.example.test", cert, &[]), CertDecision::Valid);
    let probs = vec!["unbekannte Zertifizierungsstelle".to_string()];
    assert!(matches!(c.decide("web.example.test", cert, &probs), CertDecision::Ask { text, .. } if !text.contains("geändert")));
    c.accept("web.example.test", cert);
    c.save(dir.path()).unwrap();
    let c = AcceptedCerts::load(dir.path());
    assert_eq!(c.decide("web.example.test", cert, &probs), CertDecision::Accepted);
    assert!(matches!(c.decide("web.example.test", b"zertifikat-b", &probs), CertDecision::Ask { text, .. } if text.contains("geändert")));
}

/// Führt das Skript in node mit einer kleinen DOM-Attrappe aus.
#[test]
fn script_fills_form_in_fake_dom() {
    let node = std::env::var("RJ_NODE").unwrap_or_else(|_| "node".into());
    if std::process::Command::new(&node).arg("--version").output().is_err() {
        eprintln!("ÜBERSPRUNGEN: node nicht vorhanden");
        return;
    }
    let pw = "Prüf-\"Kennwort\"-ä 🚀";
    let cases = [
        (FormLogin { username_selector: Some("#user".into()), password_selector: Some("#pw".into()), submit_selector: Some("#go".into()), ..Default::default() }, "rj:ok", "click"),
        (FormLogin { username_selector: Some("#user".into()), password_selector: Some("#pw".into()), ..Default::default() }, "rj:ok", "submit"),
        (FormLogin { password_selector: Some("#fehlt".into()), ..Default::default() }, "rj:kein-kennwortfeld", ""),
        (FormLogin { script: Some("rj.fill('#user', rj.username); rj.fill('#pw', rj.password); return 'rj:fertig';".into()), ..Default::default() }, "rj:fertig", ""),
    ];
    for (login, want, action) in cases {
        let script = login_script(&login, "admin", &Secret::from_str_exact(pw)).unwrap();
        let harness = format!(
            r#"
const log = [];
class Ev {{ constructor(t){{ this.type = t; }} }}
globalThis.Event = Ev;
function mk(id) {{
  const el = {{ id, _v: '', focus(){{}}, dispatchEvent(e){{ log.push(id + ':' + e.type); }}, click(){{ log.push(id + ':click'); }} }};
  return el;
}}
const proto = {{}};
Object.defineProperty(proto, 'value', {{ get(){{ return this._v; }}, set(v){{ this._v = v; log.push('set'); }} }});
const form = {{ requestSubmit(){{ log.push('submit'); }} }};
const els = {{ '#user': Object.setPrototypeOf(mk('user'), proto), '#pw': Object.setPrototypeOf(mk('pw'), proto), '#go': mk('go') }};
els['#pw'].form = form;
globalThis.document = {{ querySelector(s){{ return els[s] || null; }} }};
const r = {script};
console.log(JSON.stringify({{ r, user: els['#user']._v, pw: els['#pw']._v, log }}));
"#,
            script = script.as_str()
        );
        let out = std::process::Command::new(&node).arg("-e").arg(&harness).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["r"], want);
        if want != "rj:kein-kennwortfeld" {
            assert_eq!(v["user"], "admin");
            assert_eq!(v["pw"], pw);
        }
        if !action.is_empty() {
            assert!(v["log"].as_array().unwrap().iter().any(|e| e.as_str().unwrap().ends_with(action)), "{v}");
        }
    }
}
