use std::{
    path::Path,
    thread::{self, JoinHandle},
};
use thiserror::Error;

#[cfg(target_os = "macos")]
mod ort_runtime {
    use anyhow::{bail, Context, Result};
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        io::Write,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    const RUNTIME: &[u8] = include_bytes!(env!("SPUR_BUNDLED_ORT_PATH"));
    const SHA256: &str = env!("SPUR_BUNDLED_ORT_SHA256");
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

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
                return Err(e)
                    .with_context(|| format!("read runtime cache {}", destination.display()))
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

    #[doc(hidden)]
    pub fn ensure_ort_runtime_at(base: &Path) -> Result<()> {
        let path = materialize_ort_runtime(base)?;
        initialize_from_path(&path)
    }

    pub(super) fn initialize_from_path(path: &Path) -> Result<()> {
        let path = path
            .to_str()
            .context("ONNX Runtime cache path is not UTF-8")?;
        let builder = ort::init_from(path);
        // The wrapper resolves the shared library through a Rust-panic API.
        // This catches that initialization panic only; it cannot catch native aborts.
        std::panic::catch_unwind(|| {
            ort::api();
        })
        .map_err(|_| anyhow::anyhow!("ONNX Runtime library or API could not be loaded"))?;
        if !builder
            .commit()
            .context("initialize ONNX Runtime environment")?
        {
            bail!("ONNX Runtime environment was initialized before the bundled runtime");
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("native initialization failed: {0}")]
    Initialization(String),
    #[error("runtime is closing")]
    Closing,
    #[error("native work panicked")]
    WorkPanicked,
    #[error("native environment still has owners: {0}")]
    Busy(String),
}

pub struct RuntimeOwner {
    closing: bool,
    closed: bool,
    jobs: Vec<JoinHandle<()>>,
}
impl RuntimeOwner {
    #[cfg(all(feature = "test-support", target_os = "macos"))]
    pub fn start_with_runtime_path(path: &Path) -> Result<Self, RuntimeError> {
        ort_runtime::initialize_from_path(path)
            .map_err(|e| RuntimeError::Initialization(format!("{e:#}")))?;
        Ok(Self {
            closing: false,
            closed: false,
            jobs: Vec::new(),
        })
    }
    pub fn start(cache: &Path) -> Result<Self, RuntimeError> {
        #[cfg(target_os = "macos")]
        ort_runtime::ensure_ort_runtime_at(cache)
            .map_err(|e| RuntimeError::Initialization(format!("{e:#}")))?;
        #[cfg(not(target_os = "macos"))]
        {
            let _ = cache;
            if !ort::init()
                .commit()
                .map_err(|e| RuntimeError::Initialization(e.to_string()))?
            {
                return Err(RuntimeError::Initialization(
                    "environment already initialized".into(),
                ));
            }
        }
        Ok(Self {
            closing: false,
            closed: false,
            jobs: Vec::new(),
        })
    }
    #[cfg(feature = "test-support")]
    pub fn submit(&mut self, job: impl FnOnce() + Send + 'static) -> Result<(), RuntimeError> {
        if self.closing {
            return Err(RuntimeError::Closing);
        }
        self.jobs.push(thread::spawn(job));
        Ok(())
    }
    pub fn close(&mut self) -> Result<(), RuntimeError> {
        self.close_inner(|| {}, || {})
    }
    #[cfg(feature = "test-support")]
    pub fn close_with_hook(&mut self, hook: impl FnOnce()) -> Result<(), RuntimeError> {
        self.close_inner(hook, || {})
    }
    pub fn close_after_join(&mut self, cleanup: impl FnOnce()) -> Result<(), RuntimeError> {
        self.close_inner(|| {}, cleanup)
    }
    fn close_inner(
        &mut self,
        hook: impl FnOnce(),
        cleanup: impl FnOnce(),
    ) -> Result<(), RuntimeError> {
        if self.closed {
            return Ok(());
        }
        self.closing = true;
        #[cfg(feature = "test-support")]
        eprintln!("phase: admission_closed");
        hook();
        let mut panicked = false;
        for job in self.jobs.drain(..) {
            panicked |= job.join().is_err();
        }
        #[cfg(feature = "test-support")]
        eprintln!("phase: jobs_joined");
        cleanup();
        if ort::environment::try_shutdown().is_err() {
            #[cfg(feature = "test-support")]
            eprintln!("phase: environment_busy");
            return Err(RuntimeError::Busy("live native owners".into()));
        }
        #[cfg(feature = "test-support")]
        eprintln!("phase: environment_released");
        self.closed = true;
        if panicked {
            return Err(RuntimeError::WorkPanicked);
        }
        Ok(())
    }
}
