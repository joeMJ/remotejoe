//! Robustheit gegen beschädigte oder bösartige Quelldateien: Verschachtelung,
//! Entpackgröße, kreisförmige Elternverweise und zufällig veränderte Eingaben
//! (feste Startwerte, damit Läufe wiederholbar sind).

use std::io::Write;
use std::path::PathBuf;

use rj_importer::plan::plan;
use rj_importer::*;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

/// Kleiner Zufallsgenerator (xorshift64*), reicht für Mutationstests.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(rng: &mut Rng, src: &[u8]) -> Vec<u8> {
    let mut b = src.to_vec();
    for _ in 0..1 + rng.below(8) {
        if b.is_empty() {
            b.push(b'<');
        }
        let i = rng.below(b.len());
        match rng.below(5) {
            0 => b[i] = rng.next() as u8,
            1 => {
                b.remove(i);
            }
            2 => b.insert(i, b"<>&\"{}[]:,\x00\xff"[rng.below(12)]),
            3 => {
                let n = rng.below(64).min(b.len() - i);
                let chunk = b[i..i + n].to_vec();
                b.splice(i..i, chunk);
            }
            _ => b.truncate(i),
        }
    }
    b
}

fn run(name: &str, bytes: &[u8]) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(name);
    std::fs::write(&p, bytes).unwrap();
    if let Ok(doc) = read_file(&p) {
        let _ = plan(&doc, "Probe");
    }
}

#[test]
fn mutated_inputs_never_panic() {
    for (file, seed) in [
        ("beispiel.rdm", 1u64),
        ("beispiel-rdm.json", 2),
        ("beispiel-royal.json", 3),
        ("beispiel.rtsx", 4),
    ] {
        let src = std::fs::read(data(file)).unwrap();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed);
        for _ in 0..300 {
            run(file, &mutate(&mut rng, &src));
        }
    }
}

#[test]
fn deeply_nested_xml_is_rejected() {
    let mut x = String::from("<RDMExport>");
    x.push_str(&"<a>".repeat(200_000));
    let err = rj_importer::rdm::read_xml(&x).err().expect("Fehler erwartet");
    assert!(err.to_string().contains("zu tief"), "{err}");
}

#[test]
fn zip_bomb_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("bombe.rtsz");
    let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("doc.json", opts).unwrap();
    let zeros = vec![b' '; 1024 * 1024];
    for _ in 0..(MAX_FILE / (1024 * 1024) + 2) {
        z.write_all(&zeros).unwrap();
    }
    z.finish().unwrap();
    assert!(std::fs::metadata(&p).unwrap().len() < 1024 * 1024);
    let err = read_file(&p).err().expect("Fehler erwartet");
    assert!(err.to_string().contains("größer"), "{err}");
}

#[test]
fn cyclic_and_deep_parents_are_kept() {
    let mut doc = SourceDocument { format: "Probe".into(), ..Default::default() };
    // Kreis A → B → A mit einem Ziel in A.
    doc.items.push(SourceItem::Folder { id: "A".into(), parent: Some("B".into()), name: "A".into(), order: 0 });
    doc.items.push(SourceItem::Folder { id: "B".into(), parent: Some("A".into()), name: "B".into(), order: 1 });
    // Kette mit 200 Ebenen.
    for i in 0..200 {
        let parent = (i > 0).then(|| format!("k{}", i - 1));
        doc.items.push(SourceItem::Folder { id: format!("k{i}"), parent, name: format!("k{i}"), order: i });
    }
    let p = plan(&doc, "Probe");
    fn count(f: &rj_core::Folder) -> usize {
        f.children.iter().map(|c| match c {
            rj_core::Node::Folder(s) => 1 + count(s),
            _ => 1,
        }).sum()
    }
    assert_eq!(count(&p.folder), 202, "kein Ordner geht verloren");
    assert!(p.report.warnings.iter().any(|w| w.contains("kreisförmigem")), "{:?}", p.report.warnings);
}
