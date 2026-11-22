//! Rechtstexte, fest ins Programm eingebettet, damit der Über-Dialog sie ohne Umweg über den Desktop zeigt:
//! Lizenz (Apache 2.0) und NOTICE, Bibliotheken Dritter (Übersicht und Lizenztexte) und – nur in der Fassung mit
//! Modulen – die Nutzungsbedingungen. Die Dateien liegen zusätzlich in den Paketen.

/// Text zu einem Schlüssel: `license`, `notice`, `third_party`, `third_party_licenses`, `terms`; sonst leer.
pub fn text(which: &str) -> &'static str {
    match which {
        "license" => include_str!("../../../LICENSE"),
        "notice" => include_str!("../../../NOTICE"),
        "third_party" => include_str!("../../../THIRD_PARTY.md"),
        "third_party_licenses" => include_str!("../../../THIRD_PARTY_LICENSES.txt"),
        "terms" => terms(),
        _ => "",
    }
}

fn terms() -> &'static str {
    include_str!("license/NUTZUNGSBEDINGUNGEN.txt")
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_texts_are_present() {
        assert!(super::text("license").contains("Apache License"));
        assert!(super::text("third_party").contains("| Paket | Version | Lizenz |"));
        assert!(super::text("third_party_licenses").len() > 100_000);
        assert_eq!(super::text("unbekannt"), "");
        assert!(!super::text("terms").is_empty());
    }
}
