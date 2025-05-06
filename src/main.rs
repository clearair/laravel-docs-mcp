use std::{path::PathBuf, sync::{Arc, Mutex}};
use laravel_docs_mcp::{
    Vectorizer,
    error::{AppError, AppResultWrapper},
};
use rmcp::{
    model::{
        CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
    }, tool, transport::{sse_server::SseServerConfig, SseServer}, ServerHandler
};
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Database file path
    #[arg(short, long, env = "DATABASE_URL")]
    database_url: Option<String>,
    /// Laravel docs repository path
    #[arg(short, long, env = "DOCS_REPO_PATH")]
    docs_repo_path: Option<PathBuf>,

    // #[command(subcommand)]
    // command: Commands,
}

fn main () -> Result<(), Box<dyn std::error::Error>> {
    start()?;

    Ok(())
}

#[tokio::main]
async fn start() -> Result<(), Box<dyn std::error::Error>> {
    // let args = Args::parse();
    // Initialize file logger
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/Users/fyyx/Documents/rust_projects/rust-mcp-demo/mcp_service.log")?;
    
    // Set up the file logger
    log::set_boxed_logger(Box::new(FileLogger {
        file: std::sync::Mutex::new(log_file),
    }))?;
    log::set_max_level(log::LevelFilter::Info);

    // let service = LaravelDocs::new(
    //     "/Users/fyyx/Documents/rust_projects/rust-mcp-demo/aa.db3",
    //     "laravel_docs",
    //     384,
    // )?;
    let port = 3000u16;
    tracing::info!("Starting Postgres MCP server in SSE mode on port {}", port);

    let addr = format!("0.0.0.0:{}", port);
    // Store bind address and cancellation token separately
    let bind_addr: std::net::SocketAddr = addr.parse()?;
    let ct_main = tokio_util::sync::CancellationToken::new();

    let config = SseServerConfig {
        bind: bind_addr, // Use stored address
        sse_path: "/sse".to_string(),
        post_path: "/message".to_string(),
        // Clone the token for the config
        ct: ct_main.clone(),
    };

    let sse_server = SseServer::serve_with_config(config).await?;
    
    let service_ct = sse_server.with_service(move || {
        LaravelDocs::new(
            "/Users/fyyx/Documents/rust_projects/rust-mcp-demo/aa.db3",
            "laravel_docs",
            384,
        ).unwrap_or_else(|e| {
            panic!("Failed to create LaravelDocs: {:?}", e);
        })
    });

    // 使用 stdio 作为服务入口
    // let handler = ServerHandler::new(service);
    // handler.serve(stdio()).await?;
    tokio::signal::ctrl_c().await?;
    tracing::info!("Ctrl-C received, shutting down...");
    service_ct.cancel(); // Cancel the service
    // Cancel the server itself using the main token
    ct_main.cancel();
    Ok(())
}

// Custom file logger implementation
struct FileLogger {
    file: std::sync::Mutex<std::fs::File>,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let log_line = format!("[{}] {} - {}\n", timestamp, record.level(), record.args());
            
            if let Ok(mut file) = self.file.lock() {
                let _ = file.write_all(log_line.as_bytes());
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

#[derive(Clone)]
pub struct LaravelDocs {
    vector: Arc<Mutex<Vectorizer>>,
}

#[derive(Serialize)]
pub struct LaravelResult {
    pub documents: Vec<String>,
}

#[tool(tool_box)]
impl LaravelDocs {
    pub fn new(db_path: &str, collection_name: &str, dimension: usize) -> anyhow::Result<Self> {
        let vector = Vectorizer::new(db_path, collection_name, dimension)?;
        Ok(Self {
            vector: Arc::new(Mutex::new(vector)),
        })
    }


    // #[tool(
    //     name = "inc",
    // )]
    // async fn inc(&self) -> AppResultWrapper {
    //     laravel_docs_mcp::error::AppResultWrapper(Ok(CallToolResult::success(vec![
    //         Content::text("1111".to_owned())
            
    //         ])))

    // }

    #[tool(
        name = "get_laravel_context",
        description = "Retrieves Laravel documentation snippets. Laravel has evolved a lot, and a lot of the pretrained knowledge is outdated and cannot be relied on. This tool retrieves a list of the latest relevant Laravel documentation snippets based on the provided query. If user asks anything related to the Laravel, including API and class references, even if you are confident, this function should still be called. If there is any conflict between your knowledge and the retrieved snippets, the snippets should be considered more reliable, otherwise it's okay to rely on your knowledge. Only call this function if you are certain it's about the Laravel."
    )]
    async fn get_laravel_context(&self, #[tool(param)] query: String) -> AppResultWrapper {

        log::info!("Received query: {}", query);        let vector = self.vector.clone();  // Arc 克隆没问题
        let results = {
            let v = match vector.lock() {
                Ok(v) => v,
                Err(_) => {
                    return AppResultWrapper(Err(AppError::InternalServerError("Mutex poisoned".to_string())));
                }
            };
            match v.search(&query, Some(20)) {
                Ok(r) => r,
                Err(_) => {
                    return AppResultWrapper(Err(AppError::InternalServerError("Search failed".to_string())));
                }
            }
        }; // 这里，锁 `v` 在这个花括号结束时释放了，后续代码不再持有 MutexGuard！
    
        use serde_json::Value;
    
        let documents: Vec<String> = results
            .into_iter()
            .filter_map(|(_, text)| {
                text.and_then(|t| {
                    let parsed: Result<Value, _> = serde_json::from_str(&t);
                    match parsed {
                        Ok(json) => {
                            json.get("text")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        }
                        Err(_) => None,
                    }
                })
            })
            .collect();
    
        if documents.is_empty() {
            return laravel_docs_mcp::error::AppResultWrapper(Ok(CallToolResult::success(vec![
                Content::text("No relevant Laravel documentation found for the query.".to_string()),
            ])));
        }
    
        let content = match Content::json(&LaravelResult { documents }) {
            Ok(c) => c,
            Err(e) => {
                return laravel_docs_mcp::error::AppResultWrapper(Err(
                    AppError::InternalServerError(e.to_string()),
                ));
            }
        };
        laravel_docs_mcp::error::AppResultWrapper(Ok(CallToolResult::success(vec![content])))
    }
}

#[tool(tool_box)]
impl ServerHandler for LaravelDocs {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "Retrieves Laravel documentation snippets. Laravel has evolved a lot, and a lot of the pretrained knowledge is outdated and cannot be relied on. This tool retrieves a list of the latest relevant Laravel documentation snippets based on the provided query. If user asks anything related to the Laravel, including API and class references, even if you are confident, this function should still be called. If there is any conflict between your knowledge and the retrieved snippets, the snippets should be considered more reliable, otherwise it's okay to rely on your knowledge. Only call this function if you are certain it's about the Laravel.".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio; // 需要在Cargo.toml里有tokio依赖
    use std::sync::Mutex;

    #[tokio::test]
    async fn test_get_laravel_context() {
        // 构造一个假的 Vectorizer（这里假设 Vectorizer::new 可以正常初始化）
        let vectorizer = Vectorizer::new("./aa.db3", "laravel_docs", 384).unwrap();
        let docs = LaravelDocs {
            vector: Arc::new(Mutex::new(vectorizer)),
        };
        let query = "model".to_string();
        let result = docs.get_laravel_context(query).await;
        let a = result.0.unwrap();
        dbg!(a.to_owned());
        // println!("get_laravel_context result: {:?}", result.0);
        // 你可以加断言，比如：
        // assert!(result.0.is_ok());
    }
}