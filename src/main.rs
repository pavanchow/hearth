use clap::{Parser, Subcommand};
use hearth::{build_router, Server};

#[derive(Parser)]
#[command(name = "hearth", about = "A from-scratch HTTP/1.1 server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Serve a directory over HTTP
    Serve {
        /// Directory to serve as static files
        #[arg(long, default_value = "./public")]
        dir: String,
        /// Port to listen on
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Address to bind
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Serve { dir, port, bind } => {
            let router = build_router(&dir);
            let server = Server::new(router);
            let addr = format!("{bind}:{port}");
            println!("hearth serving {dir} on http://{addr}");
            if let Err(e) = server.listen(&addr) {
                eprintln!("failed to start server: {e}");
                std::process::exit(1);
            }
        }
    }
}
