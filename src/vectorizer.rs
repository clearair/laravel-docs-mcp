use anyhow::{Result, anyhow};
use bytemuck::cast_slice;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use rusqlite::{Connection, ffi::sqlite3_auto_extension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

/// Metric type for vector similarity
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Metric {
    Cosine,
    Dot,
    Euclidean,
}

impl Metric {
    fn as_str(&self) -> &'static str {
        match self {
            Metric::Cosine => "cosine",
            Metric::Dot => "dot",
            Metric::Euclidean => "euclidean",
        }
    }
}

/// Parameters for vector collection
pub struct VectorParams {
    dimension: u32,
    metric: Metric,
}

impl VectorParams {
    pub fn new(dimension: u32) -> Self {
        Self {
            dimension,
            metric: Metric::Cosine,
        }
    }

    pub fn with_metric(mut self, metric: Metric) -> Self {
        self.metric = metric;
        self
    }
}

/// Vectorizer for text embedding using sqlite-vec
pub struct SqliteVector {
    conn: Connection,
}

impl SqliteVector {
    /// Creates a new SqliteVector with the specified database path
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        // Register the sqlite-vec extension
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }

        // Open the database connection
        let conn = Connection::open(db_path)?;

        Ok(Self { conn })
    }

    /// Creates a vector collection with the specified name and parameters
    pub fn create_vector_collection(&self, name: &str, params: VectorParams) -> Result<()> {
        let sql = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS {} USING vec0(embedding FLOAT[{}])",
            name, params.dimension
        );

        println!("Executing SQL: {}", sql);
        self.conn.execute(&sql, [])?;
        self.set_metadata(name)?;
        Ok(())
    }

    /// Adds an item to the vector collection
    pub fn add_item(&self, collection: &str, embedding: &[f32]) -> Result<()> {
        let sql = format!("insert into {} (embedding) values (?)", collection);
        let mut stmt = self.conn.prepare(sql.as_str())?;

        let byte_slice = unsafe {
            std::slice::from_raw_parts(
                embedding.as_ptr() as *const u8,
                std::mem::size_of_val(embedding),
            )
        };
        stmt.execute(rusqlite::params![byte_slice])?;
        Ok(())
    }

    /// Adds an item to the vector collection
    pub fn add_mate(&self, collection: &str, id: usize, mate_data: &str) -> Result<()> {
        let meta_table = format!("{}_metadata", collection);
        let sql = format!("insert into {} (id, metadata) values (?, ?)", meta_table);
        let mut stmt = self.conn.prepare(sql.as_str())?;

        stmt.execute(rusqlite::params![id, mate_data])?;
        Ok(())
    }

    /// Sets metadata for an item
    pub fn set_metadata(&self, collection: &str) -> Result<()> {
        // Create metadata table if it doesn't exist
        let meta_table = format!("{}_metadata", collection);
        let create_sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (id INTEGER PRIMARY KEY, metadata BLOB)",
            meta_table
        );

        println!("Executing metadata SQL: {}", create_sql);
        self.conn.execute(&create_sql, [])?;

        Ok(())
    }

    /// Performs a similarity search
    pub fn search(
        &self,
        collection: &str,
        embedding: &[f32],
        limit: u32,
    ) -> Result<Vec<(i64, Option<String>)>> {
        // Convert embedding to JSON string for search
        // let embedding_json = serde_json::to_string(embedding)?;

        let meta_table = format!("{}_metadata", collection);

        // Join with metadata table to get the stored text
        // The correct syntax for searching in a vec0 table uses the MATCH operator with k=? constraint
        let sql = format!(
            "SELECT v.rowid, m.metadata
             FROM {} v
             LEFT JOIN {} m ON v.rowid = m.id
             WHERE v.embedding MATCH ?1 AND k=?2
             ORDER BY distance
             LIMIT ?2",
            collection, meta_table,
        );

        println!("Executing search SQL: {}", sql);
        let mut stmt = self.conn.prepare(&sql)?;
        // let e= embedding;
        let rows = stmt.query_map(params![cast_slice(embedding), limit as i64], |row| {
            let id: i64 = row.get(0)?;
            let metadata: Option<String> = row.get(1)?;
            Ok((id, metadata))
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    pub fn generate_batch_sql(base_sql: &str, items_len: usize, value_format: &str) -> String {
        // 提前设置字符串长度 items_len - 1 这部分是 , 的长度
        let total_length = base_sql.len() + value_format.len() * items_len + items_len - 1;

        let mut result = String::with_capacity(total_length);
        result.push_str(base_sql);

        let values = vec![value_format; items_len];
        result.push_str(&values.join(","));
        result
    }

    pub fn add_items(&mut self, collection: &str, items: Vec<(usize, &[f32])>) -> Result<()> {
        self.batch_insert(
            collection,
            "(embedding)",
            "(?)",
            items.into_iter().map(|(_, embedding)| {
                let byte_slice = unsafe {
                    std::slice::from_raw_parts(
                        embedding.as_ptr() as *const u8,
                        std::mem::size_of_val(embedding),
                    )
                };
                vec![
                    // rusqlite::types::Value::from(id as i64),
                    rusqlite::types::Value::from(byte_slice.to_vec()),
                ]
            }),
        )
    }

    /// 批量插入 metadata
    pub fn add_mates(&mut self, collection: &str, mates: Vec<(usize, &str)>) -> Result<()> {
        self.batch_insert(
            &format!("{}_metadata", collection),
            "(id, metadata)",
            "(?, ?)",
            mates.into_iter().map(|(id, text)| {
                vec![
                    rusqlite::types::Value::from(id as i64),
                    rusqlite::types::Value::from(text.to_string()),
                ]
            }),
        )
    }

    fn batch_insert<I>(
        &mut self,
        collection: &str,
        columns: &str,
        value_format: &str,
        rows: I,
    ) -> Result<()>
    where
        I: Iterator<Item = Vec<rusqlite::types::Value>>,
    {
        let rows: Vec<_> = rows.collect();
        if rows.is_empty() {
            return Ok(());
        }

        let batch_sql = Self::generate_batch_sql(
            &format!("insert into {} {} values ", collection, columns),
            rows.len(),
            value_format,
        );

        let flat_params: Vec<_> = rows.into_iter().flatten().collect();

        let tx = self.conn.transaction()?;
        // dbg!(&batch_sql);
        tx.execute(&batch_sql, rusqlite::params_from_iter(flat_params))?;
        tx.commit()?;
        Ok(())
    }
}

/// Vectorizer for text embedding using sqlite-vec
#[derive(Clone)]
pub struct Vectorizer {
    vector_db: Arc<Mutex<SqliteVector>>,
    pub model_name: String,
    dimension: usize,
    model: Arc<TextEmbedding>,
}
const CHUNK_SIZE: usize = 500;

impl Vectorizer {
    /// Creates a new Vectorizer with the specified database path
    pub fn new<P: AsRef<Path>>(db_path: P, model_name: &str, dimension: usize) -> Result<Self> {
        let model: TextEmbedding = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2)
                .with_cache_dir("~/.fastembed_cache".into())
                .with_show_download_progress(true),
        )?;
        // let model_path = "/Users/fyyx/Documents/rust_projects/rust-mcp-demo/.fastembed_cache/model.onnx";
        // // let options = InitOptions::new(EmbeddingModel::Custom(model_path));
        // let model: TextEmbedding = TextEmbedding::try_new(
        //      InitOptions::new(EmbeddingModel::AllMiniLML6V2)
        // )?;
        // Create or open the vector database
        let vector_db = SqliteVector::new(db_path)
            .map_err(|e| anyhow!("Failed to create/open vector database: {}", e))?;

        // Self::clean(model_name, &vector_db)?;
        // Create the collection if it doesn't exist
        Ok(Self {
            vector_db: Arc::new(Mutex::new(vector_db)),
            model_name: model_name.to_string(),
            dimension,
            model: Arc::new(model),
        })
    }

    pub fn create_table(&self) -> Result<()> {
        let params = VectorParams::new(self.dimension as u32).with_metric(Metric::Cosine);

        let v = self
            .vector_db
            .lock()
            .map_err(|_| anyhow!("Mutex poisoned"))?;
        v.create_vector_collection(&self.model_name, params)
            .map_err(|e| anyhow!("Failed to create vector collection: {}", e))?;
        Ok(())
    }

    /// Stores a text embedding in the database
    pub fn store_embedding(&self, id: usize, text: &str, embedding: &[f32]) -> Result<()> {
        // Store the embedding in the database
        let vd = self
            .vector_db
            .lock()
            .map_err(|_| anyhow!("Mutex poisoned"))?;

        vd.add_item(&self.model_name, embedding)
            .map_err(|e| anyhow!("Failed to add embedding: {}", e))?;

        // Store the text as metadata
        vd.add_mate(&self.model_name, id, text)
            .map_err(|e| anyhow!("Failed to set metadata: {}", e))?;

        Ok(())
    }

    pub fn store_docs(&mut self, texts: Vec<&str>) -> Result<()> {
        for (index, chunk) in texts.chunks(CHUNK_SIZE).enumerate() {
            let embeddings = self.embeds(chunk.to_vec())?;
            let mut items = Vec::new();
            let mut mates = Vec::new();
            for ((id, text), embedding) in chunk.iter().enumerate().zip(embeddings.iter()) {
                // id 需要全局唯一，这里加上 chunk 的偏移量
                let global_id = id + (index * CHUNK_SIZE) + 1;
                items.push((global_id, embedding.as_slice()));
                mates.push((global_id, *text));
            }
            let mut vd = self
                .vector_db
                .lock()
                .map_err(|_| anyhow!("Mutex poisoned"))?;
            vd.add_items(&self.model_name, items)?;
            vd.add_mates(&self.model_name, mates)?;
        }
        Ok(())
    }

    pub fn embeds(&self, texts: Vec<&str>) -> Result<Vec<Vec<f32>>> {
        self.model.embed(texts, None)
    }

    /// Performs a similarity search
    pub fn search(&self, text: &str, limit: Option<usize>) -> Result<Vec<(i64, Option<String>)>> {
        // Search for similar embeddings

        let limit = match limit {
            Some(l) => l as u32,
            None => 20u32,
        };
        // dbg!(limit);
        let binding = self.embeds(vec![text])?;
        let embedding = binding
            .first()
            .ok_or_else(|| anyhow!("Failed to generate embedding for the text"))?;
        let vd = self
            .vector_db
            .lock()
            .map_err(|_| anyhow!("Mutex poisoned"))?;

        let results = vd
            .search(&self.model_name, embedding, limit)
            .map_err(|e| anyhow!("Failed to search: {}", e))?;

        Ok(results)
    }

    pub fn clean(&self) -> Result<()> {
        let main_table = &self.model_name;
        let meta_table = format!("{}_metadata", main_table);
        let sql_main = format!("DROP TABLE IF EXISTS {}", main_table);
        let sql_meta = format!("DROP TABLE IF EXISTS {}", meta_table);
        let vd = self
            .vector_db
            .lock()
            .map_err(|_| anyhow!("Mutex poisoned"))?;

        vd.conn.execute(sql_main.as_str(), [])?;
        vd.conn.execute(sql_meta.as_str(), [])?;
        Ok(())
    }

    /// Generates a simple mock embedding for demonstration purposes
    /// In a real implementation, this would use a proper embedding model
    pub fn mock_embed(&self, text: &str) -> Vec<f32> {
        // This is just a mock implementation for demonstration
        // In a real system, you would use a proper embedding model

        // Create a deterministic but simple embedding based on character values
        let mut embedding = vec![0.0; self.dimension];

        for (i, c) in text.chars().enumerate() {
            let pos = i % self.dimension;
            embedding[pos] += (c as u32 % 256) as f32 / 128.0 - 1.0;
        }

        // Normalize the embedding
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for value in &mut embedding {
                *value /= magnitude;
            }
        }

        embedding
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::{self, BufRead};

    use super::*;
    #[test]
    fn test_search_docs() {
        // let documents = vec![
        //     "passage: Hello, World!",
        //     "query: Hello, World!",
        //     "passage: This is an example passage.",
        //     // You can leave out the prefix but it's recommended
        //     "fastembed-rs is licensed under Apache  2.0",
        // ];

        let file = File::open("/Users/fyyx/Documents/rust_projects/rust-mcp-demo/artifacts/chunks/docs_chunks_SZ_400_O_20.jsonl").unwrap();
        let reader = io::BufReader::new(file);
        let documents: Vec<String> = reader.lines().collect::<Result<_, _>>().unwrap();
        let documents = documents.iter().map(|i| i.as_str()).collect::<Vec<&str>>();
        let mut vector = Vectorizer::new("./aa.db3", "laravel_livewire_docs", 384).unwrap();
        vector.clean().unwrap();
        vector.create_table().unwrap();
        vector.store_docs(documents.clone()).unwrap();
        let result: Vec<(i64, Option<String>)> =
            vector.search(documents.first().unwrap(), None).unwrap();

        dbg!(result);
        // assert_eq!(
        //     result
        //         .iter()
        //         .map(|(_, doc)| doc.as_deref().unwrap_or(""))
        //         .collect::<Vec<&str>>(),
        //     documents
        // );
    }
}
