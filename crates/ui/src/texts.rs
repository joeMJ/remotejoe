//! Oberflächentexte aus Ressourcendateien: `res/texte.de.json` (Kern) und je Modul `res/texte.<modul>.de.json`,
//! eingebunden nur mit dem Modul (die Fassung ohne Module enthält ihre Texte nicht).

use std::collections::HashMap;
use std::sync::OnceLock;

static TEXTS: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Textdateien dieser Fassung.
fn sources() -> Vec<&'static str> {
    #[allow(unused_mut)]
    let mut v = vec![include_str!("../res/texte.de.json")];
    #[cfg(feature = "standorte")]
    v.push(include_str!("../res/texte.standorte.de.json"));
    #[cfg(feature = "aruba")]
    v.push(include_str!("../res/texte.aruba.de.json"));
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
        t.insert("app.title".into(), crate::modules::EDITION.into());
        t
    })
}

/// Text zu einem Schlüssel; fehlt er, erscheint der Schlüssel selbst.
pub fn text(key: &str) -> &str {
    table().get(key).map(String::as_str).unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    /// Textdatei; fehlt eine Moduldatei (Quellen ohne Module), ist sie leer.
    fn file(name: &str) -> HashMap<String, String> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("res").join(name);
        match std::fs::read_to_string(&p) {
            Ok(t) => serde_json::from_str(&t).unwrap(),
            Err(_) if name != "texte.de.json" => HashMap::new(),
            Err(e) => panic!("{}: {e}", p.display()),
        }
    }

    #[test]
    fn known_and_unknown() {
        assert_eq!(super::text("state.connected"), "Verbunden");
        assert_eq!(super::text("gibt.es.nicht"), "gibt.es.nicht");
        assert_eq!(super::text("app.title"), crate::modules::EDITION);
    }

    /// Die Kerndatei nennt keine Modulinhalte (Schlüssel und Texte).
    #[test]
    fn core_texts_do_not_mention_aruba() {
        let hits: Vec<String> = file("texte.de.json").into_iter().filter(|(k, v)| k.to_lowercase().contains("aruba") || v.to_lowercase().contains("aruba")).map(|(k, _)| k).collect();
        assert!(hits.is_empty(), "Aruba in texte.de.json: {hits:?}");
    }

    /// Kein Schlüssel steht in zwei Dateien.
    #[test]
    fn text_files_are_disjoint() {
        let core = file("texte.de.json");
        let st = file("texte.standorte.de.json");
        let ar = file("texte.aruba.de.json");
        for k in st.keys() {
            assert!(!core.contains_key(k) && !ar.contains_key(k), "{k} doppelt");
        }
        for k in ar.keys() {
            assert!(!core.contains_key(k), "{k} doppelt");
        }
    }

    /// Jeder Schlüssel, den die Oberfläche außerhalb der Module und außerhalb von `#ifdef RJ_…` nennt, steht in der
    /// Kerndatei (sonst zeigte die Fassung ohne Module den Schlüssel statt des Textes).
    #[test]
    fn core_sources_find_their_texts_in_the_core_file() {
        let core = file("texte.de.json");
        let mut all = core.clone();
        all.extend(file("texte.standorte.de.json"));
        all.extend(file("texte.aruba.de.json"));
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("cpp");
        let mut missing = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            let mut depth = 0usize;
            for line in text.lines() {
                let l = line.trim_start();
                if l.starts_with("#ifdef RJ_STANDORTE") || l.starts_with("#ifdef RJ_ARUBA") {
                    depth += 1;
                    continue;
                }
                if depth > 0 && l.starts_with("#if") {
                    depth += 1;
                    continue;
                }
                if depth > 0 && l.starts_with("#endif") {
                    depth -= 1;
                    continue;
                }
                if depth > 0 {
                    continue;
                }
                for part in line.split('"').skip(1).step_by(2) {
                    if all.contains_key(part) && !core.contains_key(part) {
                        missing.push(format!("{}: {part}", p.file_name().unwrap().to_string_lossy()));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "Fehlen in texte.de.json: {missing:?}");
    }
}
