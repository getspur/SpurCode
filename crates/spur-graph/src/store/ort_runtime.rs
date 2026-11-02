use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};

const RUNTIME: &[u8] = include_bytes!(env!("SPUR_BUNDLED_ORT_PATH"));
const SHA256: &str = env!("SPUR_BUNDLED_ORT_SHA256");
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
static INIT: OnceLock<Result<(), String>> = OnceLock::new();

fn valid(bytes: &[u8]) -> bool {
    format!("{:x}", Sha256::digest(bytes)) == SHA256
}

pub(crate) fn materialize_ort_runtime(cache: &Path) -> Result<PathBuf> {
    if !valid(RUNTIME) {
        bail!("bundled ONNX Runtime checksum mismatch");
    }
    let directory = cache.join("onnxruntime").join("1.22.0").join(SHA256);
    fs::create_dir_all(&directory)
        .with_context(|| format!("create runtime cache {}", directory.display()))?;
    let destination = directory.join("libonnxruntime.dylib");
    match fs::read(&destination) {
        Ok(bytes) if valid(&bytes) => return Ok(destination.canonicalize()?),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(e).with_context(|| format!("read runtime cache {}", destination.display()))
        }
    }
    let temporary = directory.join(format!(
        ".runtime-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(RUNTIME)?;
        file.sync_all()?;
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("materialize runtime in {}", directory.display()))?;
    if !valid(&fs::read(&destination)?) {
        bail!("materialized ONNX Runtime checksum mismatch");
    }
    Ok(destination.canonicalize()?)
}

pub(crate) fn ensure_ort_runtime() -> Result<()> {
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".spur").join("cache"))
        .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(|home| PathBuf::from(home).join("spur")))
        .context("no user cache location for ONNX Runtime")?;
    ensure_ort_runtime_at(&base)
}

pub(crate) fn ensure_ort_runtime_at(base: &Path) -> Result<()> {
    INIT.get_or_init(|| {
        let result = (|| -> Result<()> {
            let path = materialize_ort_runtime(base)?;
            let path = path
                .to_str()
                .context("ONNX Runtime cache path is not UTF-8")?;
            if !ort::init_from(path)
                .commit()
                .context("initialize ONNX Runtime environment")?
            {
                bail!("ONNX Runtime environment was initialized before the bundled runtime");
            }
            Ok(())
        })();
        result.map_err(|e| format!("{e:#}"))
    })
    .as_ref()
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}
