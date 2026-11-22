//! Oberflächentexte aus Ressourcendateien: `res/texte.de.json` (Kern) und je Modul `res/texte.<modul>.de.json`,
//! eingebunden nur mit dem Modul (die Fassung ohne Module enthält ihre Texte nicht).

use std::collections::HashMap;
use std::sync::OnceLock;

static TEXTS: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Textdateien dieser Fassung.
fn sources() -> Vec<&'static str> {
    let mut v = vec![include_str!("../res/texte.de.json")];
    v.extend(crate::plugins::texts());
    v
}

fn table() -> &'static HashMap<String, String> {
    TEXTS.get_or_init(|| {
        let mut t = HashMap::new();
        for s in sources() {
            let part: HashMap<String, String> = serde_json::from_str(s).expect("Textdatei ist gültig");
            t.extend(part);
        }
        // Name der Fassung in Fenstertitel und Titelleiste.
        t.insert("app.title".into(), crate::plugins::APP_NAME.into());
        t
    })
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
        assert_eq!(super::text("app.title"), crate::plugins::APP_NAME);
    }
}
