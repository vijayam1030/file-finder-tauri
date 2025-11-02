use nucleo_matcher::{Matcher, Config, Utf32Str};
use crate::fzf_search::FileIndex;

/// Simple and effective fuzzy search using nucleo
pub struct SimpleSearchEngine {
    pub files: Vec<FileIndex>,
    matcher: Matcher,
}

impl SimpleSearchEngine {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// Search files using nucleo's advanced fuzzy matching
    pub fn search(&mut self, query: &str, limit: usize) -> Vec<(i64, FileIndex)> {
        if query.is_empty() {
            return Vec::new();
        }

        let start_time = std::time::Instant::now();
        let mut results = Vec::with_capacity(limit * 2);

        // For each file, create a searchable text combining name and path
        for file in &self.files {
            // Create a combined search string: "filename path/to/file"
            let search_text = format!("{} {}", file.name, file.path);
            
            // Use nucleo to compute fuzzy match score - it works with string slices directly
            let mut search_buf = Vec::new();
            let mut query_buf = Vec::new();
            
            let search_str = Utf32Str::new(&search_text, &mut search_buf);
            let query_str = Utf32Str::new(query, &mut query_buf);
            
            if let Some(score) = self.matcher.fuzzy_match(search_str, query_str) {
                results.push((score as i64, file.clone()));
            }
        }

        // Sort by score (higher is better) and take top results
        results.sort_by(|a, b| b.0.cmp(&a.0));
        results.truncate(limit);

        let duration = start_time.elapsed();
        println!("Simple Search: Found {} results for '{}' in {}μs", 
                 results.len(), query, duration.as_micros());

        // Debug output for first few results
        for (i, (score, file)) in results.iter().enumerate().take(5) {
            println!("  Result {}: score={} name='{}' path='{}'", i, score, file.name, file.path);
        }

        results
    }

    /// Multi-word search that handles "desktop comp" -> finds files with both terms
    pub fn multi_word_search(&mut self, query: &str, limit: usize) -> Vec<(i64, FileIndex)> {
        if query.is_empty() {
            return Vec::new();
        }

        let words: Vec<&str> = query.split_whitespace().collect();
        if words.len() <= 1 {
            return self.search(query, limit);
        }

        let start_time = std::time::Instant::now();
        let mut results = Vec::with_capacity(limit * 2);

        for file in &self.files {
            let search_text = format!("{} {}", file.name, file.path);
            
            let mut total_score = 0i64;
            let mut matched_words = 0;

            // Check each word independently
            for word in &words {
                let mut search_buf = Vec::new();
                let mut word_buf = Vec::new();
                
                let search_str = Utf32Str::new(&search_text, &mut search_buf);
                let word_str = Utf32Str::new(word, &mut word_buf);
                
                if let Some(score) = self.matcher.fuzzy_match(search_str, word_str) {
                    total_score += score as i64;
                    matched_words += 1;
                }
            }

            // Require at least most words to match (flexible for fuzzy matching)
            let required_matches = if words.len() <= 2 { words.len() } else { words.len() - 1 };
            if matched_words >= required_matches {
                // Boost score based on number of matched words
                let final_score = total_score * (matched_words as i64);
                results.push((final_score, file.clone()));
            }
        }

        // Sort by score and take top results  
        results.sort_by(|a, b| b.0.cmp(&a.0));
        results.truncate(limit);

        let duration = start_time.elapsed();
        println!("Multi-word Search: Found {} results for '{}' in {}μs", 
                 results.len(), query, duration.as_micros());

        // Debug output
        for (i, (score, file)) in results.iter().enumerate().take(5) {
            println!("  Result {}: score={} name='{}' path='{}'", i, score, file.name, file.path);
        }

        results
    }
}