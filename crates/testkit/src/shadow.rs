//! FreeRDP-Shadow-Server auf einer virtuellen Anzeige (Xvfb), für NLA-Tests.

use std::fs;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::{free_port, have_program, wait_for_port, Skip};

pub struct Shadow {
    pub port: u16,
    pub dir: tempfile::TempDir,
    xvfb: Option<Child>,
    server: Option<Child>,
}

impl Shadow {
    /// `sam_line`: Eintrag der SAM-Datei (`benutzer:domäne:LM:NT:::`).
    pub fn start(sam_line: &str) -> Result<Shadow, Skip> {
        for p in ["/usr/bin/freerdp-shadow-cli3", "/usr/bin/Xvfb"] {
            if !have_program(p) {
                return Err(Skip(format!("{p} fehlt")));
            }
        }
        let dir = tempfile::Builder::new().prefix("rj-shadow").tempdir().map_err(|e| Skip(e.to_string()))?;
        let sam = dir.path().join("SAM");
        fs::write(&sam, format!("{sam_line}\n")).map_err(|e| Skip(e.to_string()))?;
        let display = format!(":{}", 50 + (std::process::id() % 40));
        let xvfb = Command::new("/usr/bin/Xvfb")
            .args([display.as_str(), "-screen", "0", "800x600x24", "-nolisten", "tcp"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Skip(format!("Xvfb: {e}")))?;
        std::thread::sleep(Duration::from_millis(500));
        let port = free_port();
        let server = Command::new("/usr/bin/freerdp-shadow-cli3")
            .arg(format!("/port:{port}"))
            .arg("/bind-address:127.0.0.1")
            .arg(format!("/sam-file:{}", sam.display()))
            .arg("/sec:nla")
            .env("DISPLAY", &display)
            .env("WLOG_LEVEL", "ERROR")
            .current_dir(dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Skip(format!("shadow: {e}")))?;
        let mut s = Shadow { port, dir, xvfb: Some(xvfb), server: Some(server) };
        if !wait_for_port(port, Duration::from_secs(10)) {
            s.stop();
            return Err(Skip("Shadow-Server nimmt keine Verbindungen an".into()));
        }
        Ok(s)
    }

    pub fn stop(&mut self) {
        for c in [self.server.take(), self.xvfb.take()].into_iter().flatten() {
            let mut c = c;
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Shadow {
    fn drop(&mut self) {
        self.stop();
    }
}
