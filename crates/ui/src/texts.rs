//! Oberflächentexte aus einer Ressourcendatei (`res/texte.de.json`).

use std::collections::HashMap;
use std::sync::OnceLock;

static TEXTS: OnceLock<HashMap<String, String>> = OnceLock::new();

fn table() -> &'static HashMap<String, String> {
    TEXTS.get_or_init(|| serde_json::from_str(include_str!("../res/texte.de.json")).expect("Textdatei ist gültig"))
}

/// Text zu einem Schlüssel; fehlt er, erscheint der Schlüssel selbst.
pub fn text(key: &str) -> &str {
    table().get(key).map(String::as_str).unwrap_or(key)
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_and_unknown() {
        assert_eq!(super::text("state.connected"), "Verbunden");
        assert_eq!(super::text("gibt.es.nicht"), "gibt.es.nicht");
    }
}
