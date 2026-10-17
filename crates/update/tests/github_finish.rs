//! „Veröffentlichung abschließen“ gegen eine GitHub-Attrappe (127.0.0.1): Entwurf mit Archiv und
//! Prüfsummen wird geprüft, signiert, hochgeladen und veröffentlicht; Abweichungen brechen ab.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rj_update::github::{finish_release, Endpoints};
use rj_update::sign::{self, SIG, SUMS};
use rj_update::UpdateError;

const TOKEN: &str = "github_pat_Prüf-Schlüssel";

#[derive(Default)]
struct State {
    draft: bool,
    files: HashMap<String, Vec<u8>>,
    uploaded: Vec<(String, Vec<u8>)>,
    patched: Option<String>,
    auth_seen: Vec<String>,
}

struct Mock {
    base: String,
    state: Arc<Mutex<State>>,
}

fn start() -> Mock {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let state = Arc::new(Mutex::new(State { draft: true, ..Default::default() }));
    let st = state.clone();
    let b = base.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { break };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let (head_end, len) = loop {
                let n = s.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break (0, 0);
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                    let len = head.lines().find_map(|l| l.strip_prefix("content-length: ")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    break (p + 4, len);
                }
            };
            while buf.len() < head_end + len {
                let n = s.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let body = buf[head_end.min(buf.len())..].to_vec();
            let first = head.lines().next().unwrap_or("").to_string();
            let mut parts = first.split_whitespace();
            let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
            let mut g = st.lock().unwrap();
            if let Some(a) = head.lines().find(|l| l.to_lowercase().starts_with("authorization:")) {
                g.auth_seen.push(a.to_string());
            }
            let (status, ctype, out): (u16, &str, Vec<u8>) = if method == "GET" && path.starts_with("/repos/o/r/releases?") {
                let mut assets: Vec<serde_json::Value> = g
                    .files
                    .keys()
                    .map(|n| serde_json::json!({ "name": n, "size": g.files[n].len(), "url": format!("{b}/repos/o/r/releases/assets/{n}"), "browser_download_url": "" }))
                    .collect();
                assets.extend(g.uploaded.iter().map(|(n, d)| serde_json::json!({ "name": n, "size": d.len(), "url": format!("{b}/repos/o/r/releases/assets/{n}"), "browser_download_url": "" })));
                let rel = serde_json::json!([{ "id": 11, "tag_name": "v1.2.3", "draft": g.draft, "prerelease": false, "body": "Neu", "assets": assets }]);
                (200, "application/json", rel.to_string().into_bytes())
            } else if method == "GET" && path.starts_with("/repos/o/r/releases/assets/") {
                let n = path.rsplit('/').next().unwrap();
                match g.files.get(n) {
                    Some(d) => (200, "application/octet-stream", d.clone()),
                    None => (404, "text/plain", Vec::new()),
                }
            } else if method == "POST" && path.starts_with("/repos/o/r/releases/11/assets?name=") {
                let n = path.rsplit('=').next().unwrap().to_string();
                g.uploaded.push((n, body));
                (201, "application/json", b"{}".to_vec())
            } else if method == "PATCH" && path == "/repos/o/r/releases/11" {
                g.patched = Some(String::from_utf8_lossy(&body).to_string());
                g.draft = false;
                (200, "application/json", b"{}".to_vec())
            } else {
                (404, "text/plain", Vec::new())
            };
            let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len());
            let _ = s.write_all(&out);
        }
    });
    Mock { base, state }
}

fn ep(m: &Mock) -> Endpoints {
    Endpoints { api: m.base.clone(), uploads: m.base.clone() }
}

#[test]
fn draft_is_checked_signed_and_published() {
    let m = start();
    let key = sign::generate().unwrap();
    let archive = b"Archiv-Inhalt".to_vec();
    let name = "remotejoe-1.2.3-linux-x86_64.tar.zst".to_string();
    let sums = sign::sums_text(&[(name.clone(), sign::sha256_hex(&archive))]);
    {
        let mut g = m.state.lock().unwrap();
        g.files.insert(name.clone(), archive);
        g.files.insert(SUMS.into(), sums.clone().into_bytes());
    }
    let done = finish_release(&ep(&m), "o/r", "v1.2.3", TOKEN, &key, &key.verifying_key(), Duration::from_secs(5), None).unwrap();
    assert_eq!(done.archives, [name]);
    let g = m.state.lock().unwrap();
    let (n, sig) = &g.uploaded[0];
    assert_eq!(n, SIG);
    sign::verify(&key.verifying_key(), sums.as_bytes(), sig).unwrap();
    let patch: serde_json::Value = serde_json::from_str(g.patched.as_ref().unwrap()).unwrap();
    assert_eq!((patch["draft"].as_bool(), patch["prerelease"].as_bool()), (Some(false), Some(false)));
    assert!(g.auth_seen.iter().all(|a| a.contains(&format!("Bearer {TOKEN}"))));
}

#[test]
fn manipulated_archive_is_not_signed() {
    let m = start();
    let key = sign::generate().unwrap();
    let name = "remotejoe-1.2.3-linux-x86_64.tar.zst".to_string();
    let sums = sign::sums_text(&[(name.clone(), sign::sha256_hex(b"echt"))]);
    {
        let mut g = m.state.lock().unwrap();
        g.files.insert(name, b"anders".to_vec());
        g.files.insert(SUMS.into(), sums.into_bytes());
    }
    let e = finish_release(&ep(&m), "o/r", "v1.2.3", TOKEN, &key, &key.verifying_key(), Duration::from_secs(5), None).unwrap_err();
    assert!(matches!(e, UpdateError::BadChecksum), "{e}");
    let g = m.state.lock().unwrap();
    assert!(g.uploaded.is_empty() && g.patched.is_none(), "nichts signiert oder veröffentlicht");
}

#[test]
fn missing_assets_time_out_and_published_release_is_refused() {
    let m = start();
    let key = sign::generate().unwrap();
    // Paketierung noch nicht fertig: keine Dateien.
    let e = finish_release(&ep(&m), "o/r", "v1.2.3", TOKEN, &key, &key.verifying_key(), Duration::from_millis(1), None).unwrap_err();
    assert!(matches!(e, UpdateError::Timeout), "{e}");
    // Schon veröffentlicht.
    m.state.lock().unwrap().draft = false;
    let e = finish_release(&ep(&m), "o/r", "v1.2.3", TOKEN, &key, &key.verifying_key(), Duration::from_millis(1), None).unwrap_err();
    assert!(matches!(e, UpdateError::Install(_)), "{e}");
    // Fremde Signatur-Datei im Entwurf: Abbruch.
    m.state.lock().unwrap().draft = true;
    let name = "remotejoe-1.2.3-linux-x86_64.tar.zst".to_string();
    {
        let mut g = m.state.lock().unwrap();
        g.files.insert(name.clone(), b"x".to_vec());
        g.files.insert(SUMS.into(), sign::sums_text(&[(name, sign::sha256_hex(b"x"))]).into_bytes());
        g.files.insert(SIG.into(), b"alt".to_vec());
    }
    let e = finish_release(&ep(&m), "o/r", "v1.2.3", TOKEN, &key, &key.verifying_key(), Duration::from_secs(5), None).unwrap_err();
    assert!(matches!(e, UpdateError::Install(_)), "{e}");
}
