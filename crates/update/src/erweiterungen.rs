//! Index einer Erweiterung in der Update-Quelle (`/erweiterungen/<kennung>/<plattformordner>/index.json`): alle
//! Fassungen mit Version, Schnittstelle, Mindestversion von remotejoe, Abhängigkeiten und Dateiname. remotejoe wählt
//! die neueste passende Fassung; das Werkzeug `rj-update packen-plugin` pflegt den Index. Die signierten
//! `SHA256SUMS` des Ordners decken Index und Pakete.

use serde::{Deserialize, Serialize};

use crate::Version;

pub const INDEX_FILE: &str = "index.json";
/// Katalog aller Erweiterungen der Quelle (`/erweiterungen/katalog.json`, mit signierten `SHA256SUMS` daneben):
/// Kennung und Anzeigename. remotejoe nimmt ihn für Lizenzen ohne Modulliste (alle Erweiterungen) und für Namen
/// noch nicht installierter Erweiterungen; der Kern selbst nennt keine.
pub const KATALOG_FILE: &str = "katalog.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KatalogEintrag {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Katalog {
    #[serde(default)]
    pub erweiterungen: Vec<KatalogEintrag>,
}

impl Katalog {
    pub fn parse(text: &[u8]) -> Result<Katalog, String> {
        serde_json::from_slice(text).map_err(|e| format!("{KATALOG_FILE} unlesbar: {e}"))
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    pub fn name_of(&self, id: &str) -> Option<&str> {
        self.erweiterungen.iter().find(|e| e.id == id).map(|e| e.name.as_str())
    }

    pub fn ids(&self) -> Vec<String> {
        self.erweiterungen.iter().map(|e| e.id.clone()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fassung {
    pub version: String,
    pub api: u32,
    pub min_remotejoe: String,
    #[serde(default)]
    pub braucht: Vec<String>,
    pub datei: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Index {
    pub kennung: String,
    #[serde(default)]
    pub fassungen: Vec<Fassung>,
}

impl Index {
    pub fn parse(text: &[u8]) -> Result<Index, String> {
        serde_json::from_slice(text).map_err(|e| format!("{INDEX_FILE} unlesbar: {e}"))
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Neueste Fassung, die zu diesem Kern passt (Schnittstelle `api`, Mindestversion ≤ `current`).
    pub fn pick(&self, current: &Version, api: u32) -> Option<&Fassung> {
        // Eine Vorabversion zählt als die Version, auf die sie hinführt (wie beim Laden der Erweiterungen).
        let mut current = current.clone();
        current.pre.clear();
        self.fassungen
            .iter()
            .filter(|f| f.api == api && Version::parse(&f.min_remotejoe).is_ok_and(|m| m <= current))
            .filter_map(|f| Version::parse(&f.version).ok().map(|v| (v, f)))
            .max_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, f)| f)
    }

    /// Fassung eintragen oder ersetzen, neueste zuerst.
    pub fn put(&mut self, f: Fassung) {
        self.fassungen.retain(|x| x.version != f.version);
        self.fassungen.push(f);
        self.fassungen.sort_by(|a, b| Version::parse(&b.version).ok().cmp(&Version::parse(&a.version).ok()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Der Index liefert die neueste Fassung, die zu Schnittstelle und Programmversion passt; `put` ersetzt und
    /// sortiert; Text und zurück bleibt gleich.
    #[test]
    fn index_picks_the_newest_compatible_version() {
        let f = |v: &str, api: u32, min: &str| Fassung { version: v.into(), api, min_remotejoe: min.into(), braucht: vec![], datei: format!("remotejoe-x-{v}-linux-x86_64.tar.zst") };
        let mut idx = Index { kennung: "x".into(), fassungen: vec![f("1.0.0", 1, "0.1.60"), f("1.2.0", 1, "0.1.70"), f("1.1.0", 1, "0.1.60"), f("2.0.0", 2, "0.1.60")] };
        let cur = |s: &str| Version::parse(s).unwrap();
        assert_eq!(idx.pick(&cur("0.1.60"), 1).map(|x| x.version.as_str()), Some("1.1.0"), "1.2.0 braucht 0.1.70, 2.0.0 andere Schnittstelle");
        assert_eq!(idx.pick(&cur("0.1.70"), 1).map(|x| x.version.as_str()), Some("1.2.0"));
        assert_eq!(idx.pick(&cur("0.1.60"), 2).map(|x| x.version.as_str()), Some("2.0.0"));
        assert_eq!(idx.pick(&cur("0.1.59"), 1), None);
        idx.put(f("1.1.0", 1, "0.1.61"));
        assert_eq!(idx.fassungen.iter().map(|x| x.version.as_str()).collect::<Vec<_>>(), ["2.0.0", "1.2.0", "1.1.0", "1.0.0"]);
        assert_eq!(idx.pick(&cur("0.1.60"), 1).map(|x| x.version.as_str()), Some("1.0.0"), "1.1.0 braucht jetzt 0.1.61");
        assert_eq!(Index::parse(idx.to_text().as_bytes()).unwrap(), idx);
        assert!(Index::parse(b"{").is_err());
    }

    #[test]
    fn catalogue_round_trip_and_names() {
        let k = Katalog { erweiterungen: vec![KatalogEintrag { id: "basis".into(), name: "Basis".into() }, KatalogEintrag { id: "kind".into(), name: "Kind".into() }] };
        let k2 = Katalog::parse(k.to_text().as_bytes()).unwrap();
        assert_eq!(k2, k);
        assert_eq!(k.name_of("kind"), Some("Kind"));
        assert_eq!(k.ids(), ["basis", "kind"]);
        assert_eq!(Katalog::parse(b"{}").unwrap(), Katalog::default());
    }
}
