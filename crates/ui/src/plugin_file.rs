//! Erweiterungen aus einer Datei einspielen (Einstellungen → Updates → Erweiterungen, „Aus Datei einspielen …“):
//! für den Fall, dass der Download nicht klappt. Gleiche Regeln wie beim Download – nur mit Lizenz und laufender
//! Wartung, Signatur mit dem Update-Schlüssel, passende Plattform und Programmversion – und vorher eine Prüfung des
//! Zusammenspiels: Würde nach dem Einspielen eine Erweiterung nicht mehr zu einer anderen passen (Fassung der
//! Anschlussstelle, Mindestversion, fehlende Oberfunktion), wird nichts eingespielt; passende Fassungen gehören dann
//! gemeinsam ausgewählt. Eingespielt wird wie beim Download als wartendes Paket (`<id>.neu`), aktiv nach dem nächsten
//! Start; eine noch nicht geladene Erweiterung sofort.

use std::path::Path;

use rj_plugin_api::Manifest;
use rj_update::sign;
use serde_json::{json, Value};

use crate::app::App;
use crate::plugin_fetch::{dirs, installed_manifest, swap_pending, unpack_verified, verify_dir};
use crate::plugins::{check_compat, check_manifest, library_name, plugins_dir, resolve_order};

/// Was die Prüfung über die Umgebung wissen muss (für Tests getrennt vom echten Zustand).
pub struct Umgebung {
    pub key: sign::VerifyingKey,
    /// Plattform dieses Programms (`rj_update::platform()`).
    pub platform: String,
    /// Programmversion.
    pub current: String,
    /// Kennungen der lizenzierten Erweiterungen.
    pub lizenziert: Vec<String>,
    /// Installierte Erweiterungen (ein wartendes Paket zählt als installiert).
    pub installiert: Vec<Manifest>,
}

/// Ein geprüftes Paket.
struct Paket {
    datei: String,
    dateiname: String,
    manifest: Option<Manifest>,
    pruefungen: Vec<(String, bool, String)>,
}

impl Paket {
    fn ok(&self) -> bool {
        self.manifest.is_some() && self.pruefungen.iter().all(|(_, ok, _)| *ok)
    }
}

/// `remotejoe-<kennung>-<version>-<plattform>.<zip|tar.zst>` zerlegen: (Kennung, Rest „<version>-<plattform>“, Endung).
fn split_name(name: &str) -> Option<(&str, &str, &str)> {
    let rest = name.strip_prefix("remotejoe-")?;
    let (rest, ext) = if let Some(r) = rest.strip_suffix(".tar.zst") { (r, "tar.zst") } else { (rest.strip_suffix(".zip")?, "zip") };
    let (id, rest) = rest.split_once('-')?;
    Some((id, rest, ext))
}

/// Ein Paket prüfen: Name, Plattform, Signatur und Prüfsummen (entpackt in einen Probeordner), Version im Paket,
/// Programmversion und Schnittstelle, Lizenz, Verhältnis zur installierten Fassung.
fn pruefe_paket(pfad: &Path, u: &Umgebung) -> Paket {
    let dateiname = pfad.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut p = Paket { datei: pfad.display().to_string(), dateiname: dateiname.clone(), manifest: None, pruefungen: Vec::new() };
    let check = |p: &mut Paket, was: &str, ok: bool, text: String| p.pruefungen.push((was.to_string(), ok, text));
    let Some((id, rest, ext)) = split_name(&dateiname) else {
        check(&mut p, "Datei", false, "kein Paket einer Erweiterung (erwartet remotejoe-<name>-<version>-<plattform>.zip bzw. .tar.zst)".into());
        return p;
    };
    // Plattform: Endung und Name müssen zu diesem Programm passen.
    let want_ext = if u.platform.starts_with("windows") { "zip" } else { "tar.zst" };
    let Some(version) = rest.strip_suffix(&format!("-{}", u.platform)).filter(|_| ext == want_ext) else {
        let other = ["-linux-x86_64-ubuntu2404", "-linux-x86_64", "-windows-x86_64"].iter().find(|s| rest.ends_with(*s)).map(|s| s.trim_start_matches('-')).unwrap_or("unbekannt");
        check(&mut p, "Plattform", false, format!("Paket für {other}, dieses Programm ist {}", u.platform));
        return p;
    };
    check(&mut p, "Plattform", true, u.platform.clone());
    let data = match std::fs::read(pfad) {
        Ok(d) => d,
        Err(e) => {
            check(&mut p, "Datei", false, format!("nicht lesbar: {e}"));
            return p;
        }
    };
    // Signatur und Prüfsummen: in einen Probeordner entpacken und wie beim Laden prüfen.
    let probe = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => {
            check(&mut p, "Signatur", false, format!("Probeordner: {e}"));
            return p;
        }
    };
    let m = rj_update::archive::unpack(&dateiname, &data, probe.path()).map_err(|e| e.to_string()).and_then(|_| verify_dir(probe.path(), id, &u.key));
    let m = match m {
        Ok(m) => m,
        Err(e) => {
            check(&mut p, "Signatur", false, format!("ungültig – {e}"));
            return p;
        }
    };
    check(&mut p, "Signatur", true, "gültig (Update-Schlüssel)".into());
    if m.version != version {
        check(&mut p, "Version", false, format!("Paket enthält {}, der Dateiname nennt {version}", m.version));
    }
    match check_manifest(&m, &u.current) {
        Ok(()) => check(&mut p, "Programmversion", true, format!("braucht remotejoe {} oder neuer", m.min_remotejoe)),
        Err(e) => check(&mut p, "Programmversion", false, e),
    }
    let lizenz = u.lizenziert.iter().any(|x| x == &m.id);
    check(&mut p, "Lizenz", lizenz, if lizenz { "in der Lizenz enthalten".into() } else { "nicht in der Lizenz enthalten".into() });
    if let Some(inst) = u.installiert.iter().find(|x| x.id == m.id) {
        match (rj_update::Version::parse(&inst.version), rj_update::Version::parse(&m.version)) {
            (Ok(a), Ok(b)) if b < a => check(&mut p, "Installiert", false, format!("{} ist älter als die installierte Fassung {}", m.version, inst.version)),
            (Ok(a), Ok(b)) if b == a => check(&mut p, "Installiert", true, format!("{} ist schon installiert – wird erneut eingespielt", m.version)),
            _ => check(&mut p, "Installiert", true, format!("{} → {}", inst.version, m.version)),
        }
    }
    p.manifest = Some(m);
    p
}

/// Probleme des Zusammenspiels einer Menge von Erweiterungen (Kennung, Grund): Fassungen und Mindestversionen
/// ([`check_compat`]) und fehlende Oberfunktion ([`resolve_order`]).
fn probleme(set: &[Manifest]) -> Vec<(String, String)> {
    let mut v = check_compat(set);
    v.extend(resolve_order(set).1);
    v
}

/// Alle gewählten Pakete prüfen, dazu das Zusammenspiel mit dem Installierten nach dem Einspielen. Ergebnis für den
/// Prüfdialog: `ok` (alles passt), `pakete` (je Paket Name, installierte und neue Version, Prüfungen) und
/// `zusammenspiel` (neue Probleme nach dem Einspielen; vorher schon bestehende zählen nicht).
pub fn pruefen(pfade: &[String], u: &Umgebung) -> Value {
    let pakete: Vec<Paket> = pfade.iter().map(|p| pruefe_paket(Path::new(p), u)).collect();
    let neu: Vec<&Manifest> = pakete.iter().filter_map(|p| p.manifest.as_ref()).collect();
    let mut doppelt = Vec::new();
    for (i, m) in neu.iter().enumerate() {
        if neu[..i].iter().any(|x| x.id == m.id) && !doppelt.contains(&m.name) {
            doppelt.push(m.name.clone());
        }
    }
    let mut nachher: Vec<Manifest> = u.installiert.iter().filter(|m| !neu.iter().any(|n| n.id == m.id)).cloned().collect();
    nachher.extend(neu.iter().map(|m| (*m).clone()));
    let vorher = probleme(&u.installiert);
    let name_of = |id: &str| nachher.iter().find(|m| m.id == id).map(|m| m.name.clone()).unwrap_or_else(|| id.to_string());
    let mut zusammen: Vec<String> = probleme(&nachher).into_iter().filter(|x| !vorher.contains(x)).map(|(id, why)| {
        // Meldungen der Fassungsprüfung beginnen schon mit dem Namen; nur die übrigen bekommen ihn vorangestellt.
        let name = name_of(&id);
        if why.starts_with(&name) { why } else { format!("{name}: {why}") }
    }).collect();
    for name in doppelt {
        zusammen.push(format!("{name}: mehrfach ausgewählt – nur eine Fassung wählen"));
    }
    let alle_pakete_ok = !pakete.is_empty() && pakete.iter().all(Paket::ok);
    let ok = alle_pakete_ok && zusammen.is_empty();
    let installiert = |id: &str| u.installiert.iter().find(|m| m.id == id).map(|m| m.version.clone());
    json!({
        "ok": ok,
        "pakete": pakete.iter().map(|p| json!({
            "datei": p.datei,
            "dateiname": p.dateiname,
            "id": p.manifest.as_ref().map(|m| m.id.clone()),
            "name": p.manifest.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| p.dateiname.clone()),
            "neu": p.manifest.as_ref().map(|m| m.version.clone()),
            "installiert": p.manifest.as_ref().and_then(|m| installiert(&m.id)),
            "ok": p.ok(),
            "pruefungen": p.pruefungen.iter().map(|(was, ok, text)| json!({ "was": was, "ok": ok, "text": text })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "zusammenspiel": { "ok": zusammen.is_empty(), "probleme": zusammen, "hinweis": "Passende Fassungen zusammengehöriger Erweiterungen gemeinsam auswählen und einspielen." },
    })
}

/// Installierte Erweiterungen im Ordner der Erweiterungen (ein wartendes Paket zählt als installiert).
fn installierte() -> Vec<Manifest> {
    let Some(base) = plugins_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(base) else { return Vec::new() };
    let mut ids: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| !n.ends_with(".alt"))
        .map(|n| n.trim_end_matches(".neu").to_string())
        .collect();
    ids.sort();
    ids.dedup();
    ids.iter().filter_map(|id| installed_manifest(id)).collect()
}

impl App {
    fn umgebung(&self) -> Result<Umgebung, String> {
        if self.demo {
            return Err("Vorführung".into());
        }
        // Wie beim Download: nur mit Lizenz und laufender Wartung.
        if let Some(grund) = crate::license::wartung_grund() {
            return Err(grund);
        }
        let key_text = self.update.key_override.clone().unwrap_or_else(|| crate::update::UPDATE_KEY.to_string());
        Ok(Umgebung {
            key: sign::public_key_from_text(&key_text).map_err(|e| e.to_string())?,
            platform: rj_update::platform(),
            current: rj_update::CURRENT_VERSION.to_string(),
            lizenziert: crate::plugin_fetch::licensed_ids(),
            installiert: installierte(),
        })
    }

    /// Gewählte Pakete prüfen (Prüfdialog), ohne etwas zu ändern.
    pub fn plugins_file_check(&self, pfade: &[String]) -> Result<Value, String> {
        Ok(pruefen(pfade, &self.umgebung()?))
    }

    /// Gewählte Pakete erneut prüfen und, wenn alles passt, alle gemeinsam einspielen (wartendes Paket, aktiv nach dem
    /// nächsten Start; eine noch nicht geladene Erweiterung sofort). Scheitert eines, bleibt der alte Stand aller.
    pub fn plugins_file_install(&mut self, pfade: &[String]) -> Result<Value, String> {
        if !self.plugin_jobs.is_empty() {
            return Err("Die Erweiterungen werden gerade heruntergeladen – bitte danach erneut versuchen.".into());
        }
        let u = self.umgebung()?;
        let r = pruefen(pfade, &u);
        if !r["ok"].as_bool().unwrap_or(false) {
            return Err("Die Prüfung ist nicht bestanden – nichts eingespielt.".into());
        }
        let mut done: Vec<(String, String, String, std::path::PathBuf)> = Vec::new();
        for p in r["pakete"].as_array().into_iter().flatten() {
            let (id, version, datei, dateiname) = (p["id"].as_str().unwrap_or_default(), p["neu"].as_str().unwrap_or_default(), p["datei"].as_str().unwrap_or_default(), p["dateiname"].as_str().unwrap_or_default());
            let res = std::fs::read(datei).map_err(|e| format!("{dateiname}: {e}")).and_then(|data| unpack_verified(id, dateiname, &data, &u.key));
            match res {
                Ok(neu) => done.push((id.to_string(), version.to_string(), datei.to_string(), neu)),
                Err(e) => {
                    for (_, _, _, d) in &done {
                        let _ = std::fs::remove_dir_all(d);
                    }
                    return Err(format!("{dateiname}: {e} – nichts eingespielt, der bisherige Stand bleibt."));
                }
            }
        }
        let mut out = Vec::new();
        for (id, version, datei, neu) in done {
            let swapped = !crate::plugins::loaded().is_loaded(&id) && {
                swap_pending(neu.parent().unwrap_or(&neu));
                dirs(&id).is_some_and(|(d, _)| d.join(library_name(&id)).is_file())
            };
            eprintln!("remotejoe: Erweiterung {id} {version} von Hand eingespielt aus {datei}");
            let j = json!({ "state": "installed", "id": id, "version": version, "swapped": swapped, "neustart": true, "datei": datei });
            self.plugin_fetch_last.insert(id.clone(), j.clone());
            out.push(j);
        }
        Ok(json!({ "eingespielt": out }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rj_update::sign::{SIG, SUMS};

    const PLATFORM: &str = "linux-x86_64";

    fn mf(id: &str, version: &str, braucht: &[&str], bereit: &[&str], nutzt: &[&str]) -> Manifest {
        let v = |l: &[&str]| l.iter().map(|x| x.to_string()).collect();
        Manifest { id: id.into(), name: id.to_uppercase(), version: version.into(), api: rj_plugin_api::API_VERSION, min_remotejoe: "0.1.60".into(), braucht: v(braucht), stellt_bereit: v(bereit), nutzt: v(nutzt) }
    }

    /// Signiertes Paket im Format der Quelle (`remotejoe-<id>-<version>-<plattform>.tar.zst`) in `dir` schreiben.
    fn paket(dir: &Path, m: &Manifest, sk: &sign::SigningKey, platform: &str) -> String {
        let src = tempfile::tempdir().unwrap();
        let lib = library_name(&m.id);
        let mtext = serde_json::to_string(m).unwrap();
        std::fs::write(src.path().join(&lib), b"bibliothek").unwrap();
        std::fs::write(src.path().join(rj_plugin_api::MANIFEST_FILE), &mtext).unwrap();
        let sums = sign::sums_text(&[(lib, sign::sha256_hex(b"bibliothek")), (rj_plugin_api::MANIFEST_FILE.into(), sign::sha256_hex(mtext.as_bytes()))]);
        std::fs::write(src.path().join(SUMS), &sums).unwrap();
        std::fs::write(src.path().join(SIG), sign::sign(sk, sums.as_bytes())).unwrap();
        let p = dir.join(format!("remotejoe-{}-{}-{platform}.tar.zst", m.id, m.version));
        std::fs::write(&p, rj_update::archive::pack_tar_zst(src.path()).unwrap()).unwrap();
        p.display().to_string()
    }

    fn umgebung(sk: &sign::SigningKey, installiert: Vec<Manifest>) -> Umgebung {
        Umgebung { key: sk.verifying_key(), platform: PLATFORM.into(), current: "0.1.67".into(), lizenziert: vec!["basis".into(), "netz".into()], installiert }
    }

    fn pruefung<'a>(r: &'a Value, i: usize, was: &str) -> Option<&'a Value> {
        r["pakete"][i]["pruefungen"].as_array().unwrap().iter().find(|x| x["was"] == was)
    }

    /// Gültiges Paket: alle Prüfungen bestanden, installierte und neue Version genannt.
    #[test]
    fn valid_package_passes() {
        let sk = sign::generate().unwrap();
        let d = tempfile::tempdir().unwrap();
        let f = paket(d.path(), &mf("basis", "1.3.0", &[], &[], &[]), &sk, PLATFORM);
        let r = pruefen(&[f], &umgebung(&sk, vec![mf("basis", "1.2.2", &[], &[], &[])]));
        assert_eq!(r["ok"], true, "{r:#}");
        assert_eq!((r["pakete"][0]["installiert"].as_str(), r["pakete"][0]["neu"].as_str()), (Some("1.2.2"), Some("1.3.0")));
        assert_eq!(pruefung(&r, 0, "Signatur").unwrap()["ok"], true);
    }

    /// Manipuliert, fremd signiert, falsche Plattform, zu altes remotejoe, nicht lizenziert, älter als installiert,
    /// kein Paket: jeweils abgelehnt mit Grund.
    #[test]
    fn each_check_rejects_with_a_reason() {
        let sk = sign::generate().unwrap();
        let d = tempfile::tempdir().unwrap();
        let u = umgebung(&sk, vec![mf("netz", "1.2.0", &[], &[], &[])]);
        let fail = |f: String, was: &str, text: &str| {
            let r = pruefen(&[f], &u);
            assert_eq!(r["ok"], false, "{r:#}");
            let c = pruefung(&r, 0, was).unwrap_or_else(|| panic!("keine Prüfung {was}: {r:#}"));
            assert!(c["ok"] == false && c["text"].as_str().unwrap().contains(text), "{was}: {c}");
        };
        // Manipuliert: ein Byte im Archiv geändert.
        let f = paket(d.path(), &mf("basis", "1.3.0", &[], &[], &[]), &sk, PLATFORM);
        let mut b = std::fs::read(&f).unwrap();
        let n = b.len() / 2;
        b[n] ^= 0xff;
        std::fs::write(&f, b).unwrap();
        fail(f, "Signatur", "ungültig");
        let fremd = sign::generate().unwrap();
        fail(paket(d.path(), &mf("basis", "1.3.1", &[], &[], &[]), &fremd, PLATFORM), "Signatur", "ungültig");
        fail(paket(d.path(), &mf("basis", "1.3.2", &[], &[], &[]), &sk, "linux-x86_64-ubuntu2404"), "Plattform", "linux-x86_64-ubuntu2404");
        let mut neu = mf("basis", "1.3.3", &[], &[], &[]);
        neu.min_remotejoe = "0.2.0".into();
        fail(paket(d.path(), &neu, &sk, PLATFORM), "Programmversion", "0.2.0 oder neuer");
        fail(paket(d.path(), &mf("fremd", "1.0.0", &[], &[], &[]), &sk, PLATFORM), "Lizenz", "nicht in der Lizenz");
        fail(paket(d.path(), &mf("netz", "1.1.0", &[], &[], &[]), &sk, PLATFORM), "Installiert", "älter");
        let leer = d.path().join("notizen.zip");
        std::fs::write(&leer, b"x").unwrap();
        fail(leer.display().to_string(), "Datei", "kein Paket");
        // Schon installiert (gleiche Fassung): erlaubt, als Hinweis.
        let r = pruefen(&[paket(d.path(), &mf("netz", "1.2.0", &[], &[], &[]), &sk, PLATFORM)], &u);
        assert_eq!(r["ok"], true);
        assert!(pruefung(&r, 0, "Installiert").unwrap()["text"].as_str().unwrap().contains("schon installiert"));
    }

    /// Der Fall vom 23.11. umgekehrt: eine neue Oberfunktion allein würde die installierte Hersteller-Erweiterung
    /// unpassend machen → abgelehnt mit Hinweis; beide zusammen → in Ordnung. Fehlende Oberfunktion ebenso.
    #[test]
    fn combination_is_checked_before_anything_is_installed() {
        let sk = sign::generate().unwrap();
        let d = tempfile::tempdir().unwrap();
        let installiert = vec![mf("basis", "1.2.2", &[], &["adapter"], &[]), mf("netz", "1.2.0", &["basis"], &[], &["adapter"])];
        let u = umgebung(&sk, installiert);
        let basis = paket(d.path(), &mf("basis", "1.3.0", &[], &["adapter@3"], &[]), &sk, PLATFORM);
        let netz = paket(d.path(), &mf("netz", "1.3.0", &["basis>=1.3.0"], &[], &["adapter@3"]), &sk, PLATFORM);
        let r = pruefen(std::slice::from_ref(&basis), &u);
        assert_eq!(r["ok"], false);
        assert_eq!(r["pakete"][0]["ok"], true, "das Paket selbst ist in Ordnung");
        let p = r["zusammenspiel"]["probleme"][0].as_str().unwrap();
        assert!(p.starts_with("NETZ 1.2.0 passt nicht zu BASIS 1.3.0"), "{p}");
        // Nur die Hersteller-Erweiterung: braucht eine neuere Oberfunktion.
        let r = pruefen(std::slice::from_ref(&netz), &u);
        assert!(r["zusammenspiel"]["probleme"][0].as_str().unwrap().contains("braucht BASIS 1.3.0 oder neuer"), "{r:#}");
        // Beide zusammen.
        assert_eq!(pruefen(&[basis.clone(), netz], &u)["ok"], true);
        // Zweimal dieselbe Erweiterung.
        let basis2 = paket(d.path(), &mf("basis", "1.3.1", &[], &["adapter@3"], &[]), &sk, PLATFORM);
        assert!(pruefen(&[basis, basis2], &umgebung(&sk, vec![]))["zusammenspiel"]["probleme"][0].as_str().unwrap().contains("mehrfach"));
        // Hersteller-Erweiterung ohne installierte Oberfunktion.
        let r = pruefen(&[paket(d.path(), &mf("netz", "1.3.1", &["basis"], &[], &[]), &sk, PLATFORM)], &umgebung(&sk, vec![]));
        assert!(r["zusammenspiel"]["probleme"][0].as_str().unwrap().contains("„basis“ (nicht vorhanden)"), "{r:#}");
        // Leere Auswahl ist nie „ok“.
        assert_eq!(pruefen(&[], &umgebung(&sk, vec![]))["ok"], false);
    }
}
