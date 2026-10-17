//! Pakete packen und sicher entpacken: `tar.zst` (Linux) und `zip` (Windows).
//! Beim Entpacken sind nur normale Dateien und Verzeichnisse mit relativen
//! Pfaden ohne `..` erlaubt; Verknüpfungen und Gerätedateien führen zum Abbruch.

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::{Result, UpdateError};

/// Höchstgröße des entpackten Inhalts.
const MAX_UNPACKED: u64 = 2 * 1024 * 1024 * 1024;

fn safe_path(dest: &Path, rel: &Path) -> Result<PathBuf> {
    if rel.as_os_str().is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(UpdateError::Archive(format!("unzulässiger Pfad „{}“", rel.display())));
    }
    Ok(dest.join(rel))
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let rel = p.strip_prefix(base).expect("unterhalb").to_path_buf();
        out.push(rel);
        if e.file_type()?.is_dir() {
            walk(&p, base, out)?;
        }
    }
    Ok(())
}

/// Packt den Inhalt von `src` (nicht den Ordner selbst) als `tar.zst`.
pub fn pack_tar_zst(src: &Path) -> Result<Vec<u8>> {
    let mut files = Vec::new();
    walk(src, src, &mut files)?;
    let mut tar = tar::Builder::new(Vec::new());
    tar.mode(tar::HeaderMode::Deterministic);
    tar.follow_symlinks(false);
    for rel in files {
        let full = src.join(&rel);
        let meta = fs::symlink_metadata(&full)?;
        if meta.is_dir() {
            tar.append_dir(&rel, &full)?;
        } else if meta.is_file() {
            tar.append_path_with_name(&full, &rel)?;
        } else {
            return Err(UpdateError::Archive(format!("„{}“ ist keine normale Datei", rel.display())));
        }
    }
    let raw = tar.into_inner()?;
    Ok(ruzstd::encoding::compress_to_vec(raw.as_slice(), ruzstd::encoding::CompressionLevel::Fastest))
}

/// Packt den Inhalt von `src` als `zip` (Windows-Pakete).
pub fn pack_zip(src: &Path) -> Result<Vec<u8>> {
    let mut files = Vec::new();
    walk(src, src, &mut files)?;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for rel in files {
        let name = rel.to_string_lossy().replace('\\', "/");
        let full = src.join(&rel);
        if full.is_dir() {
            z.add_directory(name, opts).map_err(|e| UpdateError::Archive(e.to_string()))?;
        } else {
            z.start_file(name, opts).map_err(|e| UpdateError::Archive(e.to_string()))?;
            z.write_all(&fs::read(&full)?)?;
        }
    }
    Ok(z.finish().map_err(|e| UpdateError::Archive(e.to_string()))?.into_inner())
}

/// Entpackt nach `dest` (muss leer sein oder fehlen); Format nach Dateiname.
pub fn unpack(name: &str, data: &[u8], dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    if name.ends_with(".zip") {
        unpack_zip(data, dest)
    } else if name.ends_with(".tar.zst") {
        unpack_tar_zst(data, dest)
    } else {
        Err(UpdateError::Archive(format!("unbekanntes Format „{name}“")))
    }
}

fn unpack_tar_zst(data: &[u8], dest: &Path) -> Result<()> {
    let dec = ruzstd::decoding::StreamingDecoder::new(data).map_err(|e| UpdateError::Archive(e.to_string()))?;
    let mut ar = tar::Archive::new(dec);
    let mut total = 0u64;
    for e in ar.entries().map_err(|e| UpdateError::Archive(e.to_string()))? {
        let mut e = e.map_err(|e| UpdateError::Archive(e.to_string()))?;
        let rel = e.path().map_err(|e| UpdateError::Archive(e.to_string()))?.into_owned();
        let out = safe_path(dest, &rel)?;
        let kind = e.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(&out)?;
        } else if kind.is_file() {
            total += e.size();
            if total > MAX_UNPACKED {
                return Err(UpdateError::Archive("entpackt zu groß".into()));
            }
            if let Some(p) = out.parent() {
                fs::create_dir_all(p)?;
            }
            let mut f = fs::File::create(&out)?;
            std::io::copy(&mut (&mut e).take(MAX_UNPACKED), &mut f)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = e.header().mode().unwrap_or(0o644) & 0o755;
                fs::set_permissions(&out, fs::Permissions::from_mode(mode))?;
            }
        } else {
            return Err(UpdateError::Archive(format!("„{}“: nur Dateien und Verzeichnisse erlaubt", rel.display())));
        }
    }
    Ok(())
}

fn unpack_zip(data: &[u8], dest: &Path) -> Result<()> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| UpdateError::Archive(e.to_string()))?;
    let mut total = 0u64;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| UpdateError::Archive(e.to_string()))?;
        let rel = f.enclosed_name().ok_or_else(|| UpdateError::Archive(format!("unzulässiger Pfad (Eintrag {})", i)))?;
        let out = safe_path(dest, &rel)?;
        if f.is_dir() {
            fs::create_dir_all(&out)?;
        } else if f.is_file() {
            total += f.size();
            if total > MAX_UNPACKED {
                return Err(UpdateError::Archive("entpackt zu groß".into()));
            }
            if let Some(p) = out.parent() {
                fs::create_dir_all(p)?;
            }
            std::io::copy(&mut (&mut f).take(MAX_UNPACKED), &mut fs::File::create(&out)?)?;
        } else {
            return Err(UpdateError::Archive(format!("Eintrag {}: nur Dateien und Verzeichnisse erlaubt", i)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip_and_traversal() {
        let src = tempfile::tempdir().unwrap();
        fs::write(src.path().join("remotejoe"), b"programm").unwrap();
        fs::create_dir(src.path().join("res")).unwrap();
        fs::write(src.path().join("res/texte äö.json"), "{}").unwrap();
        for (name, data) in [("p.tar.zst", pack_tar_zst(src.path()).unwrap()), ("p.zip", pack_zip(src.path()).unwrap())] {
            let dest = tempfile::tempdir().unwrap();
            unpack(name, &data, dest.path()).unwrap();
            assert_eq!(fs::read(dest.path().join("remotejoe")).unwrap(), b"programm");
            assert_eq!(fs::read_to_string(dest.path().join("res/texte äö.json")).unwrap(), "{}");
        }
        // Archiv mit „../“: Abbruch, nichts außerhalb geschrieben.
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_size(1);
        h.set_mode(0o644);
        let name = b"../ausbruch";
        h.as_old_mut().name[..name.len()].copy_from_slice(name);
        h.set_cksum();
        b.append(&h, &b"x"[..]).unwrap();
        let data = ruzstd::encoding::compress_to_vec(b.into_inner().unwrap().as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
        let base = tempfile::tempdir().unwrap();
        let dest = base.path().join("ziel");
        assert!(unpack("x.tar.zst", &data, &dest).is_err());
        assert!(!base.path().join("ausbruch").exists());
        // Symbolische Verknüpfung: Abbruch.
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        b.append_link(&mut h, "link", "/etc/passwd").unwrap();
        let data = ruzstd::encoding::compress_to_vec(b.into_inner().unwrap().as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
        assert!(unpack("x.tar.zst", &data, &base.path().join("z2")).is_err());
    }
}
