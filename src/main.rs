use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::Context;
use document_templating_mcp::{server::DocumentMcpServer, store::Store};
use rmcp::{transport::stdio, ServiceExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "document_templating_mcp=info".into()),
        )
        .init();

    let data_dir = env::var_os("DOCUMENT_TEMPLATING_MCP_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"));
    let store = Store::open(&data_dir).map_err(anyhow::Error::new)?;
    tracing::info!(path = %data_dir.display(), "document store ready");

    let transport =
        env::var("DOCUMENT_TEMPLATING_MCP_TRANSPORT").unwrap_or_else(|_| "stdio".to_string());
    match transport.as_str() {
        "http" => {
            let token = env::var("DOCUMENT_TEMPLATING_MCP_TOKEN")
                .context("DOCUMENT_TEMPLATING_MCP_TOKEN is required in HTTP mode")?;
            let address = env::var("DOCUMENT_TEMPLATING_MCP_HTTP_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
                .parse::<SocketAddr>()
                .context("invalid DOCUMENT_TEMPLATING_MCP_HTTP_ADDR")?;
            return document_templating_mcp::web::serve(store, address, token).await;
        }
        "stdio" => {}
        _ => anyhow::bail!("DOCUMENT_TEMPLATING_MCP_TRANSPORT must be 'stdio' or 'http'"),
    }

    let service = DocumentMcpServer::new(store)
        .serve(stdio())
        .await
        .context("failed to start MCP stdio server")?;
    service.waiting().await.context("MCP server failed")?;
    Ok(())
}
