//! Update über das Hauptprogramm gegen eine Attrappe: Prüfen und Installieren als
//! Hintergrundaufgabe, Rückfrage bei offenen Sitzungen. Die Quelle ist im Programm fest
//! GitHub; der Test setzt sie über `source_override` auf die Attrappe.

use std::time::{Duration, Instant};

use rj_testkit::gitea::GiteaMock;
use rj_ui::app::App;
use rj_ui::sessions::UiEvent;
use rj_update::release::{Source, SourceKind};
use rj_update::install;
use rj_update::sign::{self, SIG, SUMS};
#[cfg(not(windows))]
use rj_update::{archive, asset_name};
use serde_json::{json, Value};

fn wait_result(app: &mut App, kind: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        for e in app.poll() {
            if let UiEvent::JobResult { kind: k, json, .. } = e {
                if k == kind {
                    return serde_json::from_str(&json).unwrap();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("kein Ergebnis {kind}");
}

fn edit(app: &mut App, v: Value) -> Value {
    serde_json::from_str(&app.edit(&v.to_string())).unwrap()
}

/// Windows: Die Prüfung nennt die neue Version ohne Windows-Paket in der Veröffentlichung,
/// „Installieren“ liefert nur die Release-Seite; nichts wird ersetzt.
#[cfg(windows)]
#[test]
fn windows_shows_version_and_release_page() {
    let mock = GiteaMock::start("team/remotejoe");
    mock.set_release("v9.0.0", "Änderungen\n- MSI", &[]);
    let mut app = App::new(false);
    app.config_path = None;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(install::main_program()), b"alt").unwrap();
    app.update.dir_override = Some(dir.path().to_path_buf());
    assert_eq!(rj_ui::update::github_source().releases_page().unwrap(), "https://github.com/joeMJ/remotejoe/releases/latest");
    app.update.source_override = Some(Source { kind: SourceKind::Gitea, base_url: mock.base.clone(), repo: "team/remotejoe".into() });
    let page = format!("{}/team/remotejoe/releases/latest", mock.base);

    assert_eq!(app.update_status()["page_only"], true);
    edit(&mut app, json!({"op": "update_check"}));
    let r = wait_result(&mut app, "update_check");
    assert_eq!(r["state"], "newer", "{r}");
    assert_eq!(r["found"]["version"], "9.0.0");
    assert_eq!(r["found"]["notes"], "Änderungen\n- MSI");
    assert_eq!(r["page"], page);
    assert_eq!(app.update_status()["found"]["version"], "9.0.0");

    let r = edit(&mut app, json!({"op": "update_install", "confirm": true}));
    assert_eq!(r, json!({"state": "page", "url": page}));
    assert_eq!(std::fs::read(dir.path().join(install::main_program())).unwrap(), b"alt");
}

/// Windows, als MSI installiert: Die Prüfung wählt die MSI; geladen wird nur, was die signierte `SHA256SUMS`
/// bestätigt und was mit dem Zertifikat des Herausgebers signiert ist. Jede fehlgeschlagene Prüfung lässt keine
/// MSI liegen und nichts bereit zur Installation. Eine Veröffentlichung ohne MSI führt zur Release-Seite.
/// Mit `RJ_TEST_SIGNED_MSI` (eine signierte MSI, z. B. aus `scripts/build-msi.ps1`) wird auch der gute Weg geprüft.
#[cfg(windows)]
#[test]
fn windows_msi_update_is_verified_before_it_is_offered() {
    let key = sign::generate().unwrap();
    let mock = GiteaMock::start("team/remotejoe");
    let msi = rj_update::msi::msi_name("9.0.0");
    let set = |data: &[u8], listed: &[u8]| {
        let sums = sign::sums_text(&[(msi.clone(), sign::sha256_hex(listed))]);
        let mut st = mock.state.lock().unwrap();
        st.files.insert(msi.clone(), data.to_vec());
        st.files.insert(SUMS.into(), sums.clone().into_bytes());
        st.files.insert(SIG.into(), sign::sign(&key, sums.as_bytes()).into_bytes());
    };
    let mut app = App::new(false);
    app.config_path = None;
    app.update.msi_override = Some(true);
    app.update.key_override = Some(sign::public_key_text(&key.verifying_key()));
    app.update.source_override = Some(Source { kind: SourceKind::Gitea, base_url: mock.base.clone(), repo: "team/remotejoe".into() });
    assert_eq!(app.update_status()["msi"], true);
    assert!(edit(&mut app, json!({"op": "update_apply", "confirm": true}))["error"].is_string(), "ohne geprüfte MSI nichts zu installieren");
    let path = rj_update::msi::download_dir().join(&msi);
    let check = |app: &mut App| {
        edit(app, json!({"op": "update_check"}));
        wait_result(app, "update_check")
    };
    let download = |app: &mut App| {
        assert_eq!(edit(app, json!({"op": "update_install"}))["state"], "downloading");
        wait_result(app, "update_install")
    };

    // Unsigniert, aber in der Liste: Authenticode lehnt ab.
    set(b"keine echte MSI", b"keine echte MSI");
    mock.set_release("v9.0.0", "Neu", &[&msi, SUMS, SIG]);
    let r = check(&mut app);
    assert_eq!((r["state"].as_str(), r["msi"].as_bool()), (Some("newer"), Some(true)), "{r}");
    assert_eq!(r["found"]["archive"], msi);
    let r = download(&mut app);
    assert_eq!(r["state"], "error", "{r}");
    assert!(r["error"].as_str().unwrap().contains("Authenticode"), "{r}");
    assert!(!path.exists() && app.update_status()["ready"].is_null());

    // Prüfsumme passt nicht zur Liste.
    set(b"veraendert", b"keine echte MSI");
    let r = download(&mut app);
    assert!(r["error"].as_str().unwrap().contains("Prüfsumme"), "{r}");
    assert!(!path.exists());

    // Veröffentlichung ohne MSI: Release-Seite.
    mock.set_release("v9.0.0", "Neu", &[SUMS, SIG]);
    let r = check(&mut app);
    assert!(r["no_msi"].is_string() && r["page"].is_string(), "{r}");
    assert_eq!(edit(&mut app, json!({"op": "update_install"}))["state"], "page");

    // Guter Weg mit einer echten, signierten MSI (optional).
    let Some(signed) = std::env::var_os("RJ_TEST_SIGNED_MSI").filter(|p| std::path::Path::new(p).is_file()) else {
        eprintln!("ÜBERSPRUNGEN (guter Weg): RJ_TEST_SIGNED_MSI nicht gesetzt");
        return;
    };
    let data = std::fs::read(signed).unwrap();
    set(&data, &data);
    mock.set_release("v9.0.0", "Neu", &[&msi, SUMS, SIG]);
    check(&mut app);
    let r = download(&mut app);
    assert_eq!(r["state"], "ready", "{r}");
    assert_eq!(app.update_status()["ready"], "9.0.0");
    assert_eq!(app.update_status()["progress"].as_u64(), Some(data.len() as u64));
    assert_eq!(std::fs::read(&path).unwrap(), data);
    // Offene Sitzung: erst Rückfrage; installiert wird hier nicht (das beendet remotejoe).
    app.sessions.open_demo_frame("rdp-001", 8, 8, 1);
    assert_eq!(edit(&mut app, json!({"op": "update_apply"}))["state"], "confirm");
    let _ = std::fs::remove_file(&path);
}

#[cfg(not(windows))]
#[test]
fn check_and_install_from_the_fixed_source() {
    let dir = tempfile::tempdir().unwrap();
    // Paket 9.0.0 mit eigenem Testschlüssel.
    let key = sign::generate().unwrap();
    let prog = tempfile::tempdir().unwrap();
    std::fs::write(prog.path().join(install::main_program()), b"neu").unwrap();
    let data = archive::pack_tar_zst(prog.path()).unwrap();
    let name = asset_name("9.0.0", &rj_update::platform());
    let sums = sign::sums_text(&[(name.clone(), sign::sha256_hex(&data))]);
    let mock = GiteaMock::start("team/remotejoe");
    {
        let mut st = mock.state.lock().unwrap();
        st.files.insert(name.clone(), data);
        st.files.insert(SUMS.into(), sums.clone().into_bytes());
        st.files.insert(SIG.into(), sign::sign(&key, sums.as_bytes()).into_bytes());
    }
    mock.set_release("v9.0.0", "Änderungen\n- Update-Tab", &[&name, SUMS, SIG]);

    let mut app = App::new(false);
    app.config_path = None;
    let target = dir.path().join("remotejoe");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join(install::main_program()), b"alt").unwrap();
    app.update.dir_override = Some(target.clone());
    app.update.key_override = Some(sign::public_key_text(&key.verifying_key()));

    // Die Quelle ist fest GitHub; im Status steht keine Quelle zur Auswahl mehr.
    let st = app.update_status();
    assert!(st.get("source").is_none() && st.get("repo").is_none() && st.get("token_entry").is_none(), "{st}");
    assert_eq!(rj_ui::update::github_source().repo, "joeMJ/remotejoe");
    assert_eq!(rj_ui::update::github_source().kind, SourceKind::Github);
    // Nur die Prüfung beim Start ist einstellbar.
    let r = edit(&mut app, json!({"op": "update_settings", "check_on_start": true, "source": "gitea", "repo": "fremd/repo"}));
    assert_eq!(r["check_on_start"], true, "{r}");
    assert!(app.config.update.check_on_start);
    // Proxy: Rechner:Port wird geprüft; „auto“ wird als leer (Vorgabe) gespeichert.
    assert!(edit(&mut app, json!({"op": "update_settings", "proxy": "proxy:keinport"}))["error"].is_string());
    assert_eq!(edit(&mut app, json!({"op": "update_settings", "proxy": "proxy.example:3128"}))["proxy"], "proxy.example:3128");
    assert_eq!(edit(&mut app, json!({"op": "update_settings", "proxy": "none"}))["proxy"], "none");
    assert_eq!(edit(&mut app, json!({"op": "update_settings", "proxy": "auto"}))["proxy"], "");
    // Die Vorführquelle gibt es nur im Vorführbetrieb.
    assert!(edit(&mut app, json!({"op": "update_demo_source", "base_url": mock.base}))["error"].is_string());
    app.update.source_override = Some(Source { kind: SourceKind::Gitea, base_url: mock.base.clone(), repo: "team/remotejoe".into() });

    edit(&mut app, json!({"op": "update_check"}));
    let r = wait_result(&mut app, "update_check");
    assert_eq!(r["state"], "newer", "{r}");
    assert_eq!(r["found"]["version"], "9.0.0");
    assert_eq!(r["found"]["notes"], "Änderungen\n- Update-Tab");
    assert!(mock.state.lock().unwrap().requests.iter().any(|(p, h)| p.ends_with("/releases/latest") && !h.iter().any(|(k, _)| k == "authorization")), "keine Zugangsdaten an die Quelle");

    // Offene Sitzung: erst Bestätigung.
    app.sessions.open_demo_frame("rdp-001", 8, 8, 1);
    let r = edit(&mut app, json!({"op": "update_install"}));
    assert_eq!(r["state"], "confirm", "{r}");
    assert_eq!(std::fs::read(target.join(install::main_program())).unwrap(), b"alt");
    edit(&mut app, json!({"op": "update_install", "confirm": true}));
    let r = wait_result(&mut app, "update_install");
    assert_eq!(r["state"], "installed", "{r}");
    assert_eq!(std::fs::read(target.join(install::main_program())).unwrap(), b"neu");
    assert_eq!(std::fs::read(dir.path().join("remotejoe.alt").join(install::main_program())).unwrap(), b"alt");
}
