use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::response::Response;

pub struct StaticServer {
    root: PathBuf,
}

impl StaticServer {
    pub fn new(root: &str) -> StaticServer {
        StaticServer {
            root: fs::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root)),
        }
    }

    /// Resolve a request path against the served root, refusing to escape
    /// it. `..` components are rejected outright rather than merely
    /// normalized away, and the final resolved path is checked to still be
    /// inside the canonicalized root.
    pub fn resolve(&self, req_path: &str) -> Option<PathBuf> {
        let decoded = percent_decode(req_path);
        let rel = decoded.trim_start_matches('/');
        let rel_path = Path::new(rel);

        for comp in rel_path.components() {
            match comp {
                Component::Normal(_) => {}
                Component::CurDir => {}
                _ => return None, // ParentDir, RootDir, Prefix all rejected
            }
        }

        let candidate = self.root.join(rel_path);
        let full = if candidate.is_dir() {
            candidate.join("index.html")
        } else {
            candidate
        };

        let canonical = fs::canonicalize(&full).ok()?;
        if !canonical.starts_with(&self.root) {
            return None;
        }
        Some(canonical)
    }

    pub fn serve(&self, req_path: &str) -> Option<Response> {
        let path = self.resolve(req_path)?;
        if !path.is_file() {
            return None;
        }
        let bytes = fs::read(&path).ok()?;
        let content_type = content_type_for(&path);
        Some(
            Response::ok()
                .with_header("Content-Type", content_type)
                .with_body(bytes),
        )
    }
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hearth_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("index.html"), "<h1>root</h1>").unwrap();
        fs::write(dir.join("sub/page.html"), "<h1>sub</h1>").unwrap();
        fs::write(dir.join("style.css"), "body{}").unwrap();
        dir
    }

    #[test]
    fn serves_existing_file_with_correct_content_type() {
        let dir = make_fixture("serve_ok");
        let ss = StaticServer::new(dir.to_str().unwrap());
        let resp = ss.serve("/style.css").expect("file should be found");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("Content-Type").unwrap(), "text/css; charset=utf-8");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_returns_none_for_404() {
        let dir = make_fixture("serve_404");
        let ss = StaticServer::new(dir.to_str().unwrap());
        assert!(ss.serve("/does-not-exist.html").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn blocks_path_traversal_with_dotdot() {
        let dir = make_fixture("traversal");
        // a secret file that lives OUTSIDE the served root
        let outside = dir.parent().unwrap().join("hearth_test_outside_secret.txt");
        fs::write(&outside, "top secret").unwrap();

        let ss = StaticServer::new(dir.to_str().unwrap());
        assert!(ss.resolve("/../hearth_test_outside_secret.txt").is_none());
        assert!(ss.resolve("/sub/../../hearth_test_outside_secret.txt").is_none());
        assert!(ss.serve("/../hearth_test_outside_secret.txt").is_none());

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_file(&outside);
    }

    #[test]
    fn blocks_encoded_path_traversal() {
        let dir = make_fixture("traversal_encoded");
        let ss = StaticServer::new(dir.to_str().unwrap());
        assert!(ss.resolve("/%2e%2e/%2e%2e/etc/passwd").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn serves_directory_index() {
        let dir = make_fixture("dir_index");
        let ss = StaticServer::new(dir.to_str().unwrap());
        // sub/ has no index.html, so it 404s (None) rather than serving.
        assert!(ss.serve("/sub/").is_none());
        let root_resp = ss.serve("/").expect("root index.html should serve");
        assert_eq!(root_resp.body, b"<h1>root</h1>");
        let _ = fs::remove_dir_all(&dir);
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
