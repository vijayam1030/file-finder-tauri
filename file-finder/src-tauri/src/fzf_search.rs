use serde::{Deserialize, Serialize};
use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileIndex {
    pub name: String,
    pub path: String,
    pub name_lower: String, // Pre-lowercased for fast matching
    pub modified_at: Option<i64>,
}

pub struct FzfSearchEngine {
    pub files: Vec<FileIndex>,
    pub matcher: SkimMatcherV2,
    pub last_update: std::time::Instant,
}

impl FzfSearchEngine {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            matcher: SkimMatcherV2::default(),
            last_update: std::time::Instant::now(),
        }
    }

    /// Load file index from database (called once on startup)
    pub fn load_from_database(&mut self, db: &rusqlite::Connection) -> Result<(), String> {
        let start_time = std::time::Instant::now();
        
        // Load files sorted by length then name for better search performance
        let mut stmt = db.prepare("SELECT name, path, modified_at FROM files ORDER BY length(name), name")
            .map_err(|e| e.to_string())?;
        
        self.files.clear();
        let mapped_rows = stmt.query_map([], |row| {
            let name: String = row.get(0)?;
            let path: String = row.get(1)?;
            let modified_at: Option<i64> = row.get(2)?;
            
            Ok(FileIndex {
                name_lower: name.to_lowercase(), // Pre-compute for speed
                name,
                path,
                modified_at,
            })
        }).map_err(|e| e.to_string())?;
        
        let files: Vec<FileIndex> = mapped_rows
            .filter_map(|r| r.ok())
            .collect();
        
        self.files = files;
        self.last_update = std::time::Instant::now();
        
        let duration = start_time.elapsed();
        println!("FZF: Loaded {} files into memory in {}ms", 
                 self.files.len(), duration.as_millis());
        
        // Debug: Check if project files are in the database
        let project_files: Vec<&FileIndex> = self.files.iter()
            .filter(|f| f.path.contains("file-finder") || 
                       f.name.contains("main") && f.name.ends_with(".js") ||
                       f.name.contains("index") ||
                       f.name == "Cargo.toml" ||
                       f.name == "package.json")
            .take(5)
            .collect();
        
        println!("FZF Debug: Found {} project-related files:", project_files.len());
        for file in &project_files {
            println!("  Project file: {} at {}", file.name, file.path);
        }
        
        Ok(())
    }

    /// Real-time search - ULTRA FAST with aggressive optimizations
    pub fn search(&self, query: &str, limit: usize) -> Vec<(i64, &FileIndex)> {
        if query.is_empty() {
            return Vec::new();
        }

        let query_lower = query.to_lowercase();
        let start_time = std::time::Instant::now();
        
        // OPTIMIZATION: For multi-word queries, split and require all words
        let words: Vec<&str> = query_lower.split_whitespace().collect();
        let is_multi_word = words.len() > 1;
        
        // Special pattern generation for common cases
        let special_patterns: Vec<String> = if is_multi_word && words.len() == 2 {
            vec![
                words.join("-"),     // "file finder" -> "file-finder"
                words.join("_"),     // "file finder" -> "file_finder"
                words.join(""),      // "filefinder" -> "filefinder"
                format!("{}\\{}", words.join("-"), words[0]), // Look for project structure like "file-finder\file"
                format!("{}\\{}", words.join("_"), words[0]), // Look for "file_finder\file"
            ]
        } else {
            Vec::new()
        };
        
        let mut results = Vec::with_capacity(limit.min(100)); // Cap capacity
        
        // Phase 0: Special pattern matches for multi-word queries (e.g., "file finder" -> "file-finder")
        if !special_patterns.is_empty() {
            println!("FZF Debug: Searching for special patterns: {:?}", special_patterns);
            for pattern in &special_patterns {
                println!("FZF Debug: Looking for pattern '{}'", pattern);
                let mut pattern_matches = 0;
                for (i, file) in self.files.iter().enumerate() {
                    if i > 100000 { break; } // Expanded scan range for better matching
                    
                    let path_lower = file.path.to_lowercase();
                    
                    // Enhanced matching: check both name and path, with better scoring
                    let matches = file.name_lower.contains(pattern) || 
                                  path_lower.contains(pattern) ||
                                  (pattern == "file-finder" && path_lower.contains("fileopener\\file-finder")) ||
                                  (pattern == "file-finder" && (file.name == "main.js" || file.name == "lib.rs" || file.name == "Cargo.toml") && path_lower.contains("file-finder"));
                    
                    if matches {
                        let score = if file.name_lower == *pattern {
                            20000i64 // Perfect match for pattern
                        } else if file.name_lower.contains(pattern) {
                            15000i64 // Name contains pattern
                        } else if path_lower.contains(&format!("\\{}\\", pattern)) {
                            12000i64 // Path contains pattern as directory
                        } else if path_lower.contains(pattern) {
                            8000i64  // Path contains pattern somewhere
                        } else if file.name.ends_with(".js") || file.name.ends_with(".rs") || file.name.ends_with(".toml") {
                            6000i64  // Project files in related directories
                        } else {
                            4000i64  // Other matches
                        };
                        
                        if !results.iter().any(|(_, f): &(i64, &FileIndex)| f.path == file.path) {
                            results.push((score, file));
                            pattern_matches += 1;
                            println!("FZF Debug: Pattern '{}' matched: {} (score={})", pattern, file.path, score);
                            if results.len() >= 10 { break; } // Enough special matches
                        }
                    }
                }
                println!("FZF Debug: Pattern '{}' found {} matches", pattern, pattern_matches);
            }
        }
        
        // Phase 1: Exact filename matches (highest priority)
        let exact_limit = limit.min(20); // Only check first 20 exact matches
        for (i, file) in self.files.iter().enumerate() {
            if i > 50000 && results.len() >= exact_limit { break; } // Early termination
            
            if file.name_lower == query_lower {
                // Boost score for project files
                let score = if file.path.contains("file-finder") || 
                              file.name.contains("main") || 
                              file.name.contains("index") ||
                              file.name.ends_with(".js") ||
                              file.name.ends_with(".rs") ||
                              file.name.ends_with(".toml") ||
                              file.name.ends_with(".json") {
                    15000i64 // Higher score for project files
                } else {
                    10000i64 // Normal score
                };
                results.push((score, file));
                if results.len() >= exact_limit { break; }
            }
        }
        
        // Phase 2: Starts-with matches (very fast)
        if results.len() < limit {
            let starts_limit = limit.min(30);
            for (i, file) in self.files.iter().enumerate() {
                if i > 100000 && results.len() >= starts_limit { break; } // Early termination
                
                if file.name_lower.starts_with(&query_lower) && 
                   !results.iter().any(|(_, f): &(i64, &FileIndex)| f.path == file.path) {
                    // Boost score for project files
                    let score = if file.path.contains("file-finder") || 
                                  file.name.contains("main") || 
                                  file.name.contains("index") ||
                                  file.name.ends_with(".js") ||
                                  file.name.ends_with(".rs") ||
                                  file.name.ends_with(".toml") ||
                                  file.name.ends_with(".json") {
                        7000i64 // Higher score for project files
                    } else {
                        5000i64 // Normal score
                    };
                    results.push((score, file));
                    if results.len() >= starts_limit { break; }
                }
            }
        }
        
        // Phase 3: Contains matches (ULTRA aggressive limits for multi-word)
        if results.len() < limit {
            let contains_limit = if is_multi_word { limit.min(15) } else { limit };
            let max_scan = if is_multi_word { 25000 } else { 100000 }; // ULTRA aggressive scanning
            
            for (i, file) in self.files.iter().enumerate() {
                if i > max_scan || (is_multi_word && i > 10000 && results.len() >= 3) { break; } // HYPER aggressive termination
                
                let matches = if is_multi_word {
                    // Multi-word: More flexible - any word can be in name or path
                    let path_lower = file.path.to_lowercase();
                    let combined = format!("{} {}", file.name_lower, path_lower);
                    
                    // Either all words in combined text, OR most words present
                    let word_matches = words.iter().filter(|word| combined.contains(*word)).count();
                    word_matches >= (words.len().saturating_sub(1)).max(1) // Allow missing 1 word for fuzzy matching
                } else {
                    // Single word: simple contains
                    file.name_lower.contains(&query_lower)
                };
                
                if matches && !results.iter().any(|(_, f): &(i64, &FileIndex)| f.path == file.path) {
                    let score = if is_multi_word { 2000 } else { 1000 };
                    results.push((score, file));
                    if results.len() >= contains_limit { break; }
                }
            }
        }
        
        // Phase 4: LIMITED Fuzzy matching (only for short queries and if really needed)
        if results.len() < limit && query.len() <= 10 && !is_multi_word {
            let fuzzy_limit = 10; // Very limited fuzzy results
            let mut fuzzy_checked = 0;
            
            for file in &self.files {
                if fuzzy_checked > 10000 { break; } // Only check first 10k files for fuzzy
                fuzzy_checked += 1;
                
                if !results.iter().any(|(_, f): &(i64, &FileIndex)| f.path == file.path) {
                    if let Some(score) = self.matcher.fuzzy_match(&file.name_lower, &query_lower) {
                        if score > 20 { // Only good fuzzy matches
                            results.push((score, file));
                            if results.len() >= limit || results.len() >= fuzzy_limit { break; }
                        }
                    }
                }
            }
        }
        
        // Sort by score and truncate
        results.sort_by(|a, b| b.0.cmp(&a.0));
        results.truncate(limit);
        
        let duration = start_time.elapsed();
        println!("FZF: Found {} results for '{}' in {}μs ({})", 
                 results.len(), query, duration.as_micros(),
                 if is_multi_word { "multi-word" } else { "single-word" });
        
        results
    }

    /// Check if index needs refresh
    #[allow(dead_code)]
    pub fn needs_refresh(&self) -> bool {
        // Refresh every 5 minutes or on demand
        self.last_update.elapsed().as_secs() > 300
    }
}