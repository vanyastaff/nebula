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

/// Upper bound on the byte length of a Unix-domain-socket path the plugin
/// transport is willing to bind. `sun_path` in `sockaddr_un` is only
/// 108 bytes on Linux / 104 on macOS; leaving headroom for the trailing
/// NUL and for platforms with tighter limits keeps `UnixListener::bind()`
/// from surprising callers with `ENAMETOOLONG`.
///
/// Exposed so host-side callers (nebula-sandbox) that allocate the socket
/// path themselves can enforce the same cap before spawning the plugin.
#[cfg(unix)]
pub const MAX_UNIX_SOCKET_PATH_BYTES: usize = 100;

/// Environment variable the host uses to tell the plugin exactly which
/// socket address to bind (#260). When set, the plugin MUST bind this
/// address and announce it in the handshake — no plugin-chosen path.
///
/// The corresponding [`ENV_SOCKET_KIND`] indicates `"unix"` or `"pipe"`.
pub const ENV_SOCKET_ADDR: &str = "NEBULA_PLUGIN_SOCKET_ADDR";

/// Environment variable that pairs with [`ENV_SOCKET_ADDR`] and names the
/// transport kind (`"unix"` for Unix domain sockets, `"pipe"` for Windows
/// named pipes). Required when [`ENV_SOCKET_ADDR`] is set.
pub const ENV_SOCKET_KIND: &str = "NEBULA_PLUGIN_SOCKET_KIND";

/// Bind a transport listener, return it paired with the handshake line the
/// plugin should print on stdout before calling [`PluginListener::accept`].
///
/// # Host-controlled address (#260)
///
/// If the host sets [`ENV_SOCKET_ADDR`] + [`ENV_SOCKET_KIND`] (which the
/// Nebula `ProcessSandbox` always does), the plugin MUST bind exactly that
/// address. A compromised plugin that tries to print a different address
/// in its handshake is rejected by the host-side validator in
/// `nebula-sandbox`, preventing the "forged handshake → hijack sibling
/// plugin socket" attack.
///
/// When the env vars are not set (standalone plugin development, ad-hoc
/// tests) the plugin falls back to self-allocating a per-plugin directory
/// — the pre-#260 behaviour, kept for DX.
pub fn bind_listener() -> io::Result<(PluginListener, String)> {
    match classify_env(
        std::env::var(ENV_SOCKET_ADDR).ok(),
        std::env::var(ENV_SOCKET_KIND).ok(),
    ) {
        EnvBindMode::HostProvided { addr, kind } => bind_listener_at(&addr, &kind),
        EnvBindMode::SelfAllocate => {
            #[cfg(unix)]
            {
                bind_unix()
            }
            #[cfg(windows)]
            {
                bind_named_pipe()
            }
        },
        EnvBindMode::PartialError(err) => Err(err),
    }
}

/// Decision the plugin transport makes from the `(ADDR, KIND)` env pair.
enum EnvBindMode {
    /// Both env vars set — bind at the host-provided address.
    HostProvided { addr: String, kind: String },
    /// Neither set — self-allocate a per-plugin directory (ad-hoc/dev).
    SelfAllocate,
    /// Exactly one set — misconfiguration, fail fast.
    PartialError(io::Error),
}

/// Pure classifier split out of [`bind_listener`] so the decision logic
/// can be exercised without mutating process-global env state.
///
/// Partial configuration is a host bug (or tampering): one env var alone
/// cannot express the host's chosen transport. Silently falling through
/// to self-allocation would hide the misconfigured spawn and defeat the
/// #260 handshake-forgery mitigation whenever the sandbox expected to
/// enforce a host-chosen address.
fn classify_env(addr: Option<String>, kind: Option<String>) -> EnvBindMode {
    match (addr, kind) {
        (Some(addr), Some(kind)) => EnvBindMode::HostProvided { addr, kind },
        (Some(_), None) => EnvBindMode::PartialError(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{ENV_SOCKET_ADDR} is set but {ENV_SOCKET_KIND} is missing; \
                 both must be set together"
            ),
        )),
        (None, Some(_)) => EnvBindMode::PartialError(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{ENV_SOCKET_KIND} is set but {ENV_SOCKET_ADDR} is missing; \
                 both must be set together"
            ),
        )),
        (None, None) => EnvBindMode::SelfAllocate,
    }
}

fn bind_listener_at(addr: &str, kind: &str) -> io::Result<(PluginListener, String)> {
    match kind {
        "unix" => {
            #[cfg(unix)]
            {
                bind_unix_at(addr)
            }
            #[cfg(not(unix))]
            {
                let _ = addr;
                Err(io::Error::other(format!(
                    "{ENV_SOCKET_KIND}=unix requested but this platform is not Unix"
                )))
            }
        },
        "pipe" => {
            #[cfg(windows)]
            {
                bind_pipe_at(addr)
            }
            #[cfg(not(windows))]
            {
                let _ = addr;
                Err(io::Error::other(format!(
                    "{ENV_SOCKET_KIND}=pipe requested but this platform is not Windows"
                )))
            }
        },
        other => Err(io::Error::other(format!(
            "unknown value for {ENV_SOCKET_KIND}: `{other}` (expected `unix` or `pipe`)"
        ))),
    }
}

#[cfg(unix)]
fn bind_unix_at(addr: &str) -> io::Result<(PluginListener, String)> {
    use std::os::unix::fs::PermissionsExt;

    let socket_path = PathBuf::from(addr);
    // Host is responsible for creating the parent directory with 0700
    // before spawn. Plugin binds the socket file itself and sets 0600.
    let listener = tokio::net::UnixListener::bind(&socket_path)?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;

    let line = format!(
        "NEBULA-PROTO-{HANDSHAKE_VERSION}|unix|{}",
        socket_path.display()
    );
    Ok((
        PluginListener::Unix {
            listener,
            // Cleanup is the host's responsibility — we didn't create
            // the directory, so we don't remove it.
            cleanup: CleanupGuard { dir: None },
        },
        line,
    ))
}

#[cfg(windows)]
fn bind_pipe_at(addr: &str) -> io::Result<(PluginListener, String)> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(addr)?;
    let line = format!("NEBULA-PROTO-{HANDSHAKE_VERSION}|pipe|{addr}");
    Ok((
        PluginListener::NamedPipe {
            server: Some(server),
        },
        line,
    ))
}

#[cfg(unix)]
fn bind_unix() -> io::Result<(PluginListener, String)> {
    use std::os::unix::{ffi::OsStrExt, fs::PermissionsExt};

    fn socket_path_len(path: &std::path::Path) -> usize {
        path.as_os_str().as_bytes().len()
    }

    fn unix_temp_roots() -> Vec<PathBuf> {
        let system_tmp = std::env::temp_dir();
        let short_tmp = PathBuf::from("/tmp");
        if system_tmp == short_tmp {
            vec![system_tmp]
        } else {
            vec![short_tmp, system_tmp]
        }
    }

    let mut last_err: Option<io::Error> = None;
    for root in unix_temp_roots() {
        for _ in 0..32 {
            let nonce = uuid::Uuid::new_v4().simple().to_string();
            let candidate = root.join(format!("nebula-{}", &nonce[..8]));
            let socket_path = candidate.join("sock");
            if socket_path_len(&socket_path) > MAX_UNIX_SOCKET_PATH_BYTES {
                continue;
            }
            match std::fs::create_dir(&candidate) {
                Ok(()) => {
                    if let Err(e) =
                        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o700))
                    {
                        let _ = std::fs::remove_dir_all(&candidate);
                        last_err = Some(e);
                        continue;
                    }

                    let listener = match tokio::net::UnixListener::bind(&socket_path) {
                        Ok(listener) => listener,
                        Err(e) => {
                            let _ = std::fs::remove_dir_all(&candidate);
                            last_err = Some(e);
                            continue;
                        },
                    };
                    if let Err(e) = std::fs::set_permissions(
                        &socket_path,
                        std::fs::Permissions::from_mode(0o600),
                    ) {
                        drop(listener);
                        let _ = std::fs::remove_dir_all(&candidate);
                        last_err = Some(e);
                        continue;
                    }

                    let line = format!(
                        "NEBULA-PROTO-{HANDSHAKE_VERSION}|unix|{}",
                        socket_path.display()
                    );
                    return Ok((
                        PluginListener::Unix {
                            listener,
                            cleanup: CleanupGuard {
                                dir: Some(candidate),
                            },
                        },
                        line,
                    ));
                },
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    last_err = Some(e);
                    continue;
                },
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        io::Error::other("failed to allocate unix socket path for plugin transport")
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_partial_env_error(mode: EnvBindMode) {
        match mode {
            EnvBindMode::PartialError(err) => {
                assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
                let msg = err.to_string();
                assert!(
                    msg.contains(ENV_SOCKET_ADDR) && msg.contains(ENV_SOCKET_KIND),
                    "error must name both env vars; got: {msg}",
                );
            },
            EnvBindMode::HostProvided { .. } => {
                panic!("partial env must not be classified as host-provided")
            },
            EnvBindMode::SelfAllocate => panic!(
                "partial env must not fall through to self-allocation \
                 (would defeat #260 handshake mitigation)"
            ),
        }
    }

    #[test]
    fn classify_env_rejects_addr_without_kind() {
        assert_partial_env_error(classify_env(Some("/tmp/should-not-be-used".into()), None));
    }

    #[test]
    fn classify_env_rejects_kind_without_addr() {
        assert_partial_env_error(classify_env(None, Some("unix".into())));
    }

    #[test]
    fn classify_env_both_set_is_host_provided() {
        match classify_env(Some("/tmp/ok".into()), Some("unix".into())) {
            EnvBindMode::HostProvided { addr, kind } => {
                assert_eq!(addr, "/tmp/ok");
                assert_eq!(kind, "unix");
            },
            other => panic!(
                "both env vars set must classify as HostProvided, got other variant: {}",
                match other {
                    EnvBindMode::PartialError(e) => format!("PartialError({e})"),
                    EnvBindMode::SelfAllocate => "SelfAllocate".into(),
                    EnvBindMode::HostProvided { .. } => unreachable!(),
                },
            ),
        }
    }

    #[test]
    fn classify_env_neither_set_is_self_allocate() {
        assert!(matches!(
            classify_env(None, None),
            EnvBindMode::SelfAllocate
        ));
    }
}
