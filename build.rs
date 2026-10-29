//! Bettet unter Windows das Programmsymbol und die Versionsangaben in die EXE
//! ein: Explorer, Startmenü, Taskleiste und Task-Manager zeigen dann „WPM“
//! mit Symbol. Das Symbol wird aus derselben Vorlage gezeichnet wie das im
//! Fenster und im Infobereich (src/icon/draw.rs).

use std::env;
use std::fs;
use std::path::PathBuf;

#[allow(dead_code)]
#[path = "src/icon/draw.rs"]
mod draw;

/// Größen für Explorer, Taskleiste und Startmenü bei 100 bis 200 % Skalierung.
const SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon/draw.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("wpm.ico");
    fs::write(&ico, draw::ico(&SIZES)).expect("wpm.ico schreiben");
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(ico.to_str().expect("Pfad in UTF-8"))
        .set("FileDescription", "WPM")
        .set("ProductName", "WPM")
        .set(
            "LegalCopyright",
            "Copyright (c) 2026 Stanischewski, MIT License",
        );
    resource
        .compile()
        .expect("Symbol und Versionsangaben einbetten (rc.exe bzw. windres nötig)");
}
