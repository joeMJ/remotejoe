//! Dekodierer für RFB-Rechtecke und Kodierung der erweiterten Zwischenablage.
//!
//! Bildpunktformat ist durchgehend 32 Bit, Tiefe 24, little endian, Rot an
//! Bit 16, Grün an Bit 8, Blau an Bit 0; im Speicher also B, G, R, X.

use std::io::{self, Read, Write};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::{Compression, Decompress, FlushDecompress, Status};

/// Bildpuffer (BGRA, Alpha stets 255).
#[derive(Debug, Clone, Default)]
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Framebuffer { width, height, data: vec![0; width as usize * height as usize * 4] }
    }

    fn check(&self, x: u32, y: u32, w: u32, h: u32) -> io::Result<()> {
        if x as u64 + w as u64 > self.width as u64 || y as u64 + h as u64 > self.height as u64 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Rechteck {x},{y} {w}×{h} außerhalb des Bildes")));
        }
        Ok(())
    }

    /// Setzt einen Bildpunkt aus B, G, R.
    #[inline]
    fn put(&mut self, x: u32, y: u32, px: [u8; 3]) {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.data[i] = px[0];
        self.data[i + 1] = px[1];
        self.data[i + 2] = px[2];
        self.data[i + 3] = 255;
    }

    pub fn fill(&mut self, x: u32, y: u32, w: u32, h: u32, px: [u8; 3]) -> io::Result<()> {
        self.check(x, y, w, h)?;
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, px);
            }
        }
        Ok(())
    }

    /// Rohe 32-Bit-Bildpunkte (B, G, R, X) übernehmen.
    pub fn raw(&mut self, x: u32, y: u32, w: u32, h: u32, src: &[u8]) -> io::Result<()> {
        self.check(x, y, w, h)?;
        for row in 0..h as usize {
            for col in 0..w as usize {
                let s = (row * w as usize + col) * 4;
                self.put(x + col as u32, y + row as u32, [src[s], src[s + 1], src[s + 2]]);
            }
        }
        Ok(())
    }

    pub fn copy_rect(&mut self, x: u32, y: u32, w: u32, h: u32, sx: u32, sy: u32) -> io::Result<()> {
        self.check(x, y, w, h)?;
        self.check(sx, sy, w, h)?;
        let stride = self.width as usize * 4;
        let rows: Vec<usize> = if sy < y { (0..h as usize).rev().collect() } else { (0..h as usize).collect() };
        for r in rows {
            let s = (sy as usize + r) * stride + sx as usize * 4;
            let d = (y as usize + r) * stride + x as usize * 4;
            self.data.copy_within(s..s + w as usize * 4, d);
        }
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        *self = Framebuffer::new(width, height);
    }
}

fn read_px4(r: &mut impl Read) -> io::Result<[u8; 3]> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok([b[0], b[1], b[2]])
}

/// Hextile (Kodierung 5).
pub fn hextile(fb: &mut Framebuffer, r: &mut impl Read, x: u32, y: u32, w: u32, h: u32) -> io::Result<()> {
    fb.check(x, y, w, h)?;
    let (mut bg, mut fg) = ([0u8; 3], [0u8; 3]);
    let mut ty = y;
    while ty < y + h {
        let th = 16.min(y + h - ty);
        let mut tx = x;
        while tx < x + w {
            let tw = 16.min(x + w - tx);
            let mut sub = [0u8; 1];
            r.read_exact(&mut sub)?;
            let sub = sub[0];
            if sub & 1 != 0 {
                let mut buf = vec![0u8; tw as usize * th as usize * 4];
                r.read_exact(&mut buf)?;
                fb.raw(tx, ty, tw, th, &buf)?;
            } else {
                if sub & 2 != 0 {
                    bg = read_px4(r)?;
                }
                fb.fill(tx, ty, tw, th, bg)?;
                if sub & 4 != 0 {
                    fg = read_px4(r)?;
                }
                if sub & 8 != 0 {
                    let mut n = [0u8; 1];
                    r.read_exact(&mut n)?;
                    for _ in 0..n[0] {
                        let color = if sub & 16 != 0 { read_px4(r)? } else { fg };
                        let mut xy = [0u8; 2];
                        r.read_exact(&mut xy)?;
                        let (sx, sy) = ((xy[0] >> 4) as u32, (xy[0] & 15) as u32);
                        let (sw, sh) = ((xy[1] >> 4) as u32 + 1, (xy[1] & 15) as u32 + 1);
                        if sx + sw > tw || sy + sh > th {
                            return Err(io::Error::new(io::ErrorKind::InvalidData, "Hextile-Teilrechteck außerhalb der Kachel"));
                        }
                        fb.fill(tx + sx, ty + sy, sw, sh, color)?;
                    }
                }
            }
            tx += 16;
        }
        ty += 16;
    }
    Ok(())
}

/// Zustand des ZRLE-Datenstroms (ein zlib-Strom je Verbindung).
pub struct Zrle {
    inflate: Decompress,
}

impl Default for Zrle {
    fn default() -> Self {
        Zrle { inflate: Decompress::new(true) }
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn u8(&mut self) -> io::Result<u8> {
        let b = *self.data.get(self.pos).ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "ZRLE-Daten zu kurz"))?;
        self.pos += 1;
        Ok(b)
    }

    fn cpixel(&mut self) -> io::Result<[u8; 3]> {
        Ok([self.u8()?, self.u8()?, self.u8()?])
    }
}

impl Zrle {
    /// ZRLE (Kodierung 16). `data` ist der komprimierte Block des Rechtecks.
    pub fn decode(&mut self, fb: &mut Framebuffer, data: &[u8], x: u32, y: u32, w: u32, h: u32) -> io::Result<()> {
        fb.check(x, y, w, h)?;
        let mut out = Vec::with_capacity(w as usize * h as usize * 3 + 1024);
        let mut input = data;
        loop {
            if out.capacity() - out.len() < 4096 {
                out.reserve(64 * 1024);
            }
            let before_in = self.inflate.total_in();
            let st = self
                .inflate
                .decompress_vec(input, &mut out, FlushDecompress::Sync)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("ZRLE: {e}")))?;
            let used = (self.inflate.total_in() - before_in) as usize;
            input = &input[used..];
            if input.is_empty() && out.len() < out.capacity() {
                break;
            }
            if matches!(st, Status::StreamEnd) || (used == 0 && input.is_empty()) {
                break;
            }
        }
        let mut c = Cursor { data: &out, pos: 0 };
        let mut ty = y;
        while ty < y + h {
            let th = 64.min(y + h - ty);
            let mut tx = x;
            while tx < x + w {
                let tw = 64.min(x + w - tx);
                Self::tile(fb, &mut c, tx, ty, tw, th)?;
                tx += 64;
            }
            ty += 64;
        }
        Ok(())
    }

    fn tile(fb: &mut Framebuffer, c: &mut Cursor, tx: u32, ty: u32, tw: u32, th: u32) -> io::Result<()> {
        let sub = c.u8()?;
        let n = (tw * th) as usize;
        match sub {
            0 => {
                for i in 0..n {
                    let px = c.cpixel()?;
                    fb.put(tx + (i as u32 % tw), ty + (i as u32 / tw), px);
                }
            }
            1 => {
                let px = c.cpixel()?;
                fb.fill(tx, ty, tw, th, px)?;
            }
            2..=16 => {
                let size = sub as usize;
                let palette: Vec<[u8; 3]> = (0..size).map(|_| c.cpixel()).collect::<io::Result<_>>()?;
                let bits = match size {
                    2 => 1,
                    3..=4 => 2,
                    _ => 4,
                };
                for row in 0..th {
                    let mut byte = 0u8;
                    let mut left = 0;
                    for col in 0..tw {
                        if left == 0 {
                            byte = c.u8()?;
                            left = 8;
                        }
                        left -= bits;
                        let idx = ((byte >> left) & ((1 << bits) - 1)) as usize;
                        let px = *palette.get(idx).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "ZRLE-Palettenindex"))?;
                        fb.put(tx + col, ty + row, px);
                    }
                }
            }
            128 => {
                let mut i = 0;
                while i < n {
                    let px = c.cpixel()?;
                    let len = Self::run_len(c)?;
                    for _ in 0..len.min(n - i) {
                        fb.put(tx + (i as u32 % tw), ty + (i as u32 / tw), px);
                        i += 1;
                    }
                }
            }
            130..=255 => {
                let size = (sub - 128) as usize;
                let palette: Vec<[u8; 3]> = (0..size).map(|_| c.cpixel()).collect::<io::Result<_>>()?;
                let mut i = 0;
                while i < n {
                    let b = c.u8()?;
                    let idx = (b & 127) as usize;
                    let px = *palette.get(idx).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "ZRLE-Palettenindex"))?;
                    let len = if b & 128 != 0 { Self::run_len(c)? } else { 1 };
                    for _ in 0..len.min(n - i) {
                        fb.put(tx + (i as u32 % tw), ty + (i as u32 / tw), px);
                        i += 1;
                    }
                }
            }
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("ZRLE-Unterkodierung {sub} unbekannt"))),
        }
        Ok(())
    }

    fn run_len(c: &mut Cursor) -> io::Result<usize> {
        let mut len = 1usize;
        loop {
            let b = c.u8()?;
            len += b as usize;
            if b != 255 {
                return Ok(len);
            }
        }
    }
}

/// Erweiterte Zwischenablage (Pseudo-Kodierung 0xC0A1E5CE).
pub mod extclip {
    use super::*;

    pub const PSEUDO_ENCODING: i32 = 0xC0A1E5CEu32 as i32;
    pub const FORMAT_TEXT: u32 = 1 << 0;
    pub const CAPS: u32 = 1 << 24;
    pub const REQUEST: u32 = 1 << 25;
    pub const PEEK: u32 = 1 << 26;
    pub const NOTIFY: u32 = 1 << 27;
    pub const PROVIDE: u32 = 1 << 28;

    /// Eigene Fähigkeiten: Text, alle Aktionen, Größenlimit.
    pub fn caps(limit: u32) -> Vec<u8> {
        let mut out = (CAPS | FORMAT_TEXT | REQUEST | PEEK | NOTIFY | PROVIDE).to_be_bytes().to_vec();
        out.extend_from_slice(&limit.to_be_bytes());
        out
    }

    pub fn simple(action: u32) -> Vec<u8> {
        (action | FORMAT_TEXT).to_be_bytes().to_vec()
    }

    /// `provide` mit Text: UTF-8, CRLF, Null am Ende, zlib-komprimiert.
    pub fn provide_text(text: &str) -> io::Result<Vec<u8>> {
        let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
        let mut payload = crlf.into_bytes();
        payload.push(0);
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(&(payload.len() as u32).to_be_bytes())?;
        z.write_all(&payload)?;
        let mut out = (PROVIDE | FORMAT_TEXT).to_be_bytes().to_vec();
        out.extend_from_slice(&z.finish()?);
        Ok(out)
    }

    /// Liest den Text aus einer `provide`-Nachricht (ohne Kennzeichen).
    /// Text muss gültiges UTF-8 sein; CRLF wird zu LF.
    pub fn parse_provide_text(flags: u32, data: &[u8], limit: usize) -> io::Result<Option<String>> {
        if flags & FORMAT_TEXT == 0 {
            return Ok(None);
        }
        let mut z = ZlibDecoder::new(data);
        let mut len = [0u8; 4];
        z.read_exact(&mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        if len > limit {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Zwischenablage zu groß ({len} Bytes)")));
        }
        let mut buf = vec![0u8; len];
        z.read_exact(&mut buf)?;
        while buf.last() == Some(&0) {
            buf.pop();
        }
        let text = rj_core::charset::validate_utf8(&buf, "Zwischenablage der Gegenstelle")
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(Some(text.replace("\r\n", "\n")))
    }
}

/// Text für die einfache Zwischenablage (Latin-1). `Err` mit den Zeichen,
/// die nicht darstellbar sind.
pub fn to_latin1(text: &str) -> Result<Vec<u8>, String> {
    let bad: String = text.chars().filter(|c| *c as u32 > 0xFF).take(10).collect();
    if !bad.is_empty() {
        return Err(bad);
    }
    Ok(text.replace("\r\n", "\n").chars().map(|c| c as u32 as u8).collect())
}

pub fn from_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect::<String>().replace("\r\n", "\n")
}

/// Keysym für ein Unicode-Zeichen.
pub fn keysym_for(c: char) -> u32 {
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        '\u{8}' => 0xff08,
        c if (c as u32) < 0x100 => c as u32,
        c => 0x0100_0000 | c as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_and_copyrect() {
        let mut fb = Framebuffer::new(4, 4);
        let red = [0u8, 0, 255, 0].repeat(4);
        fb.raw(0, 0, 2, 2, &red).unwrap();
        fb.copy_rect(2, 2, 2, 2, 0, 0).unwrap();
        assert_eq!(&fb.data[(2 * 4 + 2) * 4..(2 * 4 + 2) * 4 + 4], &[0, 0, 255, 255]);
        assert!(fb.raw(3, 3, 2, 2, &red).is_err(), "außerhalb wird abgelehnt");
        // Überlappendes Kopieren nach unten.
        let mut fb = Framebuffer::new(1, 3);
        fb.raw(0, 0, 1, 1, &[1, 2, 3, 0]).unwrap();
        fb.copy_rect(0, 1, 1, 2, 0, 0).unwrap();
        assert_eq!(&fb.data[4..8], &[1, 2, 3, 255]);
    }

    #[test]
    fn hextile_tiles() {
        let mut fb = Framebuffer::new(20, 16);
        let mut data = Vec::new();
        // Kachel 1 (16×16): Hintergrund blau, ein grünes Teilrechteck 2×2 bei 1,1 (Vordergrund).
        data.extend_from_slice(&[2 | 4 | 8, 255, 0, 0, 0, 0, 255, 0, 0, 1, 0x11, 0x11]);
        // Kachel 2 (4×16): roh.
        data.push(1);
        data.extend_from_slice(&[9, 9, 9, 0].repeat(4 * 16));
        hextile(&mut fb, &mut &data[..], 0, 0, 20, 16).unwrap();
        let px = |x: usize, y: usize| fb.data[(y * 20 + x) * 4..(y * 20 + x) * 4 + 3].to_vec();
        assert_eq!(px(0, 0), [255, 0, 0]);
        assert_eq!(px(1, 1), [0, 255, 0]);
        assert_eq!(px(2, 2), [0, 255, 0]);
        assert_eq!(px(3, 3), [255, 0, 0]);
        assert_eq!(px(19, 15), [9, 9, 9]);
    }

    fn zlib(data: &[u8], enc: &mut flate2::Compress) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 128);
        enc.compress_vec(data, &mut out, flate2::FlushCompress::Sync).unwrap();
        out
    }

    #[test]
    fn zrle_subencodings_with_persistent_stream() {
        let mut enc = flate2::Compress::new(Compression::default(), true);
        let mut z = Zrle::default();
        let mut fb = Framebuffer::new(70, 2);
        // Rechteck 1: 70×2 → zwei Kacheln: einfarbig, dann einfaches RLE.
        let mut raw = vec![1, 10, 20, 30];
        raw.extend_from_slice(&[128, 1, 2, 3, 5, 4, 5, 6, 5]); // Kachel 6×2: Läufe 6 + 6
        let c = zlib(&raw, &mut enc);
        z.decode(&mut fb, &c, 0, 0, 70, 2).unwrap();
        let px = |fb: &Framebuffer, x: usize, y: usize| fb.data[(y * 70 + x) * 4..(y * 70 + x) * 4 + 3].to_vec();
        assert_eq!(px(&fb, 63, 1), [10, 20, 30]);
        assert_eq!(px(&fb, 64, 0), [1, 2, 3]);
        assert_eq!(px(&fb, 69, 0), [1, 2, 3]);
        assert_eq!(px(&fb, 66, 1), [4, 5, 6]);
        // Rechteck 2 im selben Strom: gepackte Palette (2 Farben, 1 Bit) 4×2.
        let raw = vec![2, 0, 0, 0, 9, 9, 9, 0b1010_0000, 0b0101_0000];
        let c = zlib(&raw, &mut enc);
        z.decode(&mut fb, &c, 0, 0, 4, 2).unwrap();
        assert_eq!(px(&fb, 0, 0), [9, 9, 9]);
        assert_eq!(px(&fb, 1, 0), [0, 0, 0]);
        assert_eq!(px(&fb, 1, 1), [9, 9, 9]);
        // Paletten-RLE: 2 Farben, Lauf von 7 mit Index 1, dann einmal Index 0.
        let raw = vec![130, 1, 1, 1, 2, 2, 2, 0x81, 6, 0x00];
        let c = zlib(&raw, &mut enc);
        z.decode(&mut fb, &c, 0, 0, 4, 2).unwrap();
        assert_eq!(px(&fb, 2, 1), [2, 2, 2]);
        assert_eq!(px(&fb, 3, 1), [1, 1, 1]);
    }

    #[test]
    fn extended_clipboard_text() {
        let msg = extclip::provide_text("Grüße 🚀\nZeile2").unwrap();
        let flags = u32::from_be_bytes(msg[..4].try_into().unwrap());
        assert_eq!(flags, extclip::PROVIDE | extclip::FORMAT_TEXT);
        let t = extclip::parse_provide_text(flags, &msg[4..], 1 << 20).unwrap().unwrap();
        assert_eq!(t, "Grüße 🚀\nZeile2");
        assert!(extclip::parse_provide_text(flags, &msg[4..], 4).is_err(), "Größenlimit");
    }

    #[test]
    fn latin1_fallback_detects_loss() {
        assert_eq!(to_latin1("Grüße").unwrap(), b"Gr\xfc\xdfe");
        assert_eq!(to_latin1("a🚀€").unwrap_err(), "🚀€");
        assert_eq!(from_latin1(b"Gr\xfc\xdfe"), "Grüße");
    }

    #[test]
    fn keysyms() {
        assert_eq!(keysym_for('a'), 0x61);
        assert_eq!(keysym_for('ü'), 0xFC);
        assert_eq!(keysym_for('€'), 0x0100_20AC);
        assert_eq!(keysym_for('🚀'), 0x0101_F680);
    }
}
