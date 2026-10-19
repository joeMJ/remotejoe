//! Rechnername und Windows-Version eines RDP-Servers ohne Anmeldung.
//!
//! Mit NLA beginnt die Anmeldung mit CredSSP/NTLM: Auf die NTLM-Aushandlung (ohne Benutzer und
//! Kennwort) antwortet der Server mit der Challenge, die NetBIOS- und DNS-Namen sowie die
//! Windows-Version enthält. Danach wird die Verbindung beendet; es werden keine Zugangsdaten
//! gesendet. Das Serverzertifikat wird hierfür nicht geprüft (es geht nichts Geheimes hinaus).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NtlmInfo {
    pub nb_computer: Option<String>,
    pub nb_domain: Option<String>,
    pub dns_computer: Option<String>,
    pub dns_domain: Option<String>,
    /// Windows-Version (Haupt, Neben, Build).
    pub version: Option<(u8, u8, u16)>,
}

impl NtlmInfo {
    /// Rechnername wie in `Win32_ComputerSystem.Name` (NetBIOS), sonst der kurze DNS-Name.
    pub fn machine_name(&self) -> Option<String> {
        self.nb_computer.clone().or_else(|| self.dns_computer.as_ref().map(|d| d.split('.').next().unwrap_or(d).to_uppercase()))
    }

    pub fn system(&self) -> Option<String> {
        self.version.map(|(a, b, build)| format!("Windows {a}.{b}.{build}"))
    }
}

/// X.224-Verbindungsanfrage mit RDP_NEG_REQ (TLS und CredSSP).
const X224_CR: [u8; 19] = [3, 0, 0, 19, 14, 0xE0, 0, 0, 0, 0, 0, 1, 0, 8, 0, 3, 0, 0, 0];

/// NTLM NEGOTIATE_MESSAGE ohne Domäne und Arbeitsstation, mit Versionsangabe.
fn ntlm_negotiate() -> Vec<u8> {
    let mut m = b"NTLMSSP\0".to_vec();
    m.extend_from_slice(&1u32.to_le_bytes());
    m.extend_from_slice(&0xE208_82B7u32.to_le_bytes());
    m.extend_from_slice(&[0; 16]);
    m.extend_from_slice(&[10, 0, 0x61, 0x4A, 0, 0, 0, 15]);
    m
}

fn der(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    let n = body.len();
    if n < 0x80 {
        v.push(n as u8);
    } else if n < 0x100 {
        v.extend_from_slice(&[0x81, n as u8]);
    } else {
        v.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]);
    }
    v.extend_from_slice(body);
    v
}

/// TSRequest { version 3, negoTokens { { negoToken } } }.
fn ts_request(token: &[u8]) -> Vec<u8> {
    let version = der(0xA0, &der(0x02, &[3]));
    let nego = der(0xA1, &der(0x30, &der(0x30, &der(0xA0, &der(0x04, token)))));
    der(0x30, &[version, nego].concat())
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn utf16(b: &[u8]) -> String {
    let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&w)
}

/// Sucht in der Antwort die NTLM-Challenge und liest Namen und Version.
pub fn parse_challenge(resp: &[u8]) -> Option<NtlmInfo> {
    let start = resp.windows(8).position(|w| w == b"NTLMSSP\0")?;
    let m = &resp[start..];
    if u32_at(m, 8)? != 2 {
        return None;
    }
    let flags = u32_at(m, 20)?;
    let (len, off) = (u16_at(m, 40)? as usize, u32_at(m, 44)? as usize);
    let mut info = NtlmInfo::default();
    if flags & 0x0200_0000 != 0 && m.len() >= 56 {
        info.version = Some((m[48], m[49], u16_at(m, 50)?));
    }
    let ti = m.get(off..off.checked_add(len)?)?;
    let mut i = 0;
    while i + 4 <= ti.len() {
        let (id, n) = (u16_at(ti, i)?, u16_at(ti, i + 2)? as usize);
        let val = ti.get(i + 4..i + 4 + n)?;
        let s = Some(utf16(val)).filter(|s| !s.is_empty());
        match id {
            0 => break,
            1 => info.nb_computer = s,
            2 => info.nb_domain = s,
            3 => info.dns_computer = s,
            4 => info.dns_domain = s,
            _ => {}
        }
        i += 4 + n;
    }
    Some(info)
}

#[derive(Debug)]
struct AcceptAny(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Fragt den RDP-Server unter `addr` ab (`host` nur für die TLS-Namensangabe).
pub fn query(addr: SocketAddr, host: &str, timeout: Duration) -> Result<NtlmInfo, String> {
    let mut sock = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("RDP nicht erreichbar: {e}"))?;
    sock.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    sock.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    sock.write_all(&X224_CR).map_err(|e| e.to_string())?;
    let mut cc = [0u8; 19];
    sock.read_exact(&mut cc).map_err(|e| format!("keine RDP-Antwort: {e}"))?;
    // RDP_NEG_RSP (2) mit gewähltem Protokoll; RDP_NEG_FAILURE (3) oder nur TLS: keine NLA.
    if cc[11] != 2 || u32_at(&cc, 15).is_none_or(|p| p & 0x0A == 0) {
        return Err("RDP ohne NLA: kein Rechnername vor der Anmeldung".into());
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)))
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(super::probe::bare_host(host).to_string())
        .or_else(|_| rustls::pki_types::ServerName::try_from(addr.ip().to_string()))
        .map_err(|e| e.to_string())?;
    let conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
    let mut tls = rustls::StreamOwned::new(conn, sock);
    tls.write_all(&ts_request(&ntlm_negotiate())).map_err(|e| format!("TLS: {e}"))?;
    let mut buf = vec![0u8; 4096];
    let mut got = 0;
    // Antwort lesen, bis die Challenge vollständig ist (oder nichts mehr kommt).
    loop {
        match tls.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => {
                got += n;
                if let Some(i) = parse_challenge(&buf[..got]) {
                    let _ = tls.sock.shutdown(std::net::Shutdown::Both);
                    return Ok(i);
                }
                if got == buf.len() {
                    break;
                }
            }
            Err(e) => return Err(format!("keine NTLM-Antwort: {e}")),
        }
    }
    parse_challenge(&buf[..got]).ok_or_else(|| "keine NTLM-Antwort".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn av(id: u16, s: &str) -> Vec<u8> {
        let w: Vec<u8> = s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        [id.to_le_bytes().to_vec(), (w.len() as u16).to_le_bytes().to_vec(), w].concat()
    }

    /// Challenge wie von Windows (Version 10.0.19045), eingebettet in einen TSRequest.
    fn challenge() -> Vec<u8> {
        let ti = [av(2, "FIRMA"), av(1, "SRV01"), av(4, "firma.example"), av(3, "srv01.firma.example"), vec![0, 0, 0, 0]].concat();
        let mut m = b"NTLMSSP\0".to_vec();
        m.extend_from_slice(&2u32.to_le_bytes());
        m.extend_from_slice(&[0; 8]); // TargetName
        m.extend_from_slice(&0x0281_0215u32.to_le_bytes() /* mit VERSION */);
        m.extend_from_slice(&[0; 16]); // Challenge, reserviert
        m.extend_from_slice(&(ti.len() as u16).to_le_bytes());
        m.extend_from_slice(&(ti.len() as u16).to_le_bytes());
        m.extend_from_slice(&56u32.to_le_bytes());
        m.extend_from_slice(&[10, 0, 0x85, 0x4A, 0, 0, 0, 15]);
        m.extend_from_slice(&ti);
        ts_request(&m)
    }

    #[test]
    fn names_and_version_come_from_the_challenge() {
        let i = parse_challenge(&challenge()).unwrap();
        assert_eq!(i.nb_computer.as_deref(), Some("SRV01"));
        assert_eq!(i.dns_computer.as_deref(), Some("srv01.firma.example"));
        assert_eq!(i.nb_domain.as_deref(), Some("FIRMA"));
        assert_eq!(i.machine_name().as_deref(), Some("SRV01"));
        assert_eq!(i.system().as_deref(), Some("Windows 10.0.19077"));
        let short = NtlmInfo { dns_computer: Some("pc7.lan".into()), ..Default::default() };
        assert_eq!(short.machine_name().as_deref(), Some("PC7"));
    }

    #[test]
    fn broken_or_foreign_answers_are_rejected() {
        let c = challenge();
        assert!(parse_challenge(&c[..c.len() - 30]).is_none(), "abgeschnitten");
        assert!(parse_challenge(b"kein ntlm").is_none());
        let mut neg = ntlm_negotiate();
        neg.extend_from_slice(&[0; 40]);
        assert!(parse_challenge(&neg).is_none(), "Typ 1 ist keine Challenge");
    }

    #[test]
    fn requests_are_well_formed() {
        assert_eq!(X224_CR[3] as usize, X224_CR.len());
        assert_eq!(X224_CR[4] as usize, X224_CR.len() - 5);
        let n = ntlm_negotiate();
        assert_eq!((n.len(), u32_at(&n, 8)), (40, Some(1)));
        let t = ts_request(&n);
        assert_eq!((t[0], t[1] as usize), (0x30, t.len() - 2));
        assert!(t.windows(8).any(|w| w == b"NTLMSSP\0"));
    }

    #[test]
    fn server_without_nla_is_reported() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut req = [0u8; 19];
            s.read_exact(&mut req).unwrap();
            // Verbindungsbestätigung mit RDP_NEG_RSP: nur Standard-RDP-Sicherheit.
            s.write_all(&[3, 0, 0, 19, 14, 0xD0, 0, 0, 0x12, 0x34, 0, 2, 0, 8, 0, 0, 0, 0, 0]).unwrap();
        });
        let e = query(addr, "127.0.0.1", Duration::from_secs(2)).unwrap_err();
        assert!(e.contains("ohne NLA"), "{e}");
    }
}
