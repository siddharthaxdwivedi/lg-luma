//! Links the cross-built Vosk archives into the webOS ARM binary.
//!
//! `tools/build-vosk.sh` cross-builds libvosk, Kaldi, OpenFst, OpenBLAS and
//! CLAPACK into `.build/vosk`. Kaldi's static libraries reference each other
//! cyclically, and OpenBLAS and CLAPACK ship overlapping members, so the
//! archives stay separate and are linked inside one `--start-group` instead of
//! being merged. Every other target uses the placeholder engine and links
//! nothing.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // Mirrors the `target_arch = "arm"` gate on the engine module in stt.rs.
    let arm = std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("arm");
    if !arm || std::env::var("CARGO_FEATURE_VOICE_ASSISTANT").is_err() {
        return;
    }

    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let vosk = root.join(".build/vosk");
    // Re-link when a restored cache or a rebuilt archive changes.
    println!("cargo:rerun-if-changed={}", vosk.display());

    let mut archives = Vec::new();
    collect_archives(&vosk.join("lib"), &mut archives);
    collect_archives(&vosk.join("src/kaldi/src"), &mut archives);
    archives.sort();

    assert!(
        archives.iter().any(|path| path.ends_with("libvosk.a")),
        "Vosk is not cross-built: run `sh tools/build-vosk.sh` before building for ARM."
    );

    // The recognizer is C++ and the Kaldi archives are cyclic, so they all go
    // into one group, with the C++ runtime resolved after it.
    println!("cargo:rustc-link-arg=-Wl,--start-group");
    for archive in &archives {
        println!("cargo:rustc-link-arg={}", archive.display());
    }
    println!("cargo:rustc-link-arg=-Wl,--end-group");
    println!("cargo:rustc-link-arg=-lstdc++");
}

/// Collect every static library under `directory`, recursively.
fn collect_archives(directory: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_archives(&path, found);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("a") {
            found.push(path);
        }
    }
}
