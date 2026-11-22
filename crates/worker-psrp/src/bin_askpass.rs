//! `rj-askpass`: Askpass-Hilfe für ssh im SSH-Transport von pwsh (siehe `rj_psrp::askpass`).
//! ssh ruft es mit der Eingabeaufforderung als Argument auf; ausgegeben wird nur auf eine Kennwortfrage.

fn main() {
    #[cfg(unix)]
    {
        use std::io::Write;
        let prompt = std::env::args().nth(1).unwrap_or_default();
        let Some(socket) = std::env::var_os(rj_psrp::askpass::SOCKET_ENV) else { std::process::exit(1) };
        match rj_psrp::askpass::fetch(&prompt, std::path::Path::new(&socket)) {
            Some(mut pw) => {
                let mut out = std::io::stdout().lock();
                let ok = out.write_all(&pw).and_then(|_| out.write_all(b"\n")).and_then(|_| out.flush()).is_ok();
                zeroize::Zeroize::zeroize(&mut pw);
                std::process::exit(if ok { 0 } else { 1 });
            }
            None => std::process::exit(1),
        }
    }
    #[cfg(not(unix))]
    std::process::exit(1);
}
