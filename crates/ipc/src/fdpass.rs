//! Übergabe von Bildpuffern vom Worker an den Hauptprozess.
//!
//! Unix: Neben den Standardkanälen erhält der Worker ein Ende eines
//! Unix-Socket-Paars (Nummer in der Umgebungsvariable [`FRAME_FD_ENV`]).
//! Über diesen Socket gehen nur Deskriptoren (`SCM_RIGHTS`) mit einer
//! laufenden Nummer; die Nachricht `SharedFrame` im Kanal nennt dieselbe
//! Nummer. Windows: Die Nachricht nennt die Kennung im Worker, der
//! Hauptprozess dupliziert sie (siehe [`crate::shm::duplicate_from`]).

use std::io;

use crate::shm::FrameBufferWriter;

/// Umgebungsvariable mit der Nummer des Socket-Endes im Worker.
pub const FRAME_FD_ENV: &str = "RJ_FRAME_FD";

#[cfg(unix)]
mod imp {
    use std::io;
    use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
    use std::os::unix::net::UnixStream;

    /// Socket-Paar mit Nachrichtengrenzen (`SOCK_SEQPACKET`), nicht vererbbar.
    pub fn pair() -> io::Result<(UnixStream, UnixStream)> {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: socketpair schreibt zwei neue Deskriptoren in `fds`.
        if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: frische Deskriptoren, je einem Besitzer übergeben.
        Ok(unsafe { (UnixStream::from_raw_fd(fds[0]), UnixStream::from_raw_fd(fds[1])) })
    }

    /// Sendet `fd` mit der Nummer `token`.
    pub fn send_fd(sock: &UnixStream, token: u64, fd: BorrowedFd<'_>) -> io::Result<()> {
        let data = token.to_le_bytes();
        let mut iov = libc::iovec { iov_base: data.as_ptr() as *mut _, iov_len: data.len() };
        // SAFETY: Puffer für genau eine Steuernachricht mit einem Deskriptor.
        unsafe {
            let space = libc::CMSG_SPACE(std::mem::size_of::<libc::c_int>() as u32) as usize;
            let mut cbuf = vec![0u8; space];
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = cbuf.as_mut_ptr() as *mut _;
            msg.msg_controllen = space as _;
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as u32) as _;
            std::ptr::write_unaligned(libc::CMSG_DATA(c) as *mut libc::c_int, fd.as_raw_fd());
            if libc::sendmsg(sock.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// Empfängt einen Deskriptor mit seiner Nummer. Nachrichten ohne
    /// Deskriptor oder mit mehreren werden verworfen (überzählige
    /// Deskriptoren geschlossen).
    pub fn recv_fd(sock: &UnixStream) -> io::Result<(u64, OwnedFd)> {
        loop {
            let mut data = [0u8; 8];
            let mut iov = libc::iovec { iov_base: data.as_mut_ptr() as *mut _, iov_len: data.len() };
            // SAFETY: Empfangspuffer für Steuernachrichten mit bis zu vier Deskriptoren.
            unsafe {
                let space = libc::CMSG_SPACE((4 * std::mem::size_of::<libc::c_int>()) as u32) as usize;
                let mut cbuf = vec![0u8; space];
                let mut msg: libc::msghdr = std::mem::zeroed();
                msg.msg_iov = &mut iov;
                msg.msg_iovlen = 1;
                msg.msg_control = cbuf.as_mut_ptr() as *mut _;
                msg.msg_controllen = space as _;
                let n = libc::recvmsg(sock.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC);
                if n < 0 {
                    return Err(io::Error::last_os_error());
                }
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                let mut fds = Vec::new();
                let mut c = libc::CMSG_FIRSTHDR(&msg);
                while !c.is_null() {
                    if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                        let count = ((*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / std::mem::size_of::<libc::c_int>();
                        for i in 0..count {
                            let raw = std::ptr::read_unaligned((libc::CMSG_DATA(c) as *const libc::c_int).add(i));
                            fds.push(OwnedFd::from_raw_fd(raw));
                        }
                    }
                    c = libc::CMSG_NXTHDR(&msg, c);
                }
                if n as usize == data.len() && fds.len() == 1 && msg.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) == 0 {
                    return Ok((u64::from_le_bytes(data), fds.pop().expect("ein Deskriptor")));
                }
                // Sonst: verwerfen (fds werden geschlossen) und weiter lesen.
            }
        }
    }
}

#[cfg(unix)]
pub use imp::{pair, recv_fd, send_fd};

/// Socket-Ende des Workers (aus der Umgebung, einmal geöffnet).
#[cfg(unix)]
fn channel() -> io::Result<&'static std::os::unix::net::UnixStream> {
    use std::os::fd::FromRawFd;
    static CH: std::sync::OnceLock<Option<std::os::unix::net::UnixStream>> = std::sync::OnceLock::new();
    CH.get_or_init(|| {
        let fd: i32 = std::env::var(FRAME_FD_ENV).ok()?.parse().ok()?;
        // SAFETY: Der Supervisor übergibt dieses Socket-Ende genau einmal an den Worker.
        let s = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
        // SAFETY: fcntl auf den eigenen Deskriptor; nicht an Kindprozesse vererben.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        Some(s)
    })
    .as_ref()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "kein Kanal für Bildpuffer"))
}

/// Übergibt einen beliebigen Deskriptor an den Hauptprozess (Unix); der
/// Hauptprozess nimmt ihn nur als Bildpuffer an, wenn er versiegelt ist.
#[cfg(unix)]
pub fn share_fd(fd: std::os::fd::BorrowedFd<'_>) -> io::Result<u64> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let token = NEXT.fetch_add(1, Ordering::Relaxed);
    send_fd(channel()?, token, fd)?;
    Ok(token)
}

/// Übergibt einen Bildpuffer an den Hauptprozess; liefert den Wert für
/// `FromWorker::SharedFrame { handle, .. }`.
pub fn share(fb: &FrameBufferWriter) -> io::Result<u64> {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        share_fd(fb.handle().as_fd())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        Ok(fb.handle().as_raw_handle() as usize as u64)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    #[test]
    fn descriptor_roundtrip_and_garbage_is_skipped() {
        let (a, b) = pair().unwrap();
        // Müll ohne Deskriptor wird übersprungen.
        use std::io::Write;
        (&a).write_all(b"0123456789").unwrap();
        let mut w = FrameBufferWriter::create(8, 8).unwrap();
        send_fd(&a, 42, w.handle().as_fd()).unwrap();
        let (t, fd) = recv_fd(&b).unwrap();
        assert_eq!(t, 42);
        let r = crate::shm::FrameBufferReader::open(fd, 8, 8).unwrap();
        w.pixels_mut()[5] = 9;
        assert_eq!(r.pixels()[5], 9);
    }
}
