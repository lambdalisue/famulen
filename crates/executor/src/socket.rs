//! The one way to an executor: a socket in its directory, which the owner
//! alone can open, or a named pipe on Windows. No port is listened on.

use std::path::Path;

pub(crate) use system::*;

/// What makes a directory for the owner alone.
pub(crate) fn private_dir(recursive: bool) -> std::fs::DirBuilder {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
}

#[cfg(unix)]
mod system {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;
    use tokio::net::{UnixListener, UnixStream};

    const NAME: &str = "socket";

    pub(crate) type Stream = UnixStream;
    pub(crate) struct Listener(UnixListener);

    /// The longest path of a socket every system takes.
    const LONGEST: usize = 100;

    impl Listener {
        /// Listen in the directory this process works in, as its whole path
        /// may be longer than a socket's may be.
        pub(crate) fn bind() -> std::io::Result<Self> {
            let path = Path::new(NAME);
            let listener = UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self(listener))
        }

        /// Whoever connects next, of the owner's own processes.
        pub(crate) async fn accept(&mut self) -> std::io::Result<Stream> {
            loop {
                let (stream, _) = self.0.accept().await?;
                if is_owner(&stream).unwrap_or(false) {
                    return Ok(stream);
                }
            }
        }
    }

    /// It cannot be told once whoever is at the other end is gone.
    fn is_owner(stream: &UnixStream) -> std::io::Result<bool> {
        Ok(stream.peer_cred()?.uid() == rustix::process::getuid().as_raw())
    }

    /// Connect, to one of the owner's own processes alone. `None` when it is
    /// gone as soon as it is reached.
    pub(crate) async fn connect(dir: &Path) -> std::io::Result<Option<Stream>> {
        let path = dir.join(NAME);
        let stream = if path.as_os_str().len() <= LONGEST {
            UnixStream::connect(path).await?
        } else {
            let short = Short::to(dir)?;
            UnixStream::connect(short.0.join(NAME)).await?
        };
        match is_owner(&stream) {
            Ok(true) => Ok(Some(stream)),
            Ok(false) => Err(std::io::Error::other(
                "the executor's socket is another user's",
            )),
            Err(_) => Ok(None),
        }
    }

    /// A short name of a directory for as long as it is kept.
    struct Short(std::path::PathBuf);

    impl Short {
        fn to(dir: &Path) -> std::io::Result<Self> {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            // Made here and nowhere else, or not at all.
            let dir = std::path::absolute(dir)?;
            loop {
                let name = format!(
                    "/tmp/.famulen-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                );
                match std::os::unix::fs::symlink(&dir, &name) {
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    made => return made.map(|()| Self(name.into())),
                }
            }
        }
    }

    impl Drop for Short {
        fn drop(&mut self) {
            std::fs::remove_file(&self.0).ok();
        }
    }

    /// Nobody listens there any more.
    pub(crate) fn is_gone(error: &std::io::Error) -> bool {
        use std::io::ErrorKind;
        matches!(
            error.kind(),
            ErrorKind::NotFound | ErrorKind::ConnectionRefused
        )
    }
}

#[cfg(windows)]
mod system {
    use super::*;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    pub(crate) type Stream = NamedPipe;

    /// Either end.
    pub(crate) enum NamedPipe {
        Server(NamedPipeServer),
        Client(tokio::net::windows::named_pipe::NamedPipeClient),
    }

    pub(crate) struct Listener {
        name: String,
        next: NamedPipeServer,
    }

    fn name(dir: &Path) -> String {
        // FNV-1a, as a name must be the same whatever builds this.
        let hash = dir
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
            });
        format!(r"\\.\pipe\famulen-executor-{hash:016x}")
    }

    impl Listener {
        pub(crate) fn bind() -> std::io::Result<Self> {
            let name = name(&std::env::current_dir()?);
            let next = ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(&name)?;
            Ok(Self { name, next })
        }

        pub(crate) async fn accept(&mut self) -> std::io::Result<Stream> {
            self.next.connect().await?;
            let next = ServerOptions::new()
                .reject_remote_clients(true)
                .create(&self.name)?;
            Ok(NamedPipe::Server(std::mem::replace(&mut self.next, next)))
        }
    }

    pub(crate) async fn connect(dir: &Path) -> std::io::Result<Option<Stream>> {
        const ALL_BUSY: i32 = 231;
        let name = name(dir);
        loop {
            match ClientOptions::new().open(&name) {
                Ok(client) => return Ok(Some(NamedPipe::Client(client))),
                Err(error) if error.raw_os_error() == Some(ALL_BUSY) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub(crate) fn is_gone(error: &std::io::Error) -> bool {
        error.kind() == std::io::ErrorKind::NotFound
    }

    mod io {
        use super::NamedPipe;
        use std::pin::Pin;
        use std::task::{Context, Poll};
        use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

        macro_rules! either {
            ($self:ident, $pipe:ident => $do:expr) => {
                match $self.get_mut() {
                    NamedPipe::Server($pipe) => $do,
                    NamedPipe::Client($pipe) => $do,
                }
            };
        }

        impl AsyncRead for NamedPipe {
            fn poll_read(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<std::io::Result<()>> {
                either!(self, p => Pin::new(p).poll_read(cx, buf))
            }
        }

        impl AsyncWrite for NamedPipe {
            fn poll_write(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::io::Result<usize>> {
                either!(self, p => Pin::new(p).poll_write(cx, buf))
            }
            fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
                either!(self, p => Pin::new(p).poll_flush(cx))
            }
            fn poll_shutdown(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                either!(self, p => Pin::new(p).poll_shutdown(cx))
            }
        }
    }
}
