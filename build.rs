//! Compiles assets/autospaces.rc with the Windows SDK `rc.exe` and links the
//! resulting .res into the binary (embeds the application icon, ID 1).
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

fn main() {
    println!("cargo:rerun-if-changed=assets/autospaces.rc");
    println!("cargo:rerun-if-changed=assets/autospaces.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Some(rc) = find_rc() else {
        println!("cargo:warning=rc.exe not found (install the Windows SDK); building without an icon");
        return;
    };
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("autospaces.res");
    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&out)
        .arg("assets/autospaces.rc")
        .status()
        .expect("failed to run rc.exe");
    assert!(status.success(), "rc.exe failed");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
