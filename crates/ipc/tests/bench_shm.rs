//! Messung (von Hand): Durchsatz des geteilten Bildpuffers.
//! `cargo test --release -p remotejoe-ipc --test bench_shm -- --ignored --nocapture`

use std::time::Instant;

use rj_ipc::shm::*;

#[test]
#[ignore]
fn throughput_full_hd() {
    let (w, h) = (1920u32, 1080u32);
    let mut fb = FrameBufferWriter::create(w, h).unwrap();
    let r = FrameBufferReader::open(fb.share_handle().unwrap(), w, h).unwrap();
    let src = vec![0x5au8; (w * h * 4) as usize];
    let n = 300;
    let start = Instant::now();
    let mut sum = 0u64;
    for i in 0..n {
        fb.blit(0, 0, w, h, &src, (w * 4) as usize);
        let px = r.pixels();
        sum = sum.wrapping_add(px[(i * 4099) % px.len()] as u64);
    }
    let t = start.elapsed();
    let mb = (w * h * 4) as f64 * n as f64 / 1e6;
    eprintln!("Bildpuffer 1920×1080: {n} Bilder in {:.1} ms, {:.0} MB/s, {:.2} ms je Bild (Prüfsumme {sum})", t.as_secs_f64() * 1e3, mb / t.as_secs_f64(), t.as_secs_f64() * 1e3 / n as f64);
    let start = Instant::now();
    for _ in 0..200 {
        let f = FrameBufferWriter::create(1280, 800).unwrap();
        let _r = FrameBufferReader::open(f.share_handle().unwrap(), 1280, 800).unwrap();
    }
    eprintln!("Anlegen und Öffnen 1280×800: {:.3} ms je Puffer", start.elapsed().as_secs_f64() * 1e3 / 200.0);
}
