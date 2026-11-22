//! Geheimnisse schwärzen: Benutzernamen und Kennwörter aller Zugangsdaten der Schlüsseldatenbank aus Texten und
//! JSON entfernen (für Berichte der Erweiterungen, die nie Zugangsdaten sehen). Anmeldenamen nur in Zeilen mit
//! Anmeldebezug (sonst träfe z. B. ein Konto „root“ jedes „Designated root“ einer STP-Ausgabe), Kennwörter überall
//! als ganzes Wort.

use serde_json::Value;
use uuid::Uuid;

use crate::app::App;

/// `secret` nur als ganzes Wort ersetzen: davor und danach kein Buchstabe, keine Ziffer, kein `_` (sonst würde ein
/// Anmeldename wie „joe“ auch „remotejoe“ treffen).
pub fn replace_word(text: &str, secret: &str) -> String {
    if secret.is_empty() || !text.contains(secret) {
        return text.to_string();
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(secret) {
        let before = rest[..pos].chars().next_back().or_else(|| out.chars().next_back());
        let after = rest[pos + secret.len()..].chars().next();
        out.push_str(&rest[..pos]);
        if word(before) || word(after) {
            out.push_str(secret);
        } else {
            out.push_str("[entfernt]");
        }
        rest = &rest[pos + secret.len()..];
    }
    out.push_str(rest);
    out
}

/// Zu ersetzende Geheimnisse.
#[derive(Default)]
pub struct Secrets {
    pub users: Vec<String>,
    pub passwords: Vec<String>,
}

impl Drop for Secrets {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.users.zeroize();
        self.passwords.zeroize();
    }
}

/// Zeile mit Anmeldebezug (Anmeldename kann vorkommen).
fn login_line(line: &str) -> bool {
    let l = line.to_lowercase();
    ["user", "login", "logged", "username", "benutzer", "konto", "account", "anmeld"].iter().any(|k| l.contains(k))
}

/// Eine Zeile bereinigen: Kennwörter immer, Anmeldenamen nur in Zeilen mit Anmeldebezug.
pub fn scrub_line(line: &str, secrets: &Secrets) -> String {
    let mut out = line.to_string();
    for p in &secrets.passwords {
        out = replace_word(&out, p);
    }
    if login_line(&out) {
        for u in &secrets.users {
            out = replace_word(&out, u);
        }
    }
    out
}

/// Alle Texte eines JSON-Werts (in beliebiger Tiefe) zeilenweise bereinigen.
pub fn scrub(v: &mut Value, secrets: &Secrets) {
    match v {
        Value::Object(o) => o.values_mut().for_each(|x| scrub(x, secrets)),
        Value::Array(a) => a.iter_mut().for_each(|x| scrub(x, secrets)),
        Value::String(s) => {
            if !secrets.users.is_empty() || !secrets.passwords.is_empty() {
                *s = s.split('\n').map(|l| scrub_line(l, secrets)).collect::<Vec<_>>().join("\n");
            }
        }
        _ => {}
    }
}

impl App {
    /// Benutzernamen und Kennwörter aller Zugangsdaten im Index (Kennwörter ab 4, Namen ab 3 Zeichen).
    pub(crate) fn secrets(&self) -> Secrets {
        let Some(vault) = self.vault.as_ref() else { return Secrets::default() };
        let mut entries: Vec<Uuid> = self.index.logins.iter().map(|l| l.entry.entry).collect();
        self.index.for_each_settings(|s| entries.extend(s.credentials.values().map(|r| r.entry)));
        entries.sort();
        entries.dedup();
        let mut out = Secrets::default();
        for e in entries {
            if let Some(c) = vault.credential(e) {
                let user = c.data.username.trim().to_string();
                if user.chars().count() >= 3 {
                    out.users.push(user);
                }
                let pw = String::from_utf8_lossy(c.data.password.as_bytes()).into_owned();
                if pw.chars().count() >= 4 {
                    out.passwords.push(pw);
                }
            }
        }
        // Längere zuerst, damit Teilstücke nicht vorher ersetzt werden.
        for v in [&mut out.users, &mut out.passwords] {
            v.sort_by_key(|s| std::cmp::Reverse(s.len()));
            v.dedup();
        }
        out
    }

    /// JSON-Wert ohne Geheimnisse der Schlüsseldatenbank.
    pub(crate) fn redact(&self, mut v: Value) -> Value {
        let secrets = self.secrets();
        scrub(&mut v, &secrets);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secrets_are_replaced_only_as_whole_words() {
        assert_eq!(replace_word("remotejoe 0.1", "joe"), "remotejoe 0.1");
        assert_eq!(replace_word("login as: joe", "joe"), "login as: [entfernt]");
        assert_eq!(replace_word("joe,joes p15,(joe)", "joe"), "[entfernt],joes p15,([entfernt])");
        assert_eq!(replace_word("user_joe joe_x", "joe"), "user_joe joe_x");
        let mut v = json!({ "meta": { "program": "remotejoe" }, "x": "Konto „joe“ ist kein Admin-Konto" });
        scrub(&mut v, &Secrets { users: vec!["joe".into()], passwords: vec![] });
        assert_eq!(v["meta"]["program"], "remotejoe");
        assert_eq!(v["x"], "Konto „[entfernt]“ ist kein Admin-Konto");
    }

    #[test]
    fn user_names_only_in_login_lines_passwords_everywhere() {
        let sec = Secrets { users: vec!["root".into()], passwords: vec!["Geheim-123".into()] };
        let mut v = json!("Designated root has priority 32768\nThis bridge is the root\nUser \"root\" has logged in 8 times\nkey Geheim-123 here\n");
        scrub(&mut v, &sec);
        let out = v.as_str().unwrap();
        assert!(out.contains("Designated root has priority"), "{out}");
        assert!(out.contains("This bridge is the root"), "{out}");
        assert!(out.contains("User \"[entfernt]\" has logged in"), "{out}");
        assert!(!out.contains("Geheim-123"), "{out}");
    }
}
