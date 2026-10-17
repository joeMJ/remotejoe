//! Hyper-V über die pwsh-Attrappe (kein echter Hyper-V-Host).

use rj_core::{PsrpTransport, Secret, WinrmAuth};
use rj_psrp::hyperv::{self, HvAction};
use rj_psrp::{PsError, PsSession, PsTarget};

#[test]
fn list_and_actions_via_fake_pwsh() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("vms.json");
    std::fs::write(
        &out,
        r#"[{"Id":"6f1c0000-0000-4000-8000-000000000002","Name":"zweite \"VM\" ä","State":"Off","CpuUsage":0,"MemoryAssigned":0,"UptimeSeconds":0,"Addresses":[]},
{"Id":"6f1c0000-0000-4000-8000-000000000001","Name":"Erste","State":"Running","CpuUsage":12,"MemoryAssigned":2147483648,"UptimeSeconds":3600,"Addresses":["fe80::1","192.0.2.20"]}]"#,
    )
    .unwrap();
    std::env::set_var("RJ_PWSH", format!("{}/tests/fake-pwsh", env!("CARGO_MANIFEST_DIR")));
    std::env::set_var("RJ_FAKE_OUTPUT", &out);
    let t = PsTarget {
        host: "hv.example.test".into(),
        username: "admin".into(),
        domain: None,
        transport: PsrpTransport::Winrm { https: true, port: None, auth: WinrmAuth::Ntlm, verify_certificate: true },
    };
    let mut s = PsSession::start(&t, &Secret::from_str_exact("x")).unwrap();
    let vms = hyperv::list(&mut s).unwrap();
    assert_eq!(vms.len(), 2);
    assert_eq!(vms[0].name, "Erste");
    assert!(vms[0].running());
    assert_eq!(vms[0].console_host(), Some("192.0.2.20"));
    assert_eq!(vms[1].name, "zweite \"VM\" ä");
    assert_eq!(vms[1].console_host(), None);

    hyperv::action(&mut s, &vms[1].id, HvAction::Start).unwrap();
    hyperv::action(&mut s, &vms[0].id, HvAction::Shutdown).unwrap();
    let err = hyperv::action(&mut s, "unbekannt", HvAction::Restart).unwrap_err();
    assert!(matches!(err, PsError::Session(m) if m.contains("nicht gefunden")));
    let cmds = std::fs::read_to_string(dir.path().join("vms.json.befehle")).unwrap();
    let lines: Vec<&str> = cmds.lines().collect();
    assert_eq!(lines[0], "Get-VM -Id '6f1c0000-0000-4000-8000-000000000002' | Start-VM -ErrorAction Stop");
    assert_eq!(lines[1], "Get-VM -Id '6f1c0000-0000-4000-8000-000000000001' | Stop-VM -Force -ErrorAction Stop");
}
