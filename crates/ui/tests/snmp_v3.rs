//! SNMPv3 mit getrennten Kennwörtern: über die Bearbeitungsschicht
//! eingegeben, in der KDBX gespeichert (Authentisierung im Kennwortfeld,
//! Verschlüsselung im geschützten Zusatzfeld) und gegen snmpd geprüft.

use rj_core::Secret;
use rj_kdbx::{KdfChoice, MasterKey, Vault};
use rj_testkit::snmpd::{Snmpd, V3_AUTH, V3_PRIV, V3_USER};
use rj_ui::app::{snmp_credentials, App};

#[test]
fn v3_separate_passwords_roundtrip_and_query() {
    let agent = match Snmpd::start() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {}", e.0);
            return;
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snmp.kdbx");
    let key = MasterKey::password("pw");
    let mut v = Vault::create(&path, key.clone(), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    v.save(None).unwrap();
    let mut app = App::new(false);
    app.index = v.index().index.clone();
    app.vault = Some(v);
    let r: serde_json::Value = serde_json::from_str(&app.edit(
        &serde_json::json!({
            "op": "snmp_save", "name": "Switch", "host": "127.0.0.1", "port": agent.port, "version": "v3",
            "username": V3_USER, "auth_proto": "sha1", "auth_password": V3_AUTH, "priv_proto": "aes128", "priv_password": V3_PRIV
        })
        .to_string(),
    ))
    .unwrap();
    assert_eq!(r["save"]["saved"], true, "{r}");
    app.lock(false);

    // Neu öffnen: beide Kennwörter getrennt und byte-genau gespeichert.
    let v = Vault::open(&path, key, rj_core::config::IndexStorage::KdbxCustomData).unwrap();
    let d = v.index().index.snmp_devices[0].clone();
    let c = v.credential(d.credential.entry).unwrap();
    assert_eq!(c.data.username, V3_USER);
    assert_eq!(c.data.password.as_bytes(), V3_AUTH.as_bytes());
    assert_eq!(v.field(d.credential.entry, rj_core::SNMP_PRIV_FIELD).unwrap().as_bytes(), V3_PRIV.as_bytes());
    assert!(!String::from_utf8(rj_core::schema::to_json(&v.index().index).unwrap()).unwrap().contains(V3_PRIV));

    let creds = snmp_credentials(&v, &d).unwrap();
    let dev = rj_snmp::Device { host: d.host.clone(), port: d.port.unwrap(), credentials: creds.clone(), timeout: std::time::Duration::from_secs(3) };
    let mut client = rj_snmp::Client::open(&dev).unwrap();
    let vals = client.get(&["1.3.6.1.2.1.1.5.0"]).unwrap();
    assert_eq!(vals.len(), 1);
    // Gegenprobe: dasselbe Kennwort für beides (alte Ablage) wird abgelehnt.
    let rj_snmp::Credentials::V3 { user, auth, auth_password, privacy } = creds else { panic!() };
    let same = rj_snmp::Credentials::V3 { user, auth, auth_password: auth_password.clone(), privacy: privacy.map(|(p, _)| (p, Secret::from_bytes(auth_password.as_bytes().to_vec()))) };
    let dev = rj_snmp::Device { credentials: same, ..dev };
    assert!(rj_snmp::Client::open(&dev).and_then(|mut c| c.get(&["1.3.6.1.2.1.1.5.0"])).is_err());
}
