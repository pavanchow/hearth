use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use hearth::{build_router, Server};

/// Start the real server on an OS-assigned ephemeral port, in a background
/// thread, and return the port so the test can connect a plain TcpStream
/// client to it, exactly like a real client would.
fn start_test_server(dir: &str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    drop(listener); // free the port, then let Server::listen rebind it

    let dir = dir.to_string();
    thread::spawn(move || {
        let router = build_router(&dir);
        let server = Server::new(router);
        let _ = server.listen(&format!("127.0.0.1:{port}"));
    });

    // give the listener a moment to bind
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    port
}

fn send_raw(port: u16, raw: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    stream.write_all(b"").unwrap();
    // half-close write side is not available portably here, so we rely on
    // Connection: close in the request and read until EOF.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[test]
fn get_returns_expected_response() {
    let dir = std::env::temp_dir().join(format!("hearth_it_get_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<h1>hello from hearth</h1>").unwrap();

    let port = start_test_server(dir.to_str().unwrap());
    let resp = send_raw(port, "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");

    assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"), "unexpected status line: {resp}");
    assert!(resp.contains("hello from hearth"), "body missing: {resp}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn health_route_works() {
    let dir = std::env::temp_dir().join(format!("hearth_it_health_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let port = start_test_server(dir.to_str().unwrap());
    let resp = send_raw(port, "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");

    assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"), "unexpected status line: {resp}");
    assert!(resp.contains("\"status\":\"ok\""), "unexpected body: {resp}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_path_returns_404() {
    let dir = std::env::temp_dir().join(format!("hearth_it_404_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let port = start_test_server(dir.to_str().unwrap());
    let resp = send_raw(port, "GET /does-not-exist HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");

    assert!(resp.starts_with("HTTP/1.1 404 Not Found\r\n"), "unexpected status line: {resp}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_request_gets_400_and_server_survives() {
    let dir = std::env::temp_dir().join(format!("hearth_it_400_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let port = start_test_server(dir.to_str().unwrap());
    let resp = send_raw(port, "NOT A REQUEST AT ALL\r\n\r\n");
    assert!(resp.starts_with("HTTP/1.1 400"), "expected 400, got: {resp}");

    // the server must still be alive for the next connection
    let resp2 = send_raw(port, "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    assert!(resp2.starts_with("HTTP/1.1 200 OK\r\n"), "server did not survive: {resp2}");

    let _ = std::fs::remove_dir_all(&dir);
}
