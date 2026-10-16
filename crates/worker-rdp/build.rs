//! Übersetzt die C-Hülle und erzeugt die Rust-Anbindung mit bindgen.

fn main() {
    println!("cargo:rerun-if-changed=csrc/rjrdp.c");
    println!("cargo:rerun-if-changed=csrc/rjrdp.h");
    println!("cargo:rustc-check-cfg=cfg(rj_no_freerdp)");
    let lib = match pkg_config::Config::new().atleast_version("3.0").probe("freerdp-client3") {
        Ok(l) => l,
        Err(e) => {
            println!("cargo:warning=FreeRDP 3 nicht gefunden ({e}); RDP-Worker ohne Protokoll gebaut");
            println!("cargo:rustc-cfg=rj_no_freerdp");
            return;
        }
    };
    let mut build = cc::Build::new();
    build.file("csrc/rjrdp.c").warnings(false);
    for p in &lib.include_paths {
        build.include(p);
    }
    build.compile("rjrdp");
    let bindings = bindgen::Builder::default()
        .header("csrc/rjrdp.h")
        .allowlist_function("rj_.*")
        .allowlist_type("rj_.*")
        .allowlist_var("RJ_.*")
        .generate()
        .expect("bindgen");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings.write_to_file(out.join("rjrdp.rs")).expect("Anbindung schreiben");
}
