//! SNMP v2c und v3 (authPriv): Schnittstellenzähler (Durchsatz, Fehler),
//! CPU, Speicher, Betriebszeit; Verlauf im Speicher; Ampel nach
//! Schwellwerten. Zugangsdaten (Community, v3-Benutzer) kommen aus der KDBX.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use rj_core::Secret;
use serde::Serialize;
use snmp2::{Oid, SyncSession, Value};

#[derive(Debug, thiserror::Error)]
pub enum SnmpError {
    #[error("SNMP: Zeitüberschreitung oder keine Antwort ({0})")]
    Timeout(String),
    #[error("SNMP: {0}")]
    Protocol(String),
    #[error("SNMP-Adresse: {0}")]
    Io(#[from] std::io::Error),
}

/// Authentisierung für SNMPv3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthProto {
    Md5,
    Sha1,
    Sha256,
}

/// Verschlüsselung für SNMPv3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivProto {
    Des,
    Aes128,
}

/// Zugang zu einem Gerät.
#[derive(Debug, Clone)]
pub enum Credentials {
    V2c { community: Secret },
    V3 { user: String, auth: AuthProto, auth_password: Secret, privacy: Option<(PrivProto, Secret)> },
}

#[derive(Debug, Clone)]
pub struct Device {
    pub host: String,
    pub port: u16,
    pub credentials: Credentials,
    pub timeout: Duration,
}

/// Ein gelesener Wert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Val {
    Int(i64),
    Counter32(u32),
    Counter64(u64),
    Gauge(u32),
    Ticks(u32),
    Text(String),
    Oid(String),
    Missing,
}

impl Val {
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Val::Int(i) if i >= 0 => Some(i as u64),
            Val::Counter32(c) | Val::Gauge(c) | Val::Ticks(c) => Some(c as u64),
            Val::Counter64(c) => Some(c),
            _ => None,
        }
    }
}

fn convert(v: &Value) -> Val {
    match v {
        Value::Integer(i) => Val::Int(*i),
        Value::Counter32(c) => Val::Counter32(*c),
        Value::Counter64(c) => Val::Counter64(*c),
        Value::Unsigned32(c) => Val::Gauge(*c),
        Value::Timeticks(t) => Val::Ticks(*t),
        Value::OctetString(s) => Val::Text(String::from_utf8_lossy(s).into_owned()),
        Value::ObjectIdentifier(o) => Val::Oid(o.to_id_string()),
        _ => Val::Missing,
    }
}

fn oid(s: &str) -> Result<Oid<'static>, SnmpError> {
    let parts: Vec<u64> = s.trim_start_matches('.').split('.').map(|p| p.parse().map_err(|_| SnmpError::Protocol(format!("OID {s} ungültig")))).collect::<Result<_, _>>()?;
    Oid::from(&parts).map(|o| o.to_owned()).map_err(|_| SnmpError::Protocol(format!("OID {s} ungültig")))
}

/// Verbindung zu einem Gerät.
pub struct Client {
    sess: SyncSession,
    device: String,
}

fn map_err(e: snmp2::Error, device: &str) -> SnmpError {
    match e {
        snmp2::Error::Receive | snmp2::Error::Send => SnmpError::Timeout(device.to_string()),
        other => SnmpError::Protocol(other.to_string()),
    }
}

impl Client {
    pub fn open(d: &Device) -> Result<Client, SnmpError> {
        let addr = format!("{}:{}", d.host, d.port);
        let sess = match &d.credentials {
            Credentials::V2c { community } => SyncSession::new_v2c(addr.as_str(), community.as_bytes(), Some(d.timeout), 0)?,
            Credentials::V3 { user, auth, auth_password, privacy } => {
                let proto = match auth {
                    AuthProto::Md5 => snmp2::v3::AuthProtocol::Md5,
                    AuthProto::Sha1 => snmp2::v3::AuthProtocol::Sha1,
                    AuthProto::Sha256 => snmp2::v3::AuthProtocol::Sha256,
                };
                let level = match privacy {
                    None => snmp2::v3::Auth::AuthNoPriv,
                    Some((p, pw)) => snmp2::v3::Auth::AuthPriv {
                        cipher: match p {
                            PrivProto::Des => snmp2::v3::Cipher::Des,
                            PrivProto::Aes128 => snmp2::v3::Cipher::Aes128,
                        },
                        privacy_password: pw.as_bytes().to_vec(),
                    },
                };
                let sec = snmp2::v3::Security::new(user.as_bytes(), auth_password.as_bytes()).with_auth_protocol(proto).with_auth(level);
                let mut s = SyncSession::new_v3(addr.as_str(), Some(d.timeout), 0, sec)?;
                s.init().map_err(|e| map_err(e, &addr))?;
                s
            }
        };
        Ok(Client { sess, device: addr })
    }

    fn retry<T>(&mut self, mut f: impl FnMut(&mut SyncSession) -> Result<T, snmp2::Error>) -> Result<T, SnmpError> {
        match f(&mut self.sess) {
            // v3: Sicherheitskontext erneuert (Boots/Zeit) → einmal wiederholen.
            Err(snmp2::Error::AuthUpdated) => f(&mut self.sess).map_err(|e| map_err(e, &self.device)),
            r => r.map_err(|e| map_err(e, &self.device)),
        }
    }

    /// Einzelne Werte lesen.
    pub fn get(&mut self, oids: &[&str]) -> Result<Vec<(String, Val)>, SnmpError> {
        let parsed: Vec<Oid> = oids.iter().map(|o| oid(o)).collect::<Result<_, _>>()?;
        let refs: Vec<&Oid> = parsed.iter().collect();
        self.retry(|s| {
            let pdu = s.get_many(&refs)?;
            Ok(pdu.varbinds.map(|(o, v)| (o.to_id_string(), convert(&v))).collect())
        })
    }

    /// Teilbaum lesen (GETNEXT bis zum Ende des Präfixes).
    pub fn walk(&mut self, prefix: &str) -> Result<Vec<(String, Val)>, SnmpError> {
        let prefix = prefix.trim_start_matches('.').to_string();
        let mut cur = prefix.clone();
        let mut out = Vec::new();
        for _ in 0..10_000 {
            let o = oid(&cur)?;
            let next = self.retry(|s| {
                let pdu = s.getnext(&o)?;
                Ok(pdu.varbinds.map(|(o, v)| (o.to_id_string(), convert(&v))).next())
            })?;
            match next {
                Some((id, v)) if id.starts_with(&format!("{prefix}.")) && v != Val::Missing => {
                    cur = id.clone();
                    out.push((id, v));
                }
                _ => break,
            }
        }
        Ok(out)
    }
}

pub mod oids {
    pub const SYS_DESCR: &str = "1.3.6.1.2.1.1.1.0";
    pub const SYS_UPTIME: &str = "1.3.6.1.2.1.1.3.0";
    pub const SYS_NAME: &str = "1.3.6.1.2.1.1.5.0";
    pub const IF_DESCR: &str = "1.3.6.1.2.1.2.2.1.2";
    pub const IF_OPER: &str = "1.3.6.1.2.1.2.2.1.8";
    pub const IF_IN_OCTETS: &str = "1.3.6.1.2.1.2.2.1.10";
    pub const IF_IN_ERRORS: &str = "1.3.6.1.2.1.2.2.1.14";
    pub const IF_OUT_OCTETS: &str = "1.3.6.1.2.1.2.2.1.16";
    pub const IF_OUT_ERRORS: &str = "1.3.6.1.2.1.2.2.1.20";
    pub const IF_HC_IN: &str = "1.3.6.1.2.1.31.1.1.1.6";
    pub const IF_HC_OUT: &str = "1.3.6.1.2.1.31.1.1.1.10";
    pub const HR_PROC_LOAD: &str = "1.3.6.1.2.1.25.3.3.1.2";
    pub const HR_STORAGE_TYPE: &str = "1.3.6.1.2.1.25.2.3.1.2";
    pub const HR_STORAGE_UNITS: &str = "1.3.6.1.2.1.25.2.3.1.4";
    pub const HR_STORAGE_SIZE: &str = "1.3.6.1.2.1.25.2.3.1.5";
    pub const HR_STORAGE_USED: &str = "1.3.6.1.2.1.25.2.3.1.6";
    pub const HR_STORAGE_RAM: &str = "1.3.6.1.2.1.25.2.1.2";
}

/// Differenz eines Zählers mit Überlauf (32 oder 64 Bit).
pub fn counter_delta(prev: u64, cur: u64, bits: u32) -> u64 {
    if cur >= prev {
        cur - prev
    } else {
        let max = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
        (max - prev) + cur + 1
    }
}

/// Rohwerte einer Abfrage.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sample {
    pub uptime_ticks: u32,
    pub cpu_percent: Option<f64>,
    pub mem_percent: Option<f64>,
    /// Je Schnittstelle: (Name, ein, aus, Fehler ein, Fehler aus, 64-Bit-Zähler?, aktiv?)
    pub interfaces: BTreeMap<u32, IfCounters>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IfCounters {
    pub name: String,
    pub in_octets: u64,
    pub out_octets: u64,
    pub in_errors: u64,
    pub out_errors: u64,
    pub hc: bool,
    pub up: bool,
}

/// Durchsatz einer Schnittstelle.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IfRate {
    pub index: u32,
    pub name: String,
    pub in_bps: f64,
    pub out_bps: f64,
    pub errors_per_s: f64,
    pub up: bool,
}

fn idx(oid: &str) -> Option<u32> {
    oid.rsplit('.').next()?.parse().ok()
}

/// Liest alle Werte für die Überwachung.
pub fn sample(c: &mut Client) -> Result<Sample, SnmpError> {
    let up = c.get(&[oids::SYS_UPTIME])?;
    let uptime_ticks = up.first().and_then(|v| v.1.as_u64()).ok_or_else(|| SnmpError::Protocol("sysUpTime fehlt".into()))? as u32;
    let loads: Vec<u64> = c.walk(oids::HR_PROC_LOAD).unwrap_or_default().iter().filter_map(|v| v.1.as_u64()).collect();
    let cpu_percent = (!loads.is_empty()).then(|| loads.iter().sum::<u64>() as f64 / loads.len() as f64);
    let types = c.walk(oids::HR_STORAGE_TYPE).unwrap_or_default();
    let mut mem_percent = None;
    if let Some((o, _)) = types.iter().find(|(_, v)| *v == Val::Oid(oids::HR_STORAGE_RAM.into())) {
        if let Some(i) = idx(o) {
            let vals = c.get(&[&format!("{}.{i}", oids::HR_STORAGE_SIZE), &format!("{}.{i}", oids::HR_STORAGE_USED)])?;
            if let (Some(size), Some(used)) = (vals[0].1.as_u64(), vals[1].1.as_u64()) {
                if size > 0 {
                    mem_percent = Some(used as f64 * 100.0 / size as f64);
                }
            }
        }
    }
    let mut interfaces = BTreeMap::new();
    for (o, v) in c.walk(oids::IF_DESCR)? {
        let (Some(i), Val::Text(name)) = (idx(&o), v) else { continue };
        let names = [
            format!("{}.{i}", oids::IF_HC_IN),
            format!("{}.{i}", oids::IF_HC_OUT),
            format!("{}.{i}", oids::IF_IN_OCTETS),
            format!("{}.{i}", oids::IF_OUT_OCTETS),
            format!("{}.{i}", oids::IF_IN_ERRORS),
            format!("{}.{i}", oids::IF_OUT_ERRORS),
            format!("{}.{i}", oids::IF_OPER),
        ];
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let v = c.get(&refs)?;
        let hc = v[0].1.as_u64().is_some() && v[1].1.as_u64().is_some();
        interfaces.insert(i, IfCounters {
            name,
            in_octets: if hc { v[0].1.as_u64() } else { v[2].1.as_u64() }.unwrap_or(0),
            out_octets: if hc { v[1].1.as_u64() } else { v[3].1.as_u64() }.unwrap_or(0),
            in_errors: v[4].1.as_u64().unwrap_or(0),
            out_errors: v[5].1.as_u64().unwrap_or(0),
            hc,
            up: v[6].1 == Val::Int(1),
        });
    }
    Ok(Sample { uptime_ticks, cpu_percent, mem_percent, interfaces })
}

/// Durchsatz zwischen zwei Abfragen; `None` nach einem Neustart des Geräts.
pub fn rates(prev: &Sample, cur: &Sample, dt: Duration) -> Option<Vec<IfRate>> {
    if cur.uptime_ticks < prev.uptime_ticks {
        return None;
    }
    let secs = dt.as_secs_f64().max(0.001);
    Some(
        cur.interfaces
            .iter()
            .filter_map(|(i, c)| {
                let p = prev.interfaces.get(i)?;
                let bits = if c.hc { 64 } else { 32 };
                Some(IfRate {
                    index: *i,
                    name: c.name.clone(),
                    in_bps: counter_delta(p.in_octets, c.in_octets, bits) as f64 * 8.0 / secs,
                    out_bps: counter_delta(p.out_octets, c.out_octets, bits) as f64 * 8.0 / secs,
                    errors_per_s: (counter_delta(p.in_errors, c.in_errors, 32) + counter_delta(p.out_errors, c.out_errors, 32)) as f64 / secs,
                    up: c.up,
                })
            })
            .collect(),
    )
}

/// Ampel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Light {
    Grey,
    Green,
    Yellow,
    Red,
}

/// Schwellwerte für die Ampel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub cpu_yellow: f64,
    pub cpu_red: f64,
    pub mem_yellow: f64,
    pub mem_red: f64,
    pub errors_yellow: f64,
    pub errors_red: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds { cpu_yellow: 70.0, cpu_red: 90.0, mem_yellow: 85.0, mem_red: 95.0, errors_yellow: 1.0, errors_red: 10.0 }
    }
}

/// Bewertung einer Abfrage.
pub fn light(sample: Option<&Sample>, rates: Option<&[IfRate]>, t: &Thresholds) -> Light {
    let Some(s) = sample else { return Light::Grey };
    let mut l = Light::Green;
    let mut bump = |v: f64, y: f64, r: f64| {
        if v >= r {
            l = Light::Red;
        } else if v >= y && l == Light::Green {
            l = Light::Yellow;
        }
    };
    if let Some(c) = s.cpu_percent {
        bump(c, t.cpu_yellow, t.cpu_red);
    }
    if let Some(m) = s.mem_percent {
        bump(m, t.mem_yellow, t.mem_red);
    }
    if let Some(r) = rates {
        let e: f64 = r.iter().map(|x| x.errors_per_s).sum();
        bump(e, t.errors_yellow, t.errors_red);
    }
    l
}

/// Verlauf eines Messwerts: letzte Stunde (alle Werte) und letzter Tag (je Minute gemittelt).
#[derive(Debug, Clone, Default, Serialize)]
pub struct History {
    pub hour: VecDeque<(u64, f64)>,
    pub day: VecDeque<(u64, f64)>,
    #[serde(skip)]
    minute: Option<(u64, f64, u32)>,
}

impl History {
    /// Wert zur Zeit `t` (Sekunden) hinzufügen.
    pub fn push(&mut self, t: u64, v: f64) {
        self.hour.push_back((t, v));
        while self.hour.front().is_some_and(|(x, _)| t.saturating_sub(*x) > 3600) {
            self.hour.pop_front();
        }
        let m = t / 60;
        match &mut self.minute {
            Some((cm, sum, n)) if *cm == m => {
                *sum += v;
                *n += 1;
            }
            _ => {
                if let Some((cm, sum, n)) = self.minute.take() {
                    self.day.push_back((cm * 60, sum / n as f64));
                }
                self.minute = Some((m, v, 1));
            }
        }
        while self.day.front().is_some_and(|(x, _)| t.saturating_sub(*x) > 86_400) {
            self.day.pop_front();
        }
    }
}

/// Überwachung eines Geräts.
pub struct Monitor {
    pub device: Device,
    pub thresholds: Thresholds,
    last: Option<(Instant, Sample)>,
    pub cpu: History,
    pub mem: History,
    pub throughput_in: History,
    pub throughput_out: History,
    pub light: Light,
    pub last_error: Option<String>,
    pub last_rates: Vec<IfRate>,
}

impl Monitor {
    pub fn new(device: Device) -> Self {
        Monitor {
            device,
            thresholds: Thresholds::default(),
            last: None,
            cpu: History::default(),
            mem: History::default(),
            throughput_in: History::default(),
            throughput_out: History::default(),
            light: Light::Grey,
            last_error: None,
            last_rates: Vec::new(),
        }
    }

    /// Eine Abfrage; Zeitüberschreitung oder Fehler ergibt „grau“.
    pub fn poll(&mut self, now_secs: u64) -> Light {
        let r = Client::open(&self.device).and_then(|mut c| sample(&mut c));
        match r {
            Ok(s) => {
                let now = Instant::now();
                let rates = self.last.as_ref().and_then(|(t, p)| rates(p, &s, now - *t));
                if let Some(c) = s.cpu_percent {
                    self.cpu.push(now_secs, c);
                }
                if let Some(m) = s.mem_percent {
                    self.mem.push(now_secs, m);
                }
                if let Some(r) = &rates {
                    self.throughput_in.push(now_secs, r.iter().map(|x| x.in_bps).sum());
                    self.throughput_out.push(now_secs, r.iter().map(|x| x.out_bps).sum());
                    self.last_rates = r.clone();
                }
                self.light = light(Some(&s), rates.as_deref(), &self.thresholds);
                self.last = Some((now, s));
                self.last_error = None;
            }
            Err(e) => {
                self.light = Light::Grey;
                self.last_error = Some(e.to_string());
            }
        }
        self.light
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_wrap() {
        assert_eq!(counter_delta(100, 150, 32), 50);
        assert_eq!(counter_delta(u32::MAX as u64 - 9, 10, 32), 20);
        assert_eq!(counter_delta(u64::MAX - 4, 5, 64), 10);
    }

    fn s(up: u32, inb: u64, cpu: f64) -> Sample {
        let mut interfaces = BTreeMap::new();
        interfaces.insert(1, IfCounters { name: "eth0".into(), in_octets: inb, out_octets: 0, in_errors: 0, out_errors: 0, hc: false, up: true });
        Sample { uptime_ticks: up, cpu_percent: Some(cpu), mem_percent: Some(40.0), interfaces }
    }

    #[test]
    fn rates_with_overflow_and_reboot() {
        let a = s(1000, u32::MAX as u64 - 999, 10.0);
        let b = s(2000, 1000, 10.0);
        let r = rates(&a, &b, Duration::from_secs(10)).unwrap();
        assert_eq!(r[0].in_bps, 2000.0 * 8.0 / 10.0);
        let rebooted = s(5, 10, 10.0);
        assert!(rates(&b, &rebooted, Duration::from_secs(10)).is_none());
    }

    #[test]
    fn lights() {
        let t = Thresholds::default();
        assert_eq!(light(None, None, &t), Light::Grey);
        assert_eq!(light(Some(&s(1, 0, 10.0)), None, &t), Light::Green);
        assert_eq!(light(Some(&s(1, 0, 75.0)), None, &t), Light::Yellow);
        assert_eq!(light(Some(&s(1, 0, 95.0)), None, &t), Light::Red);
        let rr = [IfRate { index: 1, name: "x".into(), in_bps: 0.0, out_bps: 0.0, errors_per_s: 20.0, up: true }];
        assert_eq!(light(Some(&s(1, 0, 10.0)), Some(&rr), &t), Light::Red);
    }

    #[test]
    fn history_hour_and_day() {
        let mut h = History::default();
        for i in 0..(2 * 3600 / 10) {
            h.push(i * 10, i as f64);
        }
        assert!(h.hour.len() <= 361);
        assert!(h.hour.front().unwrap().0 >= 7190 - 3600, "nur die letzte Stunde");
        assert!(h.day.len() >= 119, "je Minute ein Wert: {}", h.day.len());
        let (_, first) = h.day[0];
        assert!((first - 2.5).abs() < 0.001, "Mittelwert der ersten Minute");
    }
}
