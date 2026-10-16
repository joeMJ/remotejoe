//! TigerVNC-Server (Xvnc) für VNC-Tests. In der Anzeige läuft ein `xterm`,
//! das Tastatureingaben in eine Datei schreibt.

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use des::cipher::{BlockEncrypt, KeyInit};

use crate::{free_port, have_program, run, wait_for_port, Skip};

/// VNC-Kennwort (8 Bytes, mit Umlauten byte-genau zu prüfen).
pub const PASSWORD: &str = "Prüf-ä";

/// Sicherheitsverfahren des Servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VncSecurity {
    VncAuth,
    X509Vnc,
}

pub struct Xvnc {
    pub port: u16,
    pub display: String,
    pub dir: tempfile::TempDir,
    server: Option<Child>,
    xterm: Option<Child>,
}

/// Verschleierte Kennwortdatei wie `vncpasswd` (DES mit festem Schlüssel).
fn obfuscated(password: &str) -> [u8; 8] {
    const KEY: [u8; 8] = [23, 82, 107, 6, 35, 78, 88, 7];
    let key: Vec<u8> = KEY.iter().map(|b| b.reverse_bits()).collect();
    let mut block = [0u8; 8];
    for (i, b) in password.as_bytes().iter().take(8).enumerate() {
        block[i] = *b;
    }
    let c = des::Des::new_from_slice(&key).expect("8 Bytes");
    c.encrypt_block((&mut block).into());
    block
}

impl Xvnc {
    pub fn start(security: VncSecurity) -> Result<Xvnc, Skip> {
        for p in ["/usr/bin/Xvnc", "/usr/bin/xterm", "/usr/bin/xclip"] {
            if !have_program(p) {
                return Err(Skip(format!("{p} fehlt")));
            }
        }
        let dir = tempfile::Builder::new().prefix("rj-xvnc").tempdir().map_err(|e| Skip(e.to_string()))?;
        let pwfile = dir.path().join("passwd");
        fs::write(&pwfile, obfuscated(PASSWORD)).map_err(|e| Skip(e.to_string()))?;
        let port = free_port();
        let display = format!(":{}", 20 + (port % 60));
        let mut args = vec![
            display.clone(),
            "-rfbport".into(),
            port.to_string(),
            "-interface".into(),
            "127.0.0.1".into(),
            "-geometry".into(),
            "1024x768".into(),
            "-depth".into(),
            "24".into(),
            "-PasswordFile".into(),
            pwfile.display().to_string(),
            "-nolisten".into(),
            "tcp".into(),
            "-SendPrimary=0".into(),
        ];
        match security {
            VncSecurity::VncAuth => args.extend(["-SecurityTypes".into(), "VncAuth".into()]),
            VncSecurity::X509Vnc => {
                let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
                run(
                    "openssl",
                    &["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-subj", "/CN=vnc.example.test",
                      "-keyout", key.to_str().unwrap_or_default(), "-out", cert.to_str().unwrap_or_default()],
                    None,
                )?;
                args.extend([
                    "-SecurityTypes".into(),
                    "X509Vnc".into(),
                    "-X509Cert".into(),
                    cert.display().to_string(),
                    "-X509Key".into(),
                    key.display().to_string(),
                ]);
            }
        }
        let server = Command::new("/usr/bin/Xvnc")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Skip(format!("Xvnc: {e}")))?;
        let mut x = Xvnc { port, display, dir, server: Some(server), xterm: None };
        if !wait_for_port(port, Duration::from_secs(10)) {
            x.stop();
            return Err(Skip("Xvnc startet nicht".into()));
        }
        let keys = x.keys_file();
        let xterm = Command::new("/usr/bin/xterm")
            .args(["-geometry", "100x30+0+0", "-e", "sh", "-c", &format!("stty -echo; cat > '{}'", keys.display())])
            .env("DISPLAY", &x.display)
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Skip(format!("xterm: {e}")))?;
        x.xterm = Some(xterm);
        std::thread::sleep(Duration::from_millis(800));
        Ok(x)
    }

    fn keys_file(&self) -> PathBuf {
        self.dir.path().join("keys.txt")
    }

    pub fn typed_text(&self) -> String {
        fs::read_to_string(self.keys_file()).unwrap_or_default()
    }

    /// Befehl auf der Anzeige ausführen.
    pub fn x_run(&self, cmd: &str, stdin: Option<&[u8]>) -> Result<String, Skip> {
        run("env", &[&format!("DISPLAY={}", self.display), "LANG=C.UTF-8", "sh", "-c", cmd], stdin)
    }

    pub fn kill_server(&mut self) {
        if let Some(mut c) = self.server.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut c) = self.xterm.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.kill_server();
    }
}

impl Drop for Xvnc {
    fn drop(&mut self) {
        self.stop();
    }
}
