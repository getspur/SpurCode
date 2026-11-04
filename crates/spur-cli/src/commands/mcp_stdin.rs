//! Cancellable Unix stdin for the owned command servers.
//!
//! Tokio's standard stdin uses a blocking thread which cannot be cancelled when
//! a client keeps the pipe open. Use readiness-driven reads for pipes, sockets
//! and terminals so command shutdown can join without waiting for client EOF.
use std::fs::File;
use std::io::{self, IsTerminal, Read};
use std::os::fd::{AsFd, AsRawFd};
use std::pin::Pin;
use std::task::{Context, Poll};

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use tokio::io::{unix::AsyncFd, AsyncRead, ReadBuf};

type Input = Box<dyn AsyncRead + Send + Unpin>;

pub(super) fn command_stdin() -> io::Result<Input> {
    let stdin = std::io::stdin();
    let file = File::from(stdin.as_fd().try_clone_to_owned()?);
    let kind = file.metadata()?.file_type();
    // Regular files and non-terminal character devices such as /dev/null have
    // immediate reads; some reactors reject them as non-pollable.
    use std::os::unix::fs::FileTypeExt;
    if kind.is_file() || (kind.is_char_device() && !stdin.is_terminal()) {
        return Ok(Box::new(tokio::io::stdin()));
    }
    let fd = file.as_raw_fd();
    let flags = OFlag::from_bits_retain(fcntl(fd, FcntlArg::F_GETFL)?);
    fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
    let input = StdinFd { file, flags };
    Ok(Box::new(ReadyStdin(AsyncFd::new(input)?)))
}

struct StdinFd {
    file: File,
    flags: OFlag,
}

impl AsRawFd for StdinFd {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.file.as_raw_fd()
    }
}

impl Drop for StdinFd {
    fn drop(&mut self) {
        // The duplicate shares file status flags with stdin. Restore the flags
        // before releasing it, including when reactor registration fails.
        if let Err(error) = fcntl(self.file.as_raw_fd(), FcntlArg::F_SETFL(self.flags)) {
            tracing::warn!(%error, "failed to restore stdin flags");
        }
    }
}

struct ReadyStdin(AsyncFd<StdinFd>);

impl AsyncRead for ReadyStdin {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            let mut ready = std::task::ready!(self.0.poll_read_ready(cx))?;
            match ready.try_io(|inner| (&inner.get_ref().file).read(buf.initialize_unfilled())) {
                Ok(Ok(count)) => {
                    buf.advance(count);
                    return Poll::Ready(Ok(()));
                }
                Ok(Err(error)) => return Poll::Ready(Err(error)),
                Err(_) => continue,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::task::{Wake, Waker};
    use tokio::io::AsyncReadExt;

    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    fn pipe_input() -> (ReadyStdin, std::os::fd::OwnedFd, File, OFlag) {
        let (read, write) = nix::unistd::pipe().unwrap();
        let observer = File::from(read.try_clone().unwrap());
        let file = File::from(read);
        let flags = OFlag::from_bits_retain(fcntl(file.as_raw_fd(), FcntlArg::F_GETFL).unwrap());
        fcntl(
            file.as_raw_fd(),
            FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK),
        )
        .unwrap();
        (
            ReadyStdin(AsyncFd::new(StdinFd { file, flags }).unwrap()),
            write,
            observer,
            flags,
        )
    }

    #[tokio::test]
    async fn empty_buffer_completes_without_readiness() {
        let (mut input, _writer, _observer, _) = pipe_input();
        let waker = Waker::from(Arc::new(NoopWake));
        let mut cx = Context::from_waker(&waker);
        let mut bytes = [];
        let mut buf = ReadBuf::new(&mut bytes);
        assert!(matches!(
            Pin::new(&mut input).poll_read(&mut cx, &mut buf),
            Poll::Ready(Ok(()))
        ));
    }

    #[tokio::test]
    async fn eof_and_original_flags_are_preserved() {
        let (mut input, writer, observer, flags) = pipe_input();
        assert!(
            OFlag::from_bits_retain(fcntl(observer.as_raw_fd(), FcntlArg::F_GETFL).unwrap())
                .contains(OFlag::O_NONBLOCK)
        );
        drop(writer);
        let mut bytes = [0; 1];
        assert_eq!(input.read(&mut bytes).await.unwrap(), 0);
        drop(input);
        assert_eq!(
            OFlag::from_bits_retain(fcntl(observer.as_raw_fd(), FcntlArg::F_GETFL).unwrap()),
            flags
        );
    }
}
