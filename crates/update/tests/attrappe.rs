//! Ende-zu-Ende gegen eine Gitea-Attrappe (127.0.0.1): Prüfen, Laden,
//! Signatur und Prüfsumme, Installation in ein Testverzeichnis; manipulierte
//! oder fehlende Dateien führen zum Abbruch ohne Änderung.

use std::fs;

use rj_testkit::gitea::GiteaMock;
use rj_update::release::{self, Check, Source, SourceKind};
use rj_update::sign::{self, SIG, SUMS};
use rj_update::{archive, asset_name, install, Limits, UpdateError, Version};

const TOKEN: &str = "prüf-Schlüssel \"'\\ 🚀";

struct Fixture {
    mock: GiteaMock,
    src: Source,
    key: ed25519_dalek::SigningKey,
    archive: Vec<u8>,
    name: String,
}

fn fixture(version: &str) -> Fixture {
    let mock = GiteaMock::start("team/remotejoe");
    let key = sign::generate().unwrap();
    let prog = tempfile::tempdir().unwrap();
    fs::write(prog.path().join(install::main_program()), format!("remotejoe {version}")).unwrap();
    fs::write(prog.path().join("rj-worker-web"), b"worker").unwrap();
    let archive = archive::pack_tar_zst(prog.path()).unwrap();
    let name = asset_name(version, &rj_update::platform());
    let sums = sign::sums_text(&[(name.clone(), sign::sha256_hex(&archive))]);
    {
        let mut st = mock.state.lock().unwrap();
        st.token = Some(TOKEN.into());
        st.files.insert(name.clone(), archive.clone());
        st.files.insert(SUMS.into(), sums.clone().into_bytes());
        st.files.insert(SIG.into(), sign::sign(&key, sums.as_bytes()).into_bytes());
    }
    mock.set_release(&format!("v{version}"), "Neu:\n- Web im Tab", &[&name, SUMS, SIG]);
    let src = Source { kind: SourceKind::Gitea, base_url: mock.base.clone(), repo: "team/remotejoe".into() };
    Fixture { mock, src, key, archive, name }
}

fn found(f: &Fixture) -> release::Found {
    match release::check(&f.src, Some(TOKEN), &Version::parse("0.1.0").unwrap(), &rj_update::platform(), None).unwrap() {
        Check::Newer(x) => *x,
        other => panic!("{other:?}"),
    }
}

fn program_dir() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("remotejoe");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join(install::main_program()), b"remotejoe 0.1.0").unwrap();
    (base, dir)
}

fn try_install(f: &Fixture, dir: &std::path::Path) -> Result<std::path::PathBuf, UpdateError> {
    let fd = found(f);
    let data = release::download_verified(&f.src, Some(TOKEN), &fd, &f.key.verifying_key(), Limits::default(), None)?;
    install::install(&fd.archive.name, &data, dir)
}

fn unchanged(dir: &std::path::Path) {
    assert_eq!(fs::read(dir.join(install::main_program())).unwrap(), b"remotejoe 0.1.0");
    assert!(!dir.with_file_name("remotejoe.alt").exists());
    assert!(!dir.with_file_name("remotejoe.neu").exists());
}

#[test]
fn valid_package_installs_with_token() {
    let f = fixture("0.2.0");
    // Ohne Zugriffsschlüssel lehnt die Attrappe ab (Status ohne Inhalt der Antwort).
    let e = release::check(&f.src, None, &Version::parse("0.1.0").unwrap(), &rj_update::platform(), None).unwrap_err();
    assert!(matches!(e, UpdateError::Status { status: 401, .. }), "{e}");
    assert!(!e.to_string().contains(TOKEN));
    let fd = found(&f);
    assert_eq!(fd.version.to_string(), "0.2.0");
    assert_eq!(fd.release.notes(), "Neu:\n- Web im Tab");
    let (_base, dir) = program_dir();
    let old = try_install(&f, &dir).unwrap();
    assert_eq!(fs::read(dir.join(install::main_program())).unwrap(), b"remotejoe 0.2.0");
    assert_eq!(fs::read(dir.join("rj-worker-web")).unwrap(), b"worker");
    assert_eq!(fs::read(old.join(install::main_program())).unwrap(), b"remotejoe 0.1.0");
    // Zugriffsschlüssel wurde byte-genau als Kopfzeile gesendet.
    let st = f.mock.state.lock().unwrap();
    assert!(st.requests.iter().any(|(p, h)| p.ends_with("/releases/latest") && h.contains(&("authorization".into(), format!("token {TOKEN}")))));
    assert!(st.requests.iter().any(|(p, _)| p == &format!("/dl/{}", f.name)));
}

#[test]
fn older_or_same_version_is_not_offered() {
    let f = fixture("0.1.0");
    let r = release::check(&f.src, Some(TOKEN), &Version::parse("0.1.0").unwrap(), &rj_update::platform(), None).unwrap();
    assert_eq!(r, Check::UpToDate(Version::parse("0.1.0").unwrap()));
    let r = release::check(&f.src, Some(TOKEN), &Version::parse("0.2.0-rc.1").unwrap(), &rj_update::platform(), None).unwrap();
    assert!(matches!(r, Check::UpToDate(_)), "kein Herabstufen");
}

#[test]
fn manipulated_archive_is_rejected() {
    let f = fixture("0.2.0");
    let mut bad = f.archive.clone();
    let n = bad.len() / 2;
    bad[n] ^= 0x01;
    f.mock.state.lock().unwrap().files.insert(f.name.clone(), bad);
    let (_base, dir) = program_dir();
    assert!(matches!(try_install(&f, &dir), Err(UpdateError::BadChecksum)));
    unchanged(&dir);
}

#[test]
fn manipulated_sums_are_rejected() {
    let f = fixture("0.2.0");
    // Prüfsumme passend zu einem anderen Archiv, Signatur bleibt die alte.
    let other = sign::sums_text(&[(f.name.clone(), sign::sha256_hex(b"anderes Archiv"))]);
    f.mock.state.lock().unwrap().files.insert(SUMS.into(), other.into_bytes());
    let (_base, dir) = program_dir();
    assert!(matches!(try_install(&f, &dir), Err(UpdateError::BadSignature)));
    unchanged(&dir);
}

#[test]
fn wrong_signature_is_rejected() {
    let f = fixture("0.2.0");
    let stranger = sign::generate().unwrap();
    let sums = f.mock.state.lock().unwrap().files[SUMS].clone();
    f.mock.state.lock().unwrap().files.insert(SIG.into(), sign::sign(&stranger, &sums).into_bytes());
    let (_base, dir) = program_dir();
    assert!(matches!(try_install(&f, &dir), Err(UpdateError::BadSignature)));
    unchanged(&dir);
}

#[test]
fn missing_signature_is_rejected() {
    let f = fixture("0.2.0");
    // Nicht in der Veröffentlichung aufgeführt.
    f.mock.set_release("v0.2.0", "", &[&f.name, SUMS]);
    let e = release::check(&f.src, Some(TOKEN), &Version::parse("0.1.0").unwrap(), &rj_update::platform(), None).unwrap_err();
    assert!(matches!(e, UpdateError::MissingFile(ref n) if n == SIG), "{e}");
    // Aufgeführt, aber nicht abrufbar.
    f.mock.set_release("v0.2.0", "", &[&f.name, SUMS, SIG]);
    f.mock.state.lock().unwrap().files.remove(SIG);
    let (_base, dir) = program_dir();
    assert!(matches!(try_install(&f, &dir), Err(UpdateError::Status { status: 404, .. })));
    unchanged(&dir);
    // Leere Signaturdatei.
    f.mock.state.lock().unwrap().files.insert(SIG.into(), Vec::new());
    assert!(matches!(try_install(&f, &dir), Err(UpdateError::BadSignature)));
    unchanged(&dir);
}

#[test]
fn size_limit_and_cancel() {
    let f = fixture("0.2.0");
    let fd = found(&f);
    let tiny = Limits { max_bytes: 16, ..Limits::default() };
    assert!(matches!(release::download_verified(&f.src, Some(TOKEN), &fd, &f.key.verifying_key(), tiny, None), Err(UpdateError::TooLarge(16))));
    let stop = std::sync::atomic::AtomicBool::new(true);
    assert!(matches!(release::download_verified(&f.src, Some(TOKEN), &fd, &f.key.verifying_key(), Limits::default(), Some(&stop)), Err(UpdateError::Cancelled)));
}
