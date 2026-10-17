//! Bildpuffer im geteilten Speicher, ohne Dateinamen.
//!
//! Der Worker legt anonymen Speicher an und schreibt Bildpunkte als BGRA32
//! hinein; die Oberfläche bildet ihn nur lesend ab.
//!
//! - Linux: `memfd_create` mit Versiegelung gegen Verkleinern und Vergrößern
//!   (`F_SEAL_SHRINK`, `F_SEAL_GROW`, `F_SEAL_SEAL`). Ein bösartiger Worker
//!   kann den Speicher nach der Übergabe nicht mehr verkürzen (sonst würde
//!   der Zugriff im Hauptprozess mit SIGBUS enden). Der Deskriptor wird über
//!   einen Unix-Socket (`SCM_RIGHTS`) übergeben, siehe [`crate::fdpass`].
//! - Windows: anonymes Dateimapping (Section-Objekt, lässt sich nicht
//!   verkleinern); der Hauptprozess dupliziert die Kennung aus dem Worker.
//!
//! Der Lesende prüft Größe und Versiegelung und lehnt andere Deskriptoren ab.

use std::io;

/// Bytes je Bildpunkt (BGRA).
pub const BPP: usize = 4;

/// Kennung eines Bildpuffers, die zwischen Prozessen weitergegeben wird.
#[cfg(unix)]
pub type SharedHandle = std::os::fd::OwnedFd;
#[cfg(windows)]
pub type SharedHandle = std::os::windows::io::OwnedHandle;

fn size_of(width: u32, height: u32) -> io::Result<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(BPP))
        .filter(|n| *n <= 1 << 30)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Bildgröße unzulässig"))
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::fs::File;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use memmap2::{Mmap, MmapMut};

    /// Versiegelungen, die ein Bildpuffer tragen muss.
    pub const REQUIRED_SEALS: libc::c_int = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW;

    pub struct Writer {
        pub fd: OwnedFd,
        pub map: MmapMut,
    }

    pub fn create(size: usize) -> io::Result<Writer> {
        // SAFETY: memfd_create mit festem, nullterminiertem Namen.
        let raw = unsafe { libc::memfd_create(c"remotejoe-bild".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: frischer Deskriptor, gehört ab hier `fd`.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let file = File::from(fd);
        file.set_len(size.max(1) as u64)?;
        // SAFETY: fcntl auf den eigenen Deskriptor.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, REQUIRED_SEALS | libc::F_SEAL_SEAL) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: Der Speicher gehört diesem Prozess, Größe ist versiegelt.
        let map = unsafe { MmapMut::map_mut(&file)? };
        Ok(Writer { fd: file.into(), map })
    }

    pub fn duplicate(fd: &OwnedFd) -> io::Result<OwnedFd> {
        fd.try_clone()
    }

    /// Prüft einen fremden Deskriptor: versiegelt gegen Verkleinern und
    /// Vergrößern, mindestens `need` Bytes groß.
    pub fn validate(fd: &OwnedFd, need: usize) -> io::Result<()> {
        // SAFETY: fcntl/fstat auf einen gültigen Deskriptor.
        let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
        if seals < 0 || seals & REQUIRED_SEALS != REQUIRED_SEALS {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Bildpuffer ist nicht versiegelt"));
        }
        let len = File::from(fd.try_clone()?).metadata()?.len();
        if len < need as u64 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Bildpuffer zu klein"));
        }
        Ok(())
    }

    pub struct Reader {
        pub map: Mmap,
    }

    pub fn open(fd: OwnedFd, need: usize) -> io::Result<Reader> {
        validate(&fd, need)?;
        let file = File::from(fd);
        // SAFETY: nur lesende Abbildung eines versiegelten Speichers; der
        // Inhalt kann sich während des Lesens ändern (Teilbild, unkritisch).
        let map = unsafe { memmap2::MmapOptions::new().len(need.max(1)).map(&file)? };
        Ok(Reader { map })
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

    use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Memory::{
        CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    pub struct View {
        ptr: MEMORY_MAPPED_VIEW_ADDRESS,
        len: usize,
    }

    // SAFETY: Die Abbildung gehört dem Besitzer von `View`; Zugriffe laufen
    // über `&`/`&mut`.
    unsafe impl Send for View {}
    unsafe impl Sync for View {}

    impl View {
        pub fn bytes(&self) -> &[u8] {
            // SAFETY: gültige Abbildung mit `len` Bytes.
            unsafe { std::slice::from_raw_parts(self.ptr.Value as *const u8, self.len) }
        }
        pub fn bytes_mut(&mut self) -> &mut [u8] {
            // SAFETY: gültige, beschreibbare Abbildung mit `len` Bytes.
            unsafe { std::slice::from_raw_parts_mut(self.ptr.Value as *mut u8, self.len) }
        }
    }

    impl Drop for View {
        fn drop(&mut self) {
            // SAFETY: Abbildung aus MapViewOfFile.
            unsafe { UnmapViewOfFile(self.ptr) };
        }
    }

    pub struct Writer {
        pub fd: OwnedHandle,
        pub map: View,
    }

    pub fn create(size: usize) -> io::Result<Writer> {
        let size = size.max(1);
        // SAFETY: anonymes Mapping im Auslagerungsspeicher.
        let h = unsafe { CreateFileMappingW(INVALID_HANDLE_VALUE, std::ptr::null(), PAGE_READWRITE, (size as u64 >> 32) as u32, size as u32, std::ptr::null()) };
        if h.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: frische Kennung, gehört ab hier `fd`.
        let fd = unsafe { OwnedHandle::from_raw_handle(h as _) };
        // SAFETY: Abbildung der eigenen Section.
        let ptr = unsafe { MapViewOfFile(h, FILE_MAP_ALL_ACCESS, 0, 0, size) };
        if ptr.Value.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Writer { fd, map: View { ptr, len: size } })
    }

    pub fn duplicate(fd: &OwnedHandle) -> io::Result<OwnedHandle> {
        fd.try_clone()
    }

    /// Dupliziert eine Kennung aus dem Prozess `from` in diesen Prozess.
    pub fn duplicate_from(from: HANDLE, value: u64) -> io::Result<OwnedHandle> {
        let mut out: HANDLE = std::ptr::null_mut();
        // SAFETY: `from` ist eine gültige Prozesskennung mit PROCESS_DUP_HANDLE.
        let ok = unsafe { DuplicateHandle(from, value as usize as HANDLE, GetCurrentProcess(), &mut out, 0, 0, DUPLICATE_SAME_ACCESS) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: frische Kennung in diesem Prozess.
        Ok(unsafe { OwnedHandle::from_raw_handle(out as _) })
    }

    pub fn validate(fd: &OwnedHandle, need: usize) -> io::Result<()> {
        // Eine Section lässt sich nicht verkleinern; die Abbildung mit der
        // vollen Länge scheitert, wenn sie zu klein oder keine Section ist.
        open(fd.try_clone()?, need).map(|_| ())
    }

    pub struct Reader {
        pub map: View,
    }

    pub fn open(fd: OwnedHandle, need: usize) -> io::Result<Reader> {
        let len = need.max(1);
        // SAFETY: nur lesende Abbildung; scheitert bei falscher Art oder Größe.
        let ptr = unsafe { MapViewOfFile(fd.as_raw_handle() as HANDLE, FILE_MAP_READ, 0, 0, len) };
        if ptr.Value.is_null() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Bildpuffer ist kein passendes Dateimapping"));
        }
        Ok(Reader { map: View { ptr, len } })
    }
}

#[cfg(windows)]
pub use imp::duplicate_from;

/// Beschreibbarer Bildpuffer (Worker-Seite).
pub struct FrameBufferWriter {
    inner: imp::Writer,
    pub width: u32,
    pub height: u32,
}

impl FrameBufferWriter {
    pub fn create(width: u32, height: u32) -> io::Result<Self> {
        let inner = imp::create(size_of(width, height)?)?;
        Ok(FrameBufferWriter { inner, width, height })
    }

    /// Eine weitere Kennung desselben Speichers (zur Weitergabe).
    pub fn share_handle(&self) -> io::Result<SharedHandle> {
        imp::duplicate(&self.inner.fd)
    }

    /// Kennung zum Senden (gehört weiter dem Puffer).
    pub fn handle(&self) -> &SharedHandle {
        &self.inner.fd
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        #[cfg(unix)]
        return &mut self.inner.map[..];
        #[cfg(windows)]
        return self.inner.map.bytes_mut();
    }

    /// Kopiert ein Rechteck aus zeilenweisen BGRA-Daten.
    pub fn blit(&mut self, x: u32, y: u32, w: u32, h: u32, src: &[u8], src_stride: usize) {
        let stride = self.width as usize * BPP;
        let (width, height) = (self.width, self.height);
        let px = self.pixels_mut();
        for row in 0..h as usize {
            let dy = y as usize + row;
            if dy >= height as usize {
                break;
            }
            let cols = (w.min(width.saturating_sub(x))) as usize * BPP;
            let d = dy * stride + x as usize * BPP;
            let s = row * src_stride;
            px[d..d + cols].copy_from_slice(&src[s..s + cols]);
        }
    }
}

/// Lesender Bildpuffer (Oberflächenseite).
pub struct FrameBufferReader {
    inner: imp::Reader,
    pub width: u32,
    pub height: u32,
}

impl FrameBufferReader {
    /// Prüft eine vom Worker erhaltene Kennung (Größe, Versiegelung), ohne
    /// sie abzubilden.
    pub fn validate(handle: &SharedHandle, width: u32, height: u32) -> io::Result<()> {
        imp::validate(handle, size_of(width, height)?)
    }

    /// Bildet eine vom Worker erhaltene Kennung nur lesend ab; unversiegelte
    /// oder zu kleine Puffer werden abgelehnt.
    pub fn open(handle: SharedHandle, width: u32, height: u32) -> io::Result<Self> {
        let inner = imp::open(handle, size_of(width, height)?)?;
        Ok(FrameBufferReader { inner, width, height })
    }

    pub fn pixels(&self) -> &[u8] {
        let n = self.width as usize * self.height as usize * BPP;
        #[cfg(unix)]
        return &self.inner.map[..n];
        #[cfg(windows)]
        return &self.inner.map.bytes()[..n];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_and_reader_share_memory() {
        let mut w = FrameBufferWriter::create(4, 3).unwrap();
        let r = FrameBufferReader::open(w.share_handle().unwrap(), 4, 3).unwrap();
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
    fn reader_rejects_too_small_and_absurd_sizes() {
        let w = FrameBufferWriter::create(2, 2).unwrap();
        assert!(FrameBufferReader::open(w.share_handle().unwrap(), 4, 4).is_err());
        assert!(FrameBufferReader::open(w.share_handle().unwrap(), u32::MAX, u32::MAX).is_err());
        assert!(FrameBufferReader::open(w.share_handle().unwrap(), 2, 2).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sealed_buffer_cannot_be_shrunk_and_unsealed_is_rejected() {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        let w = FrameBufferWriter::create(64, 64).unwrap();
        let r = FrameBufferReader::open(w.share_handle().unwrap(), 64, 64).unwrap();
        // Verkürzen (wie ein bösartiger Worker) scheitert.
        // SAFETY: ftruncate auf den eigenen Deskriptor.
        let rc = unsafe { libc::ftruncate(w.handle().as_raw_fd(), 0) };
        assert_eq!(rc, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM));
        // Versiegelung lässt sich nicht wieder entfernen oder ändern.
        // SAFETY: fcntl auf den eigenen Deskriptor.
        assert_eq!(unsafe { libc::fcntl(w.handle().as_raw_fd(), libc::F_ADD_SEALS, libc::F_SEAL_WRITE) }, -1);
        // Lesen nach dem Versuch funktioniert (kein SIGBUS).
        assert_eq!(r.pixels().len(), 64 * 64 * 4);
        assert_eq!(r.pixels()[64 * 64 * 4 - 1], 0);
        // Unversiegelter memfd und gewöhnliche Datei werden abgelehnt.
        // SAFETY: memfd_create mit festem Namen; der Deskriptor gehört `fd`.
        let fd = unsafe { OwnedFd::from_raw_fd(libc::memfd_create(c"fremd".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)) };
        std::fs::File::from(fd.try_clone().unwrap()).set_len(64 * 64 * 4).unwrap();
        let e = FrameBufferReader::open(fd, 64, 64).err().unwrap();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
        let f = tempfile::tempfile().unwrap();
        f.set_len(64 * 64 * 4).unwrap();
        assert!(FrameBufferReader::open(OwnedFd::from(f), 64, 64).is_err());
    }
}
