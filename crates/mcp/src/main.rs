//! `rj-mcp`: Claude-Anbindung für remotejoe. Claude Desktop startet dieses Programm (MCP über stdio, JSON-RPC 2.0,
//! je Zeile eine Nachricht). Werkzeugaufrufe gehen über den lokalen Kanal an das laufende remotejoe; dort gelten die
//! Freigaben (Ordner, Anmeldeobjekte, Freigabe je Aktion). Dieses Programm selbst kennt keine Zugangsdaten.
//!
//! Den Kanal nennt remotejoe in der Datei `mcp-kanal` im Datenordner (Unix-Socket bzw. Named Pipe, nur für das
//! eigene Benutzerkonto).

use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use serde_json::{json, Value};

const PROTOCOL: &str = "2025-06-18";

/// Werkzeuge mit Beschreibung und Eingabeschema.
fn tools() -> Value {
    let id = |what: &str| json!({ "type": "object", "properties": { "id": { "type": "string", "description": what } }, "required": ["id"] });
    json!([
        { "name": "ordner_liste", "description": "Die für Claude freigegebenen Ordner in remotejoe (ID und Pfad).", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "ziele_suchen", "description": "Ziele (Verbindungen) in den freigegebenen Ordnern suchen: Teil von Name, Adresse oder Ordnerpfad; leer = alle. Liefert ID, Name, Typ, Ordner, Version, je nach Einstellung Adresse und Seriennummer.", "inputSchema": { "type": "object", "properties": { "text": { "type": "string" }, "limit": { "type": "integer", "description": "höchstens so viele (Vorgabe 50, max. 500)" } } } },
        { "name": "ziel_uebersicht", "description": "Übersicht eines Ziels: Typ, Ordner, Zugangswege und gespeichertes Inventar (System, CPUs, Speicher, Laufzeit, je nach Einstellung Adressen und Seriennummer). Nie Zugangsdaten.", "inputSchema": id("Ziel-ID aus ziele_suchen") },
        { "name": "ordner_inventar", "description": "Gespeichertes Inventar aller Ziele unter einem freigegebenen Ordner (z. B. alle APs eines Standorts mit Version).", "inputSchema": id("Ordner-ID aus ordner_liste") },
        { "name": "ziele_abgleichen", "description": "Abgleich mit einer externen Liste: je Wert (IP-Adresse, MAC, Seriennummer oder Name) die passenden Ziele in remotejoe; meldet, was nicht gefunden wurde.", "inputSchema": { "type": "object", "properties": { "werte": { "type": "array", "items": { "type": "string" }, "description": "bis zu 2000 Werte" } }, "required": ["werte"] } },
        { "name": "aruba_cluster", "description": "Zuletzt in remotejoe abgefragter Stand eines Aruba-Instant-Clusters (APs, online/offline, Anwender je AP). Ohne neue Abfrage.", "inputSchema": id("Ziel-ID des virtuellen Controllers") },
        { "name": "sitzungen", "description": "Offene Sitzungen in remotejoe zu freigegebenen Zielen.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "verbinden", "description": "Öffnet in remotejoe eine Verbindung zum Ziel. Nur bei Zielen mit für Claude freigegebenem Anmeldeobjekt; der Nutzer muss jede Aktion in remotejoe freigeben.", "inputSchema": id("Ziel-ID") },
        { "name": "inventar_abfragen", "description": "Startet in remotejoe eine neue Inventarabfrage (Anmeldung am Ziel). Freigabe wie bei verbinden; Ergebnis danach mit ziel_uebersicht.", "inputSchema": id("Ziel-ID") },
        { "name": "cluster_abfragen", "description": "Startet in remotejoe eine neue Abfrage eines Aruba-Clusters. Freigabe wie bei verbinden; Ergebnis danach mit aruba_cluster.", "inputSchema": id("Ziel-ID des virtuellen Controllers") },
        { "name": "nutzer_anzeigen", "description": "Öffnet in remotejoe die Liste der Anwender eines Aruba-Clusters. Freigabe wie bei verbinden.", "inputSchema": id("Ziel-ID des virtuellen Controllers") }
    ])
}

/// Kanal zum laufenden remotejoe aus der Datei `mcp-kanal` im Datenordner.
fn endpoint() -> Result<String, String> {
    let dir = rj_core::config::data_dir().ok_or("Datenordner von remotejoe nicht gefunden")?;
    let s = std::fs::read_to_string(dir.join("mcp-kanal")).map_err(|_| "remotejoe läuft nicht (oder die Claude-Anbindung ist dort nicht eingeschaltet)".to_string())?;
    Ok(s.trim().to_string())
}

/// Eine Anfrage an remotejoe, eine Antwortzeile zurück. Aktionen warten auf die Freigabe in remotejoe (bis 2 min).
fn ask(request: &Value) -> Result<Value, String> {
    let ep = endpoint()?;
    let mut line = request.to_string();
    line.push('\n');
    #[cfg(unix)]
    let answer = {
        let mut s = std::os::unix::net::UnixStream::connect(&ep).map_err(|_| "remotejoe läuft nicht oder ist nicht erreichbar".to_string())?;
        s.set_read_timeout(Some(Duration::from_secs(150))).map_err(|e| e.to_string())?;
        s.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        let mut r = BufReader::new(s);
        let mut out = String::new();
        r.read_line(&mut out).map_err(|e| e.to_string())?;
        out
    };
    #[cfg(windows)]
    let answer = {
        let _ = Duration::from_secs(0);
        let mut f = std::fs::OpenOptions::new().read(true).write(true).open(&ep).map_err(|_| "remotejoe läuft nicht oder ist nicht erreichbar".to_string())?;
        f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        let mut r = BufReader::new(f);
        let mut out = String::new();
        r.read_line(&mut out).map_err(|e| e.to_string())?;
        out
    };
    if answer.trim().is_empty() {
        return Err("remotejoe hat nicht geantwortet".into());
    }
    serde_json::from_str(answer.trim()).map_err(|e| format!("Antwort unlesbar: {e}"))
}

/// Eine JSON-RPC-Nachricht beantworten; `None` bei Benachrichtigungen.
fn handle(msg: &Value, call: &dyn Fn(&Value) -> Result<Value, String>) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    let result = match method {
        "initialize" => {
            let version = msg.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "remotejoe", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Zugriff auf remotejoe (Verbindungsmanager): Ziele, gespeichertes Inventar und Aruba-Clusterdaten aus den vom Nutzer freigegebenen Ordnern. Zugangsdaten gibt es nie. Aktionen muss der Nutzer in remotejoe einzeln freigeben."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or_default();
            let args = msg.pointer("/params/arguments").cloned().unwrap_or(json!({}));
            let (text, error) = match call(&json!({ "tool": name, "args": args })) {
                Ok(v) => match v.get("error").and_then(Value::as_str) {
                    Some(e) => (e.to_string(), true),
                    None => (serde_json::to_string_pretty(&v).unwrap_or_default(), false),
                },
                Err(e) => (e, true),
            };
            Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": error }))
        }
        m if m.starts_with("notifications/") => return None,
        other => Err(json!({ "code": -32601, "message": format!("Methode {other} nicht unterstützt") })),
    };
    let id = id?;
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg, &ask),
            Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_basics() {
        let none = |_: &Value| -> Result<Value, String> { Err("remotejoe läuft nicht".into()) };
        let init = handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }), &none).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "remotejoe");
        assert!(handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }), &none).is_none());
        let list = handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }), &none).unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 11);
        let call = handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "ziele_suchen", "arguments": {} } }), &none).unwrap();
        assert_eq!(call["result"]["isError"], true);
        assert!(call["result"]["content"][0]["text"].as_str().unwrap().contains("läuft nicht"));
        let ok = |r: &Value| -> Result<Value, String> { Ok(json!({ "echo": r["tool"] })) };
        let call = handle(&json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "sitzungen" } }), &ok).unwrap();
        assert_eq!(call["result"]["isError"], false);
        assert!(call["result"]["content"][0]["text"].as_str().unwrap().contains("sitzungen"));
        let err = handle(&json!({ "jsonrpc": "2.0", "id": 5, "method": "gibtsnicht" }), &none).unwrap();
        assert_eq!(err["error"]["code"], -32601);
    }
}
