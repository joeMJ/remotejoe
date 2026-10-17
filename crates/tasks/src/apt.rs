//! Aktualisierungen für Debian/Ubuntu über SSH mit `sudo`.
//!
//! Trockenlauf (Standard): `apt-get update`, Liste der Aktualisierungen,
//! „Neustart nötig“. Ausführen: zusätzlich `apt-get upgrade` bzw.
//! `full-upgrade`. Die Kennwortabfrage von `sudo` beantwortet die
//! Makro-Engine mit dem Kennwort aus dem Zugangsdatensatz.

use serde::Serialize;

use crate::macros::{Branch, CredentialSource, Macro, MacroError, MacroIo, Pattern, Step};

/// Eine verfügbare Aktualisierung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AptUpdate {
    pub package: String,
    pub suite: String,
    pub new_version: String,
    pub arch: String,
    pub old_version: Option<String>,
}

/// Ergebnis einer apt-Aufgabe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AptReport {
    pub updates: Vec<AptUpdate>,
    pub reboot_required: bool,
    pub reboot_packages: Vec<String>,
    pub upgraded: bool,
}

/// Liest die Ausgabe von `LC_ALL=C apt list --upgradable`.
pub fn parse_upgradable(text: &str) -> Vec<AptUpdate> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with("Listing") || l.starts_with("WARNING") {
                return None;
            }
            let (name_suite, rest) = l.split_once(' ')?;
            let (package, suite) = name_suite.split_once('/')?;
            let mut parts = rest.split_whitespace();
            let new_version = parts.next()?.to_string();
            let arch = parts.next()?.to_string();
            let old_version = rest.split_once("from: ").map(|(_, v)| v.trim_end_matches(']').trim().to_string());
            Some(AptUpdate { package: package.into(), suite: suite.into(), new_version, arch, old_version })
        })
        .collect()
}

const BEGIN: &str = "RJ-AUSGABE-ANFANG";
const END: &str = "RJ-AUSGABE-ENDE";

/// Makro: Befehl mit `sudo` ausführen; Kennwortabfrage beantworten;
/// Rückgabewert über eine Endmarke erkennen.
fn sudo_step(cmd: &str, tag: &str, timeout_ms: u64) -> Vec<Step> {
    // Marken zerteilt, damit das Echo der Befehlszeile nicht passt.
    let send = format!(
        "LC_ALL=C sudo -p 'RJ-SUDO-''PROMPT: ' sh -c '{cmd}' ; echo RJ-{tag}-''RC=$?{{key:enter}}"
    );
    let done = Pattern::Regex(format!("RJ-{tag}-RC=(\\d+)"));
    vec![
        Step::Send { text: send },
        Step::ExpectAny {
            timeout_ms,
            branches: vec![
                Branch {
                    pattern: Pattern::Text("RJ-SUDO-PROMPT: ".into()),
                    steps: vec![
                        Step::Send { text: "{cred:password}{key:enter}".into() },
                        Step::ExpectAny {
                            timeout_ms,
                            branches: vec![
                                Branch { pattern: Pattern::Text("RJ-SUDO-PROMPT: ".into()), steps: vec![Step::Send { text: "{key:ctrl-c}".into() }], fail: true },
                                Branch { pattern: Pattern::Regex(format!("RJ-{tag}-RC=0")), steps: vec![], fail: false },
                                Branch { pattern: done.clone(), steps: vec![], fail: true },
                            ],
                        },
                    ],
                    fail: false,
                },
                Branch { pattern: Pattern::Regex(format!("RJ-{tag}-RC=0")), steps: vec![], fail: false },
                Branch { pattern: done, steps: vec![], fail: true },
            ],
        },
    ]
}

/// Sammelt Ausgabe zwischen Marken (für Listen ohne Root-Rechte).
struct Capture<'a> {
    inner: &'a mut dyn MacroIo,
    seen: Vec<u8>,
}

impl MacroIo for Capture<'_> {
    fn send(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.inner.send(data)
    }
    fn recv(&mut self, t: std::time::Duration) -> std::io::Result<Option<Vec<u8>>> {
        let r = self.inner.recv(t)?;
        if let Some(d) = &r {
            self.seen.extend_from_slice(d);
        }
        Ok(r)
    }
}

/// Fortschritt für die Patchday-Ansicht.
pub trait Progress {
    fn phase(&mut self, phase: &str);
    fn log(&mut self, line: &str);
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AptError {
    #[error("{0}")]
    Macro(#[from] MacroError),
    #[error("Ausgabe nicht lesbar")]
    Output,
}

/// Führt die apt-Aufgabe auf einer Shell aus (PTY über SSH).
pub fn run(io: &mut dyn MacroIo, creds: &dyn CredentialSource, execute: bool, full: bool, progress: &mut dyn Progress) -> Result<AptReport, AptError> {
    let mut cap = Capture { inner: io, seen: Vec::new() };
    progress.phase("searching");
    let update = Macro { steps: sudo_step("DEBIAN_FRONTEND=noninteractive apt-get update -q", "UPD", 600_000), on_error: vec![] };
    let r = update.run(&mut cap, creds);
    for l in &r.log {
        progress.log(l);
    }
    r.result?;

    // Liste ohne Root-Rechte, zwischen Marken.
    cap.seen.clear();
    let list = Macro {
        steps: vec![
            Step::Send { text: format!("echo {}''{}; LC_ALL=C apt list --upgradable 2>/dev/null; echo {}''{}{{key:enter}}", &BEGIN[..3], &BEGIN[3..], &END[..3], &END[3..]) },
            Step::Wait { pattern: Pattern::Text(END.into()), errors: vec![], timeout_ms: 120_000 },
        ],
        on_error: vec![],
    };
    list.run(&mut cap, creds).result?;
    let text = String::from_utf8_lossy(&cap.seen).replace('\r', "");
    let text = crate::ansi::strip(&text);
    let body = text.rsplit_once(BEGIN).map(|(_, b)| b).and_then(|b| b.split_once(END).map(|(a, _)| a)).ok_or(AptError::Output)?;
    let updates = parse_upgradable(body);
    progress.log(&format!("{} Aktualisierungen verfügbar", updates.len()));

    let mut upgraded = false;
    if execute && !updates.is_empty() {
        progress.phase("installing");
        let cmd = if full { "full-upgrade" } else { "upgrade" };
        let up = Macro { steps: sudo_step(&format!("DEBIAN_FRONTEND=noninteractive apt-get -y -q {cmd}"), "UPG", 3_600_000), on_error: vec![] };
        let r = up.run(&mut cap, creds);
        for l in &r.log {
            progress.log(l);
        }
        r.result?;
        upgraded = true;
    }

    cap.seen.clear();
    let reboot = Macro {
        steps: vec![
            Step::Send { text: format!("echo {}''{}; cat /var/run/reboot-required.pkgs 2>/dev/null; test -f /var/run/reboot-required && echo RJ-NEU''START; echo {}''{}{{key:enter}}", &BEGIN[..3], &BEGIN[3..], &END[..3], &END[3..]) },
            Step::Wait { pattern: Pattern::Text(END.into()), errors: vec![], timeout_ms: 30_000 },
        ],
        on_error: vec![],
    };
    reboot.run(&mut cap, creds).result?;
    let text = crate::ansi::strip(&String::from_utf8_lossy(&cap.seen).replace('\r', ""));
    let body = text.rsplit_once(BEGIN).map(|(_, b)| b).and_then(|b| b.split_once(END).map(|(a, _)| a)).unwrap_or("");
    let reboot_required = body.lines().any(|l| l.trim() == "RJ-NEUSTART");
    let reboot_packages = body.lines().map(str::trim).filter(|l| !l.is_empty() && *l != "RJ-NEUSTART" && !l.starts_with("echo")).map(str::to_string).collect();
    Ok(AptReport { updates, reboot_required, reboot_packages, upgraded })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORDED: &str = "Listing...\n\
        openssl/noble-updates,noble-security 3.0.13-0ubuntu3.5 amd64 [upgradable from: 3.0.13-0ubuntu3.4]\n\
        libssl3t64/noble-updates 3.0.13-0ubuntu3.5 amd64 [upgradable from: 3.0.13-0ubuntu3.4]\n\
        tzdata/noble-updates 2025b-0ubuntu0.24.04.1 all [upgradable from: 2024a-3ubuntu1.1]\n";

    #[test]
    fn parses_recorded_listing() {
        let u = parse_upgradable(RECORDED);
        assert_eq!(u.len(), 3);
        assert_eq!(u[0].package, "openssl");
        assert_eq!(u[0].suite, "noble-updates,noble-security");
        assert_eq!(u[0].new_version, "3.0.13-0ubuntu3.5");
        assert_eq!(u[0].old_version.as_deref(), Some("3.0.13-0ubuntu3.4"));
        assert_eq!(u[2].arch, "all");
        assert!(parse_upgradable("Listing...\n").is_empty());
    }

    /// Attrappe einer Shell mit aufgezeichneter apt-Ausgabe.
    struct FakeShell {
        out: std::collections::VecDeque<Vec<u8>>,
        sent: String,
        reboot: bool,
    }

    impl MacroIo for FakeShell {
        fn send(&mut self, data: &[u8]) -> std::io::Result<()> {
            let s = String::from_utf8_lossy(data).to_string();
            self.sent.push_str(&s);
            if s.contains("apt-get update") || s.contains("apt-get -y") {
                self.out.push_back(b"RJ-SUDO-PROMPT: ".to_vec());
            } else if s.contains("Kennwort-") {
                let tag = if self.sent.contains("apt-get -y") { "UPG" } else { "UPD" };
                self.out.push_back(format!("\r\nOK\r\nRJ-{tag}-RC=0\r\n$ ").into_bytes());
            } else if s.contains("apt list") {
                self.out.push_back(format!("RJ-AUSGABE-ANFANG\r\n{}RJ-AUSGABE-ENDE\r\n", RECORDED.replace('\n', "\r\n")).into_bytes());
            } else if s.contains("reboot-required") {
                let body = if self.reboot { "linux-image-generic\r\nRJ-NEUSTART\r\n" } else { "" };
                self.out.push_back(format!("RJ-AUSGABE-ANFANG\r\n{body}RJ-AUSGABE-ENDE\r\n").into_bytes());
            }
            Ok(())
        }
        fn recv(&mut self, _t: std::time::Duration) -> std::io::Result<Option<Vec<u8>>> {
            Ok(self.out.pop_front())
        }
    }

    struct Log(Vec<String>);
    impl Progress for Log {
        fn phase(&mut self, p: &str) {
            self.0.push(format!("Phase {p}"));
        }
        fn log(&mut self, l: &str) {
            self.0.push(l.to_string());
        }
    }

    #[test]
    fn dry_run_and_execute_against_fake_shell() {
        let creds = |n: &str| (n == "password").then(|| rj_core::Secret::from_str_exact("Kennwort-ä"));
        let mut sh = FakeShell { out: Default::default(), sent: String::new(), reboot: true };
        let mut log = Log(Vec::new());
        let r = run(&mut sh, &creds, false, false, &mut log).unwrap();
        assert_eq!(r.updates.len(), 3);
        assert!(r.reboot_required);
        assert_eq!(r.reboot_packages, vec!["linux-image-generic"]);
        assert!(!r.upgraded);
        assert!(!sh.sent.contains("apt-get -y"), "Trockenlauf installiert nichts");
        assert!(!log.0.join("\n").contains("Kennwort-ä"), "Kennwort nie im Protokoll");
        let mut sh = FakeShell { out: Default::default(), sent: String::new(), reboot: false };
        let r = run(&mut sh, &creds, true, true, &mut log).unwrap();
        assert!(r.upgraded);
        assert!(sh.sent.contains("apt-get -y -q full-upgrade"));
        assert!(!r.reboot_required);
    }
}
