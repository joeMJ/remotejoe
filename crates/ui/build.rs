//! Baut die Qt-Anbindung (cxx-qt) und die C++-Oberfläche (Widgets).

use cxx_qt_build::CxxQtBuilder;

fn main() {
    let mut b = CxxQtBuilder::new().qt_module("Gui").qt_module("Widgets").qt_module("Svg").qt_module("Network").qt_module("PrintSupport");
    // Linux: Meldung von logind über das Aufwachen aus dem Standby (D-Bus).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        b = b.qt_module("DBus");
    }
    // Module (Cargo-Features): ihre C++-Dateien nur mit dem Feature, dazu ein Schalter je Modul.
    let standorte = std::env::var_os("CARGO_FEATURE_STANDORTE").is_some();
    let aruba = std::env::var_os("CARGO_FEATURE_ARUBA").is_some();
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    // SAFETY: Der Rückruf setzt nur Übersetzerschalter.
    b = unsafe {
        b.cc_builder(move |cc| {
            // MSVC liest Quelltexte sonst in der Systemcodepage: Umlaute und Sonderzeichen in Texten wären falsch.
            if msvc {
                cc.flag("/utf-8");
            }
            if standorte {
                cc.define("RJ_STANDORTE", None);
            }
            if aruba {
                cc.define("RJ_ARUBA", None);
            }
        })
    };
    let mut files: Vec<&str> = CORE.to_vec();
    if standorte {
        files.extend_from_slice(STANDORTE);
    }
    if aruba {
        files.extend_from_slice(ARUBA);
    }
    b.qrc("res/remotejoe.qrc").include_dir("cpp").file("src/bridge.rs").cpp_files(files).build();
    // Windows: Programmsymbol in remotejoe.exe. Explorer, Startmenü und Taskleiste nehmen es aus der exe; ohne
    // eingebettetes Symbol zeigte die Taskleiste nach einem Update ein leeres Symbol.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=res/windows/remotejoe.rc");
        println!("cargo:rerun-if-changed=../../packaging/windows/remotejoe.ico");
        embed_resource::compile("res/windows/remotejoe.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}

/// Oberfläche ohne Module.
const CORE: &[&str] = &[
    "cpp/mainwindow.h", "cpp/mainwindow.cpp", "cpp/editor.h", "cpp/editor.cpp", "cpp/views.cpp", "cpp/titlebar.h", "cpp/titlebar.cpp",
    "cpp/appearance.h", "cpp/appearance.cpp", "cpp/settingstab.h", "cpp/settingstab.cpp", "cpp/masstab.h", "cpp/masstab.cpp",
    "cpp/mergetab.h", "cpp/mergetab.cpp", "cpp/sharedtab.h", "cpp/sharedtab.cpp", "cpp/reporttab.h", "cpp/reporttab.cpp",
    "cpp/waysedit.h", "cpp/waysedit.cpp", "cpp/exporttab.h", "cpp/exporttab.cpp", "cpp/overviewtab.h", "cpp/overviewtab.cpp",
    "cpp/folderoverviewtab.h", "cpp/folderoverviewtab.cpp", "cpp/orphantab.h", "cpp/orphantab.cpp", "cpp/credmergetab.h",
    "cpp/credmergetab.cpp", "cpp/credchecktab.h", "cpp/credchecktab.cpp", "cpp/mcpserver.h", "cpp/mcpserver.cpp",
    "cpp/mcplogtab.h", "cpp/mcplogtab.cpp", "cpp/cards.h", "cpp/cards.cpp", "cpp/kbcapture.h", "cpp/kbcapture.cpp", "cpp/diag.h",
    "cpp/diag.cpp", "cpp/app.cpp",
];

/// Erstes Modul.
const STANDORTE: &[&str] = &[
    "cpp/modules/standorte/mainwindow_standorte.cpp", "cpp/modules/standorte/settingstab_standorte.cpp", "cpp/modules/standorte/editor_standorte.cpp", "cpp/modules/standorte/overview_standorte.cpp", "cpp/modules/standorte/sitetab.h", "cpp/modules/standorte/sitetab.cpp",
    "cpp/modules/standorte/topoview.h", "cpp/modules/standorte/topoview.cpp", "cpp/modules/standorte/portscard.h",
    "cpp/modules/standorte/portscard.cpp", "cpp/modules/standorte/portactions.h", "cpp/modules/standorte/portactions.cpp",
    "cpp/modules/standorte/vswitchtab.h", "cpp/modules/standorte/vswitchtab.cpp", "cpp/modules/standorte/configbackup.h",
    "cpp/modules/standorte/configbackup.cpp", "cpp/modules/standorte/portcfgdialog.h", "cpp/modules/standorte/portcfgdialog.cpp",
    "cpp/modules/standorte/profiledialog.h", "cpp/modules/standorte/profiledialog.cpp",
];

/// Zweites Modul.
const ARUBA: &[&str] = &[
    "cpp/modules/aruba/mainwindow_aruba.cpp", "cpp/modules/aruba/settingstab_aruba.cpp", "cpp/modules/aruba/overview_aruba.cpp", "cpp/modules/aruba/logmenu.cpp", "cpp/modules/aruba/arubaview.h", "cpp/modules/aruba/arubaview.cpp",
    "cpp/modules/aruba/arubaaptab.h", "cpp/modules/aruba/arubaaptab.cpp", "cpp/modules/aruba/aptoolstab.h",
    "cpp/modules/aruba/aptoolstab.cpp", "cpp/modules/aruba/arubalogtab.h", "cpp/modules/aruba/arubalogtab.cpp",
    "cpp/modules/aruba/arubaclientstab.h", "cpp/modules/aruba/arubaclientstab.cpp", "cpp/modules/aruba/arubabreakouttab.h",
    "cpp/modules/aruba/arubabreakouttab.cpp", "cpp/modules/aruba/arubasummarytab.h", "cpp/modules/aruba/arubasummarytab.cpp",
];
