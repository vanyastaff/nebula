//! Cross-platform socket/pipe transport for the duplex broker.
//!
//! Plugin side binds a UDS (Unix) or Named Pipe (Windows), prints the
//! handshake line to stdout, then accepts exactly one incoming connection
//! from the host. After accept the listener is dropped. Host side reads
//! the handshake line from the plugin's stdout, parses transport + address,
//! and dials via [`dial`].
//!
//! ## Handshake line format
//!
//! ```text
//! NEBULA-PROTO-2|unix|/tmp/nebula-plugin-<random>/sock  (Unix)
//! NEBULA-PROTO-2|pipe|\\.\pipe\LOCAL\nebula-plugin-<pid> (Windows)
//! ```
//!
//! Three pipe-separated fields: protocol version tag, transport kind
//! (`unix` or `pipe`), address.
//!
//! ## Auth model
//!
//! - **Unix**: socket lives inside a per-plugin directory with mode `0700`; the socket file itself
//!   is `0600`. `connect(2)` from any other uid fails.
//! - **Windows**: pipe is created under `\\.\pipe\LOCAL\` which is a session-scoped namespace —
//!   pipes created here are invisible to other logon sessions.

#[cfg(unix)]
use std::path::PathBuf;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::protocol::DUPLEX_PROTOCOL_VERSION;

/// Version tag emitted in the handshake line. Must match
/// [`DUPLEX_PROTOCOL_VERSION`].
pub const HANDSHAKE_VERSION: u32 = DUPLEX_PROTOCOL_VERSION;

/// Bind a transport listener, return it paired with the handshake line the
/// plugin should print on stdout before calling [`PluginListener::accept`].
pub fn bind_listener() -> io::Result<(PluginListener, String)> {
    #[cfg(unix)]
    {
        bind_unix()
    }
    #[cfg(windows)]
    {
        bind_named_pipe()
    }
}

#[cfg(unix)]
fn bind_unix() -> io::Result<(PluginListener, String)> {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = std::env::temp_dir();
    let mut dir = None;
    for _ in 0..16 {
        let candidate = temp_dir.join(format!("nebula-plugin-{}", uuid::Uuid::new_v4()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => {
                dir = Some(candidate);
                break;
            },
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let dir =
        dir.ok_or_else(|| io::Error::other("failed to allocate unique plugin socket directory"))?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;

    let socket_path = dir.join("sock");
    let listener = tokio::net::UnixListener::bind(&socket_path)?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;

    let line = format!(
        "NEBULA-PROTO-{HANDSHAKE_VERSION}|unix|{}",
        socket_path.display()
    );
    Ok((
        PluginListener::Unix {
            listener,
            cleanup: CleanupGuard { dir: Some(dir) },
        },
        line,
    ))
}

#[cfg(windows)]
fn bind_named_pipe() -> io::Result<(PluginListener, String)> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let pid = std::process::id();
    let name = format!(r"\\.\pipe\LOCAL\nebula-plugin-{pid}");
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)?;
    let line = format!("NEBULA-PROTO-{HANDSHAKE_VERSION}|pipe|{name}");
    Ok((
        PluginListener::NamedPipe {
            server: Some(server),
        },
        line,
    ))
}

/// A bound listener waiting for the host to connect.
pub enum PluginListener {
    /// Unix domain socket listener. Holds the bound `UnixListener` plus a
    /// drop-guard that removes the parent directory on listener drop.
    #[cfg(unix)]
    Unix {
        /// The bound listener.
        listener: tokio::net::UnixListener,
        /// Cleanup guard that removes the temp directory on drop.
        cleanup: CleanupGuard,
    },
    /// Windows named pipe server. Single-instance; `connect()` on
    /// [`NamedPipeServer`](tokio::net::windows::named_pipe::NamedPipeServer)
    /// waits for the host to connect.
    #[cfg(windows)]
    NamedPipe {
        /// Server handle, taken on `accept`.
        server: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
    },
}

impl PluginListener {
    /// Accept exactly one incoming connection and return the resulting stream.
    /// The listener is consumed; no further connect attempts can succeed.
    pub async fn accept(self) -> io::Result<PluginStream> {
        match self {
            #[cfg(unix)]
            PluginListener::Unix { listener, cleanup } => {
                let (stream, _addr) = listener.accept().await?;
                // The cleanup guard moves into the stream so the directory
                // persists until the stream itself drops.
                Ok(PluginStream::Unix {
                    stream,
                    _cleanup: cleanup,
                })
            },
            #[cfg(windows)]
            PluginListener::NamedPipe { mut server } => {
                let server = server
                    .take()
                    .ok_or_else(|| io::Error::other("listener already consumed"))?;
                server.connect().await?;
                Ok(PluginStream::NamedPipeServer(server))
            },
        }
    }
}

/// Host-side dial: parse a handshake line and connect to the announced
/// address.
pub async fn dial(handshake_line: &str) -> io::Result<PluginStream> {
    let line = handshake_line.trim();
    let mut parts = line.splitn(3, '|');
    let version = parts
        .next()
        .ok_or_else(|| io::Error::other("missing version in handshake"))?;
    let kind = parts
        .next()
        .ok_or_else(|| io::Error::other("missing transport kind in handshake"))?;
    let addr = parts
        .next()
        .ok_or_else(|| io::Error::other("missing address in handshake"))?;

    let expected = format!("NEBULA-PROTO-{HANDSHAKE_VERSION}");
    if version != expected {
        return Err(io::Error::other(format!(
            "protocol version mismatch: plugin said `{version}`, host expects `{expected}`"
        )));
    }

    match kind {
        "unix" => {
            #[cfg(unix)]
            {
                let stream = tokio::net::UnixStream::connect(addr).await?;
                Ok(PluginStream::Unix {
                    stream,
                    _cleanup: CleanupGuard { dir: None },
                })
            }
            #[cfg(not(unix))]
            {
                let _ = addr;
                Err(io::Error::other(
                    "unix transport requested but this platform is not Unix",
                ))
            }
        },
        "pipe" => {
            #[cfg(windows)]
            {
                use tokio::net::windows::named_pipe::ClientOptions;
                let client = ClientOptions::new().open(addr)?;
                Ok(PluginStream::NamedPipeClient(client))
            }
            #[cfg(not(windows))]
            {
                let _ = addr;
                Err(io::Error::other(
                    "named pipe transport requested but this platform is not Windows",
                ))
            }
        },
        other => Err(io::Error::other(format!(
            "unknown transport kind in handshake: `{other}`"
        ))),
    }
}

/// A duplex stream returned by [`PluginListener::accept`] (plugin side) or
/// [`dial`] (host side). Implements `AsyncRead + AsyncWrite`.
pub enum PluginStream {
    /// Unix domain socket stream. Both plugin-side (server) and host-side
    /// (client) use this variant. The `_cleanup` guard is `Some` on the
    /// plugin side only.
    #[cfg(unix)]
    Unix {
        /// Underlying stream.
        stream: tokio::net::UnixStream,
        /// Cleanup guard (plugin side only).
        _cleanup: CleanupGuard,
    },
    /// Windows named pipe, plugin-side server handle (post-connect).
    #[cfg(windows)]
    NamedPipeServer(tokio::net::windows::named_pipe::NamedPipeServer),
    /// Windows named pipe, host-side client handle.
    #[cfg(windows)]
    NamedPipeClient(tokio::net::windows::named_pipe::NamedPipeClient),
}

impl AsyncRead for PluginStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            PluginStream::Unix { stream, .. } => Pin::new(stream).poll_read(cx, buf),
            #[cfg(windows)]
            PluginStream::NamedPipeServer(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(windows)]
            PluginStream::NamedPipeClient(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for PluginStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            #[cfg(unix)]
            PluginStream::Unix { stream, .. } => Pin::new(stream).poll_write(cx, buf),
            #[cfg(windows)]
            PluginStream::NamedPipeServer(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(windows)]
            PluginStream::NamedPipeClient(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            PluginStream::Unix { stream, .. } => Pin::new(stream).poll_flush(cx),
            #[cfg(windows)]
            PluginStream::NamedPipeServer(s) => Pin::new(s).poll_flush(cx),
            #[cfg(windows)]
            PluginStream::NamedPipeClient(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            PluginStream::Unix { stream, .. } => Pin::new(stream).poll_shutdown(cx),
            #[cfg(windows)]
            PluginStream::NamedPipeServer(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(windows)]
            PluginStream::NamedPipeClient(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// RAII guard that removes the per-plugin temp directory when the owning
/// [`PluginStream`] (or [`PluginListener`], if never accepted) is dropped.
/// Host side constructs this with `dir: None` — cleanup is only the
/// plugin's responsibility.
#[cfg(unix)]
pub struct CleanupGuard {
    dir: Option<PathBuf>,
}

#[cfg(unix)]
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Some(dir) = self.dir.take() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
