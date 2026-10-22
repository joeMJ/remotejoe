//! Aruba-Werkzeuge: die Aruba Instant Python Tools (GitHub `joeMJ/aruba-instant-python-tools`, Zweig `main`) im
//! Datenordner unter `aptools/` – Sammlung, eigene Python-Umgebung und die Protokolle der Läufe (`aptools/<Baumpfad>/`).
//! Fehlt Python unter Windows, kann es wie in der `setup.bat` der Sammlung von python.org installiert werden (nur
//! für den Benutzer, Signatur der Python Software Foundation geprüft).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::{json, Value};

pub const REPO_ZIP: &str = "https://github.com/joeMJ/aruba-instant-python-tools/archive/refs/heads/main.zip";
pub const REPO_MAIN_COMMIT: &str = "https://api.github.com/repos/joeMJ/aruba-instant-python-tools/commits/main";
pub const PY_VERSION: &str = "3.12.10";
const STAND_FILE: &str = ".remotejoe-stand.json";

/// `<Datenordner>/aptools`.
pub fn root() -> Option<PathBuf> {
    rj_core::config::data_dir().map(|d| d.join("aptools"))
}

pub fn collection_dir(root: &Path) -> PathBuf {
    root.join("sammlung")
}

pub fn venv_dir(root: &Path) -> PathBuf {
    root.join("python")
}

/// Python der eigenen Umgebung.
pub fn venv_python(root: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir(root).join("Scripts").join("python.exe")
    } else {
        venv_dir(root).join("bin").join("python")
    }
}

/// Befehl ohne Konsolenfenster (Windows) und ohne Eingabe.
pub fn command(program: &Path) -> Command {
    let mut c = Command::new(program);
    c.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Python ab 3.9 auf dem System: Programm, Zusatzargumente (z. B. `-3` für `py`) und Version.
pub fn find_python() -> Option<(PathBuf, Vec<String>, String)> {
    let mut candidates: Vec<(PathBuf, Vec<String>)> = Vec::new();
    if cfg!(windows) {
        candidates.push(("py".into(), vec!["-3".into()]));
        candidates.push(("python".into(), vec![]));
        // Frisch installiert: PATH dieses Prozesses ist noch alt.
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            if let Ok(rd) = std::fs::read_dir(Path::new(&local).join("Programs").join("Python")) {
                let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.join("python.exe").is_file()).collect();
                dirs.sort();
                for d in dirs.into_iter().rev() {
                    candidates.push((d.join("python.exe"), vec![]));
                }
            }
        }
    } else {
        candidates.push(("python3".into(), vec![]));
        candidates.push(("python".into(), vec![]));
    }
    candidates.into_iter().find_map(|(prog, pre)| {
        let out = command(&prog)
            .args(&pre)
            .args(["-c", "import sys; print('%d.%d.%d' % sys.version_info[:3]); sys.exit(0 if sys.version_info >= (3, 9) else 1)"])
            .output()
            .ok()?;
        out.status.success().then(|| (prog, pre, String::from_utf8_lossy(&out.stdout).trim().to_string()))
    })
}

/// Stand der Sammlung und der Python-Umgebung (für die Anzeige).
pub fn status(root: &Path) -> Value {
    let coll = collection_dir(root);
    let stand: Value = std::fs::read(coll.join(STAND_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    // Version der Sammlung (TOOLSUITE_VERSION in ap_check.py).
    let version = std::fs::read_to_string(coll.join("ap_check.py"))
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("TOOLSUITE_VERSION").map(|r| r.trim_start_matches([' ', '=']).trim().trim_matches('"').to_string())));
    json!({
        "root": root,
        "installed": coll.join("ap_check.py").is_file(),
        "commit": stand["commit"],
        "at": stand["at"],
        "version": version,
        "venv": venv_python(root).is_file(),
    })
}

fn copy_tree(src: &Path, dst: &Path, keep: &[&str]) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &to, &[])?;
        } else if keep.iter().any(|k| e.file_name() == *k) && to.exists() {
            // Eigene Einstellungen bleiben; die neue Vorlage liegt daneben.
            let mut neu = to.clone().into_os_string();
            neu.push(".neu");
            std::fs::copy(e.path(), PathBuf::from(neu))?;
        } else {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

/// Sammlung von GitHub laden (Zweig `main`) und nach `aptools/sammlung` legen; eine vorhandene `config.json` bleibt
/// (die neue Vorlage als `config.json.neu`). Liefert den Commit-Stand (falls GitHub ihn nennt).
pub fn install_collection(root: &Path, cancel: &AtomicBool) -> Result<Option<String>, String> {
    use rj_update::http::{self, Control};
    use rj_update::Limits;
    let ctl = |max: u64, secs: u64| Control::new(Limits { max_bytes: max, timeout: Duration::from_secs(secs) }, Some(cancel));
    let commit = http::get(REPO_MAIN_COMMIT, &[("Accept".into(), "application/vnd.github+json".into())], "den Stand der Sammlung", &ctl(2 << 20, 30))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["sha"].as_str().map(str::to_string));
    let data = http::get(REPO_ZIP, &[], "die Aruba Instant Python Tools", &ctl(64 << 20, 300)).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let tmp = root.join(".neu");
    let _ = std::fs::remove_dir_all(&tmp);
    rj_update::archive::unpack("main.zip", &data, &tmp).map_err(|e| e.to_string())?;
    // Im Archiv liegt ein Projektordner (`aruba-instant-python-tools-main`).
    let inner = std::fs::read_dir(&tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.join("ap_check.py").is_file());
    let Some(inner) = inner else {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err("Im Archiv fehlt ap_check.py".into());
    };
    let res = copy_tree(&inner, &collection_dir(root), &["config.json"]).map_err(|e| e.to_string());
    let _ = std::fs::remove_dir_all(&tmp);
    res?;
    let stand = json!({ "commit": commit, "at": crate::overview::inventory::now() });
    std::fs::write(collection_dir(root).join(STAND_FILE), stand.to_string()).map_err(|e| e.to_string())?;
    Ok(commit)
}

fn run(c: &mut Command, what: &str) -> Result<String, String> {
    let out = c.output().map_err(|e| format!("{what}: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        let tail: Vec<&str> = text.lines().rev().take(6).collect();
        Err(format!("{what} fehlgeschlagen: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" / ")))
    }
}

/// Eigene Python-Umgebung anlegen bzw. auffrischen und die Bibliotheken aus `requirements.txt` der Sammlung
/// installieren. `proxy`: `Rechner:Port` für pip (aus der Proxy-Einstellung der Updates), sonst wie das System.
pub fn setup_python(root: &Path, proxy: Option<&str>) -> Result<String, String> {
    let req = collection_dir(root).join("requirements.txt");
    if !req.is_file() {
        return Err("Die Sammlung ist noch nicht geladen".into());
    }
    let py = venv_python(root);
    if !py.is_file() {
        let (prog, pre, _) = find_python().ok_or("NO_PYTHON")?;
        run(command(&prog).args(&pre).arg("-m").arg("venv").arg(venv_dir(root)), "Python-Umgebung anlegen").map_err(|e| {
            if cfg!(unix) {
                format!("{e}. Unter Ubuntu fehlt dafür oft das Paket python3-venv (sudo apt install python3-venv).")
            } else {
                e
            }
        })?;
    }
    let mut pip = command(&py);
    pip.args(["-m", "pip", "install", "--disable-pip-version-check", "-r"]).arg(&req);
    if let Some(p) = proxy {
        pip.env("HTTPS_PROXY", format!("http://{p}")).env("HTTP_PROXY", format!("http://{p}"));
    }
    run(&mut pip, "Bibliotheken installieren (pip)")?;
    let v = run(command(&py).args(["-c", "import paramiko; print(paramiko.__version__)"]), "paramiko prüfen")?;
    Ok(v.trim().to_string())
}

/// Windows: Python von python.org laden, Signatur der Python Software Foundation prüfen und nur für diesen
/// Benutzer installieren (wie die `setup.bat` der Sammlung).
#[cfg(windows)]
pub fn install_python(root: &Path, cancel: &AtomicBool) -> Result<String, String> {
    use rj_update::http::{self, Control};
    use rj_update::Limits;
    let file = if std::env::var("PROCESSOR_ARCHITECTURE").is_ok_and(|a| a.eq_ignore_ascii_case("ARM64")) {
        format!("python-{PY_VERSION}-arm64.exe")
    } else {
        format!("python-{PY_VERSION}-amd64.exe")
    };
    let url = format!("https://www.python.org/ftp/python/{PY_VERSION}/{file}");
    let ctl = Control::new(Limits { max_bytes: 100 << 20, timeout: Duration::from_secs(900) }, Some(cancel));
    let data = http::get(&url, &[], "den Python-Installer", &ctl).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let exe = root.join(&file);
    std::fs::write(&exe, data).map_err(|e| e.to_string())?;
    if let Err(e) = rj_update::authenticode::verify_publisher(&exe, "Python Software Foundation") {
        let _ = std::fs::remove_file(&exe);
        return Err(format!("Python-Installer abgelehnt: {e}"));
    }
    let r = run(
        command(&exe).args(["/quiet", "InstallAllUsers=0", "PrependPath=1", "Include_launcher=1", "Include_pip=1", "Include_test=0", "Shortcuts=0"]),
        "Python installieren",
    );
    let _ = std::fs::remove_file(&exe);
    r?;
    find_python().map(|(_, _, v)| v).ok_or_else(|| "Python wurde installiert, aber nicht gefunden".to_string())
}

/// Starter für die Werkzeuge. Erste Zeile auf stdin: JSON mit Benutzer, Kennwort und den Antworten auf die
/// Rückfragen (Textstück → Antwort). Eine unbekannte Rückfrage beendet den Lauf mit Code 3, statt zu raten.
/// Bildschirmlöschen (`cls`/`clear`) wird zu einer Markierungszeile (Statustafel neu), Unterprozesse starten unter
/// Windows ohne Konsolenfenster. Danach läuft das Werkzeug unverändert (`runpy`) im Ordner der Sammlung.
pub const STARTER: &str = r#"import builtins, getpass, json, os, runpy, subprocess, sys
cfg = json.loads(sys.stdin.readline())
user = cfg["user"]
password = cfg["password"]
ANSWERS = [(k, user if v == "@user" else v) for k, v in cfg["answers"]]
def answer(prompt=""):
    text = str(prompt)
    for key, value in ANSWERS:
        if key in text:
            print(text + value, flush=True)
            return value
    print(text, flush=True)
    print("remotejoe: unbekannte Rueckfrage - Lauf abgebrochen.", flush=True)
    sys.exit(3)
def secret(prompt=""):
    print(str(prompt) + "***", flush=True)
    return password
def system(cmd):
    if str(cmd).strip() in ("cls", "clear"):
        print("\x1e", flush=True)
        return 0
    return real_system(cmd)
builtins.input = answer
getpass.getpass = secret
real_system = os.system
os.system = system
if os.name == "nt":
    real_init = subprocess.Popen.__init__
    def quiet_init(self, *a, **k):
        k["creationflags"] = k.get("creationflags", 0) | 0x08000000
        real_init(self, *a, **k)
    subprocess.Popen.__init__ = quiet_init
tool = sys.argv[1]
sys.argv = sys.argv[1:]
sys.path.insert(0, os.getcwd())
runpy.run_path(tool, run_name="__main__")
"#;

/// Antworten auf die Rückfragen, die alle Werkzeuge stellen (manuelle Eingabe, nichts speichern).
pub fn common_answers() -> Vec<(String, String)> {
    [
        ("Anmeldespeicher des Betriebssystems", "3"),
        ("credentials.bin", "n"),
        ("zu pr\u{fc}fenden SSH-Benutzernamen", "@user"),
        ("SSH-Benutzername", "@user"),
        ("Anmeldedaten f\u{fc}r alle diese Ziele identisch", "j"),
        ("gespeichert werden", "n"),
        ("Erneut versuchen", "n"),
        ("Down'-Conductor", "n"),
        ("veralteten Anmeldedaten", "n"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// Ein Werkzeug der Sammlung mit seinen Parametern.
#[derive(Clone)]
pub struct Tool {
    pub script: &'static str,
    /// Anfang des Ergebnisordners, den das Werkzeug im Ordner der Sammlung anlegt.
    pub prefix: &'static str,
    pub args: Vec<String>,
    pub answers: Vec<(String, String)>,
    /// Schreibend (Neustart, Update): Vorprüfung je Controller, mehrere Controller in einem Lauf.
    pub write: bool,
}

impl Tool {
    pub fn by_name(name: &str, url: Option<&str>, reboot: bool) -> Result<Tool, String> {
        let mut answers = common_answers();
        Ok(match name {
            "analyse" => Tool { script: "ap_check.py", prefix: "ap_check_", args: vec!["--log".into()], answers, write: false },
            "inventar" => Tool { script: "ap_list.py", prefix: "ap_list_", args: vec!["--apname".into(), "--log".into()], answers, write: false },
            "reload" => {
                answers.push(("Reload-Versuch ber\u{fc}cksichtigt".into(), "n".into()));
                Tool { script: "ap_swarm_reload.py", prefix: "logs_", args: vec!["--log".into()], answers, write: true }
            }
            "update" => {
                let url = url.map(str::trim).filter(|u| !u.is_empty()).ok_or("Firmware-URL fehlt")?;
                if !(url.starts_with("http://") || url.starts_with("https://")) || url.contains(char::is_whitespace) {
                    return Err("Die Firmware-URL muss mit http:// oder https:// beginnen und darf keine Leerzeichen enthalten".into());
                }
                answers.push(("Wollen Sie wirklich fortfahren".into(), "j".into()));
                answers.push(("automatisch neu gestartet".into(), if reboot { "j" } else { "n" }.into()));
                answers.push(("Conductor als 'Down'".into(), "n".into()));
                Tool { script: "ap_swarm_update.py", prefix: "update_", args: vec!["-f".into(), url.to_string(), "--log".into()], answers, write: true }
            }
            other => return Err(format!("Unbekanntes Werkzeug „{other}“")),
        })
    }
}

/// Laufender oder beendeter Werkzeuglauf.
#[derive(Default)]
pub struct Run {
    /// Ziel oder Ordner, für den der Lauf gestartet wurde.
    pub id: String,
    pub tool: String,
    pub title: String,
    pub out: std::sync::Arc<std::sync::Mutex<String>>,
    /// Zuletzt gezeichnete Statustafel (Update: Zustand je Cluster).
    pub board: std::sync::Arc<std::sync::Mutex<String>>,
    pub done: std::sync::Arc<AtomicBool>,
    pub stop: std::sync::Arc<AtomicBool>,
    pub child: std::sync::Arc<std::sync::Mutex<Option<std::process::Child>>>,
    /// Ergebnisordner der beendeten Teilläufe.
    pub results: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
}

impl Run {
    fn handle(&self) -> Run {
        Run {
            id: self.id.clone(),
            tool: self.tool.clone(),
            title: self.title.clone(),
            out: self.out.clone(),
            board: self.board.clone(),
            done: self.done.clone(),
            stop: self.stop.clone(),
            child: self.child.clone(),
            results: self.results.clone(),
        }
    }
}

/// Ein Controller (VC): Name, Adresse, Zugangsdaten und – für die Vorprüfung schreibender Werkzeuge – die
/// Verbindungsdaten.
pub struct Vc {
    pub name: String,
    pub host: String,
    pub user: String,
    pub password: rj_core::Secret,
    pub spec: crate::patch::JobSpec,
    pub port: u16,
}

/// Ein Aufruf des Werkzeugs: ein oder mehrere Controller mit gleichen Zugangsdaten, Ablageordner der Ergebnisse.
pub struct Task {
    pub vcs: Vec<Vc>,
    pub dest: PathBuf,
}

/// Dateiname aus einem Namen im Baum (keine Pfadtrenner oder unter Windows verbotenen Zeichen).
pub fn file_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c }).collect();
    let t = t.trim().trim_end_matches('.').to_string();
    if t.is_empty() { "_".into() } else { t }
}

fn append(out: &std::sync::Mutex<String>, s: &str) {
    let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
    o.push_str(s);
    // Nur das Ende behalten (lange Läufe).
    if o.len() > 2_000_000 {
        let cut = o.len() - 1_500_000;
        let cut = (cut..o.len()).find(|i| o.is_char_boundary(*i)).unwrap_or(cut);
        o.drain(..cut);
    }
}

/// Zeile der Statustafel von `ap_swarm_update.py` (Kopf, Trennlinie, „IP | Status“)?
fn board_line(l: &str) -> bool {
    let t = l.trim_start();
    t.starts_with("--- Echtzeit-Monitoring")
        || t.starts_with("Zielversion:")
        || t.starts_with("Updatequelle:")
        || t.starts_with("IP-ADRESSE")
        || (t.len() > 20 && t.chars().all(|c| c == '-'))
        || t.split_once(" | ").is_some_and(|(ip, _)| ip.trim().parse::<std::net::Ipv4Addr>().is_ok())
}

/// Schreibende Werkzeuge: Verwaltung (`show summary`) und Admin-Konto (`hash-mgmt-user`) am Controller prüfen.
fn precheck(vc: &Vc, known: rj_ssh::known_hosts::KnownHosts) -> Result<(), String> {
    let (rt, conn, _, _, _) = crate::overview::remote::ssh_open(&vc.spec, vc.port, known)?;
    let r = rt.block_on(crate::overview::remote::cli_session(&conn, &["no paging", "show summary", "show running-config"]));
    rt.block_on(conn.disconnect());
    let r = r?;
    let get = |i: usize| r.outputs.get(i).map(String::as_str).unwrap_or("");
    crate::aruba::write_check(crate::aruba::management(get(1)), &crate::aruba::admin_users(get(2)), &vc.user)
}

/// Werkzeug für jede Aufgabe nacheinander starten; Ausgabe in `run.out` (Statustafel in `run.board`), Ergebnisordner
/// danach nach `task.dest` verschieben. Schreibende Werkzeuge prüfen vorher jeden Controller und lassen nicht
/// passende mit Grund aus. Läuft im Hintergrund.
pub fn run_tool(root: &Path, tool: &Tool, tasks: Vec<Task>, known: rj_ssh::known_hosts::KnownHosts, run: &Run) {
    use std::io::{BufRead, Write};
    let coll = collection_dir(root);
    let py = venv_python(root);
    let starter = root.join("remotejoe_start.py");
    if let Err(e) = std::fs::write(&starter, STARTER) {
        append(&run.out, &format!("Starter konnte nicht angelegt werden: {e}\n"));
        run.done.store(true, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let dirs = |p: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| e.path()).filter(|x| x.is_dir() && x.file_name().is_some_and(|n| n.to_string_lossy().starts_with(tool.prefix))).collect()).unwrap_or_default()
    };
    let n = tasks.len();
    for (i, task) in tasks.into_iter().enumerate() {
        if run.stop.load(std::sync::atomic::Ordering::SeqCst) {
            append(&run.out, "\nAbgebrochen.\n");
            break;
        }
        let mut vcs = task.vcs;
        if tool.write {
            append(&run.out, "\n===== Vorprüfung: Verwaltung und Admin-Konto =====\n");
            vcs.retain(|vc| match precheck(vc, known.clone()) {
                Ok(()) => {
                    append(&run.out, &format!("{} ({}): ok\n", vc.name, vc.host));
                    true
                }
                Err(e) => {
                    append(&run.out, &format!("{} ({}): übersprungen – {e}\n", vc.name, vc.host));
                    false
                }
            });
            if vcs.is_empty() {
                append(&run.out, "Kein Controller übrig.\n");
                continue;
            }
        }
        let names: Vec<String> = vcs.iter().map(|v| format!("{} ({})", v.name, v.host)).collect();
        append(&run.out, &format!("\n===== {} – {} von {n} =====\n", names.join(", "), i + 1));
        let before = dirs(&coll);
        let hosts: Vec<&str> = vcs.iter().map(|v| v.host.as_str()).collect();
        let mut cmd = command(&py);
        cmd.arg("-u").arg(&starter).arg(tool.script).arg(hosts.join(",")).args(&tool.args).current_dir(&coll).env("PYTHONIOENCODING", "utf-8").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                append(&run.out, &format!("Start fehlgeschlagen: {e}\n"));
                continue;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let cfg = json!({
                "user": vcs[0].user,
                "password": String::from_utf8_lossy(vcs[0].password.as_bytes()),
                "answers": tool.answers,
            });
            let mut line = cfg.to_string().into_bytes();
            line.push(b'\n');
            let _ = stdin.write_all(&line);
            zeroize::Zeroize::zeroize(&mut line);
        }
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        *run.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        let out2 = run.out.clone();
        let err_thread = stderr.map(|e| {
            std::thread::spawn(move || {
                for l in std::io::BufReader::new(e).lines().map_while(Result::ok) {
                    append(&out2, &format!("{l}\n"));
                }
            })
        });
        if let Some(o) = stdout {
            let mut board = String::new();
            // Nach der Statustafel kommt die Zusammenfassung (gleiche Zeilenform „IP | Status“): ins Protokoll.
            let mut summary = false;
            let mut blank = false;
            for l in std::io::BufReader::new(o).lines().map_while(Result::ok) {
                if l.contains("FINALE ZUSAMMENFASSUNG") {
                    summary = true;
                }
                // Mehrere Leerzeilen hintereinander (Reste der Werkzeugausgabe) als eine.
                if l.trim().is_empty() {
                    if blank {
                        continue;
                    }
                    blank = true;
                } else {
                    blank = false;
                }
                if l.trim() == "\u{1e}" {
                    // Neue Statustafel: die bisherige ist vollständig.
                    if !board.is_empty() {
                        *run.board.lock().unwrap_or_else(|e| e.into_inner()) = std::mem::take(&mut board);
                    }
                } else if board_line(&l) && tool.write && !summary {
                    board.push_str(&l);
                    board.push('\n');
                    *run.board.lock().unwrap_or_else(|e| e.into_inner()) = board.clone();
                } else {
                    append(&run.out, &format!("{l}\n"));
                }
            }
        }
        if let Some(t) = err_thread {
            let _ = t.join();
        }
        let code = run.child.lock().unwrap_or_else(|e| e.into_inner()).take().and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
        append(&run.out, &format!("----- beendet (Code {}) -----\n", code.map_or("?".into(), |c| c.to_string())));
        for d in dirs(&coll).into_iter().filter(|d| !before.contains(d)) {
            let to = task.dest.join(d.file_name().unwrap_or_default());
            let moved = std::fs::create_dir_all(&task.dest).and_then(|_| std::fs::rename(&d, &to).or_else(|_| copy_tree(&d, &to, &[]).and_then(|_| std::fs::remove_dir_all(&d))));
            match moved {
                Ok(()) => {
                    append(&run.out, &format!("Ergebnis: {}\n", to.display()));
                    run.results.lock().unwrap_or_else(|e| e.into_inner()).push(to);
                }
                Err(e) => append(&run.out, &format!("Ergebnis konnte nicht verschoben werden ({e}): {}\n", d.display())),
            }
        }
    }
    let _ = std::fs::remove_file(&starter);
    run.done.store(true, std::sync::atomic::Ordering::SeqCst);
}

use crate::app::App;
use crate::sessions::UiEvent;
use rj_core::Protocol;
use uuid::Uuid;

impl App {
    fn aptools_root(&self) -> Result<PathBuf, String> {
        if !self.config.features.aruba {
            return Err("Aruba-Werkzeuge sind ausgeschaltet (Einstellungen → Zusatzfunktionen)".into());
        }
        if self.demo {
            return Err("In der Vorführung nicht verfügbar".into());
        }
        root().ok_or_else(|| "Kein Datenordner".to_string())
    }

    /// Stand der Sammlung und der Python-Umgebung; dazu, ob ein Lauf aktiv ist.
    pub fn aptools_status(&self) -> Result<Value, String> {
        let root = self.aptools_root()?;
        let mut s = status(&root);
        s["python"] = json!(find_python().map(|(_, _, v)| v));
        s["can_install_python"] = json!(cfg!(windows));
        s["running"] = json!(self.aptools_run.as_ref().is_some_and(|r| !r.done.load(std::sync::atomic::Ordering::SeqCst)));
        s["write_running"] = json!(self.aptools_write.as_ref().is_some_and(|r| !r.done.load(std::sync::atomic::Ordering::SeqCst)));
        Ok(s)
    }

    fn proxy_for_pip(&self) -> Option<String> {
        match rj_update::proxy::ProxyMode::from_setting(&self.config.update.proxy) {
            rj_update::proxy::ProxyMode::Manual(m) => Some(m),
            _ => None,
        }
    }

    /// Sammlung laden bzw. aktualisieren und die Python-Umgebung einrichten (`python`: vorher Python installieren,
    /// nur Windows, nach Zustimmung). Ergebnis „aptools_setup“; fehlt Python: `need_python`.
    pub fn aptools_setup(&mut self, c: &Value) -> Result<Value, String> {
        let root = self.aptools_root()?;
        let install_py = c.get("python").and_then(Value::as_bool).unwrap_or(false);
        let proxy = self.proxy_for_pip();
        rj_update::proxy::set_mode(rj_update::proxy::ProxyMode::from_setting(&self.config.update.proxy));
        let job = self.jobs.spawn_cancellable("Aruba-Werkzeuge einrichten", move |job, cancel| {
            let res: Result<Value, String> = (|| {
                #[cfg(windows)]
                if install_py {
                    install_python(&root, &cancel)?;
                }
                #[cfg(not(windows))]
                let _ = install_py;
                let commit = install_collection(&root, &cancel)?;
                match setup_python(&root, proxy.as_deref()) {
                    Ok(paramiko) => Ok(json!({ "commit": commit, "paramiko": paramiko })),
                    Err(e) if e == "NO_PYTHON" => Ok(json!({ "commit": commit, "need_python": true })),
                    Err(e) => Err(e),
                }
            })();
            Box::new(move |_app: &mut App, cancelled| {
                if cancelled {
                    return Vec::new();
                }
                let json = res.unwrap_or_else(|e| json!({ "error": e }));
                vec![UiEvent::JobResult { job, kind: "aptools_setup".into(), target: String::new(), json: json.to_string() }]
            })
        });
        Ok(json!({ "pending": job }))
    }

    /// Aruba-VCs für einen Lauf: das Ziel selbst oder alle als Aruba-AP erkannten Ziele im Ordner (je Cluster steht
    /// nur der VC-Eintrag im Baum).
    fn aptools_targets(&self, id: Uuid) -> Vec<Uuid> {
        fn walk(n: &rj_core::Node, devices: &std::collections::HashMap<Uuid, &'static str>, out: &mut Vec<Uuid>) {
            match n {
                rj_core::Node::Target(t) if devices.get(&t.id) == Some(&"aruba-ap") => out.push(t.id),
                rj_core::Node::Target(_) => {}
                rj_core::Node::Folder(f) => f.children.iter().for_each(|c| walk(c, devices, out)),
            }
        }
        if self.target(id).is_some() {
            return vec![id];
        }
        let mut out = Vec::new();
        if let Some(n) = self.index.path_to(id).and_then(|p| p.last().copied()) {
            walk(n, &self.overview.devices, &mut out);
        }
        out
    }

    fn tree_dest(&self, root: &Path, id: Uuid) -> PathBuf {
        let path: Vec<String> = self.index.path_to(id).unwrap_or_default().iter().map(|n| file_name(n.name())).collect();
        path.iter().fold(root.to_path_buf(), |p, n| p.join(n))
    }

    /// Controller eines Ziels bzw. Ordners mit Zugangsdaten (Fehler: Name und Grund).
    fn aptools_vcs(&self, id: Uuid) -> (Vec<(Uuid, Vc)>, Vec<String>) {
        let mut ok = Vec::new();
        let mut problems = Vec::new();
        for tid in self.aptools_targets(id) {
            let Some(t) = self.target(tid).cloned() else { continue };
            let creds = self.inventory_view(&t).and_then(|(_, r)| self.credential_payload(&r, Protocol::Ssh));
            match creds {
                Ok(p) => {
                    let host = self.overview.services_ip(&tid.to_string()).unwrap_or_else(|| t.host.clone());
                    let port = if t.protocol == Protocol::Ssh { t.effective_port() } else { 22 };
                    let spec = crate::patch::JobSpec { protocol: Protocol::Ssh, host: host.clone(), port, credentials: p.clone(), transport: None, legacy: self.target_legacy_of(tid) };
                    ok.push((tid, Vc { name: t.name.clone(), host, user: p.username.clone(), password: rj_core::Secret::from_bytes(p.password.0.clone()), spec, port }));
                }
                Err(e) => problems.push(format!("{}: {e}", t.name)),
            }
        }
        (ok, problems)
    }

    /// Werkzeug für ein Ziel oder einen Ordner starten (`id`, `tool`: analyse, inventar, reload, update; bei update
    /// `url` und `reboot`). Lesende Werkzeuge laufen je Controller nacheinander (Ergebnis je Ziel), schreibende in
    /// einem Lauf je Satz gleicher Zugangsdaten (Ergebnis im Ordner bzw. beim Ziel). Je ein lesender und ein
    /// schreibender Lauf zur Zeit.
    pub fn aptools_run(&mut self, c: &Value) -> Result<Value, String> {
        let root = self.aptools_root()?;
        let tool = Tool::by_name(c.get("tool").and_then(Value::as_str).unwrap_or("analyse"), c.get("url").and_then(Value::as_str), c.get("reboot").and_then(Value::as_bool).unwrap_or(false))?;
        let slot = if tool.write { &self.aptools_write } else { &self.aptools_run };
        if slot.as_ref().is_some_and(|r| !r.done.load(std::sync::atomic::Ordering::SeqCst)) {
            return Err(if tool.write { "Es läuft bereits ein Neustart oder Update" } else { "Es läuft bereits eine Analyse oder ein Inventar" }.into());
        }
        if !venv_python(&root).is_file() || !collection_dir(&root).join(tool.script).is_file() {
            return Err("Die Aruba-Werkzeuge sind noch nicht eingerichtet".into());
        }
        let id = c.get("id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()).ok_or("Ziel oder Ordner fehlt")?;
        let (vcs, problems) = self.aptools_vcs(id);
        if vcs.is_empty() {
            return Err(if problems.is_empty() {
                "Keine Aruba-Controller gefunden (Ziele mit Aruba-Symbol, erkannt über „Inventar abfragen“)".to_string()
            } else {
                format!("Keine Zugangsdaten: {}", problems.join("; "))
            });
        }
        let started = vcs.len();
        let title = if started == 1 { vcs[0].1.name.clone() } else { format!("{started} Controller") };
        let tasks: Vec<Task> = if tool.write {
            // Gleiche Zugangsdaten → ein Lauf (das Werkzeug arbeitet die Cluster parallel ab).
            let dest = self.tree_dest(&root, id);
            let mut groups: Vec<Task> = Vec::new();
            for (_, vc) in vcs {
                match groups.iter_mut().find(|g| g.vcs[0].user == vc.user && g.vcs[0].password.as_bytes() == vc.password.as_bytes()) {
                    Some(g) => g.vcs.push(vc),
                    None => groups.push(Task { vcs: vec![vc], dest: dest.clone() }),
                }
            }
            groups
        } else {
            vcs.into_iter().map(|(tid, vc)| Task { dest: self.tree_dest(&root, tid), vcs: vec![vc] }).collect()
        };
        let run = Run { id: id.to_string(), tool: c.get("tool").and_then(Value::as_str).unwrap_or("analyse").to_string(), title: title.clone(), ..Default::default() };
        for p in &problems {
            append(&run.out, &format!("Übersprungen – {p}\n"));
        }
        let handle = run.handle();
        let known = self.known_hosts_ref();
        let t2 = tool.clone();
        std::thread::spawn(move || run_tool(&root, &t2, tasks, known, &handle));
        if tool.write {
            self.aptools_write = Some(run);
        } else {
            self.aptools_run = Some(run);
        }
        Ok(json!({ "started": started, "title": title, "skipped": problems }))
    }

    /// Ausgabe, Statustafel und Ergebnisse des lesenden (`write` = false) bzw. schreibenden Laufs.
    pub fn aptools_output(&self, c: &Value) -> Value {
        let slot = if c.get("write").and_then(Value::as_bool).unwrap_or(false) { &self.aptools_write } else { &self.aptools_run };
        match slot {
            None => json!({ "run": null }),
            Some(r) => json!({ "run": {
                "id": r.id,
                "tool": r.tool,
                "title": r.title,
                "stopped": r.stop.load(std::sync::atomic::Ordering::SeqCst),
                "text": r.out.lock().unwrap_or_else(|e| e.into_inner()).clone(),
                "board": r.board.lock().unwrap_or_else(|e| e.into_inner()).clone(),
                "done": r.done.load(std::sync::atomic::Ordering::SeqCst),
                "results": r.results.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|p| json!({ "path": p, "files": std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| e.path()).filter(|x| x.is_file()).collect::<Vec<_>>()).unwrap_or_default() })).collect::<Vec<_>>(),
            }}),
        }
    }

    /// Lauf abbrechen: keine weiteren Ziele, laufendes Werkzeug beenden.
    pub fn aptools_stop(&mut self, c: &Value) -> Value {
        let slot = if c.get("write").and_then(Value::as_bool).unwrap_or(false) { &self.aptools_write } else { &self.aptools_run };
        if let Some(r) = slot {
            r.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(c) = r.child.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                let _ = c.kill();
            }
        }
        json!({ "ok": true })
    }

    /// Controller eines Ziels bzw. Ordners für den Bestätigungsdialog: Name, Adresse und der zuletzt bekannte Stand
    /// für schreibende Werkzeuge (aus „Cluster abfragen“; sonst unbekannt).
    pub fn aptools_targets_json(&self, c: &Value) -> Result<Value, String> {
        self.aptools_root()?;
        let id = c.get("id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()).ok_or("Ziel oder Ordner fehlt")?;
        let list: Vec<Value> = self
            .aptools_targets(id)
            .into_iter()
            .filter_map(|tid| {
                let t = self.target(tid)?;
                let cluster = self.overview.aruba.get(&tid.to_string());
                Some(json!({ "name": t.name, "host": t.host, "write": cluster.map(|c| c["write"].clone()) }))
            })
            .collect();
        Ok(json!({ "targets": list, "write_running": self.aptools_write.as_ref().is_some_and(|r| !r.done.load(std::sync::atomic::Ordering::SeqCst)) }))
    }

    /// Ablageordner der Protokolle eines Ziels oder Ordners (`aptools/<Baumpfad>`), für „Ordner öffnen“.
    pub fn aptools_folder(&self, c: &Value) -> Result<Value, String> {
        let root = self.aptools_root()?;
        let id = c.get("id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()).ok_or("Ziel oder Ordner fehlt")?;
        let path: Vec<String> = self.index.path_to(id).unwrap_or_default().iter().map(|n| file_name(n.name())).collect();
        let dir = path.iter().fold(root, |p, n| p.join(n));
        Ok(json!({ "path": dir, "exists": dir.is_dir() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Echter Ablauf (Netz nötig): `RJ_APTOOLS_LIVE=<Ordner> cargo test -- --ignored aptools_live`.
    #[test]
    #[ignore]
    fn aptools_live() {
        let root = PathBuf::from(std::env::var("RJ_APTOOLS_LIVE").expect("RJ_APTOOLS_LIVE"));
        let commit = install_collection(&root, &AtomicBool::new(false)).unwrap();
        eprintln!("Stand: {commit:?} · {}", status(&root));
        eprintln!("paramiko: {}", setup_python(&root, None).unwrap());
        assert!(status(&root)["venv"].as_bool().unwrap());
    }

    fn probe_vc() -> Vc {
        let creds = rj_ipc::msg::CredentialPayload { username: "probe".into(), domain: None, password: rj_ipc::msg::SecretBytes(b"geheim-123".to_vec()), private_key: None };
        let spec = crate::patch::JobSpec { protocol: Protocol::Ssh, host: "192.0.2.1".into(), port: 22, credentials: creds, transport: None, legacy: false };
        Vc { name: "vc".into(), host: "192.0.2.1".into(), user: "probe".into(), password: rj_core::Secret::from_str_exact("geheim-123"), spec, port: 22 }
    }

    /// Starter gegen eine nicht erreichbare Beispieladresse (`RJ_APTOOLS_LIVE` mit eingerichteter Sammlung).
    #[test]
    #[ignore]
    fn aptools_live_run() {
        let root = PathBuf::from(std::env::var("RJ_APTOOLS_LIVE").expect("RJ_APTOOLS_LIVE"));
        let known = rj_ssh::known_hosts::KnownHosts::new(root.join("known_hosts"));
        for name in ["analyse", "inventar"] {
            let run = Run::default();
            let tool = Tool::by_name(name, None, false).unwrap();
            run_tool(&root, &tool, vec![Task { vcs: vec![probe_vc()], dest: root.join("Probe").join("vc") }], known.clone(), &run);
            let out = run.out.lock().unwrap().clone();
            eprintln!("{out}");
            assert!(!out.contains("unbekannte Rueckfrage"), "{name}: {out}");
            assert!(!out.contains("geheim-123"), "Kennwort nie in der Ausgabe");
            assert!(!run.results.lock().unwrap().is_empty(), "{name}: Ergebnisordner verschoben");
        }
        // Schreibend: Die Vorprüfung lässt den nicht erreichbaren Controller aus, das Werkzeug startet nicht.
        let run = Run::default();
        run_tool(&root, &Tool::by_name("reload", None, false).unwrap(), vec![Task { vcs: vec![probe_vc()], dest: root.join("Probe") }], known, &run);
        let out = run.out.lock().unwrap().clone();
        eprintln!("{out}");
        assert!(out.contains("übersprungen") && out.contains("Kein Controller übrig"), "{out}");
    }

    #[test]
    fn tools_and_update_url() {
        assert_eq!(Tool::by_name("inventar", None, false).unwrap().args, vec!["--apname", "--log"]);
        assert!(Tool::by_name("update", None, false).is_err());
        assert!(Tool::by_name("update", Some("ftp://x/8.10.0.24/"), false).is_err());
        let u = Tool::by_name("update", Some(" http://192.0.2.9/aruba/8.10.0.24/ "), true).unwrap();
        assert_eq!(u.args, vec!["-f", "http://192.0.2.9/aruba/8.10.0.24/", "--log"]);
        assert!(u.answers.iter().any(|(k, v)| k.contains("automatisch neu gestartet") && v == "j"));
        assert!(u.write && !Tool::by_name("analyse", None, false).unwrap().write);
    }

    #[test]
    fn status_board_lines() {
        assert!(board_line("--- Echtzeit-Monitoring der Cluster-Upgrades (Version 4.1.0) ---"));
        assert!(board_line("192.0.2.4        | Phase 4: Monitoring (3/12 fertig)"));
        assert!(board_line(&"-".repeat(90)));
        assert!(!board_line("[10:01:02] Starte Bearbeitung für Conductor 192.0.2.4."));
        assert!(!board_line("INFO: Ziel-Version: 8.10.0.24 | Installierte Version: 8.10.0.23"));
    }

    #[test]
    fn config_is_kept_and_status_reads_version() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("tools")).unwrap();
        std::fs::write(src.join("ap_check.py"), "TOOLSUITE_VERSION = \"2.0.0\"\n").unwrap();
        std::fs::write(src.join("config.json"), "{\"neu\": 1}").unwrap();
        std::fs::write(src.join("tools/x.sh"), "").unwrap();
        let root = dir.path().join("aptools");
        let coll = collection_dir(&root);
        std::fs::create_dir_all(&coll).unwrap();
        std::fs::write(coll.join("config.json"), "{\"eigen\": 1}").unwrap();
        copy_tree(&src, &coll, &["config.json"]).unwrap();
        assert_eq!(std::fs::read_to_string(coll.join("config.json")).unwrap(), "{\"eigen\": 1}");
        assert_eq!(std::fs::read_to_string(coll.join("config.json.neu")).unwrap(), "{\"neu\": 1}");
        assert!(coll.join("tools/x.sh").is_file());
        let s = status(&root);
        assert_eq!((s["installed"].as_bool(), s["version"].as_str()), (Some(true), Some("2.0.0")));
    }
}
