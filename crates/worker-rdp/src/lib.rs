//! RDP-Worker auf Grundlage von FreeRDP 3 (dynamisch gelinkt).

pub mod clip;

/// Gewolltes Ende einer Sitzung nach dem Grund des Servers (MS-RDPBCGR 2.2.5.1.1, Set Error Info):
/// Abmelden, Trennen, Übernahme durch eine andere Anmeldung, Zeitlimit. Dann wird nicht automatisch
/// neu verbunden. `None`: kein solcher Grund (z. B. Netz weg) – ein Abriss.
pub fn deliberate_end(error_info: u32) -> Option<&'static str> {
    Some(match error_info {
        0x01 => "Vom Administrator getrennt",
        0x02 => "Vom Administrator abgemeldet",
        0x03 => "Zeitlimit für Leerlauf auf dem Server",
        0x04 => "Zeitlimit für die Anmeldung auf dem Server",
        0x05 => "Von einer anderen Anmeldung übernommen",
        0x0B => "Getrennt durch Benutzer",
        0x0C => "Abgemeldet",
        _ => return None,
    })
}

#[cfg(test)]
mod end_tests {
    #[test]
    fn deliberate_end_reasons() {
        assert_eq!(super::deliberate_end(0x0C), Some("Abgemeldet"));
        assert_eq!(super::deliberate_end(0x0B), Some("Getrennt durch Benutzer"));
        assert_eq!(super::deliberate_end(0x05), Some("Von einer anderen Anmeldung übernommen"));
        // Kein Grund oder ein technischer Fehler: Abriss, Neuverbinden bleibt möglich.
        assert_eq!(super::deliberate_end(0), None);
        assert_eq!(super::deliberate_end(0x06), None);
        assert_eq!(super::deliberate_end(0x10), None);
    }
}
pub mod keymap;
#[cfg(not(rj_no_freerdp))]
pub mod session;

#[cfg(not(rj_no_freerdp))]
#[allow(non_camel_case_types, non_upper_case_globals, dead_code, non_snake_case)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/rjrdp.rs"));
}
