//! Tests gegen einen lokal gestarteten Net-SNMP-Agenten.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use rj_core::Secret;
use rj_snmp::*;
use rj_testkit::snmpd::*;

fn v2c(port: u16) -> Device {
    Device { host: "127.0.0.1".into(), port, credentials: Credentials::V2c { community: Secret::from_str_exact(COMMUNITY) }, timeout: Duration::from_secs(2) }
}

#[test]
fn values_throughput_v3_and_timeout() {
    let agent = match Snmpd::start() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {e}");
            return;
        }
    };
    // Werte lesen (v2c).
    let mut c = Client::open(&v2c(agent.port)).unwrap();
    let v = c.get(&[oids::SYS_NAME, oids::SYS_UPTIME]).unwrap();
    assert_eq!(v[0].1, Val::Text("rj-snmp-test".into()));
    assert!(matches!(v[1].1, Val::Ticks(_)));
    let s1 = sample(&mut c).unwrap();
    eprintln!("CPU {:?} %, Speicher {:?} %, Schnittstellen {:?}", s1.cpu_percent, s1.mem_percent, s1.interfaces.values().map(|i| &i.name).collect::<Vec<_>>());
    assert!(s1.mem_percent.is_some_and(|m| m > 0.0 && m <= 100.0), "Speicher aus hrStorage");
    let lo = *s1.interfaces.iter().find(|(_, i)| i.name == "lo").map(|(k, _)| k).expect("Schnittstelle lo");

    // Durchsatz: Verkehr auf lo erzeugen, zweite Abfrage.
    let t1 = Instant::now();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let sink = UdpSocket::bind("127.0.0.1:0").unwrap();
    for _ in 0..200 {
        sock.send_to(&[0u8; 1000], sink.local_addr().unwrap()).unwrap();
    }
    // snmpd hält die Schnittstellentabelle einige Sekunden im Zwischenspeicher.
    std::thread::sleep(Duration::from_millis(6000));
    let s2 = sample(&mut c).unwrap();
    let r = rates(&s1, &s2, t1.elapsed()).unwrap();
    let lo_rate = r.iter().find(|x| x.index == lo).unwrap();
    eprintln!("lo: {:.0} bit/s ein, {:.0} bit/s aus", lo_rate.in_bps, lo_rate.out_bps);
    assert!(lo_rate.in_bps > 200_000.0, "mindestens 200 × 1000 Bytes in ~6 s");

    // v3 authPriv (SHA, AES).
    let v3 = Device {
        credentials: Credentials::V3 {
            user: V3_USER.into(),
            auth: AuthProto::Sha1,
            auth_password: Secret::from_str_exact(V3_AUTH),
            privacy: Some((PrivProto::Aes128, Secret::from_str_exact(V3_PRIV))),
        },
        ..v2c(agent.port)
    };
    let mut c3 = Client::open(&v3).expect("v3-Anmeldung");
    assert_eq!(c3.get(&[oids::SYS_NAME]).unwrap()[0].1, Val::Text("rj-snmp-test".into()));
    // Falsches Kennwort.
    let bad = Device {
        credentials: Credentials::V3 { user: V3_USER.into(), auth: AuthProto::Sha1, auth_password: Secret::from_str_exact("falsch-falsch"), privacy: Some((PrivProto::Aes128, Secret::from_str_exact(V3_PRIV))) },
        timeout: Duration::from_millis(800),
        ..v2c(agent.port)
    };
    assert!(Client::open(&bad).and_then(|mut c| c.get(&[oids::SYS_NAME])).is_err());

    // Überwachung: grün; Zeitüberschreitung → grau.
    let mut m = Monitor::new(v2c(agent.port));
    assert_ne!(m.poll(0), Light::Grey);
    drop(agent);
    m.device.timeout = Duration::from_millis(500);
    assert_eq!(m.poll(10), Light::Grey);
    assert!(m.last_error.is_some());
}
