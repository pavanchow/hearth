use std::collections::HashMap;
use std::io::{self, Write};

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub reason: &'static str,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16) -> Response {
        Response {
            status,
            reason: reason_phrase(status),
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    pub fn ok() -> Response {
        Response::new(200)
    }

    pub fn not_found() -> Response {
        Response::new(404).with_text("404 not found")
    }

    pub fn bad_request(msg: &str) -> Response {
        Response::new(400).with_text(&format!("400 bad request: {msg}"))
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Response {
        self.headers.insert(name.to_string(), value.to_string());
        self
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Response {
        self.body = body;
        self
    }

    pub fn with_text(mut self, text: &str) -> Response {
        self.body = text.as_bytes().to_vec();
        if !self.headers.contains_key("Content-Type") {
            self.headers
                .insert("Content-Type".to_string(), "text/plain; charset=utf-8".to_string());
        }
        self
    }

    pub fn with_json(mut self, json: &str) -> Response {
        self.body = json.as_bytes().to_vec();
        self.headers
            .insert("Content-Type".to_string(), "application/json".to_string());
        self
    }

    /// Serialize the status line, headers, and body to the exact bytes that
    /// go on the wire.
    pub fn to_bytes(&self, keep_alive: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(128 + self.body.len());
        out.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", self.status, self.reason).as_bytes());
        for (k, v) in &self.headers {
            out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
        }
        if !self.headers.contains_key("Content-Length") {
            out.extend_from_slice(format!("Content-Length: {}\r\n", self.body.len()).as_bytes());
        }
        if !self.headers.contains_key("Connection") {
            let conn = if keep_alive { "keep-alive" } else { "close" };
            out.extend_from_slice(format!("Connection: {conn}\r\n").as_bytes());
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&self.body);
        out
    }

    pub fn write_to<W: Write>(&self, w: &mut W, keep_alive: bool) -> io::Result<()> {
        w.write_all(&self.to_bytes(keep_alive))
    }
}

pub fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        414 => "URI Too Long",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_response_bytes_are_correct() {
        let resp = Response::ok().with_header("Content-Type", "text/plain").with_body(b"hi".to_vec());
        let bytes = resp.to_bytes(true);
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("Content-Type: text/plain\r\n"));
        assert!(s.contains("Content-Length: 2\r\n"));
        assert!(s.contains("Connection: keep-alive\r\n"));
        assert!(s.ends_with("\r\n\r\nhi"));
    }

    #[test]
    fn connection_close_when_not_keep_alive() {
        let resp = Response::ok();
        let s = String::from_utf8(resp.to_bytes(false)).unwrap();
        assert!(s.contains("Connection: close\r\n"));
    }

    #[test]
    fn not_found_has_404_status_line() {
        let resp = Response::not_found();
        let s = String::from_utf8(resp.to_bytes(true)).unwrap();
        assert!(s.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }
}
