//! Aruba-Werkzeuge (zuschaltbar unter Einstellungen → Zusatzfunktionen): Auswertung der CLI-Ausgaben eines
//! Aruba-Instant-Clusters. Abgefragt wird am virtuellen Controller (VC) bzw. am einzelnen AP, nur mit lesenden
//! Befehlen (`show aps`, `show running-config`, `show version`, `show networks`, `show clients`).

use serde_json::{json, Value};

/// Befehle für die Cluster-Karte am VC.
pub const CLUSTER_COMMANDS: [&str; 4] = ["no paging", "show aps", "show running-config", "show summary"];

/// Verwaltung des Clusters aus `show summary` (wie `ap_quick_inventory.py`): `central` (auch On-Premises),
/// `airwave` oder `none`; `None`, wenn die Ausgabe nichts dazu enthält.
pub fn management(summary: &str) -> Option<&'static str> {
    let field = |name: &str| {
        summary.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim().eq_ignore_ascii_case(name)).then(|| v.trim().to_ascii_lowercase())
        })
    };
    let via = field("Managed Via");
    let central = field("Aruba Central");
    let airwave = field("Airwave");
    if via.is_none() && central.is_none() && airwave.is_none() {
        return None;
    }
    let via = via.unwrap_or_default();
    Some(if via.contains("central") || central.as_deref() == Some("connected") {
        "central"
    } else if via.contains("airwave") || airwave.as_deref() == Some("connected") {
        "airwave"
    } else {
        "none"
    })
}

/// Spalten einer CLI-Tabelle: Kopfzeile mit Titeln (durch mindestens zwei Leerzeichen getrennt), darunter eine
/// Zeile aus Strichen, dann die Daten. Liefert je Spalte Titel und Startposition sowie die Datenzeilen.
pub fn table<'a>(out: &'a str, must: &[&str]) -> Option<(Vec<(String, usize)>, Vec<&'a str>)> {
    let lines: Vec<&str> = out.lines().map(|l| l.trim_end_matches('\r')).collect();
    let h = lines.iter().position(|l| must.iter().all(|m| l.contains(m)))?;
    let header = lines[h];
    let mut cols = Vec::new();
    let dashes = lines.get(h + 1).filter(|l| l.trim_start().starts_with("--"));
    if let Some(d) = dashes {
        // Spaltengrenzen aus der Strichzeile (wie in den Aruba-Werkzeugen); Titel aus der Kopfzeile darüber.
        let b = d.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'-' && (i == 0 || b[i - 1] == b' ') {
                let end = d[i..].find(' ').map_or(d.len(), |e| i + e);
                let next = d[end..].find('-').map_or(header.len().max(d.len()), |e| end + e);
                let title = header.get(i..next.min(header.len())).unwrap_or("").trim().to_string();
                cols.push((title, i));
                i = end;
            } else {
                i += 1;
            }
        }
    } else {
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
    }
    let start = if dashes.is_some() { h + 2 } else { h + 1 };
    // Datenzeilen bis zur Leerzeile; Summenzeilen wie „Number of Clients   :0“ gehören nicht dazu.
    let summary = |l: &str| l.split_once(':').is_some_and(|(k, v)| k.ends_with(' ') && !v.starts_with(' ') && k.trim().chars().all(|c| c.is_alphanumeric() || c == ' '));
    let rows = lines[start..].iter().copied().take_while(|l| !l.trim().is_empty()).filter(|l| !summary(l)).collect();
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

/// MAC, IP und Name der APs aus der AP-Tabelle von `show summary` („MAC  IP Address  Name  Clients …“, wie
/// `ap_quick_inventory.py`). Die Tabelle der Anwender darüber („MAC  Name  IP Address  ESSID …“) hat keine Spalte
/// „Clients“ und wird so nicht verwechselt.
pub fn summary_aps(summary: &str) -> Vec<(String, String, String)> {
    let Some((cols, rows)) = table(summary, &["MAC", "IP Address", "Name", "Clients"]) else { return Vec::new() };
    rows.iter()
        .filter_map(|row| {
            let mac = cell(row, &cols, "MAC")?.to_ascii_lowercase();
            let ip = cell(row, &cols, "IP Address")?.trim_end_matches('*').to_string();
            let name = cell(row, &cols, "Name").unwrap_or_default().to_string();
            is_mac(&mac).then_some((mac, ip, name))
        })
        .collect()
}

/// Cluster-Karte: APs aus `show aps` (online) und aus `allowed-ap` ohne Eintrag in `show aps` (offline).
/// Offline-APs kennt nur ihre MAC. Die MAC der online-APs kommt aus `show summary` (über die IP, sonst den Namen)
/// oder aus dem Namen, wenn der AP wie seine MAC heißt. Bleibt ein online-AP ohne MAC, ist die Zuordnung nicht
/// sicher (`offline_uncertain`).
pub fn cluster_json(show_aps: &str, running_config: &str, summary: &str, vc_ip: &str) -> Value {
    let known = summary_aps(summary);
    let mut aps = Vec::new();
    if let Some((cols, rows)) = table(show_aps, &["Name", "IP Address"]) {
        for row in rows {
            let Some(name) = cell(row, &cols, "Name") else { continue };
            let Some(ip_raw) = cell(row, &cols, "IP Address") else { continue };
            let ip = ip_raw.trim_end_matches('*');
            if segment_of(ip).is_none() {
                continue;
            }
            let mac = known
                .iter()
                .find(|(_, kip, _)| kip == ip)
                .or_else(|| known.iter().find(|(_, _, kname)| kname == name))
                .map(|(m, _, _)| m.clone())
                .or_else(|| is_mac(name).then(|| name.to_ascii_lowercase()));
            aps.push(json!({
                "name": name,
                "mac": mac,
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
    let online_macs: Vec<String> = aps.iter().filter_map(|a| a["mac"].as_str()).map(str::to_string).collect();
    let all_macs = online_macs.len() == aps.len();
    let allowed = allowed_aps(running_config);
    let offline: Vec<String> = allowed.iter().filter(|m| !online_macs.contains(m)).cloned().collect();
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
        // Fehlt einem online-AP die MAC, kann eine nicht gefundene MAC auch zu ihm gehören.
        list.push(json!({ "name": m, "ip": null, "online": false, "uncertain": !all_macs, "conductor": false, "clients": null, "in_segment": true }));
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

/// Admin-Konten aus der laufenden Konfiguration: `hash-mgmt-user <Name> …` ohne `usertype` (eine Zeile mit
/// `usertype read-only` oder `usertype guest-provisioning` ist kein Admin).
pub fn admin_users(running_config: &str) -> Vec<String> {
    running_config
        .lines()
        .filter_map(|l| l.trim().strip_prefix("hash-mgmt-user "))
        .filter(|rest| !rest.contains("usertype"))
        .filter_map(|rest| rest.split_whitespace().next().map(str::to_string))
        .collect()
}

/// Dürfen schreibende Werkzeuge (Neustart, Update) an diesem Cluster laufen? Nicht bei Verwaltung über Central oder
/// AirWave (oder unbekannter Verwaltung) und nur, wenn der Benutzer aus der KDBX ein Admin-Konto ist.
pub fn write_check(managed: Option<&str>, admins: &[String], user: &str) -> Result<(), String> {
    match managed {
        Some("none") => {}
        Some("central") => return Err("wird über Central verwaltet".into()),
        Some("airwave") => return Err("wird über AirWave verwaltet".into()),
        _ => return Err("Verwaltung unbekannt (show summary ohne Antwort)".into()),
    }
    if admins.iter().any(|a| a == user) {
        Ok(())
    } else {
        Err(format!("„{user}“ ist kein Admin-Konto (hash-mgmt-user) in der Konfiguration"))
    }
}

/// Log-Befehl: `show log system` bzw. `show log security`, wahlweise mit `| include <Filter>`. Im Filter sind
/// Steuerzeichen und `|` nicht erlaubt (kein zweiter Befehl), führende und folgende Leerzeichen werden entfernt.
pub fn log_command(kind: &str, filter: Option<&str>) -> Result<String, String> {
    let base = match kind {
        "system" => "show log system",
        "security" => "show log security",
        other => return Err(format!("Unbekanntes Log „{other}“")),
    };
    match filter.map(str::trim).filter(|f| !f.is_empty()) {
        None => Ok(base.to_string()),
        Some(f) if f.chars().any(|c| c.is_control() || c == '|') => Err("Der Filterbegriff darf keine Steuerzeichen und kein „|“ enthalten".into()),
        Some(f) if f.chars().count() > 200 => Err("Der Filterbegriff ist zu lang (höchstens 200 Zeichen)".into()),
        Some(f) => Ok(format!("{base} | include {f}")),
    }
}

/// Befehle für die Seite eines einzelnen APs (am AP selbst, mit den Zugangsdaten des VC).
/// Befehle für „Nutzer anzeigen“ bzw. „Nutzer finden“ am VC: alle Anwender des Clusters und die APs (für den Sprung
/// vom Anwender zu seinem AP).
pub const CLIENT_COMMANDS: [&str; 3] = ["no paging", "show clients", "show aps"];

/// Anwender des Clusters (`show clients` am VC) als Tabelle; `ap_col` ist die Spalte „Access Point“, `aps` ordnet
/// AP-Namen (klein geschrieben) die Einträge aus `show aps` zu, wie sie die AP-Seite braucht.
pub fn clients_json(show_clients: &str, show_aps: &str, vc_ip: &str) -> Value {
    let mut t = table_json(show_clients, &["Name", "IP Address", "MAC Address"]);
    let ap_col = t["columns"].as_array().and_then(|c| c.iter().position(|n| n == "Access Point"));
    let cluster = cluster_json(show_aps, "", "", vc_ip);
    let aps: serde_json::Map<String, Value> = cluster["aps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["name"].as_str()?.to_lowercase(), a.clone())))
        .collect();
    t["ap_col"] = json!(ap_col);
    t["aps"] = Value::Object(aps);
    t
}

pub const AP_COMMANDS: [&str; 5] = ["no paging", "show version", "show interface", "show network", "show clients"];

/// MAC des APs aus `show interface`: bond0, sonst eth0 (wie `ap_list.py`).
pub fn interface_mac(out: &str) -> Option<String> {
    let mut eth0 = None;
    let mut current = "";
    for line in out.lines() {
        let t = line.trim();
        if let Some(name) = ["bond0", "eth0", "eth1", "bond1"].iter().find(|n| t.starts_with(&format!("{n} is "))) {
            current = name;
        }
        if let Some(rest) = t.split("address is ").nth(1) {
            let mac = rest.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
            if is_mac(&mac) {
                match current {
                    "bond0" => return Some(mac),
                    "eth0" if eth0.is_none() => eth0 = Some(mac),
                    _ => {}
                }
            }
        }
    }
    eth0
}

/// Tabelle allgemein (Spalten wie in der Kopfzeile, Werte zwischen den Spaltenanfängen), für die Anzeige.
pub fn table_json(out: &str, must: &[&str]) -> Value {
    let Some((cols, rows)) = table(out, must) else { return json!({ "columns": [], "rows": [] }) };
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            cols.iter()
                .enumerate()
                .map(|(i, (_, start))| {
                    let end = cols.get(i + 1).map_or(row.len(), |(_, e)| *e).min(row.len());
                    row.get((*start).min(row.len())..end).unwrap_or("").trim().to_string()
                })
                .collect()
        })
        .filter(|r: &Vec<String>| r.iter().any(|c| !c.is_empty()))
        .collect();
    json!({ "columns": cols.iter().map(|(n, _)| n).collect::<Vec<_>>(), "rows": rows })
}

/// Seite eines APs aus den Ausgaben von [`AP_COMMANDS`]. Anwender nur dieses APs, wenn die Tabelle eine Spalte
/// „Access Point“ hat (am Conductor listet `show clients` den ganzen Cluster).
pub fn ap_json(outputs: &[String], name: &str, ip: &str) -> Value {
    let get = |i: usize| outputs.get(i).map(String::as_str).unwrap_or("");
    let version = crate::overview::remote::parse_aruba(get(1)).unwrap_or_default();
    let field = |k: &str| version.iter().find(|(f, _)| *f == k).map(|(_, v)| v.clone());
    let networks = table_json(get(3), &["Profile Name"]);
    let mut clients = table_json(get(4), &["IP Address", "MAC Address"]);
    if let Some(col) = clients["columns"].as_array().and_then(|c| c.iter().position(|x| x == "Access Point")) {
        if let Some(rows) = clients["rows"].as_array_mut() {
            rows.retain(|r| r[col] == name || r[col] == ip);
        }
    }
    json!({
        "name": name,
        "ip": ip,
        "system": field("system"),
        "uptime": field("uptime"),
        "mac": interface_mac(get(2)),
        "networks": networks,
        "clients": clients,
    })
}

/// Nur Vorführung (Bildschirmfotos): erfundener Cluster mit 24 APs und eine AP-Seite.
pub fn demo() -> (Value, Value) {
    // Namensspalte 30 Zeichen breit wie bei langen AP-Namen am Gerät.
    let mut rows = String::from("Name                          IP Address       Mode    Spectrum  Clients  Type         Serial #\n----                          ----------       ----    --------  -------  ----         --------\n");
    let named = [(3, "aula"), (4, "mensa"), (7, "lehrerzimmer-altbau-og2-AP07")];
    let off = [16, 21, 23];
    let clients = [0, 12, 5, 31, 18, 0, 3, 9, 14, 2, 0, 7, 11, 6, 0, 4, 0, 8, 1, 22, 3, 0, 5, 0];
    let mut config = String::from("version 8.10.0.0-8.10.0\n");
    let mut summary = String::from("MAC                IP Address       Name                          Clients\n---                ----------       ----                          -------\n");
    for i in 0..24usize {
        let mac = format!("02:00:5e:00:{:02x}:{:02x}", 0x40 + i, (i * 37 + 18) % 256);
        config.push_str(&format!("allowed-ap {mac}\n"));
        if off.contains(&i) {
            continue;
        }
        let name = named.iter().find(|(n, _)| *n == i).map_or(mac.clone(), |(_, n)| n.to_string());
        let ip = if i == 13 { "198.51.100.7".to_string() } else { format!("192.0.2.{}", 180 + i * 2) };
        let ip = if i == 0 { format!("{ip}*") } else { ip };
        summary.push_str(&format!("{mac:<19}{:<17}{name:<30}{}\n", ip.trim_end_matches('*'), clients[i]));
        rows.push_str(&format!("{name:<30}{ip:<17}access  disable   {:<9}515(indoor)  CNZZQX{:04}\n", clients[i], i + 1));
    }
    let mut cluster = cluster_json(&rows, &config, &summary, "192.0.2.240");
    cluster["managed"] = json!("central");
    cluster["write"] = json!({ "ok": false, "reason": "wird über Central verwaltet" });
    cluster["at"] = json!(crate::overview::inventory::now());
    let outputs = vec![
        String::new(),
        "ArubaOS (MODEL: 515), Version 8.10.0.23 LSR\nAP uptime is 3 days 4 hours 12 minutes 2 seconds".to_string(),
        "bond0 is up\nHardware is Ethernet, address is 02:00:5e:00:41:37".to_string(),
        "Profile Name  ESSID       Clients  Type      Band  Authentication Method\n------------  -----       -------  ----      ----  ---------------------\nschule-1x     Schule      9        employee  all   802.1x\ngast          Gast-WLAN   3        guest     all   None".to_string(),
        "Name        IP Address    MAC Address        OS     ESSID      Access Point       Channel\n----        ----------    -----------        --     -----      ------------       -------\nanna        198.51.100.5  02:00:5e:00:10:01  Win10  Schule     02:00:5e:00:41:37  36E\nbernd       198.51.100.6  02:00:5e:00:10:02  iOS    Gast-WLAN  02:00:5e:00:41:37  1\nclara       198.51.100.9  02:00:5e:00:10:03  Win11  Schule     02:00:5e:00:41:37  36E".to_string(),
    ];
    let mut ap = ap_json(&outputs, "02:00:5e:00:41:37", "192.0.2.182");
    ap["at"] = json!(crate::overview::inventory::now());
    ap["complete"] = json!(true);
    (cluster, ap)
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
        let c = cluster_json(SHOW_APS, CONFIG, "", "192.0.2.200");
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
    fn ap_page_from_version_interface_networks_and_clients() {
        let outputs = vec![
            String::new(),
            "ArubaOS (MODEL: 515), Version 8.10.0.23 LSR\nAP uptime is 3 hours 56 minutes 26 seconds".to_string(),
            "eth0 is up, line protocol is up\nHardware is Gigabit Ethernet, address is 02:00:5E:00:53:12\nbond0 is up\nHardware is Ethernet, address is 02:00:5e:00:53:99".to_string(),
            "Networks\n--------\nProfile Name  ESSID       Clients  Type      Band  Authentication Method\n------------  -----       -------  ----      ----  ---------------------\nschule-1x     Schule      7        employee  all   802.1x\ngast          Gast-WLAN   2        guest     all   None".to_string(),
            "Client List\n-----------\nName        IP Address    MAC Address        OS     ESSID   Access Point       Channel\n----        ----------    -----------        --     -----   ------------       -------\nanna        198.51.100.5  02:00:5e:00:10:01  Win10  Schule  02:00:5e:00:53:12  36E\nbernd       198.51.100.6  02:00:5e:00:10:02  iOS    Gast    andere-ap          1".to_string(),
        ];
        let a = ap_json(&outputs, "02:00:5e:00:53:12", "192.0.2.210");
        assert_eq!(a["mac"], "02:00:5e:00:53:99", "bond0 vor eth0");
        assert!(a["uptime"].as_str().is_some_and(|u| u.contains("3 Std")), "{a}");
        assert_eq!(a["networks"]["rows"][1][1], "Gast-WLAN");
        assert_eq!(a["networks"]["rows"][0][5], "802.1x");
        let rows = a["clients"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "nur Anwender dieses APs");
        assert_eq!(rows[0][0], "anna");
    }

    #[test]
    fn show_network_and_empty_client_list_as_on_the_test_ap() {
        let net = "Networks\r\n--------\r\nProfile Name     ESSID            Clients  Type      Band          Authentication Method  Key Management  IP Assignment  Status   Zone  Coding   Active  Max IPv4 Users\r\n------------     -----            -------  ----      ----          ---------------------  --------------  -------------  ------   ----  ------   ------  -------------\r\nPROBESCHULE      PROBESCHULE      0        employee  2.4ghz,5ghz   None                   WPA2-AES        Default VLAN   Enabled  -     Default  Yes     N/A\r\nson-selfservice  son-selfservice  0        employee  2.4ghz        None                   none            VLAN 23        Enabled  -     Default  Yes     N/A";
        let t = table_json(net, &["Profile Name"]);
        assert_eq!(t["columns"][7], "IP Assignment");
        assert_eq!(t["rows"][0][7], "Default VLAN", "Wert mit Leerzeichen bleibt ganz");
        assert_eq!(t["rows"][1][1], "son-selfservice");
        assert_eq!(t["rows"].as_array().unwrap().len(), 2);
        let cl = "\r\nClient List\r\n-----------\r\nName  IP Address  MAC Address  OS  ESSID  Access Point  Channel  Type  Role  IPv6 Address  Signal  Speed (mbps)  \r\n----  ----------  -----------  --  -----  ------------  -------  ----  ----  ------------  ------  ------------  \r\nNumber of Clients   :0\r\nInfo timestamp      :72853";
        let c = table_json(cl, &["IP Address", "MAC Address"]);
        assert_eq!(c["rows"].as_array().unwrap().len(), 0, "{c}");
        assert_eq!(c["columns"][11], "Speed (mbps)");
    }

    #[test]
    fn management_from_show_summary() {
        assert_eq!(management("Name             :probe\nManaged Via      :Local\nAruba Central    :Not Set Up\nAirwave          :Not Set Up"), Some("none"));
        assert_eq!(management("Managed Via      :Aruba Central\nAruba Central    :Connected"), Some("central"));
        assert_eq!(management("Aruba Central    :connected"), Some("central"));
        assert_eq!(management("Managed Via      :AirWave\nAirwave          :Connected"), Some("airwave"));
        assert_eq!(management("Name :probe"), None);
    }

    #[test]
    fn write_actions_need_unmanaged_cluster_and_admin_account() {
        let cfg = "hash-mgmt-password\nhash-mgmt-user admin password hash 0123abcd\nhash-mgmt-user leser password hash 4567ef usertype read-only\n";
        let admins = admin_users(cfg);
        assert_eq!(admins, vec!["admin".to_string()]);
        assert!(write_check(Some("none"), &admins, "admin").is_ok());
        assert!(write_check(Some("none"), &admins, "leser").unwrap_err().contains("kein Admin-Konto"));
        assert!(write_check(Some("none"), &admins, "operator").is_err());
        assert!(write_check(Some("central"), &admins, "admin").unwrap_err().contains("Central"));
        assert!(write_check(Some("airwave"), &admins, "admin").unwrap_err().contains("AirWave"));
        assert!(write_check(None, &admins, "admin").is_err());
    }

    #[test]
    fn log_commands_with_filter() {
        assert_eq!(log_command("system", None).unwrap(), "show log system");
        assert_eq!(log_command("security", Some("  198.51.100.5 ")).unwrap(), "show log security | include 198.51.100.5");
        assert_eq!(log_command("system", Some("")).unwrap(), "show log system");
        assert!(log_command("system", Some("a | b")).is_err());
        assert!(log_command("system", Some("a\nreload all")).is_err());
        assert!(log_command("debug", None).is_err());
    }

    #[test]
    fn named_aps_make_offline_assignment_uncertain_and_no_allowed_list_means_unknown() {
        let named = SHOW_APS.replace("02:00:5e:00:53:13  ", "aula               ");
        let c = cluster_json(&named, CONFIG, "", "192.0.2.200");
        assert_eq!(c["offline_uncertain"], true);
        assert_eq!(c["offline"], 1, "4 freigegeben, 3 online");
        let none = cluster_json(SHOW_APS, "version 8.10\r\n", "", "192.0.2.200");
        assert_eq!((none["total"].as_u64(), none["offline"].as_u64(), none["allowed_known"].as_bool()), (Some(3), Some(0), Some(false)));
    }

    // Aufbau wie show summary am Test-AP: erst die Anwender (ohne Spalte „Clients“), dann die APs.
    const SUMMARY: &str = "Name                    :TEST-IVC\r\n\
MAC  Name  IP Address  ESSID  Access Point  \r\n\
---  ----  ----------  -----  ------------  \r\n\
\r\n\
3 Access Points\r\n\
---------------\r\n\
MAC                IP Address       Name               Clients   Need Antenna Config  \r\n\
---                ----------       ----               -------   -------------------  \r\n\
02:00:5e:00:53:12  192.0.2.210      MASTER-demo        7         No                   \r\n\
02:00:5e:00:53:13  192.0.2.211      aula               12        No                   \r\n\
02:00:5e:00:53:14  198.51.100.7     02:00:5e:00:53:14  0         No                   \r\n\
Restricted Management Access Subnets\r\n";

    #[test]
    fn clients_of_the_cluster_with_their_ap() {
        let clients = "\r\nClient List\r\n-----------\r\n\
Name    IP Address    MAC Address        OS     ESSID   Access Point       Channel  Type  Role    IPv6 Address  Signal    Speed (mbps)\r\n\
----    ----------    -----------        --     -----   ------------       -------  ----  ----    ------------  ------    ------------\r\n\
anna    198.51.100.5  02:00:5e:00:10:01  Win10  Schule  aula               36E      AC    schule  --            46(good)  173(good)\r\n\
bernd   198.51.100.6  02:00:5e:00:10:02  iOS    Gast    02:00:5e:00:53:12  1        GN    gast    --            27(good)  72(ok)\r\n\
Number of Clients   :2\r\n";
        let named = SHOW_APS.replace("02:00:5e:00:53:13  ", "aula               ");
        let c = clients_json(clients, &named, "192.0.2.200");
        assert_eq!(c["rows"].as_array().unwrap().len(), 2, "Summenzeile gehört nicht dazu");
        assert_eq!(c["ap_col"], 5);
        assert_eq!(c["rows"][0][10], "46(good)");
        assert_eq!(c["aps"]["aula"]["ip"], "192.0.2.211");
        assert_eq!(c["aps"]["02:00:5e:00:53:12"]["conductor"], true);
    }

    #[test]
    fn renamed_aps_get_their_mac_from_show_summary() {
        let named = SHOW_APS.replace("02:00:5e:00:53:12  192.0.2.210*", "MASTER-demo        192.0.2.210*").replace("02:00:5e:00:53:13  ", "aula               ");
        assert_eq!(summary_aps(SUMMARY).len(), 3);
        let c = cluster_json(&named, CONFIG, SUMMARY, "192.0.2.200");
        assert_eq!(c["offline_uncertain"], false);
        assert_eq!(c["offline"], 1);
        let aps = c["aps"].as_array().unwrap();
        assert_eq!((aps[0]["name"].as_str(), aps[0]["mac"].as_str()), (Some("MASTER-demo"), Some("02:00:5e:00:53:12")));
        assert_eq!(aps.len(), 4, "die MAC des umbenannten Masters erscheint nicht als Offline-Kachel");
        assert_eq!((aps[3]["name"].as_str(), aps[3]["online"].as_bool(), aps[3]["uncertain"].as_bool()), (Some("02:00:5e:00:53:15"), Some(false), Some(false)));
        // Ohne AP-Tabelle in show summary: unsicher wie bisher.
        assert_eq!(cluster_json(&named, CONFIG, "Name :x\r\n", "192.0.2.200")["offline_uncertain"], true);
    }
}
