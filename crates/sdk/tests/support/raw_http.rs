//! A raw TCP HTTP/1.1 server for transport tests: scripted replies, one per
//! request in arrival order, with explicit counters and gates instead of
//! timing. Each accepted connection runs on its own task and serves
//! requests until its reply ends the connection, so keep-alive reuse is
//! observable through [`Server::accepts`].

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
    task::JoinHandle,
};

/// What the server does with the next request.
pub(crate) enum Reply {
    /// Writes these bytes; the connection then serves the next request,
    /// unless the head says `Connection: close`.
    Bytes(String),
    /// Writes each piece after its delay, then closes.
    Chunks(Vec<(Vec<u8>, Duration)>),
    /// Writes a head, then holds the connection until the client closes it.
    HeadersThenHang(String),
    /// Writes a head, then closes.
    HeadersThenDisconnect(String),
    /// Closes the connection as soon as it is accepted, before reading.
    AcceptThenClose,
    /// Reads the request, then holds the connection until the client closes
    /// it.
    Hang,
    /// Reads the request, then closes without answering.
    Disconnect,
}

#[derive(Default)]
struct Counters {
    accepts: AtomicUsize,
    disconnects: AtomicUsize,
    disconnected: Notify,
    written: AtomicUsize,
    chunks_done: AtomicUsize,
}

pub(crate) struct Server {
    pub(crate) base: String,
    requests: Arc<Mutex<Vec<String>>>,
    counters: Arc<Counters>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

type Replies = Arc<Mutex<VecDeque<Reply>>>;

impl Server {
    pub(crate) async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let counters = Arc::new(Counters::default());
        let replies: Replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let task = tokio::spawn({
            let requests = Arc::clone(&requests);
            let counters = Arc::clone(&counters);
            async move {
                // Dropped with the server task, aborting every connection.
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    counters.accepts.fetch_add(1, Ordering::SeqCst);
                    connections.spawn(serve(
                        stream,
                        Arc::clone(&replies),
                        Arc::clone(&requests),
                        Arc::clone(&counters),
                    ));
                }
            }
        });
        Self {
            base,
            requests,
            counters,
            task,
        }
    }

    /// Requests read so far, raw.
    pub(crate) fn seen(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Connections accepted so far.
    pub(crate) fn accepts(&self) -> usize {
        self.counters.accepts.load(Ordering::SeqCst)
    }

    /// Bytes of [`Reply::Chunks`] pieces written so far.
    pub(crate) fn written(&self) -> usize {
        self.counters.written.load(Ordering::SeqCst)
    }

    /// [`Reply::Chunks`] replies written to the end.
    pub(crate) fn chunks_done(&self) -> usize {
        self.counters.chunks_done.load(Ordering::SeqCst)
    }

    /// Waits until a holding reply saw its client close the connection.
    pub(crate) async fn disconnected(&self) {
        loop {
            let notified = self.counters.disconnected.notified();
            if self.counters.disconnects.load(Ordering::SeqCst) > 0 {
                return;
            }
            notified.await;
        }
    }
}

fn next_reply(replies: &Replies) -> Reply {
    replies
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
        .unwrap_or(Reply::Disconnect)
}

fn next_closes_on_accept(replies: &Replies) -> bool {
    let mut replies = replies.lock().unwrap_or_else(PoisonError::into_inner);
    if matches!(replies.front(), Some(Reply::AcceptThenClose)) {
        replies.pop_front();
        return true;
    }
    false
}

async fn serve(
    mut stream: TcpStream,
    replies: Replies,
    requests: Arc<Mutex<Vec<String>>>,
    counters: Arc<Counters>,
) {
    if next_closes_on_accept(&replies) {
        return;
    }
    loop {
        let Some(request) = read_request(&mut stream).await else {
            return;
        };
        requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        match next_reply(&replies) {
            Reply::Bytes(bytes) => {
                if stream.write_all(bytes.as_bytes()).await.is_err() {
                    return;
                }
                let head = bytes.split("\r\n\r\n").next().unwrap_or_default();
                if head.to_ascii_lowercase().contains("connection: close") {
                    return;
                }
            },
            Reply::Chunks(pieces) => {
                for (piece, delay) in pieces {
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    if stream.write_all(&piece).await.is_err() {
                        return;
                    }
                    counters.written.fetch_add(piece.len(), Ordering::SeqCst);
                }
                counters.chunks_done.fetch_add(1, Ordering::SeqCst);
                return;
            },
            Reply::HeadersThenHang(head) => {
                let _ = stream.write_all(head.as_bytes()).await;
                hold_until_closed(&mut stream, &counters).await;
                return;
            },
            Reply::HeadersThenDisconnect(head) => {
                let _ = stream.write_all(head.as_bytes()).await;
                return;
            },
            Reply::Hang => {
                hold_until_closed(&mut stream, &counters).await;
                return;
            },
            Reply::AcceptThenClose | Reply::Disconnect => return,
        }
    }
}

/// Reads until the client closes the connection, then counts it.
async fn hold_until_closed(stream: &mut TcpStream, counters: &Counters) {
    let mut buffer = [0; 1024];
    while let Ok(size) = stream.read(&mut buffer).await {
        if size == 0 {
            break;
        }
    }
    counters.disconnects.fetch_add(1, Ordering::SeqCst);
    counters.disconnected.notify_waiters();
}

/// One request: its head and a `Content-Length` body. `None` when the client
/// closed the connection first.
async fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut request = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let size = stream.read(&mut buffer).await.unwrap_or(0);
        if size == 0 {
            return None;
        }
        request.extend_from_slice(&buffer[..size]);
        if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .map(|length| length.parse::<usize>().unwrap())
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                return Some(String::from_utf8_lossy(&request).into_owned());
            }
        }
    }
}
