use crate::text_splitter::RecursiveCharacterTextSplitter;
use anyhow::{Context, Result};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Represents a single text chunk with metadata
#[derive(Debug, Serialize, Deserialize)]
pub struct TextChunk {
    /// Unique identifier for the chunk
    pub id: String,
    /// The actual text content of the chunk
    pub text: String,
    /// Source file path where this chunk originated
    pub source: String,
}

/// Process markdown files into chunks and save as JSONL
pub struct TextChunker {
    /// Directory containing markdown files to process
    input_dir: PathBuf,
    /// Path to the output JSONL file
    output_file: PathBuf,
    /// Maximum size of chunks in characters
    chunk_size: usize,
    /// Overlap between chunks in characters
    chunk_overlap: usize,
    /// Text splitter instance
    splitter: RecursiveCharacterTextSplitter,
}

impl TextChunker {
    /// Initialize with input/output paths and chunking parameters
    pub fn new(input_dir: impl AsRef<Path>, chunk_size: usize, chunk_overlap: usize) -> Self {
        let input_dir = input_dir.as_ref().to_path_buf();

        // Get the base domain name from directory
        let base_domain = input_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // Ensure consistent domain naming even if passed through different pipeline stages
        let base_domain = base_domain.replace(".", "-");

        let output_file = PathBuf::from(format!(
            "artifacts/chunks/{}_chunks_SZ_{}_O_{}.jsonl",
            base_domain, chunk_size, chunk_overlap
        ));

        // Initialize the RecursiveCharacterTextSplitter with markdown-specific settings
        let splitter = RecursiveCharacterTextSplitter::new()
            .with_chunk_size(chunk_size)
            .with_chunk_overlap(chunk_overlap);

        Self {
            input_dir,
            output_file,
            chunk_size,
            chunk_overlap,
            splitter,
        }
    }

    /// Generate a unique ID based on file path
    fn generate_uid(&self, file_path: &Path) -> String {
        let path_str = file_path.to_string_lossy();
        let mut hasher = Md5::new();
        hasher.update(path_str.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Process a single markdown file into chunks
    pub fn process_file(&self, file_path: &Path) -> Result<Vec<TextChunk>> {
        // Read the file content
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

        // Generate a unique ID based on file path
        let uid = self.generate_uid(file_path);

        // Split content into chunks using RecursiveCharacterTextSplitter
        let chunks = self.splitter.split_text(&content);

        // Create TextChunk objects for each chunk
        let mut result = Vec::new();
        for (i, chunk) in chunks.into_iter().enumerate() {
            // Skip empty chunks
            if chunk.trim().is_empty() {
                continue;
            }

            let chunk_data = TextChunk {
                id: format!("{}-{}", uid, i),
                text: chunk,
                source: file_path.to_string_lossy().to_string(),
            };
            result.push(chunk_data);
        }

        println!("Processed {}: {} chunks", file_path.display(), result.len());
        Ok(result)
    }

    /// Process all markdown files in the input directory
    pub fn process_directory(&self) -> Result<Vec<TextChunk>> {
        let mut all_chunks = Vec::new();
        let mut file_count = 0;

        // Walk through the input directory
        for entry in WalkDir::new(&self.input_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "md") {
                match self.process_file(path) {
                    Ok(chunks) => {
                        all_chunks.extend(chunks);
                        file_count += 1;
                    }
                    Err(e) => {
                        eprintln!("Error processing {}: {}", path.display(), e);
                    }
                }
            }
        }

        println!(
            "\nProcessed {} files with a total of {} chunks.",
            file_count,
            all_chunks.len()
        );
        Ok(all_chunks)
    }

    /// Save chunks to JSONL file
    pub fn save_jsonl(&self, chunks: &[TextChunk]) -> Result<()> {
        // Create directory for output file if needed
        if let Some(parent) = self.output_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Delete the output file if it exists
        if self.output_file.exists() {
            fs::remove_file(&self.output_file).with_context(|| {
                format!(
                    "Failed to remove existing file: {}",
                    self.output_file.display()
                )
            })?;
        }

        // Write chunks to JSONL file
        let file = File::create(&self.output_file)
            .with_context(|| format!("Failed to create file: {}", self.output_file.display()))?;
        let mut writer = BufWriter::new(file);

        for chunk in chunks {
            let json = serde_json::to_string(chunk).context("Failed to serialize chunk to JSON")?;
            writeln!(writer, "{}", json).context("Failed to write to JSONL file")?;
        }

        println!(
            "Saved {} chunks to {}",
            chunks.len(),
            self.output_file.display()
        );
        Ok(())
    }

    /// Run the full chunking process
    pub fn run(&self) -> Result<()> {
        let chunks = self.process_directory()?;
        self.save_jsonl(&chunks)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save() {
        let tc = TextChunker::new("/Users/fyyx/Documents/laravel-comments-documentation", 400, 20);
        assert!(tc.run().is_ok());
    }
}
