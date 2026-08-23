use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::request::{ParseError, Request, MAX_HEADER_BYTES, MAX_REQUEST_LINE};
use crate::response::Response;
use crate::router::Router;

pub struct Server {
    router: Arc<Router>,
    max_connections: usize,
}

const READ_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REQUESTS_PER_CONNECTION: usize = 1000;
/// Cap on concurrent connections. Past this, new connections are closed
/// immediately so a flood cannot exhaust threads or file descriptors.
const MAX_CONNECTIONS: usize = 1024;

/// Decrements the live-connection counter when a connection's thread ends.
struct ConnGuard(Arc<AtomicUsize>);
impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Server {
    pub fn new(router: Router) -> Server {
        Server {
            router: Arc::new(router),
            max_connections: MAX_CONNECTIONS,
        }
    }

    /// Override the concurrent-connection cap.
    pub fn with_max_connections(mut self, n: usize) -> Server {
        self.max_connections = n;
        self
    }

    /// Bind and serve forever, one thread per connection. A panic or error
    /// handling a single connection is caught and logged, it never brings
    /// down the listener or any other connection.
    pub fn listen(&self, addr: &str) -> std::io::Result<()> {
        let listener = TcpListener::bind(addr)?;
        let active = Arc::new(AtomicUsize::new(0));
        for incoming in listener.incoming() {
            let stream = match incoming {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Bound concurrency. Past the cap, drop the stream (a hard close)
            // rather than spawn an unbounded thread.
            if active.fetch_add(1, Ordering::SeqCst) >= self.max_connections {
                active.fetch_sub(1, Ordering::SeqCst);
                drop(stream);
                continue;
            }
            let guard = ConnGuard(Arc::clone(&active));
            let router = Arc::clone(&self.router);
            thread::spawn(move || {
                let _guard = guard; // held for the life of the connection
                let _ = handle_connection(stream, router);
            });
        }
        Ok(())
    }
}

fn handle_connection(stream: TcpStream, router: Arc<Router>) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let mut write_stream = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    for _ in 0..MAX_REQUESTS_PER_CONNECTION {
        match Request::parse(&mut reader) {
            Ok(req) => {
                let keep_alive = req.keep_alive();
                let response = dispatch_safely(&router, &req);
                if write_stream.write_all(&response.to_bytes(keep_alive)).is_err() {
                    return Ok(());
                }
                if !keep_alive {
                    return Ok(());
                }
            }
            Err(ParseError::ConnectionClosed) => {
                return Ok(());
            }
            Err(err) => {
                let resp = response_for_parse_error(&err);
                let _ = write_stream.write_all(&resp.to_bytes(false));
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Never let a handler panic take the connection thread down silently with
/// the socket left open. Catch it and answer with 500.
fn dispatch_safely(router: &Router, req: &Request) -> Response {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| router.dispatch(req)));
    match result {
        Ok(resp) => resp,
        Err(_) => Response::new(500).with_text("500 internal server error"),
    }
}

fn response_for_parse_error(err: &ParseError) -> Response {
    match err {
        ParseError::LineTooLong => Response::new(414).with_text("414 request line too long"),
        ParseError::HeadersTooLarge => Response::new(431).with_text("431 headers too large"),
        ParseError::TooManyHeaders => Response::new(431).with_text("431 too many headers"),
        ParseError::BodyTooLarge => Response::new(413).with_text("413 body too large"),
        ParseError::Malformed(msg) => Response::bad_request(msg),
        ParseError::Io(_) => Response::new(400).with_text("400 bad request"),
        ParseError::ConnectionClosed => Response::new(400).with_text("400 bad request"),
    }
}

#[allow(dead_code)]
const _LIMITS_DOC: (usize, usize) = (MAX_REQUEST_LINE, MAX_HEADER_BYTES);
