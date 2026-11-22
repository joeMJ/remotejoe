//! Baut die Qt-Anbindung (cxx-qt) und die C++-Oberfläche (Widgets).

use cxx_qt_build::CxxQtBuilder;

fn main() {
    let mut b = CxxQtBuilder::new().qt_module("Gui").qt_module("Widgets").qt_module("Svg").qt_module("Network").qt_module("PrintSupport");
    // Linux: Meldung von logind über das Aufwachen aus dem Standby (D-Bus).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        b = b.qt_module("DBus");
    }
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    // SAFETY: Der Rückruf setzt nur Übersetzerschalter.
    b = unsafe {
        b.cc_builder(move |cc| {
            // MSVC liest Quelltexte sonst in der Systemcodepage: Umlaute und Sonderzeichen in Texten wären falsch.
            if msvc {
                cc.flag("/utf-8");
            }
        })
    };
    // Oberflächen-Schnittstelle der Erweiterungen (rjhost.h, rjpluginui.h) aus der Schnittstellenkiste.
    println!("cargo:rerun-if-changed=../plugin-api/cpp");
    b.qrc("res/remotejoe.qrc").include_dir("cpp").include_dir("../plugin-api/cpp").file("src/bridge.rs").cpp_files(CORE.to_vec()).build();
    // Windows: Programmsymbol in remotejoe.exe. Explorer, Startmenü und Taskleiste nehmen es aus der exe; ohne
    // eingebettetes Symbol zeigte die Taskleiste nach einem Update ein leeres Symbol.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=res/windows/remotejoe.rc");
        println!("cargo:rerun-if-changed=../../packaging/windows/remotejoe.ico");
        embed_resource::compile("res/windows/remotejoe.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}

/// Oberfläche des Kerns (die Erweiterungen bringen ihre Oberfläche in ihrer Bibliothek mit).
const CORE: &[&str] = &[
    "cpp/mainwindow.h", "cpp/mainwindow.cpp", "cpp/editor.h", "cpp/editor.cpp", "cpp/views.cpp", "cpp/titlebar.h", "cpp/titlebar.cpp",
    "cpp/appearance.h", "cpp/appearance.cpp", "cpp/settingstab.h", "cpp/settingstab.cpp", "cpp/masstab.h", "cpp/masstab.cpp",
    "cpp/mergetab.h", "cpp/mergetab.cpp", "cpp/sharedtab.h", "cpp/sharedtab.cpp", "cpp/reporttab.h", "cpp/reporttab.cpp",
    "cpp/waysedit.h", "cpp/waysedit.cpp", "cpp/exporttab.h", "cpp/exporttab.cpp", "cpp/overviewtab.h", "cpp/overviewtab.cpp",
    "cpp/folderoverviewtab.h", "cpp/folderoverviewtab.cpp", "cpp/orphantab.h", "cpp/orphantab.cpp", "cpp/credmergetab.h",
    "cpp/credmergetab.cpp", "cpp/credchecktab.h", "cpp/credchecktab.cpp", "cpp/mcpserver.h", "cpp/mcpserver.cpp",
    "cpp/mcplogtab.h", "cpp/mcplogtab.cpp", "cpp/cards.h", "cpp/cards.cpp", "cpp/faicons.h", "cpp/faicons.cpp", "cpp/kbcapture.h", "cpp/kbcapture.cpp", "cpp/diag.h",
    "cpp/diag.cpp", "cpp/app.cpp", "cpp/licensepage.cpp", "cpp/plugin/rjplugins.cpp",
];
