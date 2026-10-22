//! Aruba-Werkzeuge (zuschaltbar unter Einstellungen → Zusatzfunktionen): Auswertung der CLI-Ausgaben eines
//! Aruba-Instant-Clusters. Abgefragt wird am virtuellen Controller (VC) bzw. am einzelnen AP, nur mit lesenden
//! Befehlen (`show aps`, `show running-config`, `show version`, `show networks`, `show clients`).

use serde_json::{json, Value};

/// Befehle für die Cluster-Karte am VC.
pub const CLUSTER_COMMANDS: [&str; 3] = ["no paging", "show aps", "show running-config"];

/// Spalten einer CLI-Tabelle: Kopfzeile mit Titeln (durch mindestens zwei Leerzeichen getrennt), darunter eine
/// Zeile aus Strichen, dann die Daten. Liefert je Spalte Titel und Startposition sowie die Datenzeilen.
pub fn table<'a>(out: &'a str, must: &[&str]) -> Option<(Vec<(String, usize)>, Vec<&'a str>)> {
    let lines: Vec<&str> = out.lines().map(|l| l.trim_end_matches('\r')).collect();
    let h = lines.iter().position(|l| must.iter().all(|m| l.contains(m)))?;
    let header = lines[h];
    let mut cols = Vec::new();
    let b = header.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b' ' && (i == 0 || (i >= 2 && b[i - 1] == b' ' && b[i - 2] == b' ')) {
            let end = header[i..].find("  ").map_or(header.len(), |e| i + e);
            cols.push((header[i..end].trim().to_string(), i));
            i = end;
        } else {
            i += 1;
        }
    }
    let start = if lines.get(h + 1).is_some_and(|l| l.trim_start().starts_with("--")) { h + 2 } else { h + 1 };
    let rows = lines[start..].iter().copied().take_while(|l| !l.trim().is_empty()).collect();
    Some((cols, rows))
}

/// Wert einer Spalte: das erste Wort ab der Startposition der Spalte.
pub fn cell<'a>(row: &'a str, cols: &[(String, usize)], name: &str) -> Option<&'a str> {
    let (_, start) = cols.iter().find(|(n, _)| n == name)?;
    row.get(*start..)?.split_whitespace().next().filter(|w| *w != "--")
}

fn is_mac(s: &str) -> bool {
    s.len() == 17 && s.split(':').count() == 6 && s.split(':').all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Freigegebene APs (`allowed-ap <MAC>`) aus der laufenden Konfiguration, klein geschrieben.
pub fn allowed_aps(running_config: &str) -> Vec<String> {
    running_config
        .lines()
        .filter_map(|l| l.trim().strip_prefix("allowed-ap "))
        .map(|m| m.trim().to_ascii_lowercase())
        .filter(|m| is_mac(m))
        .collect()
}

fn segment_of(ip: &str) -> Option<String> {
    let p: Vec<&str> = ip.split('.').collect();
    (p.len() == 4 && p.iter().all(|x| x.parse::<u8>().is_ok())).then(|| format!("{}.{}.{}", p[0], p[1], p[2]))
}

/// Cluster-Karte: APs aus `show aps` (online) und aus `allowed-ap` ohne Eintrag in `show aps` (offline).
/// Offline-APs kennt nur ihre MAC; heißen online-APs anders als ihre MAC, ist die Zuordnung nicht sicher
/// (`offline_uncertain`).
pub fn cluster_json(show_aps: &str, running_config: &str, vc_ip: &str) -> Value {
    let mut aps = Vec::new();
    if let Some((cols, rows)) = table(show_aps, &["Name", "IP Address"]) {
        for row in rows {
            let Some(name) = cell(row, &cols, "Name") else { continue };
            let Some(ip_raw) = cell(row, &cols, "IP Address") else { continue };
            let ip = ip_raw.trim_end_matches('*');
            if segment_of(ip).is_none() {
                continue;
            }
            aps.push(json!({
                "name": name,
                "ip": ip,
                "conductor": ip_raw.ends_with('*'),
                "clients": cell(row, &cols, "Clients").and_then(|c| c.parse::<u32>().ok()),
                "mode": cell(row, &cols, "Mode"),
                "type": cell(row, &cols, "Type"),
                "serial": cell(row, &cols, "Serial #"),
                "online": true,
            }));
        }
    }
    let online_names: Vec<String> = aps.iter().filter_map(|a| a["name"].as_str()).map(str::to_ascii_lowercase).collect();
    let all_macs = online_names.iter().all(|n| is_mac(n));
    let allowed = allowed_aps(running_config);
    let offline: Vec<String> = allowed.iter().filter(|m| !online_names.contains(m)).cloned().collect();
    let offline_count = if allowed.is_empty() {
        0
    } else if all_macs {
        offline.len()
    } else {
        allowed.len().saturating_sub(aps.len())
    };
    let segment = segment_of(vc_ip).or_else(|| aps.iter().find(|a| a["conductor"] == true).and_then(|a| a["ip"].as_str().and_then(segment_of)));
    for a in aps.iter_mut() {
        let inside = segment.as_deref().is_some_and(|s| a["ip"].as_str().and_then(segment_of).as_deref() == Some(s));
        a["in_segment"] = json!(inside);
    }
    let mut list = aps.clone();
    for m in &offline {
        list.push(json!({ "name": m, "ip": null, "online": false, "conductor": false, "clients": null, "in_segment": true }));
    }
    let clients: u64 = aps.iter().filter_map(|a| a["clients"].as_u64()).sum();
    json!({
        "vc": vc_ip,
        "segment": segment.map(|s| format!("{s}.0/24")),
        "total": aps.len() + offline_count,
        "online": aps.len(),
        "offline": offline_count,
        "offline_uncertain": !allowed.is_empty() && !all_macs,
        "allowed_known": !allowed.is_empty(),
        "clients": clients,
        "aps": list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Aufbau wie am Test-AP (Spalten gekürzt), Daten erfunden.
    const SHOW_APS: &str = "\r\n3 Access Points\r\n---------------\r\n\
Name               IP Address       Mode    Spectrum  Clients  Type         IPv6 Address  Mesh Role  Zone  Serial #    radio0 Channel\r\n\
----               ----------       ----    --------  -------  ----         ------------  ---------  ----  --------    --------------\r\n\
02:00:5e:00:53:12  192.0.2.210*     access  disable   7        515(indoor)  --            N/A        -     CNZZQX0001  52E\r\n\
02:00:5e:00:53:13  192.0.2.211      access  disable   12       515(indoor)  --            N/A        -     CNZZQX0002  36\r\n\
02:00:5e:00:53:14  198.51.100.7     access  disable   0        505(indoor)  --            N/A        -     CNZZQX0003  1\r\n";

    const CONFIG: &str = "version 8.10.0.0-8.10.0\r\nvirtual-controller-ip 192.0.2.200\r\nallowed-ap 02:00:5e:00:53:12\r\nallowed-ap 02:00:5e:00:53:13\r\nallowed-ap 02:00:5e:00:53:14\r\nallowed-ap 02:00:5e:00:53:15\r\n";

    #[test]
    fn cluster_from_show_aps_and_allowed_aps() {
        let c = cluster_json(SHOW_APS, CONFIG, "192.0.2.200");
        assert_eq!((c["total"].as_u64(), c["online"].as_u64(), c["offline"].as_u64(), c["clients"].as_u64()), (Some(4), Some(3), Some(1), Some(19)));
        assert_eq!(c["segment"], "192.0.2.0/24");
        let aps = c["aps"].as_array().unwrap();
        assert_eq!(aps[0]["conductor"], true);
        assert_eq!(aps[0]["ip"], "192.0.2.210");
        assert_eq!(aps[1]["clients"], 12);
        assert_eq!(aps[1]["serial"], "CNZZQX0002");
        assert_eq!(aps[2]["in_segment"], false, "anderes Segment");
        assert_eq!(aps[3]["name"], "02:00:5e:00:53:15");
        assert_eq!(aps[3]["online"], false);
        assert_eq!(c["offline_uncertain"], false);
    }

    #[test]
    fn named_aps_make_offline_assignment_uncertain_and_no_allowed_list_means_unknown() {
        let named = SHOW_APS.replace("02:00:5e:00:53:13  ", "aula               ");
        let c = cluster_json(&named, CONFIG, "192.0.2.200");
        assert_eq!(c["offline_uncertain"], true);
        assert_eq!(c["offline"], 1, "4 freigegeben, 3 online");
        let none = cluster_json(SHOW_APS, "version 8.10\r\n", "192.0.2.200");
        assert_eq!((none["total"].as_u64(), none["offline"].as_u64(), none["allowed_known"].as_bool()), (Some(3), Some(0), Some(false)));
    }
}
