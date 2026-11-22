//! Windows-Teile der Objektübersicht: MAC eines Nachbarn im selben Netzsegment, Ping und
//! Traceroute ohne Konsolenfenster, Ausgabe der Konsolenprogramme in UTF-8.
//! Nur Windows-Bordmittel (iphlpapi, kernel32), daher die Aufrufe direkt deklariert.

use std::ffi::c_void;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const NO_ERROR: u32 = 0;
const AF_INET6: u16 = 23;
/// `MIB_IPFORWARD_TYPE`: Ziel liegt direkt im angeschlossenen Netz.
const MIB_IPROUTE_TYPE_DIRECT: u32 = 3;
/// `NL_NEIGHBOR_STATE`: ab „Probe“ ist die Hardwareadresse bekannt (bis „Permanent“).
const NL_NEIGHBOR_STATE_PROBE: i32 = 2;
const NL_NEIGHBOR_STATE_PERMANENT: i32 = 6;

/// `MIB_IPFORWARDROW` (14 DWORDs).
#[repr(C)]
#[derive(Default)]
struct MibIpForwardRow {
    dest: u32,
    mask: u32,
    policy: u32,
    next_hop: u32,
    if_index: u32,
    kind: u32,
    proto: u32,
    age: u32,
    next_hop_as: u32,
    metric: [u32; 5],
}

/// `MIB_IPNET_ROW2`: Adresse (`SOCKADDR_INET`, 28 Bytes), Schnittstelle, Hardwareadresse, Zustand.
#[repr(C)]
struct MibIpNetRow2 {
    family: u16,
    port: u16,
    flowinfo: u32,
    addr: [u8; 16],
    scope_id: u32,
    if_index: u32,
    if_luid: u64,
    phys: [u8; 32],
    phys_len: u32,
    state: i32,
    flags: u8,
    reachability: u32,
}

/// `MIB_IPNET_TABLE2`: Anzahl, danach (8-Byte-ausgerichtet) die Zeilen.
#[repr(C)]
struct MibIpNetTable2 {
    count: u32,
    rows: [MibIpNetRow2; 1],
}

#[link(name = "iphlpapi")]
extern "system" {
    fn GetBestRoute(dest: u32, source: u32, row: *mut MibIpForwardRow) -> u32;
    fn SendARP(dest: u32, source: u32, mac: *mut c_void, len: *mut u32) -> u32;
    fn GetIpNetTable2(family: u16, table: *mut *mut MibIpNetTable2) -> u32;
    fn FreeMibTable(memory: *mut c_void);
}

#[link(name = "kernel32")]
extern "system" {
    fn GetOEMCP() -> u32;
    fn MultiByteToWideChar(cp: u32, flags: u32, src: *const u8, src_len: i32, dst: *mut u16, dst_len: i32) -> i32;
}

/// MAC-Adresse von `ip`, wenn das Ziel im selben Netzsegment liegt (`aa:bb:cc:dd:ee:ff`);
/// sonst, für eigene und Loopback-Adressen und ohne Antwort `None`.
/// IPv4 fragt per ARP nach, IPv6 liest die Nachbartabelle.
pub fn neighbor_mac(ip: IpAddr) -> Option<String> {
    match ip {
        IpAddr::V4(v4) => mac_v4(v4),
        IpAddr::V6(v6) => mac_v6(v6),
    }
}

fn mac_v4(ip: Ipv4Addr) -> Option<String> {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
        return None;
    }
    // Windows erwartet die Adresse in Netzwerk-Byte-Reihenfolge.
    let dest = u32::from_ne_bytes(ip.octets());
    let mut route = MibIpForwardRow::default();
    // SAFETY: `route` ist eine gültige, beschreibbare MIB_IPFORWARDROW.
    if unsafe { GetBestRoute(dest, 0, &mut route) } != NO_ERROR || route.kind != MIB_IPROUTE_TYPE_DIRECT {
        return None; // Ziel liegt hinter einem Router (oder keine Route): MAC nicht ermittelbar.
    }
    let mut mac = [0u8; 8];
    let mut len = mac.len() as u32;
    // SAFETY: Puffer und Länge passen zueinander; SendARP schreibt höchstens `len` Bytes.
    if unsafe { SendARP(dest, 0, mac.as_mut_ptr().cast(), &mut len) } != NO_ERROR {
        return None;
    }
    format_mac(&mac[..(len as usize).min(mac.len())])
}

fn mac_v6(ip: Ipv6Addr) -> Option<String> {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return None;
    }
    let mut table: *mut MibIpNetTable2 = std::ptr::null_mut();
    // SAFETY: GetIpNetTable2 legt die Tabelle an; sie wird unten mit FreeMibTable freigegeben.
    if unsafe { GetIpNetTable2(AF_INET6, &mut table) } != NO_ERROR || table.is_null() {
        return None;
    }
    let want = ip.octets();
    // SAFETY: `count` Zeilen folgen laut API zusammenhängend ab `rows`.
    let found = unsafe {
        let rows = std::slice::from_raw_parts((*table).rows.as_ptr(), (*table).count as usize);
        rows.iter()
            .find(|r| r.family == AF_INET6 && r.addr == want && (NL_NEIGHBOR_STATE_PROBE..=NL_NEIGHBOR_STATE_PERMANENT).contains(&r.state))
            .and_then(|r| format_mac(&r.phys[..(r.phys_len as usize).min(r.phys.len())]))
    };
    // SAFETY: `table` stammt von GetIpNetTable2 und wird genau einmal freigegeben.
    unsafe { FreeMibTable(table.cast()) };
    found
}

/// Sechs Bytes als `aa:bb:cc:dd:ee:ff`; leere oder Null-Adressen ergeben `None`.
fn format_mac(b: &[u8]) -> Option<String> {
    if b.len() != 6 || b.iter().all(|&x| x == 0) {
        return None;
    }
    Some(b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(":"))
}

/// Programm aus dem Systemordner (nicht über PATH gesucht), ohne Konsolenfenster.
fn system_command(exe: &str) -> Command {
    let path = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32").join(exe))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(exe));
    let mut cmd = Command::new(path);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// `ping.exe -n 4 -w 1000 <host>` ohne Konsolenfenster.
pub fn ping_command(host: &str) -> Command {
    let mut cmd = system_command("ping.exe");
    cmd.args(["-n", "4", "-w", "1000", host]);
    cmd
}

/// `tracert.exe -d -w 1000 -h 30 <host>` ohne Konsolenfenster.
pub fn trace_command(host: &str) -> Command {
    let mut cmd = system_command("tracert.exe");
    cmd.args(["-d", "-w", "1000", "-h", "30", host]);
    cmd
}

/// Ausgabe von ping/tracert nach UTF-8. Ohne eigenes Konsolenfenster schreiben die Programme
/// in der OEM-Codepage (z. B. 850: `0x84` = „ä“), bei systemweitem UTF-8 (Codepage 65001) in UTF-8.
pub fn decode_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    // SAFETY: reine Abfrage ohne Argumente.
    let cp = unsafe { GetOEMCP() };
    decode_codepage(bytes, cp)
}

fn decode_codepage(bytes: &[u8], cp: u32) -> String {
    let Ok(len) = i32::try_from(bytes.len()) else {
        return String::from_utf8_lossy(bytes).into_owned();
    };
    // SAFETY: erst die nötige Länge abfragen, dann in einen Puffer genau dieser Länge schreiben.
    let need = unsafe { MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if need <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut wide = vec![0u16; need as usize];
    let got = unsafe { MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), need) };
    if got <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    wide.truncate(got as usize);
    String::from_utf16_lossy(&wide)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_is_lowercase_with_colons() {
        assert_eq!(format_mac(&[0x00, 0x15, 0x5D, 0x0A, 0xBC, 0xFF]).as_deref(), Some("00:15:5d:0a:bc:ff"));
        assert_eq!(format_mac(&[0; 6]), None);
        assert_eq!(format_mac(&[1, 2, 3]), None);
    }

    #[test]
    fn no_mac_for_loopback_or_outside_the_segment() {
        assert_eq!(neighbor_mac(IpAddr::V4(Ipv4Addr::LOCALHOST)), None);
        assert_eq!(neighbor_mac(IpAddr::V6(Ipv6Addr::LOCALHOST)), None);
        // Dokumentationsnetz (RFC 5737): liegt nie im eigenen Segment, läuft über den Router.
        assert_eq!(neighbor_mac("192.0.2.1".parse().unwrap()), None);
    }

    #[test]
    fn mac_of_the_default_gateway_in_the_own_segment() {
        // Router zum Dokumentationsnetz = Standardgateway; ohne Netz gibt es nichts zu prüfen.
        let mut route = MibIpForwardRow::default();
        let dest = u32::from_ne_bytes([192, 0, 2, 1]);
        if unsafe { GetBestRoute(dest, 0, &mut route) } != NO_ERROR || route.next_hop == 0 {
            eprintln!("kein Standardgateway, Prüfung übersprungen");
            return;
        }
        let gw = Ipv4Addr::from(route.next_hop.to_ne_bytes());
        let mac = neighbor_mac(IpAddr::V4(gw)).expect("Gateway im eigenen Segment hat eine MAC");
        assert_eq!(mac.len(), 17, "{mac}");
        assert!(mac.split(':').all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))), "{mac}");
    }

    #[test]
    fn oem_output_becomes_utf8() {
        // „Länge: Größe“ in Codepage 850 und 437 (gleiche Umlaute), in 1252 und in UTF-8.
        let cp850 = b"L\x84nge: Gr\x94\xe1e";
        assert_eq!(decode_codepage(cp850, 850), "Länge: Größe");
        assert_eq!(decode_codepage(cp850, 437), "Länge: Größe");
        assert_eq!(decode_codepage(b"\xc4\xd6\xdc", 1252), "ÄÖÜ");
        assert_eq!(decode_codepage("Länge".as_bytes(), 65001), "Länge");
        assert_eq!(decode_output(b""), "");
        assert_eq!(decode_output(b"Ping 127.0.0.1"), "Ping 127.0.0.1");
    }

    #[test]
    fn ping_localhost_without_console_window() {
        let cmd = ping_command("127.0.0.1");
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-n", "4", "-w", "1000", "127.0.0.1"]);
        let out = ping_command("127.0.0.1").output().expect("ping.exe startet");
        let text = decode_output(&out.stdout);
        assert!(out.status.success(), "{text}");
        assert!(text.contains("127.0.0.1"), "{text}");
        assert!(!text.contains('\u{fffd}'), "Ersatzzeichen in der Ausgabe: {text}");
    }

    #[test]
    fn traceroute_localhost_without_console_window() {
        let cmd = trace_command("127.0.0.1");
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-d", "-w", "1000", "-h", "30", "127.0.0.1"]);
        let out = trace_command("127.0.0.1").output().expect("tracert.exe startet");
        let text = decode_output(&out.stdout);
        assert!(out.status.success(), "{text}");
        assert!(text.contains("127.0.0.1"), "{text}");
        assert!(!text.contains('\u{fffd}'), "Ersatzzeichen in der Ausgabe: {text}");
    }
}
