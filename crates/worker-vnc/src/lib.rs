//! VNC-Worker: eigene Umsetzung des RFB-Protokolls (RFC 6143) mit Raw,
//! CopyRect, Hextile, ZRLE, erweiterter Zwischenablage (UTF-8),
//! VNC-Kennwort und VeNCrypt (X.509).

pub mod client;
pub mod codec;
