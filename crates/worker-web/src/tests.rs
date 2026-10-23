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
fn embedding_commands_round_trip_and_old_ones_stay() {
    let cmds = [
        WebCommand::Embed { hwnd: 0x1234_5678_9abc },
        WebCommand::Bounds { x: 0, y: 0, w: 1280, h: 720, scale: 150 },
        WebCommand::Visible { on: false },
        WebCommand::Keys { keys: vec![ReservedKey { vk: 0x78, ..Default::default() }, ReservedKey { vk: 0x22, ctrl: true, ..Default::default() }] },
    ];
    for c in cmds {
        let j = serde_json::to_vec(&c).unwrap();
        assert_eq!(serde_json::from_slice::<WebCommand>(&j).unwrap(), c);
    }
    // Der Bildmaßstab ist optional (0: unverändert).
    let b: WebCommand = serde_json::from_str(r#"{"op":"bounds","x":1,"y":2,"w":3,"h":4}"#).unwrap();
    assert_eq!(b, WebCommand::Bounds { x: 1, y: 2, w: 3, h: 4, scale: 0 });
    let n: WebCommand = serde_json::from_str(r#"{"op":"nav","action":"back"}"#).unwrap();
    assert_eq!(n, WebCommand::Nav { action: NavAction::Back });
    let e = serde_json::to_string(&WebEvent::Download { id: 1, state: "done".into(), name: "a.ica".into(), dir: "d".into(), received: 3, total: 3, error: None, opened: true }).unwrap();
    assert!(e.contains(r#""ev":"download""#) && e.contains(r#""opened":true"#), "{e}");
    let o: WebOptions = serde_json::from_str(r#"{"url":"https://x.test/","headless":true}"#).unwrap();
    assert!(o.headless && !WebOptions::default().headless);
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
    // Ohne Selektoren sucht das Skript die Felder selbst.
    let auto = login_script(&FormLogin::default(), "a", &Secret::from_str_exact("x")).unwrap();
    assert!(auto.contains("input[type=password]") && auto.contains("requestSubmit"), "{}", &*auto);
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
fn flow_checks_for_rejection_after_submit_without_resubmitting() {
    let mut f = LoginFlow::new(Some(FormLogin::default()));
    assert_eq!(f.page_loaded(""), FlowAction::None, "leere Startseite zählt nicht");
    assert_eq!(f.page_loaded("about:blank"), FlowAction::None);
    assert_eq!(f.page_loaded("https://web.example.test/"), FlowAction::InjectLogin);
    assert_eq!(f.script_result("\"rj:ok\""), FlowAction::None);
    assert_eq!(f.page_loaded("https://web.example.test/"), FlowAction::CheckRejected);
    assert_eq!(f.rejected_check("\"rj:kennwortfeld-weg\""), FlowAction::None);
    assert!(matches!(f.rejected_check("\"rj:kennwortfeld-da\""), FlowAction::Notice(m) if m.contains("abgelehnt")));
    // Hinweis steht: weiter nachprüfen, kein erneutes Abschicken, keine zweite Meldung.
    assert_eq!(f.rejected_check("\"rj:kennwortfeld-da\""), FlowAction::CheckRejected);
    assert_eq!(f.page_loaded("https://web.example.test/"), FlowAction::CheckRejected, "kein erneutes Abschicken");
    // Von Hand angemeldet: Kennwortfeld weg → Hinweis zurücknehmen, danach Ruhe.
    assert_eq!(f.rejected_check("\"rj:kennwortfeld-weg\""), FlowAction::Notice(String::new()));
    assert_eq!(f.rejected_check("\"rj:kennwortfeld-weg\""), FlowAction::None);
    assert_eq!(f.page_loaded("https://web.example.test/"), FlowAction::None);
    assert!(presence_script().contains("input[type=password]"));
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

#[test]
fn ignore_certificate_option_is_read_from_the_target() {
    let o: WebOptions = serde_json::from_str(r##"{"url":"https://192.0.2.10/","ignore_certificate":true,"login":{"username_selector":"#u"}}"##).unwrap();
    assert!(o.ignore_certificate);
    assert_eq!(o.login.unwrap().username_selector.as_deref(), Some("#u"));
    // Ohne Angabe wird nicht übergangen.
    assert!(!serde_json::from_str::<WebOptions>("{}").unwrap().ignore_certificate);
}

#[test]
fn web_command_json_and_shortcuts() {
    let c: WebCommand = serde_json::from_str(r#"{"op":"nav","action":"reload"}"#).unwrap();
    assert_eq!(c, WebCommand::Nav { action: NavAction::Reload });
    let c: WebCommand = serde_json::from_str(r#"{"op":"menu","id":0,"action":"copy"}"#).unwrap();
    assert_eq!(c, WebCommand::Menu { id: 0, index: None, action: Some("copy".into()) });
    assert!(serde_json::from_str::<WebCommand>(r#"{"op":"exec","script":"x"}"#).is_err());
    let ev = WebEvent::Status { title: "t".into(), uri: String::new(), loading: false, progress: 100, error: None, back: true, forward: false };
    let v: serde_json::Value = serde_json::from_slice(&ev.to_bytes()).unwrap();
    assert_eq!(v["ev"], "status");

    let mut m = Modifiers::default();
    assert!(m.update(0xffe3, true));
    assert_eq!(m.shortcut(0x63), Some(EditAction::Copy));
    assert_eq!(m.shortcut(0x56), Some(EditAction::Paste));
    assert_eq!(m.shortcut(0x61), None, "Strg+A erledigt die Seite selbst");
    assert!(m.update(0xffe9, true));
    assert_eq!(m.shortcut(0x63), None, "Strg+Alt+C ist kein Kopieren");
    assert!(!m.update(0x61, true));

    let items = context_menu(false, false, true, false, true);
    let get = |a: &str| items.iter().find(|i| i.action == a).unwrap().enabled;
    assert!(!get("back") && get("paste") && get("copy") && !get("cut"));
}

#[test]
fn damage_union_is_clipped() {
    let mut d = Damage::default();
    assert_eq!(d.take(100, 100), None);
    d.add(10, 10, 5, 5);
    d.add(-5, 50, 20, 100);
    d.add(0, 0, 0, 10);
    assert_eq!(d.take(100, 100), Some((0, 10, 15, 90)));
    assert_eq!(d.take(100, 100), None);
}

#[test]
fn certificates_are_kept_per_host_and_port() {
    let mut c = AcceptedCerts::default();
    let (a, b) = (vec![1u8; 16], vec![2u8; 16]);
    let p = vec!["unbekannte Stelle".to_string()];
    assert_eq!(AcceptedCerts::key("ap", 443), "ap");
    assert_eq!(AcceptedCerts::key("ap", 4343), "ap:4343");
    assert_eq!(AcceptedCerts::key("2001:db8::1", 4343), "[2001:db8::1]:4343");
    // Älterer Eintrag ohne Port (Zertifikat von 4343): gilt weiter für ap:4343.
    c.accept("ap", &b);
    assert!(matches!(c.decide("ap:4343", &b, &p), CertDecision::Accepted));
    // 443 mit anderem Zertifikat: echte Änderung (gleicher Schlüssel „ap“).
    assert!(matches!(c.decide("ap", &a, &p), CertDecision::Ask { previous: Some(_), .. }));
    // Anderer Dienst auf demselben Rechner: keine „Änderung“, sondern neu.
    c.hosts.clear();
    c.accept("ap", &a);
    assert!(matches!(c.decide("ap:4343", &b, &p), CertDecision::Ask { previous: None, .. }));
    c.accept("ap:4343", &b);
    assert!(matches!(c.decide("ap:4343", &b, &p), CertDecision::Accepted) && matches!(c.decide("ap", &a, &p), CertDecision::Accepted));
    assert!(matches!(c.decide("ap:4343", &a, &p), CertDecision::Ask { previous: Some(_), .. }), "eigener Eintrag mit Port: Änderung");
}

#[test]
fn old_launch_files_are_found_others_stay() {
    let d = tempfile::tempdir().unwrap();
    for n in ["a.ica", "b.ICA", "bericht.pdf"] {
        std::fs::write(d.path().join(n), b"x").unwrap();
    }
    let mut found: Vec<String> = crate::stale_launch_files(d.path(), std::time::Duration::ZERO).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    found.sort();
    assert_eq!(found, ["a.ica", "b.ICA"]);
    assert!(crate::stale_launch_files(d.path(), std::time::Duration::from_secs(3600)).is_empty(), "frische Dateien bleiben");
    assert!(crate::stale_launch_files(&d.path().join("fehlt"), std::time::Duration::ZERO).is_empty());
}
