//! remotejoe – Verbindungsmanager.
//!
//! Aufruf: `remotejoe [--demo] [--screenshots <ordner>]`

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--demo") {
        std::env::set_var("RJ_DEMO", "1");
    }
    if let Some(i) = args.iter().position(|a| a == "--screenshots") {
        if let Some(dir) = args.get(i + 1) {
            std::env::set_var("RJ_SCREENSHOTS", dir);
        }
    }
    std::process::exit(rj_ui::bridge::qobject::run_application());
}
