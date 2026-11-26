//! Diagnoseprotokoll: Schalter in den Einstellungen (bleibt gespeichert) bzw. `RJ_DIAG=1`. Eingeschaltet schreibt die
//! Oberfläche laufend nach `<Temp>/remotejoe-diag/laufend.log`, zusätzlich alle Aufrufe von Erweiterungen und
//! Operationen an den Kern, die länger als 50 ms dauern (Name, Dauer, Größe der Antwort).

use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Ordner des Diagnoseprotokolls (wie in diag.cpp: Temp-Ordner des Systems).
pub fn dir() -> std::path::PathBuf {
    std::env::temp_dir().join("remotejoe-diag")
}

fn env_on() -> bool {
    std::env::var_os("RJ_DIAG").is_some()
}

fn sink(line: &str) {
    crate::bridge::qobject::diag_log_line(&cxx_qt_lib::QString::from(line));
}

/// Schalter anwenden (beim Start und nach jeder Änderung). `RJ_DIAG` schaltet zusätzlich ein.
pub fn apply(on: bool) {
    let on = on || env_on();
    let was = ACTIVE.swap(on, Ordering::Relaxed);
    if on == was {
        return;
    }
    if on {
        crate::bridge::qobject::diag_set_live(true);
        rj_plugin_api::ffi::host::diag::set(Some(sink));
        sink(&format!("[diag] Zeitprotokoll an (Schwelle {} ms), remotejoe {}", rj_plugin_api::ffi::host::diag::LIMIT_MS, rj_core::schema::PROGRAM_VERSION));
    } else {
        rj_plugin_api::ffi::host::diag::set(None);
        crate::bridge::qobject::diag_set_live(false);
    }
}
