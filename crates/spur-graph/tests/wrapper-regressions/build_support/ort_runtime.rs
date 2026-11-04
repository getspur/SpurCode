use sha2::{Digest, Sha256};
use std::{env, fs, path::Path, process::Command};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn bundle() -> Result<(), Box<dyn std::error::Error>> {
    let arch = env::var("CARGO_CFG_TARGET_ARCH")?;
    let (name, archive_hash, dylib_hash) = match arch.as_str() {
        "aarch64" => (
            "arm64",
            "cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07",
            "2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a",
        ),
        "x86_64" => (
            "x86_64",
            "e4ec94a7696de74fb1b12846569aa94e499958af6ffa186022cfde16c9d617f0",
            "283e595e61cf65df7a6b1d59a1616cbd35c8b6399dd90d799d99b71a3ff83160",
        ),
        _ => return Err(format!("unsupported macOS ORT architecture: {arch}").into()),
    };
    let filename = format!("onnxruntime-osx-{name}-1.22.0.tgz");
    let out = std::path::PathBuf::from(env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    let downloaded = out.join(&filename);
    let archive = if let Some(path) = env::var_os("SPUR_ORT_ARCHIVE") {
        std::path::PathBuf::from(path)
    } else {
        if !downloaded.exists() {
            let url = format!(
                "https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/{filename}"
            );
            let temporary = out.join(format!("{filename}.download"));
            let status = Command::new("curl")
                .args([
                    "--fail",
                    "--location",
                    "--silent",
                    "--show-error",
                    "--output",
                ])
                .arg(&temporary)
                .arg(url)
                .status()?;
            if !status.success() {
                return Err("ORT archive download failed".into());
            }
            if digest(&fs::read(&temporary)?) != archive_hash {
                return Err("downloaded ORT archive SHA-256 mismatch".into());
            }
            fs::rename(&temporary, &downloaded)?;
        }
        downloaded
    };
    if digest(&fs::read(&archive)?) != archive_hash {
        return Err(format!("ORT archive SHA-256 mismatch: {}", archive.display()).into());
    }
    let member = format!("onnxruntime-osx-{name}-1.22.0/lib/libonnxruntime.1.22.0.dylib");
    let extracted = Command::new("tar")
        .arg("-xOf")
        .arg(&archive)
        .arg(&member)
        .output()?;
    if !extracted.status.success() {
        return Err(format!(
            "extract ORT dylib: {}",
            String::from_utf8_lossy(&extracted.stderr)
        )
        .into());
    }
    if digest(&extracted.stdout) != dylib_hash {
        return Err("ORT dylib SHA-256 mismatch".into());
    }
    let library = out.join("libonnxruntime.1.22.0.dylib");
    fs::write(&library, &extracted.stdout)?;
    println!(
        "cargo:rustc-env=SPUR_BUNDLED_ORT_PATH={}",
        Path::new(&library).display()
    );
    println!("cargo:rustc-env=SPUR_BUNDLED_ORT_SHA256={dylib_hash}");
    Ok(())
}
