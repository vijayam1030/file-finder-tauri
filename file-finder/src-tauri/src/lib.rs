use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::State;
use walkdir::WalkDir;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use chrono::Utc;
use regex::Regex;
use rayon::prelude::*;
use std::collections::HashSet;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchOptions {
    pub search_folders: bool,
    pub enable_fuzzy: bool,
    pub strict_mode: bool,
    pub filename_only: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            search_folders: true,
            enable_fuzzy: true,
            strict_mode: false,
            filename_only: false,
        }
    }
}

// Helper function to check if a file path is in a library/build directory
fn is_library_file(path: &str) -> bool {
    let path_l = path.to_lowercase();
    path_l.contains("/.git/") || path_l.contains("\\.git\\") ||
    path_l.contains("/node_modules/") || path_l.contains("\\node_modules\\") ||
    path_l.contains("/.vscode/") || path_l.contains("\\.vscode\\") ||
    path_l.contains("/target/") || path_l.contains("\\target\\") ||
    path_l.contains("/build/") || path_l.contains("\\build\\") ||
    path_l.contains("/dist/") || path_l.contains("\\dist\\") ||
    path_l.contains("/__pycache__/") || path_l.contains("\\__pycache__\\") ||
    path_l.contains("/site-packages/") || path_l.contains("\\site-packages\\") ||
    path_l.contains("/vendor/") || path_l.contains("\\vendor\\") ||
    path_l.contains("/.next/") || path_l.contains("\\.next\\") ||
    path_l.contains("/coverage/") || path_l.contains("\\coverage\\") ||
    path_l.contains("/out/") || path_l.contains("\\out\\") ||
    // Python/Anaconda library directories
    path_l.contains("/anaconda3/") || path_l.contains("\\anaconda3\\") ||
    path_l.contains("/miniconda3/") || path_l.contains("\\miniconda3\\") ||
    path_l.contains("/pkgs/") || path_l.contains("\\pkgs\\") ||
    path_l.contains("/envs/") || path_l.contains("\\envs\\") ||
    path_l.contains("/lib/python") || path_l.contains("\\lib\\python") ||
    // Jupyter/IPython directories
    path_l.contains("/share/jupyter/") || path_l.contains("\\share\\jupyter\\") ||
    path_l.contains("/jupyter/") || path_l.contains("\\jupyter\\") ||
    path_l.contains("/ipython/") || path_l.contains("\\ipython\\") ||
    // Other common library patterns
    path_l.contains("/program files/") || path_l.contains("\\program files\\") ||
    path_l.contains("/appdata/") || path_l.contains("\\appdata\\") ||
    path_l.contains("/.cache/") || path_l.contains("\\.cache\\")
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub last_accessed: Option<i64>,
    pub access_count: i32,
}

pub struct AppState {
    db: Mutex<Connection>,
}

impl AppState {
    fn new() -> SqlResult<Self> {
        let db_path = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("file-finder")
            .join("index.db");

        // Create directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(db_path)?;

        // Create tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                name TEXT NOT NULL,
                indexed_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS recent_files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                name TEXT NOT NULL,
                last_accessed INTEGER NOT NULL,
                access_count INTEGER DEFAULT 1
            )",
            [],
        )?;

        // Create indexes for faster search
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_name ON files(name)",
            [],
        )?;
        
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_path ON files(path)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_recent_access ON recent_files(last_accessed DESC)",
            [],
        )?;

        Ok(AppState {
            db: Mutex::new(conn),
        })
    }
}

#[tauri::command]
async fn start_indexing(_state: State<'_, AppState>) -> Result<String, String> {
    let home_dir = dirs::home_dir().ok_or("Could not find home directory")?;

    // Spawn a background task for indexing
    tauri::async_runtime::spawn(async move {
        index_directory(&home_dir).await;
    });

    Ok("Indexing started in background".to_string())
}

async fn index_directory(path: &Path) {
    let db_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("file-finder")
        .join("index.db");

    let mut conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to open database: {}", e);
            return;
        }
    };

    // Optimize database for bulk inserts
    if let Err(e) = conn.execute_batch(
        "PRAGMA synchronous = OFF;
         PRAGMA journal_mode = MEMORY;
         PRAGMA cache_size = 10000;
         PRAGMA temp_store = MEMORY;"
    ) {
        eprintln!("Failed to optimize database: {}", e);
    }

    // Clear existing files from database for fresh indexing
    if let Err(e) = conn.execute("DELETE FROM files", []) {
        eprintln!("Failed to clear existing files: {}", e);
        return;
    }
    println!("Cleared existing index, starting fresh...");

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    println!("Collecting files...");
    
    // Use HashSet for in-memory duplicate detection
    let mut seen_paths: HashSet<String> = HashSet::new();
    
    // Collect all entries first (this is I/O bound and relatively fast)
    let entries: Vec<(String, String)> = WalkDir::new(path)
        .follow_links(false)
        .max_depth(10)
        .into_iter()
        .filter_entry(|e| {
            // Skip hidden directories and common ignore patterns
            let file_name = e.file_name().to_string_lossy();
            !file_name.starts_with('.')
                && !file_name.eq("node_modules")
                && !file_name.eq("target")
                && !file_name.eq("AppData")
                && !file_name.eq("Library")
        })
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            // Index both files and directories
            if let Some(path_str) = entry.path().to_str() {
                // Check for duplicates using HashSet (O(1) lookup)
                if seen_paths.contains(path_str) {
                    return None; // Skip duplicate
                }
                
                if let Some(name) = entry.file_name().to_str() {
                    seen_paths.insert(path_str.to_string());
                    return Some((path_str.to_string(), name.to_string()));
                }
            }
            None
        })
        .collect();

    let total_count = entries.len();
    println!("Found {} unique items, inserting into database...", total_count);

    // Start a transaction for bulk insert
    let tx = match conn.transaction() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to start transaction: {}", e);
            return;
        }
    };

    // Use prepared statement for better performance
    // INSERT OR IGNORE handles any edge case duplicates at DB level (extra safety)
    let mut stmt = match tx.prepare("INSERT OR IGNORE INTO files (path, name, indexed_at) VALUES (?1, ?2, ?3)") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to prepare statement: {}", e);
            return;
        }
    };

    // Insert all entries
    for (idx, (path_str, name)) in entries.iter().enumerate() {
        let _ = stmt.execute(params![path_str, name, now]);
        
        if (idx + 1) % 10000 == 0 {
            println!("Inserted {} / {} items...", idx + 1, total_count);
        }
    }

    drop(stmt);

    // Commit the transaction
    if let Err(e) = tx.commit() {
        eprintln!("Failed to commit transaction: {}", e);
        return;
    }

    println!("Indexing complete! Total files: {}", total_count);
}

// Helper function to normalize strings by removing separators for better matching
fn normalize_for_matching(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn fuzzy_search_files(files: Vec<(String, String)>, query: &str, recent: &[String], options: &SearchOptions) -> Vec<(i64, FileEntry)> {
    // New smarter search:
    // - Tokenize the query by whitespace
    // - Prefer ordered substring matches in filename first, then in the joined path components
    // - Give a strong boost for contiguous (exact substring) matches
    // - Fall back to fuzzy matching only when ordered substring checks fail, and require a reasonable score threshold
    let matcher = SkimMatcherV2::default();
    let mut results: Vec<(i64, FileEntry)> = Vec::with_capacity(100);

    let query_trimmed = query.trim();
    if query_trimmed.is_empty() {
        return results;
    }

    let tokens: Vec<String> = query_trimmed
        .split_whitespace()
        .map(|s| s.to_lowercase())
        .collect();

    // Normalized query (no separators) for matching "finduname" to "find-uname"
    let query_normalized = normalize_for_matching(query_trimmed);

    for (path, name) in files {
        let name_l = name.to_lowercase();
        let path_l = path.to_lowercase();
        let name_normalized = normalize_for_matching(&name);

        // Check if file is in a library/build directory (should be deprioritized)
        let is_in_library_dir = is_library_file(&path);

        // Helper: check if all tokens appear in order in a haystack string
        let in_order_in = |haystack: &str| -> Option<i64> {
            let mut pos: usize = 0;
            let mut score_bonus: i64 = 0;
            for tok in &tokens {
                if let Some(found) = haystack[pos..].find(tok) {
                    // found is relative to haystack[pos..]
                    let abs = pos + found;
                    // Closer to start => slightly higher score
                    score_bonus += (1000i64.saturating_sub(abs as i64)).max(0);
                    pos = abs + tok.len();
                } else {
                    return None;
                }
            }
            Some(score_bonus)
        };

        // 1) Try filename matching - use both token-based AND normalized matching
        let mut matched_filename = false;
        let mut best_score: i64 = 0;
        
        // 1a) Normalized filename matching (ignores spaces, hyphens, underscores, dots)
        // This allows "gre word" to match "grewordlist.txt" and "finduname" to match "find-uname.py"
        // Check this FIRST because it's more permissive
        if !query_normalized.is_empty() && name_normalized.contains(&query_normalized) {
            let mut score: i64 = 2900; // High score for normalized match
            // Bonus if it's at the start
            if name_normalized.starts_with(&query_normalized) {
                score += 500;
            }
            matched_filename = true;
            best_score = score;
        }
        
        // 1b) Token-based ordered substring matching (stricter but gives higher score)
        if let Some(bonus) = in_order_in(&name_l) {
            // Check strict mode
            if options.strict_mode {
                // In strict mode, only allow exact or prefix matches
                let is_exact = name_l == query_trimmed.to_lowercase();
                let is_prefix = name_l.starts_with(&query_trimmed.to_lowercase());
                if is_exact || is_prefix {
                    let contiguous = name_l.contains(query_trimmed);
                    let mut score: i64 = 3000 + bonus;
                    if contiguous {
                        score += 1200;
                    }
                    // Use this score if it's better than normalized match
                    if score > best_score {
                        best_score = score;
                    }
                    matched_filename = true;
                }
            } else {
                // Not in strict mode, accept token match
                let contiguous = name_l.contains(query_trimmed);
                let mut score: i64 = 3000 + bonus;
                if contiguous {
                    score += 1200;
                }
                // Use this score if it's better than normalized match
                if score > best_score {
                    best_score = score;
                }
                matched_filename = true;
            }
        }
        
        // If we matched the filename via any method, add it to results
        if matched_filename {
            // Deprioritize library/build directories
            if is_in_library_dir {
                best_score = best_score / 4;
            }
            if recent.contains(&path) { best_score *= 2; }
            results.push((best_score, FileEntry { path: path.clone(), name, last_accessed: None, access_count: 0 }));
            continue;
        }

        // 2) Path components ordered substring (folder names) - skip if filename_only or !search_folders
        if options.search_folders && !options.filename_only {
            let components_joined = path_l.split(['/', '\\']).filter(|s| !s.is_empty()).collect::<Vec<&str>>().join("/");
            if let Some(bonus) = in_order_in(&components_joined) {
                let contiguous = components_joined.contains(&query_trimmed.to_lowercase());
                let mut score: i64 = 2000 + bonus;
                if contiguous { score += 800; }
                // Deprioritize library/build directories
                if is_in_library_dir {
                    score = score / 4; // Significantly reduce score for library files
                }
                if recent.contains(&path) { score *= 2; }
                results.push((score, FileEntry { path: path.clone(), name, last_accessed: None, access_count: 0 }));
                continue;
            }
        }

        // 3) Weak fuzzy fallback (lower priority) - only if fuzzy is enabled
        if options.enable_fuzzy && !options.strict_mode {
            if let Some(fuzzy_score) = matcher.fuzzy_match(&name, query_trimmed) {
                // require threshold to prevent everything matching; scale down for file-name fuzzy
                if fuzzy_score >= 60 {
                    let mut score = (fuzzy_score as i64) + 500; // base bump
                    // Deprioritize library/build directories
                    if is_in_library_dir {
                        score = score / 4; // Significantly reduce score for library files
                    }
                    if recent.contains(&path) { score *= 2; }
                    results.push((score, FileEntry { path: path.clone(), name, last_accessed: None, access_count: 0 }));
                    continue;
                }
            }

            // 4) Very last: fuzzy match against full path but with higher bar and lower weight
            if !options.filename_only {
                if let Some(full_score) = matcher.fuzzy_match(&path, query_trimmed) {
                    if full_score >= 80 {
                        let mut score = (full_score as i64) / 2; // de-prioritize full-path fuzzy
                        // Deprioritize library/build directories
                        if is_in_library_dir {
                            score = score / 4; // Significantly reduce score for library files
                        }
                        if recent.contains(&path) { score *= 2; }
                        results.push((score, FileEntry { path: path.clone(), name, last_accessed: None, access_count: 0 }));
                    }
                }
            }
        }
    }

    results
}


// Convert glob pattern to regex pattern
fn glob_to_regex(glob: &str) -> String {
    let mut regex = String::with_capacity(glob.len() * 2);
    
    // Add start anchor if the pattern doesn't start with *
    if !glob.starts_with('*') {
        regex.push('^');
    }
    
    for ch in glob.chars() {
        match ch {
            '*' => regex.push_str(".*"),      // * matches any sequence of characters
            '?' => regex.push('.'),           // ? matches any single character
            '.' => regex.push_str("\\."),     // Escape literal dots
            '+' => regex.push_str("\\+"),     // Escape literal plus
            '^' => regex.push_str("\\^"),     // Escape literal caret
            '$' => regex.push_str("\\$"),     // Escape literal dollar
            '(' => regex.push_str("\\("),     // Escape literal parentheses
            ')' => regex.push_str("\\)"),
            '[' => regex.push_str("\\["),     // Escape literal brackets
            ']' => regex.push_str("\\]"),
            '{' => regex.push_str("\\{"),     // Escape literal braces
            '}' => regex.push_str("\\}"),
            '|' => regex.push_str("\\|"),     // Escape literal pipe
            '\\' => regex.push_str("\\\\"),   // Escape literal backslash
            c => regex.push(c),               // Regular characters pass through
        }
    }
    
    // Add end anchor if the pattern doesn't end with *
    if !glob.ends_with('*') {
        regex.push('$');
    }
    
    // Make it case-insensitive by default and add debug output
    let final_regex = format!("(?i){}", regex);
    println!("Glob '{}' converted to regex: '{}'", glob, final_regex);
    final_regex
}

#[tauri::command]
async fn search_files(query: String, options: Option<SearchOptions>, state: State<'_, AppState>) -> Result<Vec<FileEntry>, String> {
    let search_opts = options.unwrap_or_default();
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    let (files, recent) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;

        // Check if this is a glob or regex pattern that needs all files
        let needs_all_files = query.contains(['*', '?', '[', ']', '(', ')', '|', '^', '$', '+', '{', '}', '\\']) 
            || query.starts_with('/') && query.ends_with('/');
        
        let files: Vec<(String, String)> = if needs_all_files {
            // For pattern searches, try to optimize with database queries when possible
            if query.starts_with("*.") && !query.contains(['[', ']', '(', ')', '|', '^', '$', '+', '{', '}']) {
                // Simple extension glob like "*.java" - use database LIKE query
                let extension = &query[2..]; // Remove "*."
                let mut stmt = db
                    .prepare("SELECT path, name FROM files WHERE name LIKE ?1 ORDER BY name LIMIT 10000")
                    .map_err(|e| e.to_string())?;
                let like_pattern = format!("%.{}", extension);
                let results: Vec<(String, String)> = stmt.query_map([&like_pattern], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                println!("Extension query '{}' found {} files", like_pattern, results.len());
                results
            } else {
                // Complex patterns - get more files but still limit for performance
                let mut stmt = db
                    .prepare("SELECT path, name FROM files ORDER BY name LIMIT 20000")
                    .map_err(|e| e.to_string())?;
                let results: Vec<(String, String)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                results
            }
        } else if query.len() >= 2 {
            // For regular text searches, use LIKE to pre-filter
            // Extract individual words for better pre-filtering
            let words: Vec<&str> = query.split_whitespace().collect();
            
            if words.len() > 1 {
                // Multi-word query: check if ALL words appear in name/path (in any order)
                // This handles cases like "gre word list" matching "grewordlist.txt"
                let mut combined_query = String::from("SELECT path, name FROM files WHERE ");
                for (i, word) in words.iter().enumerate() {
                    if i > 0 {
                        combined_query.push_str(" AND ");
                    }
                    combined_query.push_str(&format!("(name LIKE '%{}%' OR path LIKE '%{}%')", word, word));
                }
                combined_query.push_str(" ORDER BY name LIMIT 2000");
                
                let mut stmt = db.prepare(&combined_query).map_err(|e| e.to_string())?;
                let results: Vec<(String, String)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                results
            } else {
                // Single word query: use simple LIKE
                let mut stmt = db
                    .prepare("SELECT path, name FROM files WHERE name LIKE ?1 OR path LIKE ?1 ORDER BY name LIMIT 1000")
                    .map_err(|e| e.to_string())?;
                
                let like_query = format!("%{}%", query);
                let results: Vec<(String, String)> = stmt.query_map([&like_query], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                results
            }
        } else {
            // For single character queries, just limit heavily
            let mut stmt = db
                .prepare("SELECT path, name FROM files ORDER BY name LIMIT 500")
                .map_err(|e| e.to_string())?;
            
            let results: Vec<(String, String)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            results
        };

        // Get recent files for boost
        let mut recent_stmt = db
            .prepare("SELECT path FROM recent_files ORDER BY access_count DESC, last_accessed DESC LIMIT 50")
            .map_err(|e| e.to_string())?;

        let recent: Vec<String> = recent_stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        (files, recent)
    }; // Database lock is automatically released here

    // Check if query is a pattern (glob or regex)
    let is_glob_pattern = query.contains(['*', '?']) && !query.contains(['[', ']', '(', ')', '|', '^', '$', '+', '{', '}']);
    let is_regex_query = query.contains(['[', ']', '(', ')', '|', '^', '$', '.', '+', '{', '}', '\\']) 
        || (query.starts_with('/') && query.ends_with('/'));
    
    let mut results: Vec<(i64, FileEntry)> = if is_glob_pattern {
        // Handle glob pattern search (convert to regex)
        println!("Detected glob pattern: '{}', processing {} files", query, files.len());
        let regex_pattern = glob_to_regex(&query);
        println!("Converted to regex: '{}'", regex_pattern);
        match Regex::new(&regex_pattern) {
            Ok(regex) => {
                let mut match_count = 0;
                let results: Vec<_> = files.into_iter()
                    .filter_map(|(path, name)| {
                        // Check if file is in a library/build directory
                        let is_in_library_dir = is_library_file(&path);

                        // Check regex match against filename, path components, and full path
                        let name_match = regex.is_match(&name);
                        let path_components: Vec<&str> = path.split(['/', '\\'])
                            .filter(|s| !s.is_empty())
                            .collect();
                        let path_component_match = path_components.iter().any(|component| regex.is_match(component));
                        let full_path_match = regex.is_match(&path);

                        // Debug: check for java files specifically
                        if name.to_lowercase().ends_with(".java") {
                            println!("Found Java file: {} - name_match: {}, path_component_match: {}, full_path_match: {}", 
                                     name, name_match, path_component_match, full_path_match);
                        }
                        
                        if name_match || path_component_match || full_path_match {
                            match_count += 1;
                            // Use improved scoring logic similar to the new search function
                            let mut score = if name_match { 
                                // Exact filename pattern match gets highest priority
                                5000 
                            } else if path_component_match { 
                                // Folder name pattern match gets good priority
                                3000 
                            } else { 
                                // Full path match gets lower priority
                                1500 
                            };

                            // Deprioritize library/build directories
                            if is_in_library_dir {
                                score = score / 4; // Significantly reduce score for library files
                            }
                                       
                            // Boost score if file is in recent files
                            let boosted_score = if recent.contains(&path) {
                                score + 1000  // Additive boost instead of multiplicative to avoid overflow
                            } else {
                                score
                            };
                            
                            Some((
                                boosted_score,
                                FileEntry {
                                    path: path.clone(),
                                    name,
                                    last_accessed: None,
                                    access_count: 0,
                                },
                            ))
                        } else {
                            None
                        }
                    })
                    .collect();
                println!("Glob pattern '{}' matched {} files", query, match_count);
                results
            },
            Err(_) => {
                // If glob-to-regex conversion fails, fall back to new search
                fuzzy_search_files(files, &query, &recent, &search_opts)
            }
        }
    } else if is_regex_query {
        // Handle regex search
        let regex_pattern = if query.starts_with('/') && query.ends_with('/') && query.len() > 2 {
            // Remove surrounding slashes for /pattern/ syntax
            &query[1..query.len()-1]
        } else {
            &query
        };
        
        match Regex::new(regex_pattern) {
            Ok(regex) => {
                files.into_iter()
                    .filter_map(|(path, name)| {
                        // Check if file is in a library/build directory
                        let is_in_library_dir = is_library_file(&path);

                        // Check regex match against filename, path components, and full path
                        let name_match = regex.is_match(&name);
                        let path_components: Vec<&str> = path.split(['/', '\\'])
                            .filter(|s| !s.is_empty())
                            .collect();
                        let path_component_match = path_components.iter().any(|component| regex.is_match(component));
                        let full_path_match = regex.is_match(&path);
                        
                        if name_match || path_component_match || full_path_match {
                            // Use improved scoring logic consistent with new search function
                            let mut score = if name_match { 
                                // Exact filename regex match gets highest priority
                                5000 
                            } else if path_component_match { 
                                // Folder name regex match gets good priority
                                3000 
                            } else { 
                                // Full path regex match gets lower priority
                                1500 
                            };

                            // Deprioritize library/build directories
                            if is_in_library_dir {
                                score = score / 4; // Significantly reduce score for library files
                            }
                                       
                            // Boost score if file is in recent files
                            let boosted_score = if recent.contains(&path) {
                                score + 1000  // Additive boost instead of multiplicative
                            } else {
                                score
                            };
                            
                            Some((
                                boosted_score,
                                FileEntry {
                                    path: path.clone(),
                                    name,
                                    last_accessed: None,
                                    access_count: 0,
                                },
                            ))
                        } else {
                            None
                        }
                    })
                    .collect()
            },
            Err(_) => {
                // If regex is invalid, fall back to new search
                fuzzy_search_files(files, &query, &recent, &search_opts)
            }
        }
    } else {
        // Handle improved search
        fuzzy_search_files(files, &query, &recent, &search_opts)
    };

    // Sort by score (descending) and limit early for better performance
    results.sort_by(|a, b| b.0.cmp(&a.0));

    // Return top 100 results for faster response
    Ok(results.into_iter().take(100).map(|(_, entry)| entry).collect())
}

#[tauri::command]
async fn get_recent_files(state: State<'_, AppState>) -> Result<Vec<FileEntry>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let mut stmt = db
        .prepare("SELECT path, name, last_accessed, access_count FROM recent_files ORDER BY access_count DESC, last_accessed DESC LIMIT 20")
        .map_err(|e| e.to_string())?;

    let files: Vec<FileEntry> = stmt
        .query_map([], |row| {
            Ok(FileEntry {
                path: row.get(0)?,
                name: row.get(1)?,
                last_accessed: Some(row.get(2)?),
                access_count: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(files)
}

#[tauri::command]
async fn open_file(path: String, state: State<'_, AppState>) -> Result<(), String> {
    // Update recent files
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().timestamp();

    let path_obj = PathBuf::from(&path);
    let name = path_obj
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&path);

    db.execute(
        "INSERT INTO recent_files (path, name, last_accessed, access_count)
         VALUES (?1, ?2, ?3, 1)
         ON CONFLICT(path) DO UPDATE SET
            last_accessed = ?3,
            access_count = access_count + 1",
        params![path, name, now],
    )
    .map_err(|e| e.to_string())?;

    drop(db); // Release lock before opening file

    // Open file with default application
    opener::open(&path).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn open_file_with(path: String, program: String, state: State<'_, AppState>) -> Result<(), String> {
    // Update recent files
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().timestamp();

    let path_obj = PathBuf::from(&path);
    let name = path_obj
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&path);

    db.execute(
        "INSERT INTO recent_files (path, name, last_accessed, access_count)
         VALUES (?1, ?2, ?3, 1)
         ON CONFLICT(path) DO UPDATE SET
            last_accessed = ?3,
            access_count = access_count + 1",
        params![path, name, now],
    )
    .map_err(|e| e.to_string())?;

    drop(db);

    // Open file with specified program
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(&["/C", "start", "", &program, &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new(&program)
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[derive(Serialize)]
struct FileInfo {
    extension: String,
    suggested_programs: Vec<String>,
}

#[tauri::command]
async fn get_file_info(path: String) -> Result<FileInfo, String> {
    let path_obj = PathBuf::from(&path);
    let extension = path_obj
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Common program suggestions based on extension
    let suggested_programs = match extension.as_str() {
        "py" => vec!["notepad++.exe", "code.exe", "pycharm64.exe", "notepad.exe"],
        "java" => vec!["notepad++.exe", "code.exe", "idea64.exe", "notepad.exe"],
        "js" | "ts" | "jsx" | "tsx" => vec!["code.exe", "notepad++.exe", "webstorm64.exe", "notepad.exe"],
        "txt" | "md" | "log" => vec!["notepad++.exe", "notepad.exe", "code.exe"],
        "json" | "xml" | "yaml" | "yml" => vec!["notepad++.exe", "code.exe", "notepad.exe"],
        "html" | "css" => vec!["code.exe", "notepad++.exe", "chrome.exe", "notepad.exe"],
        "pdf" => vec!["AcroRd32.exe", "chrome.exe", "msedge.exe"],
        "jpg" | "jpeg" | "png" | "gif" | "bmp" => vec!["mspaint.exe", "PhotosApp.exe", "chrome.exe"],
        "mp4" | "avi" | "mkv" => vec!["vlc.exe", "wmplayer.exe"],
        "mp3" | "wav" | "flac" => vec!["vlc.exe", "wmplayer.exe"],
        "zip" | "rar" | "7z" => vec!["7zFM.exe", "WinRAR.exe"],
        "doc" | "docx" => vec!["WINWORD.EXE", "notepad.exe"],
        "xls" | "xlsx" => vec!["EXCEL.EXE", "notepad.exe"],
        "ppt" | "pptx" => vec!["POWERPNT.EXE"],
        _ => vec!["notepad.exe", "code.exe", "notepad++.exe"],
    };

    Ok(FileInfo {
        extension: extension.to_string(),
        suggested_programs: suggested_programs.iter().map(|s| s.to_string()).collect(),
    })
}

#[tauri::command]
async fn get_index_status(state: State<'_, AppState>) -> Result<IndexStatus, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;

    let last_indexed: Option<i64> = db
        .query_row(
            "SELECT MAX(indexed_at) FROM files",
            [],
            |row| row.get(0),
        )
        .ok();

    Ok(IndexStatus {
        total_files: count,
        last_indexed,
    })
}

#[derive(Serialize)]
struct IndexStatus {
    total_files: i64,
    last_indexed: Option<i64>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::new().expect("Failed to initialize app state");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            start_indexing,
            search_files,
            get_recent_files,
            open_file,
            open_file_with,
            get_file_info,
            get_index_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
