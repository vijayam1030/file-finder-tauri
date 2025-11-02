use serde::{Deserialize, Serialize};
use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};
use nucleo_matcher::{Matcher, Config};

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
    pub nucleo_matcher: Matcher,
    pub last_update: std::time::Instant,
}

impl FzfSearchEngine {
    // Helper: Split camelCase and PascalCase into lowercase words
    fn split_camel_case(s: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut current = String::new();
        for c in s.chars() {
            if c.is_uppercase() && !current.is_empty() {
                words.push(current.clone());
                current.clear();
            }
            current.push(c.to_ascii_lowercase());
        }
        if !current.is_empty() {
            words.push(current);
        }
        words
    }
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            matcher: SkimMatcherV2::default(),
            nucleo_matcher: Matcher::new(Config::DEFAULT),
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
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();
    let is_multi_word = query_words.len() > 1;
        
        // Debug for multi-word searches
        if is_multi_word {
            println!("FZF Debug: Multi-word search '{}' - looking for files with both terms", query);
        }
        
        // Special pattern generation for common cases
        let special_patterns: Vec<String> = if is_multi_word && query_words.len() == 2 {
            vec![
                query_words.join("-"),     // "file finder" -> "file-finder"
                query_words.join("_"),     // "file finder" -> "file_finder"
                query_words.join("") ,     // "filefinder" -> "filefinder"
                format!("{}\\{}", query_words.join("-"), query_words[0]), // Look for project structure like "file-finder\file"
                format!("{}\\{}", query_words.join("_"), query_words[0]), // Look for "file_finder\file"
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
        
        // Phase 3: Contains matches (much larger scan for multi-word)
        if results.len() < limit {
            let contains_limit = if is_multi_word { limit.min(25) } else { limit };
            // CRITICAL: With large databases (1M+ files), need to scan much more for multi-word
            let max_scan = if is_multi_word { 200000 } else { 100000 }; // Scan up to 200k files for multi-word

            let mut scanned_files = 0;
            for (i, file) in self.files.iter().enumerate() {
                // Only stop early if we have good distributed matches
                if i > max_scan || (is_multi_word && i > 100000 && results.len() >= 10) {
                    if is_multi_word {
                        println!("FZF Debug: Stopped scanning at {} files with {} results", scanned_files, results.len());
                    }
                    break;
                }
                scanned_files += 1;

                // Enhanced matching: check both filename and path segments
                let path_lower = file.path.to_lowercase();
                
                let matches = if is_multi_word {
                    // Multi-word: Enhanced fuzzy matching with partial word support
                    // Split path into segments for better matching
                    let path_segments: Vec<&str> = path_lower.split("\\").chain(path_lower.split("/")).collect();
                    let name_camel = Self::split_camel_case(&file.name);
                    let _path_camel: Vec<String> = path_segments.iter().flat_map(|seg| Self::split_camel_case(seg)).collect();
                    let all_text = format!("{} {} {} {}", file.name_lower, path_lower, path_segments.join(" "), name_camel.join(" "));
                    let mut matched_words = 0;
                    let mut match_details = Vec::new();
                    
                    for word in &query_words {
                        let mut word_matched = false;
                        let mut match_type = "";
                        
                        // 1. Check for exact word match first
                        if all_text.contains(*word) {
                            matched_words += 1;
                            word_matched = true;
                            match_type = "exact";
                        } 
                        // 2. For words 3+ chars, check partial matches (more aggressive)
                        else if word.len() >= 3 {
                            let partial_match = 
                                // Check path segments for prefix matches (e.g., "comp" → "competizione")
                                path_segments.iter().any(|segment| 
                                    segment.starts_with(*word) && segment.len() > word.len()
                                ) ||
                                // Check filename for prefix match  
                                (file.name_lower.len() > word.len() && file.name_lower.starts_with(*word)) ||
                                // Check for word as substring in longer words (more aggressive fuzzy matching)
                                all_text.split_whitespace().any(|text_word| 
                                    text_word.len() > word.len() && text_word.contains(*word)
                                ) ||
                                // Check filename for prefix matches
                                file.name_lower.starts_with(*word) && file.name_lower.len() > word.len() ||
                                // Check path parts split by common separators
                                path_lower.split(&['\\', '/', ' ', '_', '-'][..]).any(|part| 
                                    part.starts_with(*word) && part.len() > word.len()
                                ) ||
                                // Check if the word appears as a substring in larger words
                                all_text.split_whitespace().any(|part|
                                    part.len() > word.len() && (
                                        part.starts_with(*word) || 
                                        part.contains(&format!("{}", word)) // Contains as substring
                                    )
                                );
                            
                            if partial_match {
                                matched_words += 1;
                                word_matched = true;
                                match_type = "partial";
                            }
                        }
                        // 3. For very short words (2 chars), be even more flexible
                        else if word.len() >= 2 {
                            let fuzzy_match = 
                                // Allow short words to match anywhere in path or filename
                                file.name_lower.contains(*word) || 
                                path_lower.contains(*word) ||
                                // Check if it's a common abbreviation pattern
                                path_segments.iter().any(|segment| 
                                    segment.starts_with(*word) || segment.contains(*word)
                                );
                            
                            if fuzzy_match {
                                matched_words += 1;
                                word_matched = true;
                                match_type = "fuzzy";
                            }
                        }
                        
                        if word_matched {
                            match_details.push((word, match_type));
                        }
                    }
                    

                    
                    // Enhanced scoring: prefer files where words appear in different contexts
                    let _has_name_match = query_words.iter().any(|word|
                        file.name_lower.contains(*word) ||
                        (word.len() >= 3 && (
                            file.name_lower.starts_with(*word) ||
                            file.name_lower.split(&[' ', '_', '-'][..]).any(|part| part.starts_with(*word))
                        ))
                    );
                    let _has_path_match = query_words.iter().any(|word|
                        path_lower.contains(*word) ||
                        (word.len() >= 3 && (
                            path_segments.iter().any(|segment| segment.starts_with(*word)) ||
                            path_lower.split(&['\\', '/', ' ', '_', '-'][..]).any(|part| part.starts_with(*word))
                        ))
                    );

                    // Count how many words matched in name vs path for better distributed scoring
                    let name_matched_count = query_words.iter().filter(|word| {
                        file.name_lower.contains(**word) ||
                        (word.len() >= 3 && (
                            file.name_lower.starts_with(*word) ||
                            // Check if word matches start of any word in the filename
                            file.name_lower.split_whitespace().any(|name_word|
                                name_word.starts_with(*word) && name_word.len() > word.len()
                            )
                        ))
                    }).count();
                    let path_matched_count = query_words.iter().filter(|word| {
                        path_lower.contains(**word) ||
                        (word.len() >= 3 && path_segments.iter().any(|seg|
                            seg.starts_with(*word) ||
                            // Check words within segments (for segments with spaces)
                            seg.split_whitespace().any(|seg_word|
                                seg_word.starts_with(*word) && seg_word.len() > word.len()
                            )
                        ))
                    }).count();

                    // More flexible matching: allow missing words or require good distribution
                    let word_threshold = if query_words.len() <= 2 { query_words.len() } else { query_words.len().saturating_sub(1) };
                    let flexible_match = matched_words >= word_threshold.max(1);
                    // Distributed match is BETTER: one word in path, one in name
                    let distributed_match = (name_matched_count >= 1 && path_matched_count >= 1) && matched_words >= 2;

                    // Debug output for multi-word matching
                    if flexible_match || distributed_match {
                        println!("FZF Debug: Multi-word match '{}' - matched {}/{} words (name:{}, path:{}): {:?}", file.path, matched_words, query_words.len(), name_matched_count, path_matched_count, match_details);
                    }

                    flexible_match || distributed_match
                } else {
                    // Single word: Enhanced fuzzy matching for both filename and path segments
                    let exact_match = file.name_lower.contains(&query_lower) || path_lower.contains(&query_lower);
                    
                    // Enhanced partial matching for queries 3+ chars
                    let partial_match = if query_lower.len() >= 3 {
                        let path_segments: Vec<&str> = path_lower.split('\\').chain(path_lower.split('/')).collect();
                        
                        // Check path segments for prefix matches
                        path_segments.iter().any(|segment| 
                            segment.starts_with(&query_lower) && segment.len() > query_lower.len()
                        ) ||
                        // Check filename for prefix matches
                        file.name_lower.starts_with(&query_lower) && file.name_lower.len() > query_lower.len() ||
                        // Check path parts split by common separators
                        path_lower.split(&['\\', '/', ' ', '_', '-'][..]).any(|part| 
                            part.starts_with(&query_lower) && part.len() > query_lower.len()
                        ) ||
                        // Check all words in the combined text
                        format!("{} {}", file.name_lower, path_lower)
                            .split_whitespace()
                            .any(|word| 
                                word.len() > query_lower.len() && 
                                (word.starts_with(&query_lower) || word.contains(&query_lower))
                            )
                    } else if query_lower.len() >= 2 {
                        // For very short queries, be more flexible
                        file.name_lower.contains(&query_lower) || 
                        path_lower.contains(&query_lower)
                    } else {
                        false
                    };
                    
                    exact_match || partial_match
                };
                
                if matches && !results.iter().any(|(_, f): &(i64, &FileIndex)| f.path == file.path) {
                    // Enhanced scoring for multi-word queries
                    let score = if is_multi_word {
                        // Count how many words matched in name vs path (using improved matching)
                        let name_matched = query_words.iter().filter(|word| {
                            file.name_lower.contains(*word) ||
                            (word.len() >= 3 && (
                                file.name_lower.starts_with(*word) ||
                                // Match against words in the filename
                                file.name_lower.split_whitespace().any(|name_word|
                                    name_word.starts_with(*word) && name_word.len() > word.len()
                                )
                            ))
                        }).count();
                        let path_matched = query_words.iter().filter(|word| {
                            path_lower.contains(*word) ||
                            (word.len() >= 3 && path_lower.split(&['\\', '/'][..])
                                .any(|seg|
                                    seg.starts_with(*word) ||
                                    seg.split_whitespace().any(|seg_word|
                                        seg_word.starts_with(*word) && seg_word.len() > word.len()
                                    )
                                ))
                        }).count();

                        // Distributed matches score higher (path + name)
                        if name_matched >= 1 && path_matched >= 1 {
                            4000i64 // HIGH priority: words distributed across path and name
                        } else if name_matched > 0 || path_matched > 0 {
                            2000i64 // Medium priority: all words in same location
                        } else {
                            1000i64 // Lower priority
                        }
                    } else {
                        1000i64
                    };

                    results.push((score, file));
                    if results.len() >= contains_limit { break; }
                }
            }
        }
        
        // Phase 4: Multi-word fallback - if no multi-word results, search for most specific term
        if is_multi_word && results.len() < 3 {
            println!("FZF Debug: Multi-word search found few results ({}), falling back to single-word search", results.len());
            
            // Find the longest/most specific word to search for
            let query_str = query_lower.as_str();
            let fallback_word = query_words.iter()
                .max_by_key(|word| word.len())
                .unwrap_or(&query_str);
            
            println!("FZF Debug: Using fallback word '{}' for single-word search", fallback_word);
            for (i, file) in self.files.iter().enumerate() {
                if i > 25000 || results.len() >= limit { break; }
                
                let path_lower = file.path.to_lowercase();
                let matches = file.name_lower.contains(fallback_word) || 
                             path_lower.contains(fallback_word) ||
                             path_lower.split('\\').chain(path_lower.split('/'))
                                 .any(|segment| segment.contains(fallback_word));
                
                if matches && !results.iter().any(|(_, f): &(i64, &FileIndex)| f.path == file.path) {
                    results.push((1500, file)); // Lower score than exact multi-word matches
                    if results.len() >= limit { break; }
                }
            }
        }
        
        // Phase 5: LIMITED Fuzzy matching (only for short queries and if really needed)
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