//! Build script: give the executable a manifest.
//!
//! Windows reads a program's manifest to decide which common controls it wants and
//! whether it wants to be scaled by the system. Both are decided before `main`
//! runs, so neither can be fixed from inside the program: the manifest has to be
//! attached to the executable.
//!
//! Without it the loader hands out the version 5 common controls, which are drawn
//! in the classic style from before Windows XP. That is not only a matter of
//! taste: the classic group box fills the gap it makes for its own caption in
//! `COLOR_WINDOW`, inside its own painting code, so the parent's answer to
//! `WM_CTLCOLORSTATIC` cannot change it and every group caption sits on a white
//! patch.
//!
//! The manifest becomes a resource, which needs the Windows SDK's resource
//! compiler. When there is none the build still succeeds and says so: a program
//! without a manifest runs, it just looks like Windows 2000 and lets the system
//! scale it.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=lasso.manifest");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let Ok(out) = std::env::var("OUT_DIR") else {
        return;
    };
    let out = PathBuf::from(out);

    let Some(rc) = find_resource_compiler() else {
        println!(
            "cargo:warning=no resource compiler found, so lasso.exe will carry no manifest: \
             the settings window will use the classic controls and the system will scale it"
        );
        return;
    };

    // `rc.exe` takes a resource script, not a manifest: the script is what says
    // which resource id the file belongs at. The manifest file itself is copied
    // next to it so that the script's relative reference resolves.
    let script = out.join("lasso.rc");
    let manifest = out.join("lasso.manifest");
    let object = out.join("lasso.res");
    if std::fs::copy("lasso.manifest", &manifest).is_err() {
        println!("cargo:warning=could not copy lasso.manifest");
        return;
    }
    // `1` is `CREATEPROCESS_MANIFEST_RESOURCE_ID`: the manifest for the program
    // itself, as opposed to one for a DLL or for an isolation-aware component.
    if std::fs::write(&script, "1 24 \"lasso.manifest\"\n").is_err() {
        println!("cargo:warning=could not write the resource script");
        return;
    }

    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&object)
        .arg(&script)
        .current_dir(&out)
        .status();

    match status {
        Ok(status) if status.success() => {
            println!("cargo:rustc-link-arg-bins={}", object.display());
        }
        other => println!("cargo:warning=the resource compiler failed ({other:?})"),
    }
}

/// The Windows SDK's resource compiler, matching the target's architecture.
///
/// It is not on `PATH` in an ordinary developer shell. It ships per architecture
/// and it does refuse to build for a different one, so which copy is picked
/// matters; the newest SDK version is tried first.
fn find_resource_compiler() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("RC") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }

    let architecture = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => return None,
    };

    let mut roots = vec![PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin")];
    if let Ok(program_files) = std::env::var("ProgramFiles(x86)") {
        roots.push(PathBuf::from(program_files).join(r"Windows Kits\10\bin"));
    }
    roots.push(PathBuf::from(r"C:\Program Files\Windows Kits\10\bin"));

    let mut candidates = Vec::new();
    for root in roots {
        let Ok(versions) = std::fs::read_dir(&root) else {
            continue;
        };
        for version in versions.flatten() {
            let candidate = version.path().join(architecture).join("rc.exe");
            if candidate.is_file() {
                candidates.push(candidate);
            }
        }
    }

    // The version directories are named by SDK build, and sort chronologically.
    candidates.sort();
    candidates.pop()
}
