//! Stdio MCP server entry point for `strata-mcp`.

mod devices;
mod handlers;
mod params;
mod perfetto_pc;
mod server;
mod transport;

use rmcp::ServiceExt;

#[tokio::main]
async fn main() {
    let srv = server::StrataMcpServer::new();
    match srv.serve(rmcp::transport::io::stdio()).await {
        Ok(running) => {
            if let Err(err) = running.waiting().await {
                eprintln!("[strata-mcp] server wait error: {err}");
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("[strata-mcp] failed to start MCP stdio server: {err}");
            std::process::exit(1);
        }
    }
}
