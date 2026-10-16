//! Rahmung von Nachrichten auf einem Bytestrom.

use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Größte zulässige Rahmenlänge (Schutz vor beschädigten Längenangaben).
pub const MAX_FRAME: u32 = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("Kanal: {0}")]
    Io(#[from] io::Error),
    #[error("Kanal geschlossen")]
    Closed,
    #[error("Rahmen zu groß ({0} Bytes)")]
    TooLarge(u32),
    #[error("Nachricht nicht lesbar: {0}")]
    Decode(#[from] postcard::Error),
}

/// Schreibt Rahmen auf einen Bytestrom.
pub struct FrameWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: Write> FrameWriter<W> {
    pub fn new(inner: W) -> Self {
        FrameWriter { inner, buf: Vec::new() }
    }

    pub fn send<T: Serialize>(&mut self, id: u32, msg: &T) -> Result<(), IpcError> {
        self.buf.clear();
        self.buf.extend_from_slice(&[0; 8]);
        let mut buf = std::mem::take(&mut self.buf);
        buf = postcard::to_extend(msg, buf)?;
        let len = (buf.len() - 4) as u32;
        if len > MAX_FRAME {
            self.buf = buf;
            return Err(IpcError::TooLarge(len));
        }
        buf[..4].copy_from_slice(&len.to_le_bytes());
        buf[4..8].copy_from_slice(&id.to_le_bytes());
        let r = self.inner.write_all(&buf).and_then(|_| self.inner.flush());
        // Puffer überschreiben: er kann Zugangsdaten enthalten haben.
        zeroize::Zeroize::zeroize(&mut buf);
        self.buf = buf;
        r.map_err(|e| if e.kind() == io::ErrorKind::BrokenPipe { IpcError::Closed } else { e.into() })
    }

    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }
}

/// Liest Rahmen von einem Bytestrom.
pub struct FrameReader<R: Read> {
    inner: R,
    buf: Vec<u8>,
}

impl<R: Read> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        FrameReader { inner, buf: Vec::new() }
    }

    /// Liest den nächsten Rahmen; `Err(Closed)` bei sauberem Ende.
    pub fn recv<T: DeserializeOwned>(&mut self) -> Result<(u32, T), IpcError> {
        let mut head = [0u8; 4];
        match self.inner.read_exact(&mut head) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Err(IpcError::Closed),
            Err(e) => return Err(e.into()),
        }
        let len = u32::from_le_bytes(head);
        if len > MAX_FRAME {
            return Err(IpcError::TooLarge(len));
        }
        if len < 4 {
            return Err(IpcError::Closed);
        }
        self.buf.resize(len as usize, 0);
        self.inner.read_exact(&mut self.buf).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                IpcError::Closed
            } else {
                e.into()
            }
        })?;
        let id = u32::from_le_bytes(self.buf[..4].try_into().expect("4 Bytes"));
        let msg = postcard::from_bytes(&self.buf[4..]);
        zeroize::Zeroize::zeroize(&mut self.buf[..]);
        Ok((id, msg?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::*;

    #[test]
    fn roundtrip_over_pipe() {
        let mut bytes = Vec::new();
        let mut w = FrameWriter::new(&mut bytes);
        w.send(7, &ToWorker::Ping).unwrap();
        w.send(8, &ToWorker::Credentials(CredentialPayload {
            username: "admin".into(),
            domain: Some("EXAMPLE".into()),
            password: SecretBytes(b"Kennwo\xcc\x88rt".to_vec()),
            private_key: None,
        }))
        .unwrap();
        w.send(9, &ToWorker::Clipboard(ClipData::Text("Grüße\r\n🚀\t".into()))).unwrap();
        let mut r = FrameReader::new(&bytes[..]);
        assert!(matches!(r.recv::<ToWorker>().unwrap(), (7, ToWorker::Ping)));
        let (id, m) = r.recv::<ToWorker>().unwrap();
        assert_eq!(id, 8);
        let ToWorker::Credentials(c) = m else { panic!() };
        assert_eq!(c.password.0, b"Kennwo\xcc\x88rt");
        let (_, ToWorker::Clipboard(ClipData::Text(t))) = r.recv::<ToWorker>().unwrap() else { panic!() };
        assert_eq!(t, "Grüße\r\n🚀\t");
        assert!(matches!(r.recv::<ToWorker>(), Err(IpcError::Closed)));
    }

    #[test]
    fn rejects_oversized_length() {
        let mut data = (MAX_FRAME + 1).to_le_bytes().to_vec();
        data.extend_from_slice(&[0; 8]);
        let mut r = FrameReader::new(&data[..]);
        assert!(matches!(r.recv::<ToWorker>(), Err(IpcError::TooLarge(_))));
    }

    #[test]
    fn truncated_frame_is_closed() {
        let mut bytes = Vec::new();
        FrameWriter::new(&mut bytes).send(1, &FromWorker::Heartbeat { seq: 3 }).unwrap();
        bytes.pop();
        let mut r = FrameReader::new(&bytes[..]);
        assert!(matches!(r.recv::<FromWorker>(), Err(IpcError::Closed)));
    }
}
