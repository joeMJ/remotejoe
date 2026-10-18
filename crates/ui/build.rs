//! Baut die Qt-Anbindung (cxx-qt) und die C++-Oberfläche (Widgets).

use cxx_qt_build::CxxQtBuilder;

fn main() {
    let mut b = CxxQtBuilder::new().qt_module("Gui").qt_module("Widgets").qt_module("Svg").qt_module("Network");
    // Linux: Meldung von logind über das Aufwachen aus dem Standby (D-Bus).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        b = b.qt_module("DBus");
    }
    // MSVC liest Quelltexte sonst in der Systemcodepage: Umlaute und Sonderzeichen in Texten wären falsch.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // SAFETY: Der Rückruf setzt nur einen Übersetzerschalter.
        b = unsafe {
            b.cc_builder(|cc| {
                cc.flag("/utf-8");
            })
        };
    }
    b
        .qrc("res/remotejoe.qrc")
        .include_dir("cpp")
        .file("src/bridge.rs")
        .cpp_files(["cpp/mainwindow.h", "cpp/mainwindow.cpp", "cpp/editor.h", "cpp/editor.cpp", "cpp/views.cpp", "cpp/titlebar.h", "cpp/titlebar.cpp", "cpp/appearance.h", "cpp/appearance.cpp", "cpp/exporttab.h", "cpp/exporttab.cpp", "cpp/app.cpp"])
        .build();
}
