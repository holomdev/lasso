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

    // A stamp for the binary, printed when it starts.
    //
    // More than once now a report has been impossible to act on because it was not known
    // which build was running: the fix was in the source, the binary was older, and
    // everything either side of that was guesswork. A build that says when it was compiled
    // ends the doubt in one line. `rerun-if-env-changed` is deliberately absent: the stamp
    // is taken at each build, and a build script that does not re-run would leave an old
    // one on a new binary.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=LASSO_BUILD_UNIX={stamp}");
    // And a short git description, when git is there to ask.
    let git = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=LASSO_BUILD_GIT={git}");
    // Dirty or not, because a binary built from uncommitted changes is not the commit.
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    println!("cargo:rustc-env=LASSO_BUILD_DIRTY={}", if dirty { "dirty" } else { "clean" });

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
    //
    // The version block goes in the same script, because it is the same compiler and the
    // same pass. Without it the file has no product name, no version and no description in
    // its Properties — it shows as a bare executable of unknown origin, which is what makes
    // an unsigned download look like something that arrived uninvited.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let description = std::env::var("CARGO_PKG_DESCRIPTION")
        .unwrap_or_else(|_| "Screenshot capture for Windows".into());
    let name = std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "lasso".into());
    let authors = std::env::var("CARGO_PKG_AUTHORS").unwrap_or_default();

    // `VS_VERSION_INFO` wants the four numbers separately, and takes whatever is missing as
    // zero: "0.1.0" becomes 0,1,0,0.
    let mut parts = version.split('.').map(|p| p.parse::<u16>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    let build = parts.next().unwrap_or(0);

    let escaped = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");

    let script_text = format!(
        "1 24 \"lasso.manifest\"\n\
         \n\
         1 VERSIONINFO\n\
         FILEVERSION {major},{minor},{patch},{build}\n\
         PRODUCTVERSION {major},{minor},{patch},{build}\n\
         FILEOS 0x40004L\n\
         FILETYPE 0x1L\n\
         BEGIN\n\
         \x20   BLOCK \"StringFileInfo\"\n\
         \x20   BEGIN\n\
         \x20       BLOCK \"040904B0\"\n\
         \x20       BEGIN\n\
         \x20           VALUE \"FileDescription\", \"{description}\"\n\
         \x20           VALUE \"FileVersion\", \"{version}\"\n\
         \x20           VALUE \"InternalName\", \"{name}\"\n\
         \x20           VALUE \"OriginalFilename\", \"{name}.exe\"\n\
         \x20           VALUE \"ProductName\", \"lasso\"\n\
         \x20           VALUE \"ProductVersion\", \"{version}\"\n\
         \x20           VALUE \"CompanyName\", \"{authors}\"\n\
         \x20           VALUE \"LegalCopyright\", \"\"\n\
         \x20       END\n\
         \x20   END\n\
         \x20   BLOCK \"VarFileInfo\"\n\
         \x20   BEGIN\n\
         \x20       VALUE \"Translation\", 0x409, 1200\n\
         \x20   END\n\
         END\n",
        description = escaped(&description),
        name = escaped(&name),
        version = escaped(&version),
        authors = escaped(&authors),
    );
    if std::fs::write(&script, script_text).is_err() {
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
