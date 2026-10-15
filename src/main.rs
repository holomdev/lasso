//! lasso — screen capture tool for Windows.
//!
//! Phase 1: the `capture` module (DXGI Output Duplication with a GDI BitBlt
//! fallback, multi-monitor virtual desktop, per-monitor DPI).
//! Phase 2: the interactive region-selection overlay.
//!
//! Usage:
//!   lasso monitors
//!   lasso capture [--out FILE.png] [--backend dxgi|gdi]
//!   lasso select  [--out FILE.png]

mod capture;
mod dpi;
mod overlay;

use anyhow::{bail, Context, Result};
use capture::Backend;
use std::path::Path;
use std::time::Instant;

/// Default destination for full-screen captures.
///
/// It lives under `target/`, which version control already ignores, so running
/// the tool never leaves files behind that could be committed by accident.
const DEFAULT_CAPTURE_OUT: &str = "target/test-output/capture.png";

/// Default destination for region selections.
const DEFAULT_SELECT_OUT: &str = "target/test-output/selection.png";

fn usage() {
    eprintln!(
        "lasso — screen capture\n\n\
         USAGE:\n  \
         lasso monitors\n  \
         lasso capture [--out FILE.png] [--backend dxgi|gdi]\n  \
         lasso select  [--out FILE.png]\n\n\
         OPTIONS:\n  \
         -o, --out FILE      output file (capture default: {capture}, select default: {select})\n  \
         -b, --backend NAME  force dxgi or gdi; without it, DXGI with a GDI fallback\n  \
         -h, --help          this help\n\n\
         SELECT KEYS:\n  \
         left drag           define the region\n  \
         Enter               save the region, or the whole screen when none is selected\n  \
         Esc / right click   cancel",
        capture = DEFAULT_CAPTURE_OUT,
        select = DEFAULT_SELECT_OUT
    );
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("capture");

    // Must happen before any window or DC is created.
    let awareness = dpi::enable_per_monitor();
    eprintln!("[dpi] awareness = {}", awareness.as_str());

    match command {
        "monitors" => cmd_monitors(),
        "capture" => {
            let mut out = String::from(DEFAULT_CAPTURE_OUT);
            let mut backend: Option<Backend> = None;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--out" | "-o" => {
                        i += 1;
                        out = args.get(i).context("--out needs a value")?.clone();
                    }
                    "--backend" | "-b" => {
                        i += 1;
                        let value = args.get(i).context("--backend needs a value")?;
                        backend = Some(match value.as_str() {
                            "dxgi" => Backend::Dxgi,
                            "gdi" => Backend::Gdi,
                            other => bail!("unknown backend: {other} (use dxgi or gdi)"),
                        });
                    }
                    other => bail!("unknown argument: {other}"),
                }
                i += 1;
            }
            cmd_capture(&out, backend)
        }
        "select" => {
            let mut out = String::from(DEFAULT_SELECT_OUT);
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--out" | "-o" => {
                        i += 1;
                        out = args.get(i).context("--out needs a value")?.clone();
                    }
                    other => bail!("unknown argument: {other}"),
                }
                i += 1;
            }
            cmd_select(&out)
        }
        "-h" | "--help" | "help" => {
            usage();
            Ok(())
        }
        other => {
            usage();
            bail!("unknown command: {other}")
        }
    }
}

/// Create the destination directory so the default `target/...` paths work from
/// a clean checkout.
fn ensure_parent(path: &str) -> Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
    }
    Ok(())
}

fn cmd_monitors() -> Result<()> {
    let (vx, vy, vw, vh) = capture::virtual_desktop();
    println!("virtual desktop: {vw}x{vh} @ ({vx}, {vy})");
    for m in capture::monitors()? {
        println!(
            "{:<24} {}x{} @ ({}, {}){}",
            m.device,
            m.width(),
            m.height(),
            m.rect.0,
            m.rect.1,
            if m.primary { "  [primary]" } else { "" }
        );
    }
    Ok(())
}

fn cmd_capture(out: &str, backend: Option<Backend>) -> Result<()> {
    let (vx, vy, vw, vh) = capture::virtual_desktop();
    eprintln!("[desktop] virtual = {vw}x{vh} @ ({vx},{vy})");
    for m in capture::monitors()? {
        eprintln!(
            "[monitor] {} {}x{} @ ({},{}){}",
            m.device,
            m.width(),
            m.height(),
            m.rect.0,
            m.rect.1,
            if m.primary { " [primary]" } else { "" }
        );
    }

    let started = Instant::now();
    let frame = capture::capture_virtual_desktop(backend)?;
    let elapsed = started.elapsed();

    ensure_parent(out)?;
    frame
        .to_rgba_image()
        .save(out)
        .with_context(|| format!("could not write {out}"))?;

    println!("backend : {}", frame.backend.as_str());
    println!("size    : {}x{}", frame.width, frame.height);
    println!("origin  : ({}, {})", frame.origin.0, frame.origin.1);
    println!("stride  : {} bytes", frame.stride());
    println!("pixels  : {} bytes", frame.bgra.len());
    println!("elapsed : {:.1} ms", elapsed.as_secs_f64() * 1000.0);
    println!("file    : {out}");
    Ok(())
}

fn cmd_select(out: &str) -> Result<()> {
    let started = Instant::now();
    let frame = capture::capture_virtual_desktop(None)?;
    eprintln!(
        "[capture] {} {}x{} in {:.1} ms",
        frame.backend.as_str(),
        frame.width,
        frame.height,
        started.elapsed().as_secs_f64() * 1000.0
    );

    let Some(selection) = overlay::select_region(&frame)? else {
        println!("cancelled");
        return Ok(());
    };

    let image = frame
        .to_rgba_sub(
            selection.x,
            selection.y,
            selection.width,
            selection.height,
        )
        .context("the selected region falls outside the capture")?;

    ensure_parent(out)?;
    image
        .save(out)
        .with_context(|| format!("could not write {out}"))?;

    println!(
        "region  : {}x{} at ({}, {})",
        selection.width, selection.height, selection.x, selection.y
    );
    println!("elapsed : {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
    println!("file    : {out}");
    Ok(())
}
