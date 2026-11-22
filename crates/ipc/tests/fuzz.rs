//! Zufällig veränderte Rahmen (feste Startwerte): Der Leser darf nie in Panik
//! geraten und muss Fehler sauber melden.

use rj_ipc::msg::*;
use rj_ipc::{FrameReader, FrameWriter, IpcError};

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

fn sample() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut w = FrameWriter::new(&mut bytes);
    w.send(1, &FromWorker::Heartbeat { seq: 9 }).unwrap();
    w.send(2, &FromWorker::Output(b"Gr\xc3\xbc\xc3\x9fe\r\n".to_vec())).unwrap();
    w.send(3, &FromWorker::Clipboard(ClipData::Text("ä\t🚀".into()))).unwrap();
    w.send(4, &FromWorker::Notice("Hinweis".into())).unwrap();
    w.send(5, &ToWorker::Credentials(CredentialPayload {
        username: "u".into(),
        domain: None,
        password: SecretBytes(b"pw".to_vec()),
        private_key: None,
    }))
    .unwrap();
    bytes
}

#[test]
fn mutated_frames_never_panic() {
    let src = sample();
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for round in 0..2000 {
        let mut b = src.clone();
        for _ in 0..1 + rng.below(6) {
            if b.is_empty() {
                break;
            }
            let i = rng.below(b.len());
            match rng.below(3) {
                0 => b[i] = rng.next() as u8,
                1 => b.truncate(i),
                _ => b.insert(i, rng.next() as u8),
            }
        }
        let mut r = FrameReader::new(&b[..]);
        for _ in 0..16 {
            let res = if round % 2 == 0 { r.recv::<FromWorker>().map(|_| ()) } else { r.recv::<ToWorker>().map(|_| ()) };
            match res {
                Ok(()) => {}
                Err(IpcError::Closed) | Err(IpcError::TooLarge(_)) => break,
                Err(IpcError::Decode(_)) => {}
                Err(IpcError::Io(e)) => panic!("unerwarteter E/A-Fehler: {e}"),
            }
        }
    }
}

#[test]
fn random_garbage_is_rejected_cleanly() {
    let mut rng = Rng(42);
    for _ in 0..2000 {
        let len = rng.below(64);
        let mut b: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        // Häufig eine kleine, plausible Länge voranstellen.
        if rng.below(2) == 0 && b.len() >= 4 {
            let l = (b.len() - 4) as u32;
            b[..4].copy_from_slice(&l.to_le_bytes());
        }
        let mut r = FrameReader::new(&b[..]);
        for _ in 0..4 {
            if r.recv::<FromWorker>().is_err() {
                break;
            }
        }
    }
}
