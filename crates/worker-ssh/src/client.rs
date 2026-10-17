//! SSH-Verbindung: Anmeldung, Host-Schlüssel-Prüfung, interaktive Shell.

use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use rj_core::Secret;
use tokio::sync::{mpsc, oneshot};

use crate::known_hosts::{fingerprint, HostKeyQuestion, HostKeyStatus, KnownHosts};

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("Verbindung zu {0} fehlgeschlagen: {1}")]
    Connect(String, String),
    #[error("Zeitüberschreitung beim Verbindungsaufbau")]
    Timeout,
    #[error("Host-Schlüssel abgelehnt")]
    HostKeyRejected,
    #[error("Anmeldung fehlgeschlagen (versucht: {0})")]
    AuthFailed(String),
    #[error("Schlüssel nicht lesbar: {0}")]
    BadKey(String),
    #[error("Kennwort ist kein gültiges UTF-8")]
    PasswordEncoding,
    #[error("SSH: {0}")]
    Protocol(#[from] russh::Error),
    #[error("Kanal geschlossen")]
    Closed,
}

/// Anmeldeverfahren in der Reihenfolge der Versuche.
#[derive(Clone)]
pub enum Auth {
    Password(Secret),
    /// Privater Schlüssel im OpenSSH-Format (Ed25519, RSA, ECDSA), optional mit Passphrase.
    Key { pem: Secret, passphrase: Option<Secret> },
    /// Tastatur-interaktiv; jede Abfrage ohne Echo wird mit dem Kennwort beantwortet.
    KeyboardInteractive(Secret),
}

impl Auth {
    fn name(&self) -> &'static str {
        match self {
            Auth::Password(_) => "Kennwort",
            Auth::Key { .. } => "Schlüssel",
            Auth::KeyboardInteractive(_) => "tastatur-interaktiv",
        }
    }
}

/// Rückfrage mit Antwortkanal.
pub type HostKeyAsk = mpsc::UnboundedSender<(HostKeyQuestion, oneshot::Sender<bool>)>;

/// Verbindungsparameter.
#[derive(Clone)]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Vec<Auth>,
    pub connect_timeout: Duration,
    pub keepalive: Duration,
    /// Ältere, unsichere Verfahren zusätzlich anbieten (SHA-1-Schlüsselaustausch, CBC, HMAC-SHA1, DSA).
    pub legacy: bool,
}

struct Handler {
    host: String,
    port: u16,
    known: KnownHosts,
    ask: HostKeyAsk,
    server_key: Option<PublicKey>,
    /// Zeit, die der Benutzer für Rückfragen gebraucht hat; sie zählt nicht
    /// zum Zeitlimit des Verbindungsaufbaus.
    asking: Arc<std::sync::Mutex<AskClock>>,
}

#[derive(Debug, Default)]
struct AskClock {
    since: Option<std::time::Instant>,
    spent: Duration,
}

impl client::Handler for Handler {
    type Error = SshError;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let key = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(c) => PublicKey::from(c.public_key().clone()),
        };
        self.server_key = Some(key.clone());
        let status = self.known.check(&self.host, self.port, &key);
        if status == HostKeyStatus::Known {
            return Ok(true);
        }
        let q = HostKeyQuestion {
            host: self.host.clone(),
            port: self.port,
            algorithm: key.algorithm().to_string(),
            fingerprint: fingerprint(&key),
            status,
            key: key.to_openssh().unwrap_or_default(),
        };
        let (tx, rx) = oneshot::channel();
        if self.ask.send((q, tx)).is_err() {
            return Ok(false);
        }
        self.asking.lock().unwrap_or_else(|e| e.into_inner()).since = Some(std::time::Instant::now());
        let accepted = rx.await.unwrap_or(false);
        {
            let mut c = self.asking.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = c.since.take() {
                c.spent += t.elapsed();
            }
        }
        if accepted {
            self.known
                .remember(&self.host, self.port, &key)
                .map_err(|e| SshError::Connect(self.host.clone(), format!("Host-Schlüssel nicht speicherbar: {e}")))?;
        }
        Ok(accepted)
    }
}

impl From<russh::keys::Error> for SshError {
    fn from(e: russh::keys::Error) -> Self {
        SshError::BadKey(e.to_string())
    }
}

/// Angemeldete SSH-Verbindung.
pub struct SshConnection {
    handle: Handle<Handler>,
}

fn pw_str(s: &Secret) -> Result<String, SshError> {
    // RFC 4252: Kennwörter werden als UTF-8 übertragen; byte-genau, ohne Normalisierung.
    s.as_str().map(str::to_string).ok_or(SshError::PasswordEncoding)
}

/// Verfahren, die wir anbieten: die Standardliste, dazu `ecdh-sha2-nistp256/384/521` (kein Mangel,
/// aber manche Geräte können nur diese). Mit `legacy` zusätzlich ältere Verfahren, ganz hinten.
fn preferred(legacy: bool) -> russh::Preferred {
    use russh::{cipher, kex, mac};
    fn add<T: Clone + PartialEq>(list: &mut Vec<T>, extra: impl IntoIterator<Item = T>) {
        for e in extra {
            if !list.contains(&e) {
                list.push(e);
            }
        }
    }
    let mut p = russh::Preferred::DEFAULT;
    let mut k = p.kex.to_vec();
    add(&mut k, [kex::ECDH_SHA2_NISTP256, kex::ECDH_SHA2_NISTP384, kex::ECDH_SHA2_NISTP521]);
    if legacy {
        add(&mut k, [kex::DH_GEX_SHA1, kex::DH_G14_SHA1, kex::DH_G1_SHA1]);
        let mut c = p.cipher.to_vec();
        add(&mut c, [cipher::AES_128_CBC, cipher::AES_192_CBC, cipher::AES_256_CBC]);
        p.cipher = c.into();
        let mut m = p.mac.to_vec();
        add(&mut m, [mac::HMAC_SHA1, mac::HMAC_SHA1_ETM]);
        p.mac = m.into();
        let mut keys = p.key.to_vec();
        add(&mut keys, [russh::keys::Algorithm::Dsa]);
        p.key = keys.into();
    }
    p.kex = k.into();
    p
}

impl SshConnection {
    /// Baut die Verbindung über TCP auf und meldet an.
    pub async fn connect(params: &SshParams, known: KnownHosts, ask: HostKeyAsk) -> Result<Self, SshError> {
        let addr = (params.host.as_str(), params.port);
        let stream = tokio::time::timeout(params.connect_timeout, tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| SshError::Timeout)?
            .map_err(|e| SshError::Connect(format!("{}:{}", params.host, params.port), e.to_string()))?;
        stream.set_nodelay(true).ok();
        Self::connect_stream(params, stream, known, ask).await
    }

    /// Baut die Verbindung über einen vorhandenen Datenstrom auf (z. B. Tunnel).
    pub async fn connect_stream<S>(params: &SshParams, stream: S, known: KnownHosts, ask: HostKeyAsk) -> Result<Self, SshError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let config = Arc::new(client::Config {
            keepalive_interval: Some(params.keepalive),
            keepalive_max: 3,
            inactivity_timeout: None,
            nodelay: true,
            preferred: preferred(params.legacy),
            ..Default::default()
        });
        let asking = Arc::new(std::sync::Mutex::new(AskClock::default()));
        let handler = Handler { host: params.host.clone(), port: params.port, known, ask, server_key: None, asking: asking.clone() };
        // Zeitlimit ohne die Zeit, in der eine Rückfrage beim Benutzer liegt;
        // die Frist für die Rückfrage setzt der Hauptprozess.
        let start = tokio::time::Instant::now();
        let connecting = client::connect_stream(config, stream, handler);
        tokio::pin!(connecting);
        let result = loop {
            let deadline = {
                let c = asking.lock().unwrap_or_else(|e| e.into_inner());
                let d = start + params.connect_timeout + c.spent;
                // Während einer Rückfrage nur selten nachsehen.
                if c.since.is_some() { d.max(tokio::time::Instant::now() + Duration::from_secs(1)) } else { d }
            };
            tokio::select! {
                r = &mut connecting => break r,
                _ = tokio::time::sleep_until(deadline) => {
                    let c = asking.lock().unwrap_or_else(|e| e.into_inner());
                    if c.since.is_none() && tokio::time::Instant::now() >= start + params.connect_timeout + c.spent {
                        return Err(SshError::Timeout);
                    }
                }
            }
        };
        let mut handle = result
            .map_err(|e| match e {
                SshError::Protocol(russh::Error::UnknownKey) => SshError::HostKeyRejected,
                other => other,
            })?;

        let mut tried = Vec::new();
        for auth in &params.auth {
            tried.push(auth.name());
            let ok = match auth {
                Auth::Password(pw) => handle.authenticate_password(&params.user, pw_str(pw)?).await?.success(),
                Auth::Key { pem, passphrase } => {
                    let text = pem.as_str().ok_or_else(|| SshError::BadKey("kein Text".into()))?;
                    let pass = passphrase.as_ref().map(pw_str).transpose()?;
                    let key: PrivateKey = russh::keys::decode_secret_key(text, pass.as_deref())?;
                    let hash = handle.best_supported_rsa_hash().await?.flatten();
                    handle
                        .authenticate_publickey(&params.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                        .await?
                        .success()
                }
                Auth::KeyboardInteractive(pw) => {
                    let mut resp = handle.authenticate_keyboard_interactive_start(&params.user, None).await?;
                    let mut rounds = 0;
                    loop {
                        match resp {
                            KeyboardInteractiveAuthResponse::Success => break true,
                            KeyboardInteractiveAuthResponse::Failure { .. } => break false,
                            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                                rounds += 1;
                                if rounds > 5 {
                                    break false;
                                }
                                let answers = prompts
                                    .iter()
                                    .map(|p| if p.echo { Ok(params.user.clone()) } else { pw_str(pw) })
                                    .collect::<Result<Vec<_>, _>>()?;
                                resp = handle.authenticate_keyboard_interactive_respond(answers).await?;
                            }
                        }
                    }
                }
            };
            if ok {
                return Ok(SshConnection { handle });
            }
        }
        Err(SshError::AuthFailed(tried.join(", ")))
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Öffnet eine interaktive Shell mit Terminal.
    pub async fn open_shell(&self, term: &str, cols: u32, rows: u32) -> Result<Shell, SshError> {
        let ch = self.handle.channel_open_session().await?;
        ch.request_pty(true, term, cols, rows, 0, 0, &[]).await?;
        ch.request_shell(true).await?;
        let (mut read, write) = ch.split();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(msg) = read.wait().await {
                match msg {
                    ChannelMsg::Data { data } => {
                        if tx.send(ShellEvent::Data(data.to_vec())).is_err() {
                            return;
                        }
                    }
                    ChannelMsg::ExtendedData { data, .. } => {
                        let _ = tx.send(ShellEvent::Data(data.to_vec()));
                    }
                    ChannelMsg::ExitStatus { exit_status } => {
                        let _ = tx.send(ShellEvent::Exit(exit_status));
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
            let _ = tx.send(ShellEvent::Closed);
        });
        Ok(Shell { write, events: rx })
    }

    /// Führt einen Befehl ohne Terminal aus und sammelt Ausgabe und Rückgabewert.
    pub async fn exec(&self, command: &str) -> Result<(Vec<u8>, Vec<u8>, Option<u32>), SshError> {
        let mut ch = self.handle.channel_open_session().await?;
        ch.exec(true, command).await?;
        let (mut out, mut err, mut code) = (Vec::new(), Vec::new(), None);
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::Data { data } => out.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => err.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok((out, err, code))
    }

    /// Kanal zu einem Ziel hinter dem Server (für Tunnel).
    pub async fn direct_tcpip(&self, host: &str, port: u16) -> Result<russh::ChannelStream<client::Msg>, SshError> {
        let ch = self.handle.channel_open_direct_tcpip(host, port as u32, "127.0.0.1", 0).await?;
        Ok(ch.into_stream())
    }

    pub async fn disconnect(&self) {
        let _ = self.handle.disconnect(Disconnect::ByApplication, "", "de").await;
    }
}

/// Ereignis einer Shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    Data(Vec<u8>),
    Exit(u32),
    Closed,
}

/// Interaktive Shell.
pub struct Shell {
    write: russh::ChannelWriteHalf<client::Msg>,
    pub events: mpsc::UnboundedReceiver<ShellEvent>,
}

impl Shell {
    pub async fn write(&self, data: &[u8]) -> Result<(), SshError> {
        self.write.data_bytes(data.to_vec()).await.map_err(|_| SshError::Closed)
    }

    pub async fn resize(&self, cols: u32, rows: u32) -> Result<(), SshError> {
        self.write.window_change(cols, rows, 0, 0).await.map_err(|_| SshError::Closed)
    }

    pub async fn close(&self) {
        let _ = self.write.eof().await;
        let _ = self.write.close().await;
    }
}

/// Blockierender Zugriff auf eine Shell für die Makro-Engine. Darf nicht
/// innerhalb der Laufzeit aufgerufen werden (z. B. aus `spawn_blocking`).
pub struct BlockingShell<'a> {
    pub rt: tokio::runtime::Handle,
    pub shell: &'a mut Shell,
}

impl rj_tasks::macros::MacroIo for BlockingShell<'_> {
    fn send(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.rt.block_on(self.shell.write(data)).map_err(|e| std::io::Error::other(e.to_string()))
    }

    fn recv(&mut self, timeout: Duration) -> std::io::Result<Option<Vec<u8>>> {
        match self.rt.block_on(async { tokio::time::timeout(timeout, self.shell.events.recv()).await }) {
            Err(_) => Ok(None),
            Ok(Some(ShellEvent::Data(d))) => Ok(Some(d)),
            Ok(Some(ShellEvent::Exit(_))) => Ok(Some(Vec::new())),
            Ok(Some(ShellEvent::Closed)) | Ok(None) => {
                Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "Verbindung beendet"))
            }
        }
    }
}

#[cfg(test)]
mod preferred_tests {
    use super::preferred;
    use russh::{cipher, kex};

    #[test]
    fn ecdh_is_always_offered_and_weak_methods_only_on_request() {
        let p = preferred(false);
        assert!(p.kex.contains(&kex::ECDH_SHA2_NISTP256) && p.kex.contains(&kex::ECDH_SHA2_NISTP521));
        assert!(!p.kex.contains(&kex::DH_G14_SHA1) && !p.cipher.contains(&cipher::AES_128_CBC));
        let l = preferred(true);
        assert!(l.kex.contains(&kex::DH_G14_SHA1) && l.cipher.contains(&cipher::AES_128_CBC));
        assert_eq!(l.kex.iter().position(|k| *k == kex::MLKEM768X25519_SHA256), p.kex.iter().position(|k| *k == kex::MLKEM768X25519_SHA256), "Reihenfolge der sicheren bleibt");
    }
}
