//! `known_hosts` ist mit OpenSSH austauschbar: eine von `ssh-keyscan`
//! erzeugte und mit `ssh-keygen -H` gehashte Datei wird gelesen, und
//! `ssh-keygen -F` findet die von remotejoe geschriebenen Einträge.

use rj_ssh::known_hosts::{HostKeyStatus, KnownHosts};
use russh::keys::PublicKey;

fn have(tool: &str) -> bool {
    std::process::Command::new(tool).arg("-?").output().is_ok()
}

#[test]
fn file_is_interchangeable_with_openssh() {
    if !have("ssh-keyscan") || !have("ssh-keygen") {
        eprintln!("ÜBERSPRUNGEN: ssh-keyscan oder ssh-keygen fehlt");
        return;
    }
    let sshd = match rj_testkit::sshd::Sshd::start() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {}", e.0);
            return;
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("known_hosts");
    // 1. Von OpenSSH erzeugt (Port ≠ 22, also „[127.0.0.1]:port“).
    let scan = std::process::Command::new("ssh-keyscan").args(["-p", &sshd.port.to_string(), "-t", "ed25519", "127.0.0.1"]).output().unwrap();
    let text = String::from_utf8(scan.stdout).unwrap();
    assert!(text.starts_with(&format!("[127.0.0.1]:{} ", sshd.port)), "{text}");
    std::fs::write(&file, &text).unwrap();
    let key = PublicKey::from_openssh(text.lines().find(|l| !l.starts_with('#')).unwrap().split_once(' ').unwrap().1).unwrap();
    let kh = KnownHosts::new(&file);
    assert_eq!(kh.check("127.0.0.1", sshd.port, &key), HostKeyStatus::Known);
    assert_eq!(kh.check("127.0.0.1", 22, &key), HostKeyStatus::Unknown);

    // 2. Von OpenSSH gehasht.
    let st = std::process::Command::new("ssh-keygen").args(["-H", "-f"]).arg(&file).output().unwrap();
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    let hashed = std::fs::read_to_string(&file).unwrap();
    assert!(hashed.lines().any(|l| l.starts_with("|1|")), "{hashed}");
    assert_eq!(kh.check("127.0.0.1", sshd.port, &key), HostKeyStatus::Known);

    // 3. Von remotejoe geschrieben, von OpenSSH gefunden.
    let ours = dir.path().join("ours");
    let kh2 = KnownHosts::new(&ours);
    kh2.remember("127.0.0.1", sshd.port, &key).unwrap();
    kh2.remember("srv.example.test", 22, &key).unwrap();
    for host in [format!("[127.0.0.1]:{}", sshd.port), "srv.example.test".to_string()] {
        let f = std::process::Command::new("ssh-keygen").args(["-F", &host, "-f"]).arg(&ours).output().unwrap();
        assert!(f.status.success() && !f.stdout.is_empty(), "ssh-keygen -F {host}: {}", String::from_utf8_lossy(&f.stderr));
    }
}
