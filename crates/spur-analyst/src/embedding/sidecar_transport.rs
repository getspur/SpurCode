use super::sidecar_service::EmbedService;
use anyhow::{bail, Context as _, Result};
use futures::{stream::FuturesUnordered, StreamExt as _};
use std::{
    fs, io,
    os::unix::{fs::PermissionsExt as _, net::UnixStream as StdUnixStream},
    path::{Path, PathBuf},
};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::{UnixListener, UnixStream},
};

const SOCKET_PERMISSIONS: u32 = 0o600;

pub(super) async fn serve(
    service: EmbedService,
    socket_path: PathBuf,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<()> {
    bind_socket_path(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind embed socket at {}", socket_path.display()))?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(SOCKET_PERMISSIONS))
        .with_context(|| {
            format!(
                "failed to set permissions on embed socket {}",
                socket_path.display()
            )
        })?;

    let mut tasks = FuturesUnordered::new();
    tokio::pin!(shutdown);
    let result = loop {
        let accepted = tokio::select! {
            _ = &mut shutdown => break Ok(()),
            accepted = listener.accept() => accepted,
            joined = tasks.next(), if !tasks.is_empty() => {
                if let Some(Err(error)) = joined { tracing::debug!(%error, "embed sidecar connection ended with error"); }
                continue;
            },
        };
        let (stream, _) = match accepted.with_context(|| {
            format!(
                "failed to accept embed socket connection at {}",
                socket_path.display()
            )
        }) {
            Ok(connection) => connection,
            Err(error) => break Err(error),
        };
        let service = service.clone();
        tasks.push(async move { handle_connection(&service, stream).await });
    };
    drop(tasks);
    let blocking_result = service.close_blocking().await;
    drop(listener);
    if let Err(error) = fs::remove_file(&socket_path) {
        if error.kind() != io::ErrorKind::NotFound {
            return Err(error.into());
        }
    }
    blocking_result?;
    result
}

async fn handle_connection(service: &EmbedService, stream: UnixStream) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines
        .next_line()
        .await
        .context("failed to read embed request line")?
    {
        if line.trim().is_empty() {
            continue;
        }

        let response = service.handle_line(&line).await;
        let response = serde_json::to_vec(&response).context("failed to encode response")?;
        writer
            .write_all(&response)
            .await
            .context("failed to write embed response")?;
        writer
            .write_all(b"\n")
            .await
            .context("failed to write embed response newline")?;
    }

    Ok(())
}

fn bind_socket_path(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create embed socket parent directory {}",
                parent.display()
            )
        })?;
    }

    if path.exists() {
        match StdUnixStream::connect(path) {
            Ok(_) => bail!("embed socket already in use at {}", path.display()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                ) =>
            {
                fs::remove_file(path).with_context(|| {
                    format!("failed to remove stale embed socket {}", path.display())
                })?;
            }
            Err(error) => {
                fs::remove_file(path).with_context(|| {
                    format!(
                        "failed to remove stale embed socket {} after connect failed: {error}",
                        path.display()
                    )
                })?;
            }
        }
    }

    Ok(())
}
