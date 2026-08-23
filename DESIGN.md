# Design

Hearth is split into five small pieces: the request parser, the response writer, the router, the static file handler, and the connection model that ties them together with a CLI on top.

## The request parser (`src/request.rs`)

A `Request` is parsed straight off a `BufReader<TcpStream>`, one connection at a time, no async runtime involved. Parsing goes through three stages.

**Request line.** Read byte by byte up to a `\n`, trimming a trailing `\r`, with a hard cap (`MAX_REQUEST_LINE`, 8 KB) so a client that never sends a newline cannot make the server buffer forever. The line is split into exactly three tokens: method, target, version. Anything else, missing tokens, extra tokens, a target that does not start with `/`, or a version that is not `HTTP/1.1` or `HTTP/1.0`, is a `ParseError::Malformed` rather than a guess.

**Headers.** Read line by line the same way, tracking total bytes against `MAX_HEADER_BYTES` (64 KB) and a count against `MAX_HEADERS` (100), so a client cannot send an unbounded number of tiny headers either. Each line is split on the first `:`, name lowercased and trimmed, value trimmed, stored in a `HashMap<String, String>`. A blank line ends the header block.

**Body.** If `Transfer-Encoding: chunked` is present the body is read chunk by chunk, each chunk size a hex line followed by that many bytes and a trailing CRLF, terminated by a zero-length chunk. Otherwise `Content-Length` (default zero) says exactly how many bytes to `read_exact`. Both paths are capped at `MAX_BODY_BYTES` (10 MB).

A malformed request never panics. Every failure path returns a `ParseError` variant, and the caller decides the response.

## The response writer (`src/response.rs`)

`Response` holds a status code, a header map, and a body as bytes. `to_bytes(keep_alive)` writes the status line, then headers, then `Content-Length` and `Connection` if the caller has not set them explicitly, then a blank line, then the body, all in one pass. There is no intermediate string formatting of the whole response, so what you get is exactly what goes on the socket.

## The router (`src/router.rs`)

A `Router` holds a `HashMap<(Method, String), Handler>` for exact method and path matches, plus an optional `StaticServer`. Dispatch order is: exact route match, then the static file handler for `GET`/`HEAD`, then 404. No pattern matching, no path parameters, deliberately, a bigger router is a different project.

## Static serving and path-traversal defense (`src/static_files.rs`)

The served root is canonicalized once at startup. For every request path: percent-decode it first, then walk its path components and reject anything that is not `Normal` or `CurDir`, so a raw `..` or an encoded `%2e%2e` is rejected before it ever touches the filesystem. The remaining candidate path is joined onto the root, resolved to a directory index if it is a directory, and then canonicalized again. If the canonical result does not start with the canonical root, the request is refused. Two checks, component-level and canonical-path-level, because either one alone can be fooled by symlinks or platform-specific path quirks.

`Content-Type` is picked from a fixed extension table. Unknown extensions get `application/octet-stream` rather than a guess.

## The connection model (`src/server.rs`)

One OS thread per accepted connection. Each connection gets a read timeout (30s) and a cap on requests per connection (1000) so a single client cannot hold a thread hostage forever. Inside the loop:

1. Parse a request. A clean EOF ends the connection quietly. A parse error gets mapped to the matching status code (414, 431, 413, or 400) and the connection closes.
2. Dispatch through the router, wrapped in `catch_unwind` so a handler panic becomes a 500 instead of taking the thread, and the socket, down uncleanly.
3. Write the response. If the request asked to keep the connection alive (HTTP/1.1 default, or an explicit `Connection: keep-alive`, unless the client said `close`), loop and parse the next request off the same stream. Otherwise close.

Each connection thread is independent. A panic, a timeout, or a bad actor on one connection has no path to affect any other connection or the listener itself.
