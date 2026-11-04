use hf_hub::{api::sync::ApiBuilder, Repo, RepoType};
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
const PROFILE: &str = "model=NomicEmbedTextV15;dimensions=768;max_length=8192;pooling=mean;normalization=unit;input=legacy_preformatted;loader=fastembed-5.8.0-user-defined-memory;quantization=none;output_key=none";
const CODE_RANK_PROFILE: &str = "model=CodeRankEmbed;repo=jamie8johnson/CodeRankEmbed-onnx;revision=151669b173750250e611e1be00c812f112ea6020;dimensions=768;max_length=8192;pooling=mean;normalization=unit;input=legacy_preformatted;loader=fastembed-5.8.0-user-defined-memory;quantization=none;output_key=none";
const JINA_PROFILE: &str = "model=JinaEmbeddingsV2BaseCode;repo=jinaai/jina-embeddings-v2-base-code;dimensions=768;max_length=8192;pooling=mean;normalization=unit;input=legacy_preformatted;loader=fastembed-5.8.0-user-defined-memory;quantization=none;output_key=none";

pub struct ResolvedArtifacts {
    pub files: Vec<(PathBuf, File)>,
    pub total_bytes: u64,
    pub profile: &'static str,
    pub identity: ModelIdentity,
}

impl ResolvedArtifacts {
    // Read from the already opened handles, then verify the very bytes handed to
    // FastEmbed. Neither path replacement nor an in-place edit can substitute
    // unverified content after the descriptor was computed.
    pub fn verified_bytes(mut self) -> io::Result<Vec<Vec<u8>>> {
        let mut model = Sha256::new();
        let mut tokenizer = Sha256::new();
        model.update(b"spur-embedding-model-v1\0");
        tokenizer.update(b"spur-embedding-tokenizer-v1\0");
        part(&mut model, "profile", self.profile.as_bytes())?;
        part(&mut tokenizer, "profile", self.profile.as_bytes())?;
        let mut remaining = self.total_bytes;
        let mut result = Vec::with_capacity(FILES.len());
        for (i, (_, file)) in self.files.iter_mut().enumerate() {
            let length = file.metadata()?.len();
            remaining = remaining
                .checked_sub(length)
                .ok_or_else(|| fail("artifact size changed"))?;
            let size = usize::try_from(length).map_err(|_| fail("artifact too large"))?;
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(size)
                .map_err(|_| fail("artifact allocation failed"))?;
            file.rewind()?;
            file.take(
                length
                    .checked_add(1)
                    .ok_or_else(|| fail("artifact size overflow"))?,
            )
            .read_to_end(&mut bytes)?;
            if bytes.len() != size {
                return Err(fail("artifact size changed"));
            }
            let digest = Sha256::digest(&bytes);
            part(
                if i == 0 { &mut model } else { &mut tokenizer },
                FILES[i],
                &digest,
            )?;
            result.push(bytes);
        }
        if remaining != 0
            || format!("{:x}", model.finalize()) != self.identity.model_fingerprint
            || format!("{:x}", tokenizer.finalize()) != self.identity.tokenizer_fingerprint
        {
            return Err(fail("artifact identity changed"));
        }
        Ok(result)
    }
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

pub fn resolve(model_id: ModelId, budget: u64) -> io::Result<ResolvedArtifacts> {
    let (repo_name, source_name, profile, pinned_revision) = match model_id {
        ModelId::NomicEmbedTextV15 => (REPO, "nomic-ai/nomic-embed-text-v1.5", PROFILE, None),
        ModelId::CodeRankEmbed => (
            "models--jamie8johnson--CodeRankEmbed-onnx",
            "jamie8johnson/CodeRankEmbed-onnx",
            CODE_RANK_PROFILE,
            Some("151669b173750250e611e1be00c812f112ea6020"),
        ),
        ModelId::JinaEmbeddingsV2BaseCode => (
            "models--jinaai--jina-embeddings-v2-base-code",
            "jinaai/jina-embeddings-v2-base-code",
            JINA_PROFILE,
            None,
        ),
    };
    let cache_dir = cache()?;
    let repo = cache_dir.join(repo_name);
    let revision = match pinned_revision {
        Some(revision) => Some(revision.to_owned()),
        None => match fs::read_to_string(repo.join("refs/main")) {
            Ok(revision) => Some(revision.trim().to_owned()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        },
    };
    if let Some(revision) = &revision {
        if revision.len() != 40 || !revision.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(fail("invalid model revision"));
        }
    }
    if revision.as_ref().is_none_or(|revision| {
        FILES
            .iter()
            .any(|name| !repo.join("snapshots").join(revision).join(name).is_file())
    }) {
        let source = ApiBuilder::from_env()
            .with_cache_dir(cache_dir)
            .with_progress(true)
            .with_retries(3)
            .build()
            .map_err(io::Error::other)?
            .repo(Repo::with_revision(
                source_name.to_owned(),
                RepoType::Model,
                revision.as_deref().unwrap_or("main").to_owned(),
            ));
        for name in FILES {
            source.get(name).map_err(io::Error::other)?;
        }
    }
    let revision = match revision {
        Some(revision) => revision,
        None => fs::read_to_string(repo.join("refs/main"))?
            .trim()
            .to_owned(),
    };
    if revision.len() != 40 || !revision.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(fail("invalid model revision"));
    }
    let revision = revision.as_str();
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
    part(&mut model, "profile", profile.as_bytes())?;
    part(&mut tokenizer, "profile", profile.as_bytes())?;
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
        profile,
        identity: ModelIdentity {
            model: model_id,
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
    fn verified_bytes_reject_in_place_mutation_and_bind_open_files() {
        use super::*;
        fn fixture() -> (PathBuf, ResolvedArtifacts) {
            let dir = std::env::temp_dir().join(format!(
                "spur-verified-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&dir).unwrap();
            let mut model = Sha256::new();
            let mut tokenizer = Sha256::new();
            model.update(b"spur-embedding-model-v1\0");
            tokenizer.update(b"spur-embedding-tokenizer-v1\0");
            part(&mut model, "profile", PROFILE.as_bytes()).unwrap();
            part(&mut tokenizer, "profile", PROFILE.as_bytes()).unwrap();
            let mut files = Vec::new();
            for (i, name) in FILES.iter().enumerate() {
                let bytes = [i as u8; 4];
                let path = dir.join(i.to_string());
                fs::write(&path, bytes).unwrap();
                part(
                    if i == 0 { &mut model } else { &mut tokenizer },
                    name,
                    &Sha256::digest(bytes),
                )
                .unwrap();
                files.push((path.clone(), File::open(path).unwrap()));
            }
            let identity = ModelIdentity {
                model: ModelId::NomicEmbedTextV15,
                dimensions: 768,
                model_fingerprint: format!("{:x}", model.finalize()),
                tokenizer_fingerprint: format!("{:x}", tokenizer.finalize()),
                pooling: "mean".into(),
                normalization: "unit".into(),
                prefix: "legacy_preformatted".into(),
            };
            (
                dir,
                ResolvedArtifacts {
                    files,
                    total_bytes: 20,
                    profile: PROFILE,
                    identity,
                },
            )
        }
        let (dir, artifacts) = fixture();
        fs::write(dir.join("0"), [9u8; 4]).unwrap();
        assert!(artifacts.verified_bytes().is_err());
        fs::remove_dir_all(dir).unwrap();
        let (dir, artifacts) = fixture();
        fs::write(dir.join("replacement"), [9u8; 4]).unwrap();
        fs::rename(dir.join("replacement"), dir.join("0")).unwrap();
        assert_eq!(artifacts.verified_bytes().unwrap()[0], [0u8; 4]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn artifact_length_addition_rejects_overflow() {
        assert_eq!(super::add_length(u64::MAX - 1, 1).unwrap(), u64::MAX);
        assert!(super::add_length(u64::MAX, 1).is_err());
    }
}
