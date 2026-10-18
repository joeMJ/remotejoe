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

/// Ein „gewolltes“ Ende in den ersten Sekunden nach dem Verbinden ist meist eine Folge eines
/// abgebrochenen Anmeldevorgangs (z. B. Neuverbinden wegen Größenänderung), keine Abmeldung durch
/// den Benutzer: dann wie ein Abriss behandeln.
pub const DELIBERATE_AFTER: std::time::Duration = std::time::Duration::from_secs(15);

pub fn counts_as_deliberate(connected_for: Option<std::time::Duration>) -> bool {
    connected_for.is_some_and(|d| d >= DELIBERATE_AFTER)
}

/// Größenänderung, für die sich bei Servern ohne Größenänderung im Betrieb kein Neuverbinden lohnt
/// (bis 16 Bildpunkte je Richtung; das Bild wird dann skaliert).
pub fn small_resize(current: (u32, u32), wanted: (u32, u32)) -> bool {
    current.0.abs_diff(wanted.0) <= 16 && current.1.abs_diff(wanted.1) <= 16
}

#[cfg(test)]
mod end_tests {
    #[test]
    fn early_logoff_is_not_deliberate_and_small_resizes_are_skipped() {
        use std::time::Duration;
        assert!(!super::counts_as_deliberate(None));
        assert!(!super::counts_as_deliberate(Some(Duration::from_secs(3))));
        assert!(super::counts_as_deliberate(Some(Duration::from_secs(15))));
        // Beim Start: 2360x1270 → 2362x1278 (Kopfzeile des Tabs) – kein Neuverbinden.
        assert!(super::small_resize((2360, 1270), (2362, 1278)));
        assert!(super::small_resize((2362, 1278), (2362, 1240)) == false);
        assert!(!super::small_resize((1280, 800), (1600, 900)));
    }

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
