//! Netzhilfen für Erweiterungen ohne Zugangsdaten: TCP-Prüfung eines Ports an vielen Adressen.

use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

/// Einen Port an vielen Adressen prüfen, höchstens `parallel` gleichzeitig. Liefert die Adressen, die antworten,
/// mit der Zeit bis zur Antwort (ms), in der Reihenfolge von `hosts`. `stop` bricht zwischen zwei Runden ab.
pub fn sweep(hosts: &[IpAddr], port: u16, timeout: Duration, parallel: usize, stop: &dyn Fn() -> bool) -> Vec<(IpAddr, u64)> {
    let mut out = Vec::new();
    for chunk in hosts.chunks(parallel.max(1)) {
        if stop() {
            break;
        }
        let handles: Vec<_> = chunk
            .iter()
            .map(|&ip| {
                std::thread::spawn(move || {
                    let t0 = std::time::Instant::now();
                    TcpStream::connect_timeout(&SocketAddr::new(ip, port), timeout).ok().map(|_| (ip, t0.elapsed().as_millis() as u64))
                })
            })
            .collect();
        out.extend(handles.into_iter().filter_map(|h| h.join().ok().flatten()));
    }
    out
}
