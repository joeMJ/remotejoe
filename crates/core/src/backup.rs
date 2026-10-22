//! Sicherungen der KDBX (Einstellungen → Sicherung): Kopien der verschlüsselten Datei (und eines eigenen
//! Index `.rjindex`) mit Zeitstempel im Namen, in einem Ordner `Sicherungen` neben der KDBX und wahlweise an einem
//! zweiten Ort (z. B. NAS). Aufgeräumt werden nur Dateien nach dem eigenen Namensmuster:
//!
//! - `<name>-JJJJMMTT-HHMMSS.kdbx` (nach dem Speichern): die neuesten `keep_last`, dazu die erste jedes Tages der
//!   letzten `keep_days` Tage;
//! - `<name>-vor-update-<Version>-JJJJMMTT-HHMMSS.kdbx` (vor einem Update): die neuesten `keep_update`.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Ordner der Sicherungen neben der KDBX.
pub const LOCAL_DIR: &str = "Sicherungen";

const UPDATE_MARK: &str = "vor-update-";

/// Wie viele Sicherungen bleiben.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub keep_last: usize,
    pub keep_days: usize,
    pub keep_update: usize,
}

/// Zeitstempel „JJJJMMTT-HHMMSS“.
fn valid_stamp(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 15 && b[8] == b'-' && b.iter().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit())
}

/// Dateiname einer Sicherung ohne Endung; `update`: Version, vor deren Installation gesichert wird.
pub fn base_name(stem: &str, stamp: &str, update: Option<&str>) -> String {
    match update {
        Some(v) => format!("{stem}-{UPDATE_MARK}{v}-{stamp}"),
        None => format!("{stem}-{stamp}"),
    }
}

/// Zeitstempel und Art einer Sicherung dieser KDBX (`stem`); `None` für fremde Dateien.
fn parse<'a>(file_name: &'a str, stem: &str) -> Option<(&'a str, bool)> {
    let rest = file_name.strip_prefix(stem)?.strip_prefix('-')?.strip_suffix(".kdbx")?;
    let stamp = rest.get(rest.len().checked_sub(15)?..)?;
    if !valid_stamp(stamp) {
        return None;
    }
    let head = &rest[..rest.len() - 15];
    if head.is_empty() {
        return Some((stamp, false));
    }
    let version = head.strip_prefix(UPDATE_MARK)?.strip_suffix('-')?;
    (!version.is_empty() && !version.contains(['/', '\\'])).then_some((stamp, true))
}

/// Schreibt die Dateien einer Sicherung (`files`: Endung und Inhalt) nach `dir`, je über eine temporäre Datei und
/// Umbenennen, damit nie eine halbe Sicherung unter dem endgültigen Namen liegt. Liefert den Pfad der KDBX-Kopie.
pub fn write(dir: &Path, base: &str, files: &[(&str, &[u8])]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let mut first = None;
    for (ext, data) in files {
        let path = dir.join(format!("{base}.{ext}"));
        let tmp = dir.join(format!(".{base}.{ext}.tmp{}", std::process::id()));
        let res = (|| {
            let mut f = File::create(&tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
            fs::rename(&tmp, &path)
        })();
        if let Err(e) = res {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        first.get_or_insert(path);
    }
    first.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "nichts zu sichern"))
}

/// Tage seit 1970-01-01 für ein Datum des gregorianischen Kalenders.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn day_number(stamp: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| stamp.get(r)?.parse::<i64>().ok();
    Some(days_from_civil(n(0..4)?, n(4..6)?, n(6..8)?))
}

/// Räumt `dir` nach `r` auf; `today` ist das heutige Datum als Zeitstempel („JJJJMMTT-…“ genügt). Gelöscht werden
/// nur Sicherungen dieser KDBX (`stem`), jeweils mit einem gleichnamigen `.rjindex`. Liefert die gelöschten Dateien.
pub fn prune(dir: &Path, stem: &str, today: &str, r: Retention) -> io::Result<Vec<PathBuf>> {
    let mut regular = Vec::new();
    let mut update = Vec::new();
    for e in fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some((stamp, is_update)) = parse(name, stem) {
            let item = (stamp.to_string(), e.path());
            if is_update {
                update.push(item);
            } else {
                regular.push(item);
            }
        }
    }
    // Neueste zuerst.
    regular.sort_by(|a, b| b.0.cmp(&a.0));
    update.sort_by(|a, b| b.0.cmp(&a.0));
    let mut keep = vec![false; regular.len()];
    for k in keep.iter_mut().take(r.keep_last.max(1)) {
        *k = true;
    }
    if let Some(today) = day_number(today) {
        // Je Tag die erste (älteste) Sicherung, für die letzten keep_days Tage einschließlich heute.
        for back in 0..r.keep_days as i64 {
            if let Some(i) = regular.iter().rposition(|(s, _)| day_number(s) == Some(today - back)) {
                keep[i] = true;
            }
        }
    }
    let mut gone = Vec::new();
    let doomed = regular.iter().zip(keep).filter(|(_, k)| !k).map(|(x, _)| x).chain(update.iter().skip(r.keep_update));
    for (_, path) in doomed {
        fs::remove_file(path)?;
        gone.push(path.clone());
        let index = path.with_extension("rjindex");
        if index.exists() {
            fs::remove_file(&index)?;
            gone.push(index);
        }
    }
    Ok(gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Retention = Retention { keep_last: 10, keep_days: 7, keep_update: 3 };

    fn touch(dir: &Path, name: &str) {
        fs::write(dir.join(name), b"x").unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        v.sort();
        v
    }

    #[test]
    fn names_and_parsing() {
        assert_eq!(base_name("ziele", "20261022-143000", None), "ziele-20261022-143000");
        assert_eq!(base_name("ziele", "20261022-143000", Some("0.1.20")), "ziele-vor-update-0.1.20-20261022-143000");
        assert_eq!(parse("ziele-20261022-143000.kdbx", "ziele"), Some(("20261022-143000", false)));
        assert_eq!(parse("ziele-vor-update-0.1.20-20261022-143000.kdbx", "ziele"), Some(("20261022-143000", true)));
        assert_eq!(parse("ziele-alt-20261022-143000.kdbx", "ziele"), None, "fremde Datei");
        assert_eq!(parse("ziele-20261022-143000.kdbx", "zie"), None);
        assert_eq!(parse("ziele.kdbx", "ziele"), None);
        assert_eq!(parse("ziele-20261022-1430.kdbx", "ziele"), None);
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 3, 1) - days_from_civil(2026, 2, 28), 1);
        assert_eq!(days_from_civil(2024, 3, 1) - days_from_civil(2024, 2, 28), 2, "Schaltjahr");
    }

    #[test]
    fn write_puts_kdbx_and_index_side_by_side() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("Sicherungen");
        let p = write(&dir, "ziele-20261022-143000", &[("kdbx", b"k"), ("rjindex", b"i")]).unwrap();
        assert_eq!(p, dir.join("ziele-20261022-143000.kdbx"));
        assert_eq!(names(&dir), ["ziele-20261022-143000.kdbx", "ziele-20261022-143000.rjindex"]);
    }

    #[test]
    fn keeps_last_ten_plus_first_of_each_of_seven_days_and_three_update_copies() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path();
        // Heute 15 Sicherungen, an den sechs Tagen davor je zwei, vor acht Tagen eine.
        for i in 0..15 {
            touch(dir, &format!("ziele-20261022-10{i:02}00.kdbx"));
        }
        touch(dir, "ziele-20261022-100000.rjindex");
        for day in 16..22 {
            touch(dir, &format!("ziele-202610{day}-080000.kdbx"));
            touch(dir, &format!("ziele-202610{day}-180000.kdbx"));
        }
        touch(dir, "ziele-20261014-080000.kdbx");
        for v in ["0.1.17", "0.1.18", "0.1.19", "0.1.20"] {
            touch(dir, &format!("ziele-vor-update-{v}-202610{}-090000.kdbx", &v[4..]));
        }
        touch(dir, "andere-20261001-080000.kdbx");
        touch(dir, "notizen.txt");
        let gone = prune(dir, "ziele", "20261022-110000", R).unwrap();
        let left = names(dir);
        // Heute: die neuesten 10 (100500 … 101400) und die erste des Tages (100000 mit Index).
        assert!(left.contains(&"ziele-20261022-100000.kdbx".to_string()));
        assert!(left.contains(&"ziele-20261022-100000.rjindex".to_string()));
        assert!(left.contains(&"ziele-20261022-100500.kdbx".to_string()));
        assert!(!left.contains(&"ziele-20261022-100100.kdbx".to_string()));
        // Die sechs Tage davor: je die erste, nicht die zweite.
        for day in 16..22 {
            assert!(left.contains(&format!("ziele-202610{day}-080000.kdbx")), "Tag {day}");
            assert!(!left.contains(&format!("ziele-202610{day}-180000.kdbx")), "Tag {day}");
        }
        assert!(!left.contains(&"ziele-20261014-080000.kdbx".to_string()), "älter als sieben Tage");
        // Update-Sicherungen: die neuesten drei.
        assert!(!left.contains(&"ziele-vor-update-0.1.17-20261017-090000.kdbx".to_string()));
        assert!(left.contains(&"ziele-vor-update-0.1.20-20261020-090000.kdbx".to_string()));
        // Fremdes bleibt.
        assert!(left.contains(&"andere-20261001-080000.kdbx".to_string()));
        assert!(left.contains(&"notizen.txt".to_string()));
        assert_eq!(left.len(), 11 + 6 + 3 + 1 + 2);
        assert_eq!(gone.len(), 4 + 6 + 1 + 1);
    }
}
