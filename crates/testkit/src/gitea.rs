//! Attrappe einer Gitea-Programmierschnittstelle für Update-Tests (nur
//! `127.0.0.1`, HTTP). Liefert `/api/v1/repos/<repo>/releases/latest` und die
//! Dateien unter `/dl/<name>`; verlangt optional einen Zugriffsschlüssel und
//! merkt sich die Anfragen (Pfad und Kopfzeilen).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct State {
    /// Veröffentlichung als JSON (Platzhalter `{base}` wird ersetzt).
    pub release: String,
    /// Dateien unter `/dl/<name>`.
    pub files: HashMap<String, Vec<u8>>,
    /// Erwarteter Zugriffsschlüssel (`Authorization: token …`) für die Programmierschnittstelle.
    pub token: Option<String>,
    /// Anfragen: Pfad und Kopfzeilen (Name klein).
    pub requests: Vec<(String, Vec<(String, String)>)>,
}

pub struct GiteaMock {
    pub base: String,
    pub state: Arc<Mutex<State>>,
}

impl GiteaMock {
    pub fn start(repo: &str) -> GiteaMock {
        let l = TcpListener::bind("127.0.0.1:0").expect("Port");
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let state = Arc::new(Mutex::new(State::default()));
        let st = state.clone();
        let api = format!("/api/v1/repos/{repo}/releases/latest");
        let b = base.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let st = st.clone();
                let api = api.clone();
                let b = b.clone();
                std::thread::spawn(move || {
                    let mut r = BufReader::new(s);
                    let mut first = String::new();
                    if r.read_line(&mut first).is_err() {
                        return;
                    }
                    let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
                    let mut headers = Vec::new();
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h.trim_end().is_empty() {
                            break;
                        }
                        if let Some((k, v)) = h.trim_end().split_once(':') {
                            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
                        }
                    }
                    let (status, body) = {
                        let mut g = st.lock().unwrap();
                        g.requests.push((path.clone(), headers.clone()));
                        let auth = headers.iter().find(|(k, _)| k == "authorization").map(|(_, v)| v.clone());
                        if path == api {
                            match &g.token {
                                Some(t) if auth.as_deref() != Some(&format!("token {t}")) => (401, b"{\"message\":\"token is required\"}".to_vec()),
                                _ => (200, g.release.replace("{base}", &b).into_bytes()),
                            }
                        } else if let Some(f) = path.strip_prefix("/dl/").and_then(|n| g.files.get(n)) {
                            (200, f.clone())
                        } else {
                            (404, b"not found".to_vec())
                        }
                    };
                    let s = r.get_mut();
                    let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = s.write_all(&body);
                });
            }
        });
        GiteaMock { base, state }
    }

    /// Veröffentlichung mit den angegebenen Dateinamen (Download unter `/dl/`).
    pub fn set_release(&self, tag: &str, body: &str, assets: &[&str]) {
        let a: Vec<String> = assets
            .iter()
            .map(|n| format!("{{\"name\":\"{n}\",\"size\":{},\"browser_download_url\":\"{{base}}/dl/{n}\"}}", self.state.lock().unwrap().files.get(*n).map(|f| f.len()).unwrap_or(0)))
            .collect();
        let body = body.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        self.state.lock().unwrap().release = format!("{{\"tag_name\":\"{tag}\",\"name\":\"{tag}\",\"body\":\"{body}\",\"draft\":false,\"prerelease\":false,\"assets\":[{}]}}", a.join(","));
    }
}
