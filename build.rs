//! Compiles assets/autospaces.rc with the Windows SDK `rc.exe` and links the
//! resulting .res into the binary (application icon ID 1, dialogs, QR bitmap, VERSIONINFO).
//! The version for VERSIONINFO comes from Cargo.toml via a generated `version.h`.
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("RC") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };
    let mut roots = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(pf) = std::env::var_os(var) {
            roots.push(Path::new(&pf).join("Windows Kits").join("10").join("bin"));
        }
    }
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for bin in roots {
        let Ok(rd) = std::fs::read_dir(&bin) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let ver: Vec<u32> = name.split('.').filter_map(|s| s.parse().ok()).collect();
            let rc = e.path().join(arch).join("rc.exe");
            if ver.len() == 4 && rc.is_file() && best.as_ref().is_none_or(|(v, _)| ver > *v) {
                best = Some((ver, rc));
            }
        }
    }
    if let Some((_, rc)) = best {
        return Some(rc);
    }
    // Last resort: whatever is on PATH (e.g. a Developer Command Prompt).
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("rc.exe"))
            .find(|c| c.is_file())
    })
}

/// `(0,1,2,0, "0.1.2.0")` style pair for FILEVERSION and the version strings (pre-release suffix ignored).
fn version_parts(v: &str) -> (String, String) {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let mut n: Vec<u32> = core.split('.').filter_map(|p| p.parse().ok()).collect();
    n.resize(4, 0);
    let s: Vec<String> = n.iter().map(u32::to_string).collect();
    (s.join(","), s.join("."))
}

fn main() {
    println!("cargo:rerun-if-changed=assets/autospaces.rc");
    println!("cargo:rerun-if-changed=assets/autospaces.ico");
    println!("cargo:rerun-if-changed=assets/autospaces.manifest");
    println!("cargo:rerun-if-changed=assets/donate-qr.bmp");
    println!("cargo:rerun-if-changed=assets/donate-revolut.bmp");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Some(rc) = find_rc() else {
        println!("cargo:warning=rc.exe not found (install the Windows SDK); building without an icon");
        return;
    };
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let (comma, dotted) = version_parts(&std::env::var("CARGO_PKG_VERSION").unwrap_or_default());
    std::fs::write(
        out_dir.join("version.h"),
        format!("#define AS_VER_COMMA {comma}
#define AS_VER_STR \"{dotted}\"
"),
    )
    .expect("cannot write version.h");
    let out = out_dir.join("autospaces.res");
    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/i")
        .arg(&out_dir)
        .arg("/fo")
        .arg(&out)
        .arg("assets/autospaces.rc")
        .status()
        .expect("failed to run rc.exe");
    assert!(status.success(), "rc.exe failed");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
