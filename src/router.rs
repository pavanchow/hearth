use std::collections::HashMap;
use std::sync::Arc;

use crate::request::{Method, Request};
use crate::response::Response;
use crate::static_files::StaticServer;

pub type Handler = Arc<dyn Fn(&Request) -> Response + Send + Sync>;

#[derive(Default)]
pub struct Router {
    routes: HashMap<(Method, String), Handler>,
    static_server: Option<StaticServer>,
}

impl Router {
    pub fn new() -> Router {
        Router {
            routes: HashMap::new(),
            static_server: None,
        }
    }

    pub fn route<F>(&mut self, method: Method, path: &str, handler: F)
    where
        F: Fn(&Request) -> Response + Send + Sync + 'static,
    {
        self.routes.insert((method, path.to_string()), Arc::new(handler));
    }

    pub fn serve_dir(&mut self, root: &str) {
        self.static_server = Some(StaticServer::new(root));
    }

    /// Dispatch a request: an exact method+path route wins first, then the
    /// static file handler (GET/HEAD only), then 404.
    pub fn dispatch(&self, req: &Request) -> Response {
        if let Some(handler) = self.routes.get(&(req.method, req.path.clone())) {
            return handler(req);
        }
        if matches!(req.method, Method::Get | Method::Head) {
            if let Some(ss) = &self.static_server {
                if let Some(resp) = ss.serve(&req.path) {
                    return resp;
                }
            }
        }
        Response::not_found()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn get(path: &str) -> Request {
        Request {
            method: Method::Get,
            raw_method: "GET".to_string(),
            path: path.to_string(),
            query: None,
            version: "HTTP/1.1".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    #[test]
    fn dispatches_registered_route() {
        let mut router = Router::new();
        router.route(Method::Get, "/hi", |_req| Response::ok().with_text("hi"));
        let resp = router.dispatch(&get("/hi"));
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"hi");
    }

    #[test]
    fn returns_404_for_unknown_path() {
        let router = Router::new();
        let resp = router.dispatch(&get("/nope"));
        assert_eq!(resp.status, 404);
    }
}
