//! Net-SNMP-Agent für Tests (Hochport an 127.0.0.1, v2c und v3).

use std::fs;
use std::net::UdpSocket;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::{have_program, Skip};

pub const COMMUNITY: &str = "rj-prüf-community";
pub const V3_USER: &str = "rjsnmp";
pub const V3_AUTH: &str = "Auth-Kennwort-1";
pub const V3_PRIV: &str = "Priv-Kennwort-2";

pub struct Snmpd {
    pub port: u16,
    pub dir: tempfile::TempDir,
    child: Option<Child>,
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0").and_then(|s| s.local_addr()).map(|a| a.port()).expect("freier UDP-Port")
}

impl Snmpd {
    pub fn start() -> Result<Snmpd, Skip> {
        if !have_program("/usr/sbin/snmpd") {
            return Err(Skip("/usr/sbin/snmpd fehlt".into()));
        }
        let dir = tempfile::Builder::new().prefix("rj-snmpd").tempdir().map_err(|e| Skip(e.to_string()))?;
        let port = free_udp_port();
        let conf = dir.path().join("snmpd.conf");
        fs::write(
            &conf,
            format!(
                "agentAddress udp:127.0.0.1:{port}\n\
                 rocommunity \"{COMMUNITY}\" 127.0.0.1\n\
                 createUser {V3_USER} SHA \"{V3_AUTH}\" AES \"{V3_PRIV}\"\n\
                 rouser {V3_USER} priv\n\
                 sysName rj-snmp-test\n\
                 sysLocation Prüfraum\n"
            ),
        )
        .map_err(|e| Skip(e.to_string()))?;
        let child = Command::new("/usr/sbin/snmpd")
            .args(["-f", "-Lf", "/dev/null", "-C", "-c"])
            .arg(&conf)
            .arg("-p")
            .arg(dir.path().join("snmpd.pid"))
            .env("SNMP_PERSISTENT_DIR", dir.path())
            .env("MIBS", "")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Skip(format!("snmpd: {e}")))?;
        let s = Snmpd { port, dir, child: Some(child) };
        // Bereit, sobald snmpget antwortet.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let ok = Command::new("snmpget")
                .args(["-v2c", "-c", COMMUNITY, "-t", "1", "-r", "0", &format!("127.0.0.1:{port}"), "1.3.6.1.2.1.1.3.0"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if ok {
                return Ok(s);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(Skip("snmpd antwortet nicht".into()))
    }
}

impl Drop for Snmpd {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
