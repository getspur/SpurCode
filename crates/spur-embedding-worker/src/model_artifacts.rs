use sha2::{Digest, Sha256};
use spur_embedding::{ModelId, ModelIdentity};
use std::{
    collections::HashSet,
    env,
    fs::{self, File},
    io::{self, Read, Seek},
    path::PathBuf,
};

const REPO: &str = "models--nomic-ai--nomic-embed-text-v1.5";
const FILES: [&str; 5] = [
    "onnx/model.onnx",
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
];

// FastEmbed 5.8.0 selects Mean for Nomic and normalizes its output. Graph
// supplies preformatted inputs and sets the tokenizer maximum to 8192.
const PROFILE: &str = "model=NomicEmbedTextV15;dimensions=768;max_length=8192;pooling=mean;normalization=unit;input=legacy_preformatted";

pub struct ResolvedArtifacts {
    pub files: Vec<(PathBuf, File)>,
    pub total_bytes: u64,
    pub profile: &'static str,
    pub identity: ModelIdentity,
}

fn fail(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn add_length(total: u64, length: u64) -> io::Result<u64> {
    total
        .checked_add(length)
        .ok_or_else(|| fail("artifact size overflow"))
}

fn cache() -> io::Result<PathBuf> {
    if let Some(value) = env::var_os("SPUR_MODEL_CACHE") {
        return Ok(value.into());
    }
    if let Some(home) = env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".spur/cache/fastembed"));
    }
    env::var_os("XDG_CACHE_HOME")
        .map(|x| PathBuf::from(x).join("spur/fastembed"))
        .ok_or_else(|| fail("model cache unavailable"))
}

// Encoding v1: domain bytes followed by ordered parts, each as u64 big-endian
// name length, name bytes, u64 big-endian value length, value bytes. Artifact
// values are their SHA-256 bytes; profile is UTF-8. Names bind every role.
fn part(hash: &mut Sha256, name: &str, value: &[u8]) -> io::Result<()> {
    hash.update(
        u64::try_from(name.len())
            .map_err(|_| fail("name too long"))?
            .to_be_bytes(),
    );
    hash.update(name.as_bytes());
    hash.update(
        u64::try_from(value.len())
            .map_err(|_| fail("value too long"))?
            .to_be_bytes(),
    );
    hash.update(value);
    Ok(())
}

pub fn resolve(budget: u64) -> io::Result<ResolvedArtifacts> {
    let repo = cache()?.join(REPO);
    let revision = fs::read_to_string(repo.join("refs/main"))?;
    let revision = revision.trim();
    if revision.len() != 40 || !revision.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(fail("invalid model revision"));
    }
    let snapshot = repo.join("snapshots").join(revision).canonicalize()?;
    let snapshots = repo.join("snapshots").canonicalize()?;
    if snapshot.parent() != Some(snapshots.as_path()) {
        return Err(fail("invalid snapshot path"));
    }
    let blobs = repo.join("blobs").canonicalize().ok();
    let mut seen = HashSet::new();
    #[cfg(unix)]
    let mut seen_inodes = HashSet::new();
    let mut files = Vec::new();
    let mut sizes = Vec::new();
    let mut total = 0u64;
    for name in FILES {
        let path = snapshot.join(name).canonicalize()?;
        if !path.starts_with(&snapshot)
            && !blobs.as_ref().is_some_and(|root| path.starts_with(root))
        {
            return Err(fail("artifact outside cache"));
        }
        if !seen.insert(path.clone()) {
            return Err(fail("duplicate artifact"));
        }
        let file = File::open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(fail("artifact is not a file"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if !seen_inodes.insert((metadata.dev(), metadata.ino())) {
                return Err(fail("duplicate artifact"));
            }
        }
        total = add_length(total, metadata.len())?;
        files.push((path, file));
        sizes.push(metadata.len());
    }
    if total > budget {
        return Err(fail("artifact budget exceeded"));
    }
    let mut model = Sha256::new();
    let mut tokenizer = Sha256::new();
    model.update(b"spur-embedding-model-v1\0");
    tokenizer.update(b"spur-embedding-tokenizer-v1\0");
    part(&mut model, "profile", PROFILE.as_bytes())?;
    part(&mut tokenizer, "profile", PROFILE.as_bytes())?;
    for (i, (_, file)) in files.iter_mut().enumerate() {
        let expected = sizes[i];
        if file.metadata()?.len() != expected {
            return Err(fail("artifact changed while reading"));
        }
        let mut digest = Sha256::new();
        let mut count = 0u64;
        let mut buffer = [0u8; 32768];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            count = count
                .checked_add(u64::try_from(n).map_err(|_| fail("read size overflow"))?)
                .ok_or_else(|| fail("read size overflow"))?;
            if count > expected {
                return Err(fail("artifact changed while reading"));
            }
            digest.update(&buffer[..n]);
        }
        if count != expected {
            return Err(fail("artifact changed while reading"));
        }
        let digest = digest.finalize();
        part(
            if i == 0 { &mut model } else { &mut tokenizer },
            FILES[i],
            &digest,
        )?;
        file.rewind()?;
    }
    Ok(ResolvedArtifacts {
        files,
        total_bytes: total,
        profile: PROFILE,
        identity: ModelIdentity {
            model: ModelId::NomicEmbedTextV15,
            dimensions: 768,
            model_fingerprint: format!("{:x}", model.finalize()),
            tokenizer_fingerprint: format!("{:x}", tokenizer.finalize()),
            pooling: "mean".into(),
            normalization: "unit".into(),
            prefix: "legacy_preformatted".into(),
        },
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn artifact_length_addition_rejects_overflow() {
        assert_eq!(super::add_length(u64::MAX - 1, 1).unwrap(), u64::MAX);
        assert!(super::add_length(u64::MAX, 1).is_err());
    }
}
