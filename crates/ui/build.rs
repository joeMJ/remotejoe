//! Baut die Qt-Anbindung (cxx-qt) und die C++-Oberfläche (Widgets).

use cxx_qt_build::CxxQtBuilder;

fn main() {
    CxxQtBuilder::new()
        .qt_module("Widgets")
        .qt_module("Svg")
        .include_dir("cpp")
        .file("src/bridge.rs")
        .cpp_files(["cpp/mainwindow.h", "cpp/mainwindow.cpp", "cpp/views.cpp", "cpp/app.cpp"])
        .build();
}
