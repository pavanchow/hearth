use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::net::TcpStream;

/// Hard caps so a hostile client cannot exhaust memory with an endless line
/// or a body larger than we are willing to buffer.
pub const MAX_REQUEST_LINE: usize = 8 * 1024;
pub const MAX_HEADER_BYTES: usize = 64 * 1024;
pub const MAX_HEADERS: usize = 100;
pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
    Patch,
    Other,
}

impl Method {
    pub fn parse(s: &str) -> Method {
        match s {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "DELETE" => Method::Delete,
            "HEAD" => Method::Head,
            "OPTIONS" => Method::Options,
            "PATCH" => Method::Patch,
            _ => Method::Other,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
            Method::Options => "OPTIONS",
            Method::Patch => "PATCH",
            Method::Other => "OTHER",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    LineTooLong,
    HeadersTooLarge,
    TooManyHeaders,
    BodyTooLarge,
    Malformed(String),
    ConnectionClosed,
    Io(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::LineTooLong => write!(f, "request line too long"),
            ParseError::HeadersTooLarge => write!(f, "headers too large"),
            ParseError::TooManyHeaders => write!(f, "too many headers"),
            ParseError::BodyTooLarge => write!(f, "body too large"),
            ParseError::Malformed(s) => write!(f, "malformed request: {s}"),
            ParseError::ConnectionClosed => write!(f, "connection closed before a request arrived"),
            ParseError::Io(s) => write!(f, "io error: {s}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub raw_method: String,
    pub path: String,
    pub query: Option<String>,
    pub version: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(|v| v.as_str())
    }

    pub fn keep_alive(&self) -> bool {
        match self.header("connection") {
            Some(v) if v.eq_ignore_ascii_case("close") => false,
            Some(v) if v.eq_ignore_ascii_case("keep-alive") => true,
            _ => self.version == "HTTP/1.1",
        }
    }

    /// Parse one HTTP/1.1 request from a buffered byte stream. Leaves the
    /// stream positioned right after the body, ready for the next request
    /// on the same connection.
    pub fn parse<R: Read>(reader: &mut BufReader<R>) -> Result<Request, ParseError> {
        let line = read_line_limited(reader, MAX_REQUEST_LINE)?;
        if line.is_empty() {
            return Err(ParseError::ConnectionClosed);
        }
        let (raw_method, path, query, version) = parse_request_line(&line)?;
        let method = Method::parse(&raw_method);

        let (headers, header_bytes) = parse_headers(reader)?;
        let _ = header_bytes;

        let body = read_body(reader, &headers)?;

        Ok(Request {
            method,
            raw_method,
            path,
            query,
            version,
            headers,
            body,
        })
    }
}

fn read_line_limited<R: Read>(reader: &mut BufReader<R>, limit: usize) -> Result<String, ParseError> {
    let mut buf = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        let n = reader.read(&mut byte).map_err(|e| ParseError::Io(e.to_string()))?;
        if n == 0 {
            if buf.is_empty() {
                return Ok(String::new());
            }
            break;
        }
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
        if buf.len() > limit {
            return Err(ParseError::LineTooLong);
        }
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    String::from_utf8(buf).map_err(|_| ParseError::Malformed("non-utf8 line".into()))
}

fn parse_request_line(line: &str) -> Result<(String, String, Option<String>, String), ParseError> {
    let mut parts = line.split(' ');
    let method = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ParseError::Malformed("missing method".into()))?;
    let target = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ParseError::Malformed("missing request target".into()))?;
    let version = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ParseError::Malformed("missing HTTP version".into()))?;
    if parts.next().is_some() {
        return Err(ParseError::Malformed("too many tokens in request line".into()));
    }
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(ParseError::Malformed(format!("unsupported version {version}")));
    }
    if !target.starts_with('/') {
        return Err(ParseError::Malformed("request target must start with /".into()));
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target.to_string(), None),
    };
    Ok((method.to_string(), path, query, version.to_string()))
}

fn parse_headers<R: Read>(reader: &mut BufReader<R>) -> Result<(HashMap<String, String>, usize), ParseError> {
    let mut headers = HashMap::new();
    let mut total = 0usize;
    let mut count = 0usize;
    loop {
        let line = read_line_limited(reader, MAX_REQUEST_LINE)?;
        total += line.len() + 2;
        if total > MAX_HEADER_BYTES {
            return Err(ParseError::HeadersTooLarge);
        }
        if line.is_empty() {
            break;
        }
        count += 1;
        if count > MAX_HEADERS {
            return Err(ParseError::TooManyHeaders);
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| ParseError::Malformed(format!("bad header line: {line}")))?;
        let name = name.trim();
        if name.is_empty() || name.contains(' ') {
            return Err(ParseError::Malformed(format!("bad header name: {name}")));
        }
        let value = value.trim();
        headers.insert(name.to_ascii_lowercase(), value.to_string());
    }
    Ok((headers, total))
}

fn read_body<R: Read>(
    reader: &mut BufReader<R>,
    headers: &HashMap<String, String>,
) -> Result<Vec<u8>, ParseError> {
    if let Some(te) = headers.get("transfer-encoding") {
        if te.eq_ignore_ascii_case("chunked") {
            return read_chunked_body(reader);
        }
        return Err(ParseError::Malformed(format!("unsupported transfer-encoding: {te}")));
    }
    let len = match headers.get("content-length") {
        Some(v) => v
            .trim()
            .parse::<usize>()
            .map_err(|_| ParseError::Malformed(format!("bad content-length: {v}")))?,
        None => 0,
    };
    if len > MAX_BODY_BYTES {
        return Err(ParseError::BodyTooLarge);
    }
    if len == 0 {
        return Ok(Vec::new());
    }
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .map_err(|e| ParseError::Io(e.to_string()))?;
    Ok(body)
}

fn read_chunked_body<R: Read>(reader: &mut BufReader<R>) -> Result<Vec<u8>, ParseError> {
    let mut body = Vec::new();
    loop {
        let size_line = read_line_limited(reader, MAX_REQUEST_LINE)?;
        let size_str = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| ParseError::Malformed(format!("bad chunk size: {size_line}")))?;
        if size == 0 {
            // consume trailing headers/blank line
            loop {
                let l = read_line_limited(reader, MAX_REQUEST_LINE)?;
                if l.is_empty() {
                    break;
                }
            }
            break;
        }
        if body.len() + size > MAX_BODY_BYTES {
            return Err(ParseError::BodyTooLarge);
        }
        let mut chunk = vec![0u8; size];
        reader
            .read_exact(&mut chunk)
            .map_err(|e| ParseError::Io(e.to_string()))?;
        body.extend_from_slice(&chunk);
        // consume trailing CRLF after the chunk data
        let _ = read_line_limited(reader, MAX_REQUEST_LINE)?;
    }
    Ok(body)
}

pub fn parse_from_stream(stream: &TcpStream) -> Result<Request, ParseError> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| ParseError::Io(e.to_string()))?);
    Request::parse(&mut reader)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(s: &str) -> Result<Request, ParseError> {
        let mut reader = BufReader::new(s.as_bytes());
        Request::parse(&mut reader)
    }

    #[test]
    fn parses_well_formed_get() {
        let raw = "GET /hello?x=1 HTTP/1.1\r\nHost: example.com\r\nUser-Agent: test\r\n\r\n";
        let req = parse_str(raw).expect("should parse");
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.path, "/hello");
        assert_eq!(req.query.as_deref(), Some("x=1"));
        assert_eq!(req.version, "HTTP/1.1");
        assert_eq!(req.header("host"), Some("example.com"));
        assert_eq!(req.header("user-agent"), Some("test"));
        assert!(req.body.is_empty());
    }

    #[test]
    fn parses_post_with_content_length_body() {
        let raw = "POST /echo HTTP/1.1\r\nHost: h\r\nContent-Length: 11\r\n\r\nhello world";
        let req = parse_str(raw).expect("should parse");
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/echo");
        assert_eq!(req.body, b"hello world");
    }

    #[test]
    fn rejects_malformed_request_line() {
        let raw = "GARBAGE\r\n\r\n";
        let err = parse_str(raw).unwrap_err();
        matches!(err, ParseError::Malformed(_));
    }

    #[test]
    fn rejects_bad_method_target_missing_slash() {
        let raw = "GET foo HTTP/1.1\r\n\r\n";
        let err = parse_str(raw).unwrap_err();
        matches!(err, ParseError::Malformed(_));
    }

    #[test]
    fn rejects_unsupported_version() {
        let raw = "GET / HTTP/2.0\r\n\r\n";
        let err = parse_str(raw).unwrap_err();
        matches!(err, ParseError::Malformed(_));
    }

    #[test]
    fn rejects_oversized_request_line() {
        let long_path = "a".repeat(MAX_REQUEST_LINE + 10);
        let raw = format!("GET /{long_path} HTTP/1.1\r\n\r\n");
        let err = parse_str(&raw).unwrap_err();
        assert_eq!(err, ParseError::LineTooLong);
    }

    #[test]
    fn empty_stream_is_connection_closed() {
        let err = parse_str("").unwrap_err();
        assert_eq!(err, ParseError::ConnectionClosed);
    }

    #[test]
    fn keep_alive_default_for_http11() {
        let raw = "GET / HTTP/1.1\r\nHost: h\r\n\r\n";
        let req = parse_str(raw).unwrap();
        assert!(req.keep_alive());
    }

    #[test]
    fn connection_close_header_disables_keep_alive() {
        let raw = "GET / HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n";
        let req = parse_str(raw).unwrap();
        assert!(!req.keep_alive());
    }
}
