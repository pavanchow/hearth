pub mod request;
pub mod response;
pub mod router;
pub mod server;
pub mod static_files;

pub use request::{Method, ParseError, Request};
pub use response::Response;
pub use router::Router;
pub use server::Server;

/// Build the router Hearth's CLI serves: a static file handler over `dir`
/// plus the two built-in routes, health and echo.
pub fn build_router(dir: &str) -> Router {
    let mut router = Router::new();
    router.serve_dir(dir);

    router.route(Method::Get, "/health", |_req| {
        Response::ok().with_json("{\"status\":\"ok\"}")
    });

    router.route(Method::Post, "/echo", |req| {
        Response::ok()
            .with_header(
                "Content-Type",
                req.header("content-type").unwrap_or("application/octet-stream"),
            )
            .with_body(req.body.clone())
    });

    router
}
