use std::{collections::HashMap, path::{Path, PathBuf}, sync::{Arc, Mutex}, vec};
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use laravel_docs_mcp::{
    Vectorizer,
    error::{AppError, AppResultWrapper},
};
use rmcp::{
    model::{
        CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
    }, tool, transport::{sse_server::SseServerConfig, stdio, SseServer}, ServerHandler, ServiceExt
};
use serde::Serialize;
use tokio::sync::RwLock;
use std::fs::OpenOptions;
use std::io::Write;
use clap::{Parser, Subcommand};
use std::sync::LazyLock;

pub(crate) static MODEL: LazyLock<Arc<TextEmbedding>> = LazyLock::new(|| {
    let arg = Args::parse();
    let model_path = arg.model_path.unwrap_or_else(|| format!("{}/.fastembed_cache", std::env::var("HOME").unwrap()).into());
    // let model_path = "/Users/fyyx/Documents/rust_projects/rust-mcp-demo/~/.fastembed_cache";
    Arc::new(TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::AllMiniLML6V2)
            .with_cache_dir(model_path.into())
            .with_show_download_progress(true),
    ).unwrap())
});

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None, subcommand_required(false), subcommand = "stdio")]
struct Args {
    /// Database file path
    #[arg(short, long, env = "DATABASE_URL")]
    database_url: Option<String>,
    /// Laravel docs repository path
    #[arg(short = 'r', long, env = "DOCS_REPO_PATH")]
    docs_repo_path: Option<PathBuf>,

    #[arg(short, long, env = "MODEL_PATH")]
    model_path: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone, Copy)]
enum Commands {
    /// Run in stdio mode
    Stdio,
    /// Run in SSE mode
    Sse {
        /// Port for the SSE server to bind to
        #[arg(short, long, default_value_t = 3000)]
        port: u16,
    },
}

#[tokio::main]
async fn main () -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let database_url = args.database_url.clone().unwrap_or(format!("{}/.laravel_docs.db3", std::env::var("HOME").unwrap()));
    // let database_url = "/Users/fyyx/Documents/rust_projects/rust-mcp-demo/aa.db3".to_owned();
    if let Some(command) = args.command {
        match command {
            Commands::Stdio => start_stdio(&database_url).await?,
            Commands::Sse { port } => start_sse(&database_url, port).await?,
        }
    } else {
        start_stdio(&database_url).await?;
    }
    // start_sse(&database_url, 3000).await?;

    Ok(())
}

async fn start_stdio(database_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let service = LaravelDocs::new(database_url);
    service.serve(stdio()).await?.waiting().await?;
    
    Ok(())
}

async fn start_sse(database_url: &str, port: u16) -> Result<(), Box<dyn std::error::Error>> {

    let mut data_path: PathBuf = database_url.into();
    let log_path = format!("{}/mcp_service.log", {
        data_path.pop();
        data_path.to_str().unwrap()
    });
    println!("log_path: {}", log_path);
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    
    let _ = MODEL.clone();
    println!("model load");

    // Set up the file logger
    log::set_boxed_logger(Box::new(FileLogger {
        file: std::sync::Mutex::new(log_file),
    }))?;
    log::set_max_level(log::LevelFilter::Debug);

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
    let db_path_owned = database_url.to_string();

    let service_ct = sse_server.with_service(move || LaravelDocs::new(&db_path_owned));


    tokio::signal::ctrl_c().await?;
    tracing::info!("Ctrl-C received, shutting down...");
    service_ct.cancel();
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
                let _ = file.flush();
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

// macro_rules! define_docs_tool {
//     ($fn_name:ident, $tool_name:literal, $desc:literal, $collection:literal, $ResultType:ident) => {
//         #[tool(name = $tool_name, description = $desc)]
//         async fn $fn_name(&self, #[tool(param)] query: String) -> AppResultWrapper {
//             log::info!("Received query: {}", query);

//             let vector = match self.get_vectorizer($collection).await {
//                 Ok(v) => v,
//                 Err(e) => return AppResultWrapper(Err(e.into())),
//             };
//             let results = match vector.search(&query, Some(20)) {
//                 Ok(r) => r,
//                 Err(e) => return AppResultWrapper(Err(e.into())),
//             };
//             let documents: Vec<String> = parse_docs(results);
//             if documents.is_empty() {
//                 return AppResultWrapper(Ok(CallToolResult::success(vec![
//                     Content::text(format!("No relevant {} documentation found for the query.", $collection)),
//                 ])));

//             }

//             let content = match Content::json(&$ResultType { documents }) {
//                 Ok(c) => c,
//                 Err(e) => return AppResultWrapper(Err(AppError::InternalServerError(e.to_string()))),
//             };
//             AppResultWrapper(Ok(CallToolResult::success(vec![content])))
//         }
//     };
// }


#[derive(Clone)]
pub struct LaravelDocs {
    db_path: String,
    vectorizers: Arc<RwLock<HashMap<String, Arc<Vectorizer>>>>,
}

#[derive(Serialize)]
pub struct LaravelResult {
    pub documents: Vec<String>,
}

#[tool(tool_box)]
impl LaravelDocs {
    pub fn new(db_path: &str) -> Self {
        Self { db_path: db_path.to_string(), vectorizers: Arc::new(RwLock::new(HashMap::new())) }
    }

    async fn get_vectorizer(&self, collection: &str) -> anyhow::Result<Arc<Vectorizer>> {    
        {
            let vectorizers = self.vectorizers.read().await;
            if let Some(v) = vectorizers.get(collection) {
                return Ok(v.clone());
            }
        }
        
        //  TODO 这里还有并发漏洞之后处理
        let v = match Vectorizer::new(&self.db_path, collection, 384, MODEL.clone()) {
            Ok(v) => Arc::new(v),
            Err(e) => {
                println!("{:?}", e);
                return Err(e);
            },
        };
    
        let mut vectorizers = self.vectorizers.write().await;
        let entry = vectorizers.entry(collection.to_string()).or_insert_with(|| v.clone());
    
        Ok(entry.clone())
    }

    #[tool(name = "get_laravel_context", description = "有关laravel框架的问题 都先调用 get_laravel_context 这里的文档是最新的")]
    async fn get_laravel_context(&self, #[tool(param)] query: String) -> AppResultWrapper {
        log::info!("Received query: {}", query);
        let vector = match self.get_vectorizer("laravel_docs").await {
            Ok(v) => v,
            Err(e) => {
                println!("{:?}", e);
                return AppResultWrapper(Err(e.into()))
            },
        };
        let results = match vector.search(&query, Some(20)) {
            Ok(r) => r,
            Err(e) => {
                println!("{:?}", e);
                return AppResultWrapper(Err(e.into()))
            },
        };
        let docs = parse_docs(results);
        if docs.is_empty() {
            return AppResultWrapper(Ok(CallToolResult::success(vec![
                Content::text(format!("No relevant {} documentation found for the query.", "laravel_docs")),
            ])));
        }
        let content = match Content::json(&LaravelResult { documents: docs }) {
            Ok(c) => c,
            Err(e) => return AppResultWrapper(Err(AppError::InternalServerError(e.to_string()))),
        };
        AppResultWrapper(Ok(CallToolResult::success(vec![content])))
    }

    #[tool(name = "get_laravel_livewire_context", description = "有关laravel livewire 框架的问题 都先调用 get_laravel_livewire_context 这里的文档是最新的")]
    async fn get_laravel_livewire_context(&self, #[tool(param)] query: String) -> AppResultWrapper {
        log::info!("Received query: {}", query);
        let vector = match self.get_vectorizer("laravel_livewire_docs").await {
            Ok(v) => v,
            Err(e) => return AppResultWrapper(Err(e.into())),
        };
        let results = match vector.search(&query, Some(20)) {
            Ok(r) => r,
            Err(e) => return AppResultWrapper(Err(e.into())),
        };
        let docs = parse_docs(results);
        if docs.is_empty() {
            return AppResultWrapper(Ok(CallToolResult::success(vec![
                Content::text(format!("No relevant {} documentation found for the query.", "laravel_livewire_docs")),
            ])));
        }
        let content = match Content::json(&LaravelResult { documents: docs }) {
            Ok(c) => c,
            Err(e) => return AppResultWrapper(Err(AppError::InternalServerError(e.to_string()))),
        };
        AppResultWrapper(Ok(CallToolResult::success(vec![content])))
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
                "This tool must be called whenever user mentions Laravel, including models, controllers, attributes, routes, blade templates, migrations, API references, class references, or any other Laravel-related term. Always prefer to call this function first before answering.".to_string()),
        }
    }
}

fn parse_docs(results: Vec<(i64, Option<String>)>) -> Vec<String> {
    results.into_iter().filter_map(|(_, text)| {
        text.and_then(|t| {
            serde_json::from_str::<serde_json::Value>(&t).ok()
                .and_then(|json| json.get("text")?.as_str().map(|s| s.to_string()))
        })
    }).collect()
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio;

    #[tokio::test]
    async fn test_get_laravel_context() {
        let _ = MODEL.clone();           // force the LazyLock init

        // Construct a real Vectorizer and inject into service
        let vectorizer = Vectorizer::new("./test.db3", "test_docs", 384, MODEL.clone()).unwrap();
        let docs = LaravelDocs::new("./test.db3");
        docs.vectorizers.write().await.insert("test_docs".to_string(), Arc::new(vectorizer));
        let query = "model".to_string();
        let result = docs.get_laravel_context(query).await;
        // Assert the call result is OK and has output
        assert!(result.0.is_ok());
        let call_result = result.0.unwrap();
        dbg!(call_result.to_owned());
        // assert!(!call_result.outputs.is_empty());
    }
}