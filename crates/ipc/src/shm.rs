//! Bildpuffer im geteilten Speicher.
//!
//! Der Worker legt eine Datei an (Linux: unter `/dev/shm`, sonst im
//! Temp-Ordner), bildet sie ab und schreibt Bildpunkte als BGRA32 hinein. Die
//! Oberfläche bildet dieselbe Datei nur lesend ab. Nach dem Abbilden auf
//! beiden Seiten wird die Datei gelöscht; die Abbildungen bleiben gültig.

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

use memmap2::{Mmap, MmapMut};

/// Bytes je Bildpunkt (BGRA).
pub const BPP: usize = 4;

fn shm_dir() -> PathBuf {
    let dev = Path::new("/dev/shm");
    if cfg!(target_os = "linux") && dev.is_dir() {
        dev.to_path_buf()
    } else {
        std::env::temp_dir()
    }
}

/// Beschreibbarer Bildpuffer (Worker-Seite).
pub struct FrameBufferWriter {
    path: PathBuf,
    map: MmapMut,
    pub width: u32,
    pub height: u32,
}

impl FrameBufferWriter {
    pub fn create(width: u32, height: u32) -> io::Result<Self> {
        let size = width as usize * height as usize * BPP;
        let path = shm_dir().join(format!("remotejoe-{}-{}.fb", std::process::id(), uuid::Uuid::new_v4().simple()));
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let f = opts.open(&path)?;
        f.set_len(size.max(1) as u64)?;
        // SAFETY: Die Datei ist neu angelegt, gehört diesem Prozess und wird
        // nur über diese Abbildung beschrieben.
        let map = unsafe { MmapMut::map_mut(&f)? };
        Ok(FrameBufferWriter { path, map, width, height })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.map[..]
    }

    /// Kopiert ein Rechteck aus zeilenweisen BGRA-Daten.
    pub fn blit(&mut self, x: u32, y: u32, w: u32, h: u32, src: &[u8], src_stride: usize) {
        let stride = self.width as usize * BPP;
        for row in 0..h as usize {
            let dy = y as usize + row;
            if dy >= self.height as usize {
                break;
            }
            let cols = (w.min(self.width.saturating_sub(x))) as usize * BPP;
            let d = dy * stride + x as usize * BPP;
            let s = row * src_stride;
            self.map[d..d + cols].copy_from_slice(&src[s..s + cols]);
        }
    }

    /// Entfernt den Dateinamen (nachdem die Gegenseite abgebildet hat).
    pub fn unlink(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for FrameBufferWriter {
    fn drop(&mut self) {
        self.unlink();
    }
}

/// Lesender Bildpuffer (Oberflächenseite).
pub struct FrameBufferReader {
    map: Mmap,
    pub width: u32,
    pub height: u32,
}

impl FrameBufferReader {
    /// Öffnet einen Bildpuffer. Der Pfad kommt vom Worker und wird deshalb
    /// geprüft: nur Dateien `remotejoe-*.fb` im Ordner für geteilten Speicher,
    /// keine symbolischen Verknüpfungen.
    pub fn open(path: &Path, width: u32, height: u32) -> io::Result<Self> {
        let name_ok = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("remotejoe-") && n.ends_with(".fb") && !n.contains(['/', '\\']));
        let dir_ok = path.parent() == Some(shm_dir().as_path());
        let meta = std::fs::symlink_metadata(path)?;
        if !name_ok || !dir_ok || !meta.file_type().is_file() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Bildpuffer an unerwartetem Ort"));
        }
        let f = OpenOptions::new().read(true).open(path)?;
        // SAFETY: nur lesende Abbildung; der Inhalt kann sich während des
        // Lesens ändern, was für Bilddaten unkritisch ist (Teilbild).
        let map = unsafe { Mmap::map(&f)? };
        if map.len() < width as usize * height as usize * BPP {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Bildpuffer zu klein"));
        }
        Ok(FrameBufferReader { map, width, height })
    }

    pub fn pixels(&self) -> &[u8] {
        &self.map[..self.width as usize * self.height as usize * BPP]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_and_reader_share_memory() {
        let mut w = FrameBufferWriter::create(4, 3).unwrap();
        let r = FrameBufferReader::open(w.path(), 4, 3).unwrap();
        w.unlink();
        let red = [0u8, 0, 255, 255].repeat(2 * 2);
        w.blit(1, 1, 2, 2, &red, 8);
        let px = r.pixels();
        assert_eq!(&px[(4 + 1) * 4..(4 + 1) * 4 + 4], &[0, 0, 255, 255]);
        assert_eq!(&px[0..4], &[0, 0, 0, 0]);
        // Rand wird abgeschnitten.
        w.blit(3, 2, 2, 2, &red, 8);
        assert_eq!(&r.pixels()[(2 * 4 + 3) * 4..], &[0, 0, 255, 255]);
    }

    #[test]
    fn reader_rejects_foreign_paths() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("remotejoe-1-x.fb");
        std::fs::write(&other, vec![0u8; 64]).unwrap();
        assert!(FrameBufferReader::open(&other, 2, 2).is_err());
        let w = FrameBufferWriter::create(2, 2).unwrap();
        let link = shm_dir().join(format!("remotejoe-link-{}.fb", std::process::id()));
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(w.path(), &link).unwrap();
            assert!(FrameBufferReader::open(&link, 2, 2).is_err());
            let _ = std::fs::remove_file(&link);
        }
        assert!(FrameBufferReader::open(w.path(), 2, 2).is_ok());
        let up = shm_dir().join("..").join(w.path().file_name().unwrap());
        assert!(FrameBufferReader::open(&up, 2, 2).is_err());
    }
}
