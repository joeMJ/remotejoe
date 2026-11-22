//! `rj-update`: Werkzeug für Herausgeber von remotejoe-Paketen.
//!
//! ```text
//! rj-update plattform
//!     gibt die Plattformkennung dieses Rechners aus (z. B. linux-x86_64 oder linux-x86_64-ubuntu2404)
//! rj-update schluessel <ordner>
//!     erzeugt update-key.priv (nur lokal aufbewahren!) und update-key.pub
//! rj-update packen <programmordner> <version> <plattform> <privater-schluessel> <ausgabeordner>
//!     packt remotejoe-<version>-<plattform>.tar.zst (bzw. .zip), schreibt
//!     SHA256SUMS und signiert sie (SHA256SUMS.sig)
//! rj-update pruefen <ausgabeordner> <oeffentlicher-schluessel>
//!     prüft Signatur und Prüfsummen wie das Programm
//! rj-update hochladen <gitea-adresse> <besitzer/name> <version> <ausgabeordner> [<aenderungstext-datei>]
//!     legt die Veröffentlichung an und lädt alle Dateien des Ordners hoch;
//!     Zugriffsschlüssel aus der Umgebungsvariable RJ_GITEA_TOKEN
//! rj-update packen-ohne-signatur <programmordner> <version> <plattform> <ausgabeordner>
//!     wie „packen“, aber ohne Signatur (für die automatische Paketierung auf GitHub)
//! rj-update packen-plugin <bibliothek> <manifest.json> <plattform> <ausgabeordner> [--schluessel <datei>] [--oeffentlich <datei>]
//!     packt die Bibliothek einer Erweiterung (z. B. librj_beispiel_plugin.so) samt Manifest (Kennung, eigene
//!     Version) als remotejoe-<kennung>-<version>-<plattform>.tar.zst (Windows: .zip) mit eigener, signierter
//!     SHA256SUMS darin (der Lader prüft sie bei jedem Start), trägt die Fassung in die index.json des
//!     Ausgabeordners ein (der Ordner der Erweiterung für diese Plattform in der Quelle) und Paket wie Index in
//!     dessen SHA256SUMS (danach „signieren“)
//! rj-update katalog <ordner> <kennung>=<Anzeigename> …
//!     schreibt katalog.json (alle Erweiterungen der Quelle mit Anzeigename) und die SHA256SUMS des Ordners
//!     (der Ordner /erweiterungen/ der Quelle; danach „signieren“)
//! rj-update signieren <ordner> [--schluessel <datei>] [--oeffentlich <datei>]
//!     prüft alle Einträge der SHA256SUMS im Ordner und signiert sie (SHA256SUMS.sig) mit dem lokalen
//!     privaten Schlüssel, nur wenn er zum eingebauten öffentlichen Schlüssel passt
//! rj-update abschliessen <besitzer/name> <tag> [--minuten <n>] [--schluessel <datei>] [--oeffentlich <datei>]
//!     GitHub: prüft den von der Paketierung angelegten Entwurf, signiert SHA256SUMS mit dem
//!     lokalen privaten Schlüssel und veröffentlicht die Veröffentlichung. Zugriffsschlüssel aus
//!     der Umgebungsvariable RJ_GITHUB_TOKEN oder aus dem Schlüsselbund
//!     (`secret-tool lookup service remotejoe-github`)
//! ```
//!
//! Der private Schlüssel liegt standardmäßig unter `~/.config/remotejoe/signing/update-key.priv`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use rj_update::github::{self, Endpoints};
use rj_update::http::{self, Control};
use rj_update::sign::{self, SIG, SUMS};
use rj_update::{archive, asset_name, Limits, UpdateError, Version};
use zeroize::Zeroizing;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("plattform") if args.len() == 1 => {
            println!("{}", rj_update::platform());
            Ok(())
        }
        Some("schluessel") if args.len() == 1 => keys(&default_key_dir()),
        Some("schluessel") if args.len() == 2 => keys(Path::new(&args[1])),
        Some("packen") if args.len() == 6 => pack(Path::new(&args[1]), &args[2], &args[3], Some(Path::new(&args[4])), Path::new(&args[5])),
        Some("packen-ohne-signatur") if args.len() == 5 => pack(Path::new(&args[1]), &args[2], &args[3], None, Path::new(&args[4])),
        Some("packen-plugin") if args.len() >= 5 => pack_plugin(Path::new(&args[1]), Path::new(&args[2]), &args[3], Path::new(&args[4]), &args[5..]),
        Some("katalog") if args.len() >= 3 => katalog(Path::new(&args[1]), &args[2..]),
        Some("signieren") if args.len() >= 2 => sign_dir(Path::new(&args[1]), &args[2..]),
        Some("pruefen") if args.len() == 3 => check(Path::new(&args[1]), Path::new(&args[2])),
        Some("hochladen") if args.len() == 5 || args.len() == 6 => upload(&args[1], &args[2], &args[3], Path::new(&args[4]), args.get(5).map(PathBuf::from)),
        Some("abschliessen") if args.len() >= 3 => finish(&args[1], &args[2], &args[3..]),
        _ => {
            eprintln!("Aufruf: rj-update schluessel [<ordner>] | packen <programmordner> <version> <plattform> <privater-schluessel> <ausgabeordner> | packen-ohne-signatur <programmordner> <version> <plattform> <ausgabeordner> | packen-plugin <bibliothek> <manifest.json> <plattform> <ausgabeordner> [--schluessel <datei>] [--oeffentlich <datei>] | katalog <ordner> <kennung>=<Name> … | signieren <ordner> [--schluessel <datei>] [--oeffentlich <datei>] | pruefen <ausgabeordner> <oeffentlicher-schluessel> | hochladen <gitea-adresse> <besitzer/name> <version> <ausgabeordner> [<aenderungstext-datei>] | abschliessen <besitzer/name> <tag> [--minuten <n>] [--schluessel <datei>] [--oeffentlich <datei>] [--zusatz <datei> …]");
            return ExitCode::from(2);
        }
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e}");
            ExitCode::FAILURE
        }
    }
}

fn keys(dir: &Path) -> Result<(), UpdateError> {
    fs::create_dir_all(dir)?;
    let (priv_path, pub_path) = (dir.join("update-key.priv"), dir.join("update-key.pub"));
    if priv_path.exists() {
        return Err(UpdateError::BadKey(format!("{} existiert bereits; nicht überschrieben", priv_path.display())));
    }
    let sk = sign::generate()?;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    opts.open(&priv_path)?.write_all(sign::secret_key_text(&sk).as_bytes())?;
    fs::write(&pub_path, format!("# Öffentlicher Schlüssel für remotejoe-Updates\n{}", sign::public_key_text(&sk.verifying_key())))?;
    println!("Privater Schlüssel: {} (nur lokal aufbewahren, nie in ein Repository)", priv_path.display());
    println!("Öffentlicher Schlüssel: {} (nach crates/ui/res/update-key.pub kopieren)", pub_path.display());
    Ok(())
}

fn pack(src: &Path, version: &str, platform: &str, key: Option<&Path>, out: &Path) -> Result<(), UpdateError> {
    let v = Version::parse(version)?;
    let name = asset_name(&v.to_string(), platform);
    let data = if name.ends_with(".zip") { archive::pack_zip(src)? } else { archive::pack_tar_zst(src)? };
    fs::create_dir_all(out)?;
    fs::write(out.join(&name), &data)?;
    let sums = sign::sums_text(&[(name.clone(), sign::sha256_hex(&data))]);
    fs::write(out.join(SUMS), &sums)?;
    if let Some(key) = key {
        let sk = sign::secret_key_from_text(&Zeroizing::new(fs::read_to_string(key)?))?;
        fs::write(out.join(SIG), sign::sign(&sk, sums.as_bytes()))?;
        println!("{} ({} Bytes), {SUMS}, {SIG} in {}", name, data.len(), out.display());
    } else {
        println!("{} ({} Bytes), {SUMS} (ohne Signatur) in {}", name, data.len(), out.display());
    }
    Ok(())
}

/// Schlüsselpfade aus `--schluessel`/`--oeffentlich` (Vorgabe: lokaler Update-Schlüssel, eingebauter öffentlicher).
fn key_paths(opts: &[String]) -> Result<(PathBuf, PathBuf), UpdateError> {
    let (mut key_path, mut pub_path) = (default_key_dir().join("update-key.priv"), PathBuf::from("crates/ui/res/update-key.pub"));
    let mut it = opts.iter();
    while let Some(o) = it.next() {
        let v = it.next().ok_or_else(|| UpdateError::Url(format!("Wert für {o} fehlt")))?;
        match o.as_str() {
            "--schluessel" => key_path = PathBuf::from(v),
            "--oeffentlich" => pub_path = PathBuf::from(v),
            other => return Err(UpdateError::Url(format!("Unbekannte Option {other}"))),
        }
    }
    Ok((key_path, pub_path))
}

fn signing_key(key_path: &Path, pub_path: &Path) -> Result<sign::SigningKey, UpdateError> {
    let sk = sign::secret_key_from_text(&Zeroizing::new(fs::read_to_string(key_path).map_err(|e| UpdateError::BadKey(format!("{}: {e}", key_path.display())))?))?;
    let embedded = sign::public_key_from_text(&fs::read_to_string(pub_path).map_err(|e| UpdateError::BadKey(format!("{}: {e}", pub_path.display())))?)?;
    if sk.verifying_key() != embedded {
        return Err(UpdateError::BadKey(format!("Der private Schlüssel gehört nicht zum eingebauten öffentlichen Schlüssel ({}).", pub_path.display())));
    }
    Ok(sk)
}

/// Bibliothek einer Erweiterung als Paket für die Ablage packen (siehe Kopf der Datei).
fn pack_plugin(lib: &Path, manifest: &Path, platform: &str, out: &Path, opts: &[String]) -> Result<(), UpdateError> {
    let mtext = fs::read_to_string(manifest).map_err(|e| UpdateError::Url(format!("{}: {e}", manifest.display())))?;
    let m = rj_plugin_api::Manifest::parse(&mtext).map_err(UpdateError::Parse)?;
    let v = Version::parse(&m.version)?;
    let (key_path, pub_path) = key_paths(opts)?;
    let sk = signing_key(&key_path, &pub_path)?;
    let id = &m.id;
    let lib_name = if platform.starts_with("windows") { format!("rj_{id}_plugin.dll") } else { format!("librj_{id}_plugin.so") };
    let data = fs::read(lib).map_err(|e| UpdateError::Url(format!("{}: {e}", lib.display())))?;
    let stage = tempfile::tempdir()?;
    fs::write(stage.path().join(&lib_name), &data)?;
    fs::write(stage.path().join(rj_plugin_api::MANIFEST_FILE), &mtext)?;
    let inner = sign::sums_text(&[(lib_name.clone(), sign::sha256_hex(&data)), (rj_plugin_api::MANIFEST_FILE.into(), sign::sha256_hex(mtext.as_bytes()))]);
    fs::write(stage.path().join(SUMS), &inner)?;
    fs::write(stage.path().join(SIG), sign::sign(&sk, inner.as_bytes()))?;
    let ext = if platform.starts_with("windows") { "zip" } else { "tar.zst" };
    let name = format!("remotejoe-{id}-{v}-{platform}.{ext}");
    let pkg = if ext == "zip" { archive::pack_zip(stage.path())? } else { archive::pack_tar_zst(stage.path())? };
    fs::create_dir_all(out)?;
    fs::write(out.join(&name), &pkg)?;
    let mut entries: Vec<(String, String)> = match fs::read_to_string(out.join(SUMS)) {
        Ok(t) => t.lines().filter_map(|l| l.split_once("  ").map(|(h, n)| (n.to_string(), h.to_string()))).filter(|(n, _)| n != &name).collect(),
        Err(_) => Vec::new(),
    };
    entries.push((name.clone(), sign::sha256_hex(&pkg)));
    // Index des Ordners (alle Fassungen dieser Erweiterung für diese Plattform) ergänzen und mit in die Prüfsummen.
    use rj_update::erweiterungen::{Fassung, Index, INDEX_FILE};
    let mut index = match fs::read(out.join(INDEX_FILE)) {
        Ok(b) => Index::parse(&b).map_err(UpdateError::Parse)?,
        Err(_) => Index { kennung: id.clone(), fassungen: Vec::new() },
    };
    if index.kennung != *id {
        return Err(UpdateError::Parse(format!("{INDEX_FILE} in {} gehört zu „{}“, nicht zu „{id}“", out.display(), index.kennung)));
    }
    index.put(Fassung { version: v.to_string(), api: m.api, min_remotejoe: m.min_remotejoe.clone(), braucht: m.braucht.clone(), datei: name.clone() });
    let itext = index.to_text();
    fs::write(out.join(INDEX_FILE), &itext)?;
    entries.retain(|(n, _)| n != INDEX_FILE);
    entries.push((INDEX_FILE.to_string(), sign::sha256_hex(itext.as_bytes())));
    entries.sort();
    fs::write(out.join(SUMS), sign::sums_text(&entries))?;
    let _ = fs::remove_file(out.join(SIG));
    println!("{} ({} Bytes, enthält {lib_name} und {} mit signierter {SUMS}); {INDEX_FILE} ({} Fassungen) und {SUMS} des Ordners ergänzt, Signatur entfernt – jetzt „signieren“", name, pkg.len(), rj_plugin_api::MANIFEST_FILE, index.fassungen.len());
    Ok(())
}

/// Katalog der Erweiterungen in den Ordner schreiben (`katalog.json` und SHA256SUMS ohne Signatur).
fn katalog(out: &Path, entries: &[String]) -> Result<(), UpdateError> {
    use rj_update::erweiterungen::{Katalog, KatalogEintrag, KATALOG_FILE};
    let mut k = Katalog::default();
    for e in entries {
        let (id, name) = e.split_once('=').ok_or_else(|| UpdateError::Parse(format!("„{e}“: erwartet <kennung>=<Anzeigename>")))?;
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') || name.trim().is_empty() {
            return Err(UpdateError::Parse(format!("„{e}“: Kennung aus Kleinbuchstaben, Ziffern, Unterstrich und ein Name")));
        }
        k.erweiterungen.push(KatalogEintrag { id: id.into(), name: name.trim().into() });
    }
    fs::create_dir_all(out)?;
    let text = k.to_text();
    fs::write(out.join(KATALOG_FILE), &text)?;
    fs::write(out.join(SUMS), sign::sums_text(&[(KATALOG_FILE.to_string(), sign::sha256_hex(text.as_bytes()))]))?;
    let _ = fs::remove_file(out.join(SIG));
    println!("{KATALOG_FILE} mit {} Erweiterungen und {SUMS} in {} – jetzt „signieren“", k.erweiterungen.len(), out.display());
    Ok(())
}

/// Vorhandene SHA256SUMS eines Ordners prüfen und signieren.
fn sign_dir(dir: &Path, opts: &[String]) -> Result<(), UpdateError> {
    let (key_path, pub_path) = key_paths(opts)?;
    let sums = fs::read(dir.join(SUMS))?;
    let mut n = 0;
    for line in String::from_utf8_lossy(&sums).lines() {
        let Some((hash, name)) = line.split_once("  ") else { continue };
        let got = sign::sha256_hex(&fs::read(dir.join(name)).map_err(|e| UpdateError::Url(format!("{name}: {e}")))?);
        if got != hash {
            return Err(UpdateError::BadChecksum);
        }
        n += 1;
    }
    if n == 0 {
        return Err(UpdateError::Url(format!("{SUMS} in {} ist leer", dir.display())));
    }
    let sk = signing_key(&key_path, &pub_path)?;
    fs::write(dir.join(SIG), sign::sign(&sk, &sums))?;
    println!("{SIG} geschrieben ({n} Pakete in {SUMS})");
    check(dir, &pub_path)
}

fn check(dir: &Path, key: &Path) -> Result<(), UpdateError> {
    let vk = sign::public_key_from_text(&fs::read_to_string(key)?)?;
    let sums = fs::read(dir.join(SUMS))?;
    sign::verify(&vk, &sums, &fs::read(dir.join(SIG))?)?;
    for line in String::from_utf8_lossy(&sums).lines() {
        let Some((_, name)) = line.split_once("  ") else { continue };
        let data = fs::read(dir.join(name))?;
        sign::verify_package(&vk, &sums, &fs::read(dir.join(SIG))?, name, &data)?;
        println!("in Ordnung: {name}");
    }
    Ok(())
}

fn upload(base: &str, repo: &str, version: &str, dir: &Path, notes: Option<PathBuf>) -> Result<(), UpdateError> {
    let token = Zeroizing::new(std::env::var("RJ_GITEA_TOKEN").map_err(|_| UpdateError::BadKey("Umgebungsvariable RJ_GITEA_TOKEN fehlt".into()))?);
    let v = Version::parse(version)?;
    let base = base.trim_end_matches('/');
    let body = match notes {
        Some(p) => fs::read_to_string(p)?,
        None => String::new(),
    };
    let auth = vec![("Authorization".to_string(), format!("token {}", token.as_str())), ("Accept".into(), "application/json".into())];
    let ctl = Control::new(Limits { max_bytes: 8 * 1024 * 1024, timeout: Duration::from_secs(60) }, None);
    let mut h = auth.clone();
    h.push(("Content-Type".into(), "application/json".into()));
    let req = serde_json::json!({ "tag_name": version, "name": v.to_string(), "body": body, "draft": false, "prerelease": v.is_prerelease() });
    let r = http::request("POST", &format!("{base}/api/v1/repos/{repo}/releases"), &h, req.to_string().as_bytes(), &ctl)?;
    if r.status != 201 {
        return Err(UpdateError::Status { status: r.status, what: "Veröffentlichung anlegen".into() });
    }
    let id = serde_json::from_slice::<serde_json::Value>(&r.body).ok().and_then(|j| j["id"].as_u64()).ok_or_else(|| UpdateError::Parse("Kennung der Veröffentlichung fehlt".into()))?;
    let mut names: Vec<String> = fs::read_dir(dir)?.filter_map(|e| e.ok()).filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    for name in names {
        let data = fs::read(dir.join(&name))?;
        let boundary = format!("rjgrenze{}", sign::sha256_hex(name.as_bytes()));
        let mut form = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"attachment\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
        form.extend_from_slice(&data);
        form.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let mut h = auth.clone();
        h.push(("Content-Type".into(), format!("multipart/form-data; boundary={boundary}")));
        let ctl = Control::new(Limits { max_bytes: 8 * 1024 * 1024, timeout: Duration::from_secs(900) }, None);
        let enc: String = name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
        let r = http::request("POST", &format!("{base}/api/v1/repos/{repo}/releases/{id}/assets?name={enc}"), &h, &form, &ctl)?;
        if r.status != 201 {
            return Err(UpdateError::Status { status: r.status, what: format!("Hochladen von {name}") });
        }
        println!("hochgeladen: {name}");
    }
    Ok(())
}

fn default_key_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))).unwrap_or_else(|| PathBuf::from("."));
    base.join("remotejoe").join("signing")
}

/// Zugriffsschlüssel für GitHub: Umgebungsvariable oder Schlüsselbund (`secret-tool`).
fn github_token() -> Result<Zeroizing<String>, UpdateError> {
    if let Ok(t) = std::env::var("RJ_GITHUB_TOKEN") {
        if !t.trim().is_empty() {
            return Ok(Zeroizing::new(t.trim().to_string()));
        }
    }
    let out = std::process::Command::new("secret-tool").args(["lookup", "service", "remotejoe-github"]).output();
    match out {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => Ok(Zeroizing::new(String::from_utf8_lossy(&o.stdout).trim().to_string())),
        _ => Err(UpdateError::BadKey(
            "Kein GitHub-Zugriffsschlüssel: RJ_GITHUB_TOKEN setzen oder ihn einmal im Schlüsselbund ablegen:\n  secret-tool store --label='remotejoe GitHub' service remotejoe-github".into(),
        )),
    }
}

fn finish(repo: &str, tag: &str, opts: &[String]) -> Result<(), UpdateError> {
    let (mut minutes, mut key_path, mut pub_path) = (30u64, default_key_dir().join("update-key.priv"), PathBuf::from("crates/ui/res/update-key.pub"));
    let mut extra: Vec<(String, Vec<u8>)> = Vec::new();
    let mut it = opts.iter();
    while let Some(o) = it.next() {
        let v = it.next().ok_or_else(|| UpdateError::Url(format!("Wert für {o} fehlt")))?;
        match o.as_str() {
            "--minuten" => minutes = v.parse().map_err(|_| UpdateError::Url("--minuten erwartet eine Zahl".into()))?,
            "--schluessel" => key_path = PathBuf::from(v),
            "--oeffentlich" => pub_path = PathBuf::from(v),
            // Zusätzliche Datei (z. B. die MSI): wird angehängt und in SHA256SUMS signiert.
            "--zusatz" => {
                let p = PathBuf::from(v);
                let name = p.file_name().and_then(|n| n.to_str()).ok_or_else(|| UpdateError::Url(format!("Dateiname fehlt: {v}")))?.to_string();
                if !name.starts_with("remotejoe-") {
                    return Err(UpdateError::Url(format!("{name}: Zusatzdateien müssen mit „remotejoe-“ beginnen")));
                }
                extra.push((name, fs::read(&p)?));
            }
            other => return Err(UpdateError::Url(format!("Unbekannte Option {other}"))),
        }
    }
    let sk = sign::secret_key_from_text(&Zeroizing::new(fs::read_to_string(&key_path).map_err(|e| UpdateError::BadKey(format!("{}: {e}", key_path.display())))?))?;
    // Der Signaturschlüssel muss zu dem im Programm eingebauten öffentlichen Schlüssel gehören, sonst
    // könnte kein installiertes Programm das Paket annehmen.
    let embedded = sign::public_key_from_text(&fs::read_to_string(&pub_path).map_err(|e| UpdateError::BadKey(format!("{}: {e}", pub_path.display())))?)?;
    if embedded != sk.verifying_key() {
        return Err(UpdateError::BadKey(format!("Der private Schlüssel gehört nicht zum eingebauten öffentlichen Schlüssel ({}).", pub_path.display())));
    }
    let token = github_token()?;
    println!("Warte auf die Paketierung von {tag} (bis zu {minutes} Minuten) …");
    let done = github::finish_release_with(&Endpoints::default(), repo, tag, &token, &sk, &embedded, Duration::from_secs(minutes * 60), None, &extra)?;
    println!("Veröffentlicht: {} mit {}", done.tag, done.archives.join(", "));
    Ok(())
}
