/// A recursive character text splitter similar to Python's LangChain RecursiveCharacterTextSplitter
pub struct RecursiveCharacterTextSplitter {
    /// List of separators to use for splitting, in order of priority
    separators: Vec<String>,
    /// Maximum size of chunks in characters
    chunk_size: usize,
    /// Overlap between chunks in characters
    chunk_overlap: usize,
    /// Keep separator with the chunk
    keep_separator: bool,
}

impl RecursiveCharacterTextSplitter {
    /// Create a new RecursiveCharacterTextSplitter with default settings
    pub fn new() -> Self {
        Self {
            separators: vec![
                "\n\n\n".to_string(), // Triple newline
                "\n\n".to_string(),   // Double newline
                "\n".to_string(),     // Single newline
                ". ".to_string(),     // Period followed by space
                "! ".to_string(),     // Exclamation mark followed by space
                "? ".to_string(),     // Question mark followed by space
                ", ".to_string(),     // Comma followed by space
                " ".to_string(),      // Space
                "".to_string(),       // Character by character as last resort
            ],
            chunk_size: 400,
            chunk_overlap: 20,
            keep_separator: true,
        }
    }

    /// Set custom separators
    pub fn with_separators(mut self, separators: Vec<String>) -> Self {
        self.separators = separators;
        self
    }

    /// Set chunk size
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    /// Set chunk overlap
    pub fn with_chunk_overlap(mut self, chunk_overlap: usize) -> Self {
        self.chunk_overlap = chunk_overlap;
        self
    }

    /// Set whether to keep separator with the chunk
    pub fn with_keep_separator(mut self, keep_separator: bool) -> Self {
        self.keep_separator = keep_separator;
        self
    }

    /// Split text into chunks recursively
    pub fn split_text(&self, text: &str) -> Vec<String> {
        // If text is small enough, return it as a single chunk
        if text.len() <= self.chunk_size {
            return vec![text.to_string()];
        }

        self.split_text_with_separators(text, &self.separators)
    }

    /// Split text using the provided separators recursively
    fn split_text_with_separators(&self, text: &str, separators: &[String]) -> Vec<String> {
        // If we're at the last separator (empty string) or text is small enough, return it as a single chunk
        if separators.is_empty() || text.len() <= self.chunk_size {
            return vec![text.to_string()];
        }

        let separator = &separators[0];
        let remaining_separators = &separators[1..];

        // If separator is empty, we'll split by character
        if separator.is_empty() {
            return self.split_by_character(text);
        }

        // Split text by current separator
        let splits: Vec<&str> = text.split(separator).collect();

        // If this separator doesn't split the text or results in only one chunk,
        // try with the next separator
        if splits.len() <= 1 {
            return self.split_text_with_separators(text, remaining_separators);
        }

        // Merge splits into chunks that respect chunk_size
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();

        for (i, split) in splits.iter().enumerate() {
            // Add separator back except for the first split
            let split_with_separator = if i > 0 && self.keep_separator {
                format!("{}{}", separator, split)
            } else {
                split.to_string()
            };

            // If adding this split would exceed chunk_size, finalize current chunk and start a new one
            if !current_chunk.is_empty()
                && current_chunk.len() + split_with_separator.len() > self.chunk_size
            {
                chunks.push(current_chunk.clone());

                // Start new chunk with overlap from previous chunk if possible
                if self.chunk_overlap > 0 && !chunks.is_empty() {
                    let last_chunk = chunks.last().unwrap();
                    let overlap_chars = self.chunk_overlap;
                    let char_indices: Vec<_> = last_chunk.char_indices().collect();
                    let overlap_start_char = char_indices.len().saturating_sub(overlap_chars);
                    let overlap_start_byte = if overlap_start_char < char_indices.len() {
                        char_indices[overlap_start_char].0
                    } else {
                        0
                    };
                    current_chunk = last_chunk[overlap_start_byte..].to_string();
                } else {
                    current_chunk = String::new();
                }
            }

            // Add current split to the chunk
            current_chunk.push_str(&split_with_separator);
        }

        // Add the last chunk if it's not empty
        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }

        // If we successfully created chunks that respect the size limit, return them
        if !chunks.is_empty() && chunks.iter().all(|chunk| chunk.len() <= self.chunk_size) {
            return chunks;
        }

        // If chunks are still too large, recursively split them with remaining separators
        let mut final_chunks = Vec::new();
        for chunk in chunks {
            if chunk.len() <= self.chunk_size {
                final_chunks.push(chunk);
            } else {
                let sub_chunks = self.split_text_with_separators(&chunk, remaining_separators);
                final_chunks.extend(sub_chunks);
            }
        }

        // If we still couldn't create appropriate chunks, try with the next separator
        if final_chunks.is_empty()
            || final_chunks
                .iter()
                .any(|chunk| chunk.len() > self.chunk_size)
        {
            return self.split_text_with_separators(text, remaining_separators);
        }

        final_chunks
    }

    /// Split text by character as a last resort
    fn split_by_character(&self, text: &str) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();

        for c in text.chars() {
            if current_chunk.len() >= self.chunk_size {
                chunks.push(current_chunk);

                // Start new chunk with overlap from previous chunk if possible
                if self.chunk_overlap > 0 && !chunks.is_empty() {
                    let last_chunk = chunks.last().unwrap();
                    let overlap_chars = self.chunk_overlap;
                    let char_indices: Vec<_> = last_chunk.char_indices().collect();
                    let overlap_start_char = char_indices.len().saturating_sub(overlap_chars);
                    let overlap_start_byte = if overlap_start_char < char_indices.len() {
                        char_indices[overlap_start_char].0
                    } else {
                        0
                    };
                    current_chunk = last_chunk[overlap_start_byte..].to_string();
                } else {
                    current_chunk = String::new();
                }
            }
            current_chunk.push(c);
        }

        // Add the last chunk if it's not empty
        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }

        chunks
    }
}

impl Default for RecursiveCharacterTextSplitter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_small_text() {
        let splitter = RecursiveCharacterTextSplitter::new()
            .with_chunk_size(100)
            .with_chunk_overlap(0);

        let text = "This is a small text that should not be split.";
        let chunks = splitter.split_text(text);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_split_by_newlines() {
        let splitter = RecursiveCharacterTextSplitter::new()
            .with_chunk_size(50)
            .with_chunk_overlap(0);

        let text = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph that is longer than the others.";
        let chunks = splitter.split_text(text);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "First paragraph.");
        assert_eq!(chunks[1], "\n\nSecond paragraph.");
        assert_eq!(
            chunks[2],
            "\n\nThird paragraph that is longer than the others."
        );
    }

    #[test]
    fn test_split_with_overlap() {
        let splitter = RecursiveCharacterTextSplitter::new()
            .with_chunk_size(20)
            .with_chunk_overlap(5);

        let text = "This is a text that should be split into multiple chunks with overlap.";
        let chunks = splitter.split_text(text);

        // Check that each chunk respects the size limit
        for chunk in &chunks {
            assert!(chunk.len() <= 20);
        }

        // Check that there's overlap between consecutive chunks
        for i in 1..chunks.len() {
            let prev_chunk = &chunks[i - 1];
            let curr_chunk = &chunks[i];

            if prev_chunk.len() >= 5 {
                let overlap_text = &prev_chunk[prev_chunk.len() - 5..];
                assert!(curr_chunk.starts_with(overlap_text));
            }
        }
    }
}
