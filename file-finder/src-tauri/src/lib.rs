use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, Instant};
use tauri::State;
use walkdir::WalkDir;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use chrono::Utc;

mod llm_integration;
mod fzf_search;
mod simple_search;
use regex::Regex;
use std::collections::{HashSet, HashMap};
use rayon::prelude::*;
use fzf_search::FzfSearchEngine;
use simple_search::SimpleSearchEngine;

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
    path_l.contains("/.cache/") || path_l.contains("\\.cache\\") ||
    // Windows system directories
    path_l.contains("\\windows\\winsxs\\") || path_l.contains("/windows/winsxs/") ||
    path_l.contains("\\windows\\system32\\") || path_l.contains("/windows/system32/") ||
    path_l.contains("\\windows\\syswow64\\") || path_l.contains("/windows/syswow64/")
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub last_accessed: Option<i64>,
    pub access_count: i32,
    pub modified_at: Option<i64>,
}

pub struct AppState {
    db: Mutex<Connection>,
    // Simple cache for recent search results (query -> (timestamp, results))
    search_cache: Mutex<HashMap<String, (Instant, Vec<FileEntry>)>>,
    // Regex compilation cache for performance (pattern -> compiled regex)
    regex_cache: Mutex<HashMap<String, Regex>>,
    // LLM processor for natural language queries
    llm_processor: llm_integration::LLMProcessor,
    // FZF-style in-memory search engine for real-time search
    fzf_engine: Mutex<FzfSearchEngine>,
    // Simple, effective search engine using nucleo
    simple_engine: Mutex<SimpleSearchEngine>,
}

// Fuzzy matching helper function
fn fuzzy_match_score(text: &str, pattern: &str) -> f32 {
    let matcher = SkimMatcherV2::default();
    if let Some(score) = matcher.fuzzy_match(text, pattern) {
        // Normalize score to 0.0-1.0 range
        (score as f32 / 100.0).min(1.0).max(0.0)
    } else {
        0.0
    }
}

#[derive(Debug, Clone)]
struct PatternInfo {
    pattern_type: PatternType,
    prefix: Option<String>,
    suffix: Option<String>,
    can_use_sql_optimization: bool,
    sql_like_pattern: Option<String>,
}

#[derive(Debug, Clone)]
enum PatternType {
    SimpleGlob,      // file* or *.ext
    SimplePrefix,    // prefix.*
    PrefixSuffix,    // prefix.*suffix
    ComplexRegex,    // [a-z]+\d{2,4}
    LiteralSearch,   // plain text
}

// Comprehensive regex pattern analyzer
fn analyze_regex_pattern(query: &str) -> PatternInfo {
    let trimmed = query.trim();
    
    // Handle slash-wrapped regex
    let actual_pattern = if trimmed.starts_with('/') && trimmed.ends_with('/') && trimmed.len() > 2 {
        &trimmed[1..trimmed.len()-1]
    } else {
        trimmed
    };
    
    // Check if it's a simple glob pattern (only * and ? allowed)
    if !actual_pattern.contains(['[', ']', '(', ')', '|', '^', '$', '+', '{', '}', '\\']) {
        if actual_pattern.starts_with("*.") && actual_pattern.matches('*').count() == 1 {
            // *.ext pattern
            let extension = &actual_pattern[2..];
            return PatternInfo {
                pattern_type: PatternType::SimpleGlob,
                prefix: None,
                suffix: Some(extension.to_string()),
                can_use_sql_optimization: true,
                sql_like_pattern: Some(format!("%.{}", extension)),
            };
        } else if actual_pattern.ends_with('*') && actual_pattern.matches('*').count() == 1 {
            // prefix* pattern
            let prefix = &actual_pattern[..actual_pattern.len()-1];
            return PatternInfo {
                pattern_type: PatternType::SimpleGlob,
                prefix: Some(prefix.to_string()),
                suffix: None,
                can_use_sql_optimization: true,
                sql_like_pattern: Some(format!("{}%", prefix)),
            };
        }
    }
    
    // Check for optimizable regex prefix patterns
    if let Some(prefix) = extract_regex_prefix(actual_pattern) {
        if actual_pattern.ends_with(".*") && prefix.len() >= 2 {
            // Simple prefix.* pattern
            return PatternInfo {
                pattern_type: PatternType::SimplePrefix,
                prefix: Some(prefix.clone()),
                suffix: None,
                can_use_sql_optimization: true,
                sql_like_pattern: Some(format!("{}%", prefix)),
            };
        } else if let Some(suffix) = extract_regex_suffix(actual_pattern, &prefix) {
            // prefix.*suffix pattern
            return PatternInfo {
                pattern_type: PatternType::PrefixSuffix,
                prefix: Some(prefix.clone()),
                suffix: Some(suffix),
                can_use_sql_optimization: true,
                sql_like_pattern: Some(format!("{}%", prefix)),
            };
        }
    }
    
    // Check if it's just literal text (no regex metacharacters)
    if !actual_pattern.chars().any(|c| ".*+?^${}[]()\\|".contains(c)) {
        // Special handling for multi-word queries
        if actual_pattern.contains(' ') {
            // For multi-word queries like "word list", we want to find files that contain
            // all words, even if they're concatenated (e.g., "grewordlist")
            let words: Vec<&str> = actual_pattern.split_whitespace().collect();
            if words.len() == 2 {
                // For two-word queries, try both "word1 word2" and "word1word2"
                let concatenated = words.join("");
                return PatternInfo {
                    pattern_type: PatternType::LiteralSearch,
                    prefix: None,
                    suffix: None,
                    can_use_sql_optimization: true,
                    // Use the concatenated version for better matching
                    sql_like_pattern: Some(format!("%{}%", concatenated)),
                };
            }
        }
        
        return PatternInfo {
            pattern_type: PatternType::LiteralSearch,
            prefix: None,
            suffix: None,
            can_use_sql_optimization: true,
            sql_like_pattern: Some(format!("%{}%", actual_pattern)),
        };
    }
    
    // Complex regex pattern - needs full regex matching
    PatternInfo {
        pattern_type: PatternType::ComplexRegex,
        prefix: extract_regex_prefix(actual_pattern),
        suffix: None,
        can_use_sql_optimization: false,
        sql_like_pattern: None,
    }
}

// Extract prefix from regex patterns like "^prefix.*" or "prefix.*"
fn extract_regex_prefix(pattern: &str) -> Option<String> {
    let pattern = if pattern.starts_with('^') { &pattern[1..] } else { pattern };
    
    // Find the first regex metacharacter
    let mut prefix = String::new();
    for ch in pattern.chars() {
        if ".*+?{}[]()\\|$".contains(ch) {
            break;
        }
        prefix.push(ch);
    }
    
    if prefix.len() >= 2 && !prefix.is_empty() {
        Some(prefix)
    } else {
        None
    }
}

// Extract suffix from prefix.*suffix patterns
fn extract_regex_suffix(pattern: &str, prefix: &str) -> Option<String> {
    let pattern = if pattern.starts_with('^') { &pattern[1..] } else { pattern };
    let after_prefix = &pattern[prefix.len()..];
    
    if after_prefix.starts_with(".*") && after_prefix.len() > 2 {
        let suffix = &after_prefix[2..];
        // Check if suffix is simple (no complex regex)
        if !suffix.chars().any(|c| ".*+?{}[]()\\|^$".contains(c)) && !suffix.is_empty() {
            Some(suffix.to_string())
        } else {
            None
        }
    } else {
        None
    }
}

/// Convert a glob pattern to a regular expression
/// Supports:
/// - * matches any sequence of characters
/// - ? matches any single character
/// - [abc] matches any character in the set
/// - [a-z] matches any character in the range
/// - Everything else is treated literally
fn build_glob_regex(pattern: &str) -> String {
    let mut regex = String::with_capacity(pattern.len() * 2);
    regex.push('^'); // Anchor to start
    
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '[' => {
                // Handle character classes like [abc] or [a-z]
                regex.push('[');
                while let Some(inner) = chars.next() {
                    if inner == ']' {
                        regex.push(']');
                        break;
                    }
                    // Escape regex special characters within character class
                    match inner {
                        '^' | '-' | '\\' => {
                            regex.push('\\');
                            regex.push(inner);
                        }
                        _ => regex.push(inner),
                    }
                }
            }
            // Escape regex special characters
            '.' | '+' | '(' | ')' | '{' | '}' | '|' | '^' | '$' | '\\' => {
                regex.push('\\');
                regex.push(ch);
            }
            _ => regex.push(ch),
        }
    }
    
    regex.push('$'); // Anchor to end
    regex
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
                root_directory TEXT NOT NULL,
                indexed_at INTEGER NOT NULL,
                modified_at INTEGER
            )",
            [],
        )?;

        // Add modified_at column to existing files table if it doesn't exist
        let _ = conn.execute(
            "ALTER TABLE files ADD COLUMN modified_at INTEGER",
            [],
        ); // Ignore error if column already exists

        conn.execute(
            "CREATE TABLE IF NOT EXISTS indexed_directories (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                name TEXT NOT NULL,
                indexed_at INTEGER NOT NULL,
                is_active INTEGER DEFAULT 0
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

        conn.execute(
            "CREATE TABLE IF NOT EXISTS favorite_files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                name TEXT NOT NULL,
                favorited_at INTEGER NOT NULL
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

        // Add index for fast prefix searches on filename
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_files_name_prefix ON files(name)",
            [],
        )?;

        // Add index for path searches
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_files_path ON files(path)",
            [],
        )?;

        // Add composite index for name and path searches (for faster OR queries)
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_files_name_path ON files(name, path)",
            [],
        )?;

        // Add index for extension-based searches (optimized for *.ext patterns)
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_files_name_suffix ON files(name COLLATE NOCASE)",
            [],
        )?;

        // Migrate existing databases - add root_directory column if it doesn't exist
        let has_root_directory: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('files') WHERE name='root_directory'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0)
        ).unwrap_or(false);

        if !has_root_directory {
            println!("Migrating database: adding root_directory column");
            // Add the column with a default value
            conn.execute(
                "ALTER TABLE files ADD COLUMN root_directory TEXT NOT NULL DEFAULT ''",
                [],
            )?;
            
            // Set root_directory to empty string for existing files
            conn.execute(
                "UPDATE files SET root_directory = '' WHERE root_directory IS NULL OR root_directory = ''",
                [],
            )?;
        }

        let mut fzf_engine = FzfSearchEngine::new();
        
        // Load FZF index on startup
        if let Err(e) = fzf_engine.load_from_database(&conn) {
            println!("Failed to load FZF index: {}", e);
        }
        
        Ok(AppState {
            db: Mutex::new(conn),
            search_cache: Mutex::new(HashMap::new()),
            regex_cache: Mutex::new(HashMap::new()),
            llm_processor: llm_integration::LLMProcessor::new(),
            fzf_engine: Mutex::new(fzf_engine),
            simple_engine: Mutex::new(SimpleSearchEngine::new()),
        })
    }
}

#[tauri::command]
async fn start_indexing(_state: State<'_, AppState>) -> Result<String, String> {
    println!("start_indexing command called");
    let home_dir = dirs::home_dir().ok_or("Could not find home directory")?;
    println!("Home directory: {:?}", home_dir);

    // Spawn a background task for indexing
    tauri::async_runtime::spawn(async move {
        println!("Starting background indexing task...");
        if let Err(e) = index_directory(&home_dir, true).await {
            eprintln!("Indexing error: {}", e);
        }
        println!("Background indexing task completed");
    });

    Ok("Indexing started in background".to_string())
}

#[tauri::command]
async fn index_custom_folder(path: String, _state: State<'_, AppState>) -> Result<String, String> {
    println!("index_custom_folder command called with path: {}", path);
    let folder_path = PathBuf::from(&path);
    
    if !folder_path.exists() {
        return Err("Folder does not exist".to_string());
    }
    
    if !folder_path.is_dir() {
        return Err("Path is not a directory".to_string());
    }

    // Spawn a background task for indexing (don't clear existing files)
    tauri::async_runtime::spawn(async move {
        println!("Starting background indexing for custom folder...");
        if let Err(e) = index_directory(&folder_path, false).await {
            eprintln!("Indexing error: {}", e);
        }
        println!("Background indexing for custom folder completed");
    });

    Ok(format!("Indexing folder: {}", path))
}

async fn index_directory(path: &Path, clear_existing: bool) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("file-finder")
        .join("index.db");

    let mut conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to open database: {}", e);
            return Err(Box::new(e));
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

    // Get or create directory entry
    let root_dir_str = path.to_string_lossy().to_string();
    
    // Check if directory is already indexed
    let already_indexed: bool = conn.query_row(
        "SELECT COUNT(*) FROM indexed_directories WHERE path = ?1",
        [&root_dir_str],
        |row| row.get::<_, i32>(0).map(|count| count > 0)
    ).unwrap_or(false);
    
    if clear_existing {
        // Full reindex - clear all files from this directory
        if let Err(e) = conn.execute("DELETE FROM files WHERE root_directory = ?1", [&root_dir_str]) {
            eprintln!("Failed to clear existing files for directory: {}", e);
            return Ok(());
        }
        println!("Cleared existing index for directory: {}, starting fresh...", root_dir_str);
    } else if already_indexed {
        // Incremental update - keep existing files, only add new ones
        println!("Directory already indexed: {}, will add new files only...", root_dir_str);
    } else {
        // First time indexing this directory
        println!("First time indexing directory: {}", root_dir_str);
    }

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    
    // Add or update the directory in indexed_directories table
    let dir_name = if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        name.to_string()
    } else {
        // Handle root paths like C:\ or /
        root_dir_str.clone()
    };
    
    if let Err(e) = conn.execute(
        "INSERT OR REPLACE INTO indexed_directories (path, name, indexed_at, is_active) VALUES (?1, ?2, ?3, 1)",
        params![&root_dir_str, &dir_name, now],
    ) {
        eprintln!("Failed to save indexed directory: {}", e);
    }
    
    // Set all other directories as inactive
    if let Err(e) = conn.execute(
        "UPDATE indexed_directories SET is_active = 0 WHERE path != ?1",
        [&root_dir_str],
    ) {
        eprintln!("Failed to update directory status: {}", e);
    }

    println!("Collecting files...");
    
    // Use HashSet for in-memory duplicate detection
    let mut seen_paths: HashSet<String> = HashSet::new();
    
    // If incremental update, load existing paths from database
    if !clear_existing && already_indexed {
        println!("Loading existing files from database...");
        match conn.prepare("SELECT path FROM files WHERE root_directory = ?1") {
            Ok(mut stmt) => {
                match stmt.query_map([&root_dir_str], |row| row.get::<_, String>(0)) {
                    Ok(rows) => {
                        for path_result in rows {
                            if let Ok(path) = path_result {
                                seen_paths.insert(path);
                            }
                        }
                        println!("Loaded {} existing files, will skip them...", seen_paths.len());
                    }
                    Err(e) => eprintln!("Failed to query existing paths: {}", e)
                }
            }
            Err(e) => eprintln!("Failed to prepare query: {}", e)
        }
    }
    
    // Collect all entries first (this is I/O bound and relatively fast)
    let entries: Vec<(String, String, Option<i64>)> = WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // Skip hidden directories and common ignore patterns, but allow dotfiles
            let file_name = e.file_name().to_string_lossy();
            let is_dir = e.file_type().is_dir();
            
            // Skip hidden directories like .git, .vscode, etc. but allow dotfiles like .dockerignore, .gitignore
            let should_skip_hidden = file_name.starts_with('.') && is_dir && 
                !file_name.eq(".") && !file_name.eq("..");
            
            !should_skip_hidden
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
                    // Get file modification time
                    let modified_at = entry.metadata()
                        .ok()
                        .and_then(|metadata| metadata.modified().ok())
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|duration| duration.as_secs() as i64);
                    
                    seen_paths.insert(path_str.to_string());
                    return Some((path_str.to_string(), name.to_string(), modified_at));
                }
            }
            None
        })
        .collect();

    let total_count = entries.len();
    
    if total_count == 0 {
        println!("No new files to index.");
        return Ok(());
    }
    
    println!("Found {} new items to insert into database...", total_count);

    // Start a transaction for bulk insert
    let tx = match conn.transaction() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to start transaction: {}", e);
            return Ok(());
        }
    };

    // Use prepared statement for better performance
    // INSERT OR IGNORE handles any edge case duplicates at DB level (extra safety)
    let mut stmt = match tx.prepare("INSERT OR IGNORE INTO files (path, name, root_directory, indexed_at, modified_at) VALUES (?1, ?2, ?3, ?4, ?5)") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to prepare statement: {}", e);
            return Ok(());
        }
    };

    // Insert all entries
    let mut inserted_count = 0;
    for (idx, (path_str, name, modified_at)) in entries.iter().enumerate() {
        if let Ok(rows_changed) = stmt.execute(params![path_str, name, &root_dir_str, now, modified_at]) {
            if rows_changed > 0 {
                inserted_count += 1;
            }
        }
        
        if (idx + 1) % 10000 == 0 {
            println!("Processed {} / {} items...", idx + 1, total_count);
        }
    }

    drop(stmt);

    // Commit the transaction
    if let Err(e) = tx.commit() {
        eprintln!("Failed to commit transaction: {}", e);
        return Ok(());
    }

    println!("Indexing complete! Added {} new files (skipped {} existing)", inserted_count, total_count - inserted_count);

        // Show total number of files indexed in the database
        match conn.query_row(
            "SELECT COUNT(*) FROM files WHERE root_directory = ?1",
            [&root_dir_str],
            |row| row.get::<_, i64>(0)
        ) {
            Ok(count) => println!("Total files indexed for '{}': {}", root_dir_str, count),
            Err(e) => eprintln!("Failed to count indexed files: {}", e),
        }

        // Create FTS5 virtual table for fast full-text search
        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(name, path, content='files', content_rowid='id');",
            [],
        )?;
        // Sync FTS table with files table (for new/updated files)
        conn.execute(
            "INSERT INTO files_fts(rowid, name, path) SELECT id, name, path FROM files WHERE id NOT IN (SELECT rowid FROM files_fts);",
            [],
        )?;
    
    Ok(())
}

// Helper function to normalize strings by removing separators for better matching
fn normalize_for_matching(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn fuzzy_search_files(files: Vec<(String, String)>, query: &str, recent: &[String], favorites: &[String], options: &SearchOptions) -> Vec<(i64, FileEntry)> {
    // New smarter search:
    // - Tokenize the query by whitespace
    // - Prefer ordered substring matches in filename first, then in the joined path components
    // - Give a strong boost for contiguous (exact substring) matches
    // - Fall back to fuzzy matching only when ordered substring checks fail, and require a reasonable score threshold
    let matcher = SkimMatcherV2::default();
    let mut results: Vec<(i64, FileEntry)> = Vec::with_capacity(1000);

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

    // Early termination for fuzzy search - process more files if query looks like it needs normalization
    let search_limit = if query_normalized != query_trimmed.to_lowercase().replace(' ', "") { 1000 } else { 300 };
    for (path, name) in files.into_iter().take(search_limit) {
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
        
        // Check for exact filename match first (highest priority)
        let is_exact_match = name_l == query_trimmed.to_lowercase();
        if is_exact_match {
            best_score = 10000; // Exact match gets highest score
            matched_filename = true;
        }
        
        let query_has_extension = query_trimmed.contains('.');
        
        // Only continue with other matching strategies if not an exact match
        if !is_exact_match {
            // 1a) Normalized filename matching (ignores spaces, hyphens, underscores, dots)
            // This allows "gre word" to match "grewordlist.txt" and "finduname" to match "find-uname.py"
            // BUT: If query contains a dot (file extension), skip normalized matching to avoid false matches
            // (e.g., "lib.rs" normalized to "librs" would match "contextlib.rst" normalized to "contextlibrst")
            if !query_has_extension && !query_normalized.is_empty() && name_normalized.contains(&query_normalized) {
                let mut score: i64 = 2900; // High score for normalized match
                // Bonus if it's at the start
                if name_normalized.starts_with(&query_normalized) {
                    score += 500;
                }
                matched_filename = true;
                best_score = score;
            }
            
            // 1b) Token-based ordered substring matching (stricter but gives higher score)
            // If query has extension, require the full query as a substring (not just tokens in order)
            if query_has_extension {
                // For queries with extensions (e.g., "lib.rs"), check substring match
                let query_lower = query_trimmed.to_lowercase();
                if name_l.contains(&query_lower) {
                    let mut score: i64 = 3000; // Base score for substring match with extension
                    
                    // Much higher score if the query matches the entire filename
                    if name_l == query_lower {
                        score = 9500; // Almost as good as exact match
                    }
                    // Bonus if at the start of filename
                    else if name_l.starts_with(&query_lower) {
                        score += 1500;
                    }
                    // Bonus if the match is at a word boundary (after a separator)
                    else if name_l.contains(&format!("/{}", query_lower)) || 
                            name_l.contains(&format!("\\{}", query_lower)) ||
                            name_l.contains(&format!("-{}", query_lower)) ||
                            name_l.contains(&format!("_{}", query_lower)) {
                        score += 800;
                    }
                    
                    if score > best_score {
                        best_score = score;
                    }
                    matched_filename = true;
                }
            } else if let Some(bonus) = in_order_in(&name_l) {
                // No extension in query, use token-based matching
                // Check strict mode
                if options.strict_mode {
                    // In strict mode, only allow exact or prefix matches
                    let is_prefix = name_l.starts_with(&query_trimmed.to_lowercase());
                    if is_prefix {
                        let contiguous = name_l.contains(query_trimmed);
                        let mut score: i64 = 3000 + bonus;
                        if contiguous {
                            score += 1200;
                        }
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
                    if score > best_score {
                        best_score = score;
                    }
                    matched_filename = true;
                }
            }
        }
        
        // If we matched the filename via any method, add it to results
        if matched_filename {
            // Deprioritize library/build directories (but NOT for exact matches)
            if is_in_library_dir && !is_exact_match {
                best_score = best_score / 4;
            }
            // Boost for recent and favorite files
            if recent.contains(&path) { best_score *= 2; }
            if favorites.contains(&path) { best_score *= 3; } // Favorites get 3x boost
            results.push((best_score, FileEntry { path: path.clone(), name, last_accessed: None, access_count: 0, modified_at: None }));
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
                if favorites.contains(&path) { score *= 3; }
                results.push((score, FileEntry { path: path.clone(), name, last_accessed: None, access_count: 0, modified_at: None }));
                continue;
            }
        }

        // 3) Weak fuzzy fallback (lower priority) - only if fuzzy is enabled
        // Skip fuzzy matching for queries with file extensions (e.g., "lib.rs")
        // to avoid false matches like "contextlib.rst"
        if options.enable_fuzzy && !options.strict_mode && !query_has_extension {
            if let Some(fuzzy_score) = matcher.fuzzy_match(&name, query_trimmed) {
                // require threshold to prevent everything matching; scale down for file-name fuzzy
                if fuzzy_score >= 60 {
                    let mut score = (fuzzy_score as i64) + 500; // base bump
                    // Deprioritize library/build directories
                    if is_in_library_dir {
                        score = score / 4; // Significantly reduce score for library files
                    }
                    if recent.contains(&path) { score *= 2; }
                    if favorites.contains(&path) { score *= 3; }
                    results.push((score, FileEntry { path: path.clone(), name, last_accessed: None, access_count: 0, modified_at: None }));
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
                        if favorites.contains(&path) { score *= 3; }
                        results.push((score, FileEntry { path: path.clone(), name, last_accessed: None, access_count: 0, modified_at: None }));
                    }
                }
            }
        }
    }

    results
}

#[tauri::command]
async fn search_files(query: String, options: Option<SearchOptions>, state: State<'_, AppState>) -> Result<Vec<FileEntry>, String> {
    let search_opts = options.unwrap_or_default();
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    // NEW: LLM-Enhanced Query Processing
    // Check if query looks natural language vs technical
    let is_natural_language = query.contains(" ") && 
                             (query.to_lowercase().contains("find") || 
                              query.to_lowercase().contains("show") ||
                              query.to_lowercase().contains("where") ||
                              query.to_lowercase().contains("recent") ||
                              query.to_lowercase().contains("python") ||
                              query.to_lowercase().contains("react") ||
                              query.to_lowercase().contains("config"));
    
    let (enhanced_query, parsed_query) = if is_natural_language {
        match state.llm_processor.parse_natural_query(&query).await {
            Ok(parsed) => {
                println!("LLM PARSED QUERY: {:?}", parsed);
                if parsed.confidence > 0.7 {
                    let converted_query = state.llm_processor.convert_to_search_query(&parsed);
                    (converted_query, Some(parsed))
                } else {
                    (query.clone(), None)
                }
            }
            Err(e) => {
                println!("LLM parsing failed: {}", e);
                (query.clone(), None)
            }
        }
    } else {
        (query.clone(), None)
    };

    // Special handling for time-based queries
    if let Some(ref parsed) = parsed_query {
        if parsed.intent == llm_integration::QueryIntent::FindByTime {
            if enhanced_query.trim().is_empty() {
                println!("Detected time-based query for all files, returning recent files");
                return get_recent_files(state).await;
            } else {
                println!("Detected time-based query with filters: '{}'", enhanced_query);
                // Will use timestamp filtering in the search logic below
            }
        }
    }

    // Check cache first (for exact queries, cache for 30 seconds)
    let cache_key = format!("{}:{:?}", enhanced_query, search_opts);
    {
        let mut cache = state.search_cache.lock().map_err(|e| e.to_string())?;
        
        // Clean old entries (simple cleanup - remove entries older than 60 seconds)
        cache.retain(|_, (timestamp, _)| timestamp.elapsed().as_secs() < 60);
        
        // Check for cached result
        if let Some((timestamp, cached_results)) = cache.get(&cache_key) {
            if timestamp.elapsed().as_secs() < 30 {
                println!("CACHE HIT: Returning {} cached results for '{}'", cached_results.len(), query);
                return Ok(cached_results.clone());
            }
        }
    }

    let (files, recent, favorites) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;

        // Intelligent pattern analysis and optimization
        let pattern_info = analyze_regex_pattern(&enhanced_query);
        println!("PATTERN ANALYSIS for '{}': {:?}", enhanced_query, pattern_info);
        
        // EMERGENCY SAFETY: Quick return for very short queries to avoid massive searches
    if query.len() < 2 {
        println!("Query too short ({}), returning empty results", query.len());
        return Ok(vec![]);
    }

    // ENHANCED SEARCH: Handle "file finder" → "file-finder" and similar patterns
    let normalized_query = query.to_lowercase();
    let is_file_finder_search = normalized_query.contains("file finder") || 
                               normalized_query.contains("file-finder") || 
                               normalized_query.contains("file_finder") ||
                               normalized_query == "file finder" ||
                               normalized_query == "file-finder";
    
    // Create alternative search patterns for hyphen/space/underscore variations
    let alternative_patterns = if normalized_query.contains(' ') {
        // "file finder" → also search for "file-finder", "file_finder"
        vec![
            query.replace(' ', "-"),
            query.replace(' ', "_"),
            query.replace(' ', "")
        ]
    } else if normalized_query.contains('-') {
        // "file-finder" → also search for "file finder", "file_finder"  
        vec![
            query.replace('-', " "),
            query.replace('-', "_"),
            query.replace('-', "")
        ]
    } else {
        vec![]
    };
    
    if is_file_finder_search {
        println!("Detected file-finder search for '{}', will search alternatives: {:?}", query, alternative_patterns);
    }

    // SEARCH FILES - use optimized strategy based on pattern analysis
        let files: Vec<(String, String, Option<i64>)> = if pattern_info.can_use_sql_optimization {
            // OPTIMIZED PATH: Use SQL LIKE for pre-filtering
            let start_time = Instant::now();
            
            if let Some(sql_pattern) = &pattern_info.sql_like_pattern {
                let (base_query_sql, limit) = match pattern_info.pattern_type {
                    PatternType::SimpleGlob if pattern_info.suffix.is_some() => {
                        // EMERGENCY FIX: Ultra-low limits for 1.5M files
                        ("SELECT path, name, modified_at FROM files WHERE name LIKE ?1", 50)
                    },
                    PatternType::SimplePrefix => {
                        // EMERGENCY FIX: Much smaller limit
                        ("SELECT path, name, modified_at FROM files WHERE name LIKE ?1", 100)
                    },
                    PatternType::LiteralSearch if query.contains(' ') => {
                        // EMERGENCY FIX: Tiny limit for multi-word searches
                        ("SELECT path, name, modified_at FROM files WHERE LOWER(name) LIKE LOWER(?1)", 30)
                    },
                    _ => {
                        // EMERGENCY FIX: Minimal limit for other patterns
                        ("SELECT path, name, modified_at FROM files WHERE LOWER(name) LIKE LOWER(?1)", 25)
                    }
                };
                
                // Apply time constraints and ordering
                let query_sql = if let Some(ref parsed) = parsed_query {
                    if let Some(ref time_constraint) = parsed.time_constraint {
                        match time_constraint.as_str() {
                            "recent" | "today" => {
                                // Files modified in last 7 days, ordered by modification time
                                let seven_days_ago = chrono::Utc::now().timestamp() - (7 * 24 * 60 * 60);
                                format!("{} AND modified_at > {} ORDER BY modified_at DESC LIMIT ?2", base_query_sql, seven_days_ago)
                            }
                            "yesterday" => {
                                // Files modified yesterday
                                let yesterday = chrono::Utc::now().timestamp() - (24 * 60 * 60);
                                format!("{} AND modified_at > {} ORDER BY modified_at DESC LIMIT ?2", base_query_sql, yesterday)
                            }
                            "last_week" => {
                                // Files modified in last week
                                let last_week = chrono::Utc::now().timestamp() - (7 * 24 * 60 * 60);
                                format!("{} AND modified_at > {} ORDER BY modified_at DESC LIMIT ?2", base_query_sql, last_week)
                            }
                            _ => format!("{} ORDER BY length(name) LIMIT ?2", base_query_sql)
                        }
                    } else {
                        // Regular ordering for non-time queries
                        match pattern_info.pattern_type {
                            PatternType::SimplePrefix => format!("{} ORDER BY CASE WHEN name LIKE ?1 THEN 0 ELSE 1 END, length(name) LIMIT ?2", base_query_sql),
                            _ => if is_file_finder_search {
                                format!("{} ORDER BY CASE WHEN name LIKE '%.exe' THEN 0 WHEN name LIKE '%.bat' THEN 1 ELSE 2 END, length(name) LIMIT ?2", base_query_sql)
                            } else {
                                format!("{} ORDER BY length(name) LIMIT ?2", base_query_sql)
                            }
                        }
                    }
                } else {
                    // Regular ordering for non-natural language queries
                    match pattern_info.pattern_type {
                        PatternType::SimplePrefix => format!("{} ORDER BY CASE WHEN name LIKE ?1 THEN 0 ELSE 1 END, length(name) LIMIT ?2", base_query_sql),
                        _ => if is_file_finder_search {
                            format!("{} ORDER BY CASE WHEN name LIKE '%.exe' THEN 0 WHEN name LIKE '%.bat' THEN 1 ELSE 2 END, length(name) LIMIT ?2", base_query_sql)
                        } else {
                            format!("{} ORDER BY length(name) LIMIT ?2", base_query_sql)
                        }
                    }
                };
                
                let mut stmt = db.prepare(&query_sql).map_err(|e| e.to_string())?;
                let mut results: Vec<(String, String, Option<i64>)> = stmt.query_map([sql_pattern, &limit.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                    .map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                
                // ENHANCED: Search for alternative patterns (hyphen/space/underscore variations)
                if !alternative_patterns.is_empty() {
                    for alt_pattern in &alternative_patterns {
                        if let Some(alt_sql_pattern) = pattern_info.sql_like_pattern.as_ref() {
                            let alt_sql = alt_sql_pattern.replace(&query, alt_pattern);
                            let alt_query_sql = query_sql.replace(sql_pattern, &alt_sql);
                            
                            if let Ok(mut alt_stmt) = db.prepare(&alt_query_sql) {
                                if let Ok(mapped_rows) = alt_stmt.query_map([&alt_sql, &limit.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))) {
                                    let alt_results: Vec<(String, String, Option<i64>)> = mapped_rows
                                        .filter_map(|r| r.ok())
                                        .collect();
                                
                                    // Add alternative results, avoiding duplicates
                                    for alt_result in alt_results {
                                        if !results.iter().any(|r| r.0 == alt_result.0) {
                                            results.push(alt_result);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    // DIRECTORY SEARCH: Look for files in directories with the target name
                    if is_file_finder_search {
                        let dir_search_sql = "SELECT path, name, modified_at FROM files WHERE path LIKE ? LIMIT 10";
                        if let Ok(mut dir_stmt) = db.prepare(dir_search_sql) {
                            for pattern in std::iter::once(&query).chain(alternative_patterns.iter()) {
                                let dir_pattern = format!("%{}%", pattern);
                                if let Ok(mapped_rows) = dir_stmt.query_map([&dir_pattern], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))) {
                                    let dir_results: Vec<(String, String, Option<i64>)> = mapped_rows
                                        .filter_map(|r| r.ok())
                                        .collect();
                                
                                    // Add directory-based results
                                    for dir_result in dir_results {
                                        if !results.iter().any(|r| r.0 == dir_result.0) {
                                            results.push(dir_result);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let duration = start_time.elapsed();
                println!("OPTIMIZED SQL: Pattern '{}' → SQL '{}' found {} files in {}ms", 
                         query, sql_pattern, results.len(), duration.as_millis());
                results
            } else {
                vec![]
            }
        } else {
            // COMPLEX REGEX PATH: Emergency performance fix - drastically reduce limits
            let start_time = Instant::now();
            let limit = if pattern_info.prefix.is_some() { 100 } else { 50 }; // MUCH smaller limits!
            
            let mut stmt = db
                .prepare(&format!("SELECT path, name, modified_at FROM files LIMIT {}", limit))
                .map_err(|e| e.to_string())?;
            let results: Vec<(String, String, Option<i64>)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            let duration = start_time.elapsed();
            println!("COMPLEX REGEX: Loaded {} files for pattern '{}' in {}ms", results.len(), query, duration.as_millis());
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

        // Get favorite files for boost
        let mut fav_stmt = db
            .prepare("SELECT path FROM favorite_files")
            .map_err(|e| e.to_string())?;

        let favorites: Vec<String> = fav_stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        (files, recent, favorites)
    }; // Database lock is automatically released here

    // ENHANCED: For multi-word queries, try SQLite FTS5 first for better results
    // BUT: Skip FTS5 for date-like patterns (e.g., "Harry 07312025") and use fuzzy search instead
    let looks_like_date_query = query.chars().any(|c| c.is_ascii_digit()) && 
                               query.split_whitespace().any(|word| word.chars().all(|c| c.is_ascii_digit()) && word.len() >= 6);
    
    if query.contains(' ') && !query.contains('.') && !looks_like_date_query { // Multi-word queries without file extensions or dates
        println!("Multi-word query detected, trying SQLite FTS5 search for: '{}'", query);
        
        let fts_results = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            
            // First check if FTS table exists and has data
            let fts_count: i64 = match db.query_row("SELECT COUNT(*) FROM files_fts", [], |row| row.get(0)) {
                Ok(count) => count,
                Err(e) => {
                    println!("FTS5 table check failed: {}, will populate it", e);
                    // Try to create and populate FTS5 table
                    if let Err(create_err) = db.execute(
                        "CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(name, path, content='files', content_rowid='id');",
                        [],
                    ) {
                        println!("Failed to create FTS5 table: {}", create_err);
                        0
                    } else if let Err(populate_err) = db.execute(
                        "INSERT INTO files_fts(rowid, name, path) SELECT id, name, path FROM files WHERE id NOT IN (SELECT rowid FROM files_fts);",
                        [],
                    ) {
                        println!("Failed to populate FTS5 table: {}", populate_err);
                        0
                    } else {
                        // Get count after population
                        db.query_row("SELECT COUNT(*) FROM files_fts", [], |row| row.get(0)).unwrap_or(0)
                    }
                }
            };
            
            println!("FTS5 table has {} entries", fts_count);
            
            if fts_count == 0 {
                println!("FTS5 table is empty, skipping FTS search");
                Vec::new()
            } else {
                let fts_query = format!("{}*", query.replace(' ', "* "));
                println!("FTS5 query: '{}'", fts_query);
                
                let fts_sql = "SELECT files.path, files.name, files.modified_at 
                              FROM files_fts 
                              JOIN files ON files_fts.rowid = files.id 
                              WHERE files_fts MATCH ? 
                              ORDER BY rank 
                              LIMIT 100";
                
                match db.prepare(fts_sql) {
                    Ok(mut fts_stmt) => {
                        match fts_stmt.query_map([&fts_query], |row| {
                            Ok(FileEntry {
                                path: row.get::<_, String>(0)?,
                                name: row.get::<_, String>(1)?,
                                modified_at: row.get::<_, Option<i64>>(2)?,
                                last_accessed: None,
                                access_count: 0,
                            })
                        }) {
                            Ok(fts_rows) => {
                                let mut fts_results = Vec::new();
                                for row in fts_rows {
                                    if let Ok(r) = row {
                                        fts_results.push(r);
                                    }
                                }
                                println!("FTS5 raw query returned {} results", fts_results.len());
                                fts_results
                            }
                            Err(e) => {
                                println!("Failed to execute FTS5 query: {}", e);
                                Vec::new()
                            }
                        }
                    }
                    Err(e) => {
                        println!("Failed to prepare FTS5 query: {}", e);
                        Vec::new()
                    }
                }
            }
        };
        
        if !fts_results.is_empty() {
            println!("SQLite FTS5 found {} results for multi-word query", fts_results.len());
            
            // Cache the results
            {
                let mut cache = state.search_cache.lock().map_err(|e| e.to_string())?;
                if cache.len() >= 100 {
                    let oldest_key = cache.iter()
                        .min_by_key(|(_, (timestamp, _))| timestamp)
                        .map(|(key, _)| key.clone());
                    if let Some(key) = oldest_key {
                        cache.remove(&key);
                    }
                }
                cache.insert(cache_key, (Instant::now(), fts_results.clone()));
            }
            
            return Ok(fts_results);
        } else {
            println!("SQLite FTS5 found no results, falling back to fuzzy search with normalization");
            
            // For multi-word queries that failed FTS5, try fuzzy search with normalization
            // This handles cases like "Harry 07312025" matching "Harry_07-31-2025"
            let files_2tuple: Vec<(String, String)> = files.clone().into_iter().map(|(path, name, _)| (path, name)).collect();
            let fuzzy_results = fuzzy_search_files(files_2tuple, &query, &recent, &favorites, &search_opts);
            
            if !fuzzy_results.is_empty() {
                println!("Fuzzy search found {} results for multi-word query", fuzzy_results.len());
                
                // Cache and return fuzzy results
                let fuzzy_entries: Vec<FileEntry> = fuzzy_results.into_iter().map(|(_, entry)| entry).collect();
                {
                    let mut cache = state.search_cache.lock().map_err(|e| e.to_string())?;
                    if cache.len() >= 100 {
                        let oldest_key = cache.iter()
                            .min_by_key(|(_, (timestamp, _))| timestamp)
                            .map(|(key, _)| key.clone());
                        if let Some(key) = oldest_key {
                            cache.remove(&key);
                        }
                    }
                    cache.insert(cache_key, (Instant::now(), fuzzy_entries.clone()));
                }
                return Ok(fuzzy_entries);
            }
        }
    } else if looks_like_date_query {
        println!("Date-like query detected ('{}'), using enhanced search with normalization", query);
        
        // For date-like queries, first try to pre-filter files that might match
        let words: Vec<&str> = query.split_whitespace().collect();
        let first_word = words.get(0).unwrap_or(&query.as_str()).to_lowercase();
        
        // Pre-filter files that contain the first word (usually the name part)
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let prefiltered_files: Vec<(String, String)> = {
            let mut stmt = db.prepare("SELECT path, name FROM files WHERE LOWER(name) LIKE ? LIMIT 2000")
                .map_err(|e| e.to_string())?;
            let pattern = format!("%{}%", first_word);
            let results: Vec<(String, String)> = stmt.query_map([&pattern], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            results
        };
        
        println!("Pre-filtered {} files containing '{}' for date-like search", prefiltered_files.len(), first_word);
        
        // Now apply fuzzy search with normalization to the pre-filtered set
        let fuzzy_results = fuzzy_search_files(prefiltered_files, &query, &recent, &favorites, &search_opts);
        
        if !fuzzy_results.is_empty() {
            println!("Fuzzy search found {} results for date-like query", fuzzy_results.len());
            
            // Cache and return fuzzy results
            let fuzzy_entries: Vec<FileEntry> = fuzzy_results.into_iter().map(|(_, entry)| entry).collect();
            {
                let mut cache = state.search_cache.lock().map_err(|e| e.to_string())?;
                if cache.len() >= 100 {
                    let oldest_key = cache.iter()
                        .min_by_key(|(_, (timestamp, _))| timestamp)
                        .map(|(key, _)| key.clone());
                    if let Some(key) = oldest_key {
                        cache.remove(&key);
                    }
                }
                cache.insert(cache_key, (Instant::now(), fuzzy_entries.clone()));
            }
            return Ok(fuzzy_entries);
        }
    }

    // Analyze the query pattern using our unified pattern analyzer
    let pattern_info = analyze_regex_pattern(&query);
    
    println!("Pattern analysis for '{}': type={:?}, can_use_sql={}, prefix={:?}, suffix={:?}", 
             query, pattern_info.pattern_type, pattern_info.can_use_sql_optimization, 
             pattern_info.prefix, pattern_info.suffix);
    
    // Process files based on pattern analysis
    let mut results: Vec<(i64, FileEntry)> = match pattern_info.pattern_type {
        PatternType::SimplePrefix => {
            // For simple prefix patterns like "log*" or "^log.*"
            let prefix = pattern_info.prefix.as_deref().unwrap_or("");
            println!("Processing {} files for simple prefix pattern '{}'", files.len(), prefix);
            
            let mut exact_results: Vec<(i64, FileEntry)> = files.into_iter()
                .take(200) // Early termination for 1.5M files - stop after 200 good results
                .map(|(path, name, modified_at)| {
                    let prefix = pattern_info.prefix.as_deref().unwrap_or("");
                    let name_lower = name.to_lowercase();
                    let prefix_lower = prefix.to_lowercase();
                    
                    let mut score = if name_lower == prefix_lower {
                        15000 // Exact filename match - highest priority!
                    } else {
                        // Check if prefix matches filename without extension
                        let name_without_ext = if let Some(dot_pos) = name_lower.rfind('.') {
                            &name_lower[..dot_pos]
                        } else {
                            &name_lower
                        };
                        
                        if name_without_ext == prefix_lower {
                            14000 // Exact match without extension - very high priority!
                        } else {
                            5000 // Regular prefix match
                        }
                    };
                
                    // Boost if file is recent or favorite
                    if recent.contains(&path) {
                        score += 1000;
                    }
                    if favorites.contains(&path) {
                        score += 2000;
                    }
                    
                    (score, FileEntry {
                        path,
                        name,
                        last_accessed: None,
                        access_count: 0,
                        modified_at,
                    })
                })
            .collect();

            
            // Skip expensive fuzzy search fallback for 1.5M files performance
            if false && exact_results.len() < 50 && prefix.len() >= 3 {
                println!("Adding fuzzy search for broader coverage");
                
                let fuzzy_files: Vec<(String, String, Option<i64>)> = {
                    let db = state.db.lock().map_err(|e| e.to_string())?;
                    let mut stmt = db
                        .prepare("SELECT path, name, modified_at FROM files WHERE name LIKE ?1 OR path LIKE ?2 LIMIT 2000")
                        .map_err(|e| e.to_string())?;
                    let broad_pattern = format!("%{}%", prefix);
                    let results: Vec<(String, String, Option<i64>)> = stmt.query_map([&broad_pattern, &broad_pattern], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                        .map_err(|e| e.to_string())?
                        .filter_map(|r| r.ok())
                        .collect();
                    results
                };
            
                
                let fuzzy_results: Vec<(i64, FileEntry)> = fuzzy_files.into_iter()
                    .filter_map(|(path, name, modified_at)| {
                        if exact_results.iter().any(|(_, entry)| entry.path == path) {
                            return None;
                        }
                        
                        let name_score = fuzzy_match_score(&name.to_lowercase(), &prefix.to_lowercase());
                        let path_score = fuzzy_match_score(&path.to_lowercase(), &prefix.to_lowercase());
                        let best_score = name_score.max(path_score);
                        
                        if best_score > 0.6 {
                            let mut score = (best_score * 3000.0) as i64;
                            
                            if recent.contains(&path) {
                                score += 1000;
                            }
                            if favorites.contains(&path) {
                                score += 2000;
                            }
                            
                            Some((score, FileEntry {
                                path,
                                name,
                                last_accessed: None,
                                access_count: 0,
                                modified_at,
                            }))
                        } else {
                            None
                        }
                    })
                    .collect();
            
                
                println!("Added {} fuzzy matches to {} exact matches", fuzzy_results.len(), exact_results.len());
                exact_results.extend(fuzzy_results);
            }
            
            exact_results
        }
        
        PatternType::SimpleGlob | PatternType::PrefixSuffix | PatternType::ComplexRegex => {
            // For patterns that need regex matching
            let regex_pattern = match pattern_info.pattern_type {
                PatternType::SimpleGlob => build_glob_regex(&query),
                PatternType::PrefixSuffix => {
                    if query.starts_with('^') {
                        query.to_string()
                    } else {
                        format!("^{}$", query)
                    }
                }
                PatternType::ComplexRegex => {
                    if query.starts_with('/') && query.ends_with('/') {
                        query[1..query.len()-1].to_string()
                    } else {
                        query.to_string()
                    }
                }
                _ => unreachable!()
            };
            
            println!("Processing {} files with regex '{}' for pattern type {:?}", 
                     files.len(), regex_pattern, pattern_info.pattern_type);
            
            // Check regex cache first, then compile if needed
            let re = {
                let mut regex_cache = state.regex_cache.lock().map_err(|e| e.to_string())?;
                
                // Clean cache if it gets too large (keep only 50 recent patterns)
                if regex_cache.len() > 50 {
                    regex_cache.clear();
                }
                
                if let Some(cached_regex) = regex_cache.get(&regex_pattern) {
                    println!("REGEX CACHE HIT for pattern '{}'", regex_pattern);
                    cached_regex.clone()
                } else {
                    match Regex::new(&regex_pattern) {
                        Ok(new_regex) => {
                            regex_cache.insert(regex_pattern.clone(), new_regex.clone());
                            println!("REGEX COMPILED and cached for pattern '{}'", regex_pattern);
                            new_regex
                        }
                        Err(e) => {
                            println!("Invalid regex '{}': {}", regex_pattern, e);
                            let files_2tuple: Vec<(String, String)> = files.into_iter().map(|(path, name, _)| (path, name)).collect();
                            let fuzzy_results = fuzzy_search_files(files_2tuple, &query, &recent, &favorites, &search_opts);
                            return Ok(fuzzy_results.into_iter().map(|(_, entry)| entry).collect());
                        }
                    }
                }
            };
            
            // Now use the cached/compiled regex
            // Use parallel processing for large file sets (>1000 files) with early termination
            let matched_files: Vec<(i64, FileEntry)> = if files.len() > 1000 {
                files.into_par_iter()
                    .take(300) // Early termination - only process first 300 files for regex
                    .filter_map(|(path, name, modified_at)| {
                        if re.is_match(&name) || re.is_match(&path) {
                            let name_lower = name.to_lowercase();
                            let query_lower = query.to_lowercase();
                            
                            let mut score = if name_lower == query_lower {
                                15000 // Exact filename match - highest priority!
                            } else {
                                // Check if query matches filename without extension
                                let name_without_ext = if let Some(dot_pos) = name_lower.rfind('.') {
                                    &name_lower[..dot_pos]
                                } else {
                                    &name_lower
                                };
                                
                                if name_without_ext == query_lower {
                                    14000 // Exact match without extension - very high priority!
                                } else {
                                    4000 // Regular regex match
                                }
                            };
                            
                            if recent.contains(&path) {
                                score += 1000;
                            }
                            if favorites.contains(&path) {
                                score += 2000;
                            }
                            
                            Some((score, FileEntry {
                                path,
                                name,
                                last_accessed: None,
                                access_count: 0,
                                modified_at,
                            }))
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                // For smaller sets, sequential processing is faster due to reduced overhead
                files.into_iter()
                    .take(200) // Early termination for sequential processing too
                    .filter_map(|(path, name, modified_at)| {
                        if re.is_match(&name) || re.is_match(&path) {
                            let name_lower = name.to_lowercase();
                            let query_lower = query.to_lowercase();
                            
                            let mut score = if name_lower == query_lower {
                                15000 // Exact filename match - highest priority!
                            } else {
                                // Check if query matches filename without extension
                                let name_without_ext = if let Some(dot_pos) = name_lower.rfind('.') {
                                    &name_lower[..dot_pos]
                                } else {
                                    &name_lower
                                };
                                
                                if name_without_ext == query_lower {
                                    14000 // Exact match without extension - very high priority!
                                } else {
                                    4000 // Regular regex match
                                }
                            };
                            
                            if recent.contains(&path) {
                                score += 1000;
                            }
                            if favorites.contains(&path) {
                                score += 2000;
                            }
                            
                            Some((score, FileEntry {
                                path,
                                name,
                                last_accessed: None,
                                access_count: 0,
                                modified_at,
                            }))
                        } else {
                            None
                        }
                    })
                    .collect()
            };
            
            println!("Regex matched {} files", matched_files.len());
            
            // Add fuzzy search fallback for complex patterns with few matches
            let mut matched_files = matched_files; // Make mutable for potential extension
            if matches!(pattern_info.pattern_type, PatternType::PrefixSuffix | PatternType::ComplexRegex) && matched_files.len() < 20 {
                let clean_query = query.replace("^", "").replace(".*", "").replace("$", "").replace(r"\.", ".");
                if clean_query.len() >= 3 {
                    println!("Adding fuzzy search fallback for '{}'", clean_query);
                    
                    let files_2tuple: Vec<(String, String)> = {
                        let db = state.db.lock().map_err(|e| e.to_string())?;
                        let mut stmt = db
                            .prepare("SELECT path, name FROM files WHERE name LIKE ?1 OR path LIKE ?2 LIMIT 2000")
                            .map_err(|e| e.to_string())?;
                        let broad_pattern = format!("%{}%", clean_query);
                        let results: Vec<(String, String)> = stmt.query_map([&broad_pattern, &broad_pattern], |row| Ok((row.get(0)?, row.get(1)?)))
                            .map_err(|e| e.to_string())?
                            .filter_map(|r| r.ok())
                            .collect();
                        results
                    };
                    
                    let fuzzy_results = fuzzy_search_files(files_2tuple, &clean_query, &recent, &favorites, &search_opts);
                    
                    for (score, entry) in fuzzy_results {
                        if !matched_files.iter().any(|(_, existing)| existing.path == entry.path) {
                            matched_files.push((score / 2, entry));
                        }
                    }
                    
                    println!("Added fuzzy matches, total now: {}", matched_files.len());
                }
            }
            
            matched_files
        }
        
        PatternType::LiteralSearch => {
            // For simple text searches, use SQL optimization if available, otherwise fuzzy search
            if pattern_info.can_use_sql_optimization && !files.is_empty() {
                println!("Using SQL-optimized literal search for pattern '{}' on {} pre-filtered files", query, files.len());
                // Convert SQL-optimized results to scored FileEntry format with early termination
                files.into_iter()
                    .take(150) // Early termination - only process first 150 SQL-optimized results
                    .map(|(path, name, modified_at)| {
                        // Score based on how well the query matches (case-insensitive substring match)
                        let name_lower = name.to_lowercase();
                        let path_lower = path.to_lowercase();
                        let query_lower = query.to_lowercase();
                        
                        let mut score = if name_lower.contains(&query_lower) {
                            if name_lower == query_lower {
                                15000 // Exact filename match - highest priority!
                            } else {
                                // Check if query matches filename without extension
                                let name_without_ext = if let Some(dot_pos) = name_lower.rfind('.') {
                                    &name_lower[..dot_pos]
                                } else {
                                    &name_lower
                                };
                                
                                if name_without_ext == query_lower {
                                    14000 // Exact match without extension - very high priority!
                                } else if name_lower.starts_with(&query_lower) {
                                    4000 // Starts with query
                                } else {
                                    3000 // Contains query
                                }
                            }
                        } else if path_lower.contains(&query_lower) {
                            2000 // Path contains query
                        } else {
                            // For multi-word queries, check if all words are present in the filename
                            let words: Vec<&str> = query_lower.split_whitespace().collect();
                            if words.len() > 1 {
                                let all_words_in_name = words.iter().all(|word| name_lower.contains(word));
                                let all_words_in_path = words.iter().all(|word| path_lower.contains(word));
                                
                                if all_words_in_name {
                                    // All words found in filename - good match for multi-word queries
                                    2800
                                } else if all_words_in_path {
                                    // All words found in path
                                    1800  
                                } else {
                                    1000 // Partial match
                                }
                            } else {
                                1000 // SQL matched but we're not sure why
                            }
                        };
                        
                        // Boost for recent/favorite files
                        if recent.contains(&path) {
                            score += 1000;
                        }
                        if favorites.contains(&path) {
                            score += 2000;
                        }
                        
                        (score, FileEntry {
                            path,
                            name,
                            last_accessed: None,
                            access_count: 0,
                            modified_at,
                        })
                    })
                    .collect()
            } else {
                println!("Using fuzzy search for literal pattern '{}'", query);
                let files_2tuple: Vec<(String, String)> = files.into_iter().map(|(path, name, _)| (path, name)).collect();
                fuzzy_search_files(files_2tuple, &query, &recent, &favorites, &search_opts)
            }
        }
    };

    // Optimized sorting for 1.5M files - use partial sort for better performance
    let final_results: Vec<FileEntry> = if results.len() > 1000 {
        // For large result sets, use partial sort to get only top 500 results
        let k = 500.min(results.len());
        results.select_nth_unstable_by(k - 1, |a, b| b.0.cmp(&a.0));
        results.into_iter().take(k).map(|(_, entry)| entry).collect()
    } else if results.len() > 100 {
        // For medium result sets, use partial sort to get top 300
        let k = 300.min(results.len());
        results.select_nth_unstable_by(k - 1, |a, b| b.0.cmp(&a.0));
        results.into_iter().take(k).map(|(_, entry)| entry).collect()
    } else {
        // For small result sets, full sort is fine
        results.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        results.into_iter().take(100).map(|(_, entry)| entry).collect()
    };
    
    // Cache the results for future queries (limit cache size to 100 entries)
    {
        let mut cache = state.search_cache.lock().map_err(|e| e.to_string())?;
        if cache.len() >= 100 {
            // Remove oldest entries if cache is full
            let oldest_key = cache.iter()
                .min_by_key(|(_, (timestamp, _))| timestamp)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest_key {
                cache.remove(&key);
            }
        }
        cache.insert(cache_key, (Instant::now(), final_results.clone()));
    }

    Ok(final_results)
}

#[tauri::command]
async fn fzf_search(state: State<'_, AppState>, query: String, limit: Option<usize>) -> Result<Vec<(String, String, Option<i64>)>, String> {
    let fzf_engine = state.fzf_engine.lock().map_err(|e| e.to_string())?;
    
    let search_limit = limit.unwrap_or(50); // Default to 50 results
    let results = fzf_engine.search(&query, search_limit);
    
    println!("FZF Debug: Found {} results for query '{}'", results.len(), query);
    for (i, (score, file)) in results.iter().take(5).enumerate() {
        println!("  Result {}: score={} name='{}' path='{}'", i, score, file.name, file.path);
    }
    
    // Aggressive filtering: eliminate system junk folders unless specifically searching for them
    let filtered_results: Vec<_> = if !query.chars().all(|c| c.is_ascii_digit()) && query != "0" && query != "r" {
        results.into_iter()
            .filter(|(score, file)| {
                // Remove junk system folders for better user experience
                let is_junk_folder = 
                    // UE External folders
                    (file.name == "0" && file.path.contains("__External")) ||
                    // Browser cache/temp folders (numbered folders in browser paths)
                    (file.name.chars().all(|c| c.is_ascii_digit()) && 
                     (file.path.contains("Browser") || file.path.contains("cache") || file.path.contains("morgue"))) ||
                    // Windows WinSxS system folders (single letter folders)
                    (file.name.len() == 1 && file.name.chars().all(|c| c.is_ascii_lowercase()) && 
                     file.path.contains("WinSxS")) ||
                    // Tor Browser temp folders
                    (file.name.chars().all(|c| c.is_ascii_digit()) && file.path.contains("Tor Browser")) ||
                    // Other low-score system junk
                    (*score <= 2000 && 
                     (file.path.contains("UpdateInfo") || file.path.contains("profile.default") || 
                      file.path.contains("uuid+++") || file.name.len() <= 2));
                
                !is_junk_folder
            })
            .collect()
    } else {
        results // Keep all results if specifically searching for numbers/letters
    };
    
    println!("FZF Debug: After filtering: {} results", filtered_results.len());
    
    // Multi-word fallback: if filtering removed all results from multi-word search, try single-word search
    let final_results = if filtered_results.is_empty() && query.contains(' ') {
        println!("FZF Debug: Multi-word search filtered to 0 results, trying single-word fallback");
        
        // Find the longest/most specific word for fallback
        let words: Vec<&str> = query.split_whitespace().collect();
        let query_str = query.as_str();
        let fallback_word = words.iter()
            .max_by_key(|word| word.len())
            .unwrap_or(&query_str);
            
        println!("FZF Debug: Using fallback word '{}' for single-word search", fallback_word);
        
        // Perform single-word search with the most specific term
        let fallback_results = fzf_engine.search(fallback_word, limit.unwrap_or(20));
        
        // Apply the same filtering to fallback results, but be less aggressive for fuzzy matches
        let filtered_fallback: Vec<_> = if !fallback_word.chars().all(|c| c.is_ascii_digit()) && *fallback_word != "0" && *fallback_word != "r" {
            fallback_results.into_iter()
                .filter(|(score, file)| {
                    let is_junk_folder = 
                        (file.name == "0" && file.path.contains("__External")) ||
                        (file.name.chars().all(|c| c.is_ascii_digit()) && 
                         (file.path.contains("Browser") || file.path.contains("cache") || file.path.contains("morgue"))) ||
                        (file.name.len() == 1 && file.name.chars().all(|c| c.is_ascii_lowercase()) && 
                         file.path.contains("WinSxS")) ||
                        (file.name.chars().all(|c| c.is_ascii_digit()) && file.path.contains("Tor Browser")) ||
                        // Be less aggressive with score filtering for fuzzy fallback results
                        (*score <= 1000 && 
                         (file.path.contains("UpdateInfo") || file.path.contains("profile.default") || 
                          file.path.contains("uuid+++") || (file.name.len() <= 2 && file.name.chars().all(|c| c.is_ascii_digit()))));
                    
                    !is_junk_folder
                })
                .collect()
        } else {
            fallback_results
        };
        
        println!("FZF Debug: Fallback search found {} results after filtering", filtered_fallback.len());
        filtered_fallback
    } else {
        filtered_results
    };
    
    // Convert to expected format (path, name, modified_at)
    let formatted_results: Vec<(String, String, Option<i64>)> = final_results
        .into_iter()
        .map(|(_score, file)| (file.path.clone(), file.name.clone(), file.modified_at))
        .collect();
    
    Ok(formatted_results)
}

#[tauri::command]
async fn refresh_fzf_index(state: State<'_, AppState>) -> Result<String, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut fzf_engine = state.fzf_engine.lock().map_err(|e| e.to_string())?;
    
    fzf_engine.load_from_database(&db)?;
    Ok(format!("FZF index refreshed with {} files", fzf_engine.files.len()))
}

#[tauri::command]
async fn natural_language_search(query: String, state: State<'_, AppState>) -> Result<llm_integration::NaturalQuery, String> {
    if query.trim().is_empty() {
        return Err("Query cannot be empty".to_string());
    }

    println!("Processing natural language query: '{}'", query);
    
    match state.llm_processor.parse_natural_query(&query).await {
        Ok(parsed) => {
            println!("Successfully parsed query: {:?}", parsed);
            Ok(parsed)
        }
        Err(e) => {
            println!("Failed to parse query: {}", e);
            Err(format!("Failed to parse natural language query: {}", e))
        }
    }
}

#[tauri::command]
async fn get_recent_files(state: State<'_, AppState>) -> Result<Vec<FileEntry>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let mut stmt = db
        .prepare("SELECT rf.path, rf.name, rf.last_accessed, rf.access_count, f.modified_at 
                  FROM recent_files rf 
                  LEFT JOIN files f ON rf.path = f.path 
                  ORDER BY rf.access_count DESC, rf.last_accessed DESC LIMIT 20")
        .map_err(|e| e.to_string())?;

    let files: Vec<FileEntry> = stmt
        .query_map([], |row| {
            Ok(FileEntry {
                path: row.get(0)?,
                name: row.get(1)?,
                last_accessed: Some(row.get(2)?),
                access_count: row.get(3)?,
                modified_at: row.get(4)?,
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

#[tauri::command]
async fn debug_search_scores(state: State<'_, AppState>, query: String) -> Result<Vec<(String, i64, String)>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    
    let mut stmt = db.prepare("SELECT path, name FROM files WHERE LOWER(name) LIKE ? LIMIT 20")
        .map_err(|e| e.to_string())?;
    
    let pattern = format!("%{}%", query.to_lowercase());
    let files: Vec<(String, String)> = stmt
        .query_map([&pattern], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    
    let options = SearchOptions {
        search_folders: false,
        enable_fuzzy: true,
        strict_mode: false,
        filename_only: true,
    };
    
    let results = fuzzy_search_files(files, &query, &[], &[], &options);
    
    let debug_output: Vec<(String, i64, String)> = results.iter()
        .map(|(score, entry)| (entry.name.clone(), *score, entry.path.clone()))
        .collect();
    
    Ok(debug_output)
}

#[tauri::command]
async fn toggle_favorite(state: State<'_, AppState>, path: String) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    
    // Check if already favorited
    let is_favorited: bool = db
        .query_row(
            "SELECT 1 FROM favorite_files WHERE path = ?1",
            [&path],
            |_| Ok(true),
        )
        .unwrap_or(false);
    
    if is_favorited {
        // Remove from favorites
        db.execute("DELETE FROM favorite_files WHERE path = ?1", [&path])
            .map_err(|e| e.to_string())?;
        Ok(false)
    } else {
        // Add to favorites
        let name = Path::new(&path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        
        db.execute(
            "INSERT OR REPLACE INTO favorite_files (path, name, favorited_at) VALUES (?1, ?2, ?3)",
            params![&path, &name, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(true)
    }
}

#[tauri::command]
async fn get_favorites(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    
    let mut stmt = db
        .prepare("SELECT path FROM favorite_files ORDER BY favorited_at DESC")
        .map_err(|e| e.to_string())?;
    
    let favorites: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    
    Ok(favorites)
}

#[derive(Serialize)]
struct IndexedDirectory {
    path: String,
    name: String,
    is_active: bool,
    indexed_at: i64,
}

#[tauri::command]
async fn get_indexed_directories(state: State<'_, AppState>) -> Result<Vec<IndexedDirectory>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    
    let mut stmt = db
        .prepare("SELECT path, name, is_active, indexed_at FROM indexed_directories ORDER BY indexed_at DESC")
        .map_err(|e| e.to_string())?;
    
    let dirs: Vec<IndexedDirectory> = stmt
        .query_map([], |row| {
            Ok(IndexedDirectory {
                path: row.get(0)?,
                name: row.get(1)?,
                is_active: row.get::<_, i32>(2)? == 1,
                indexed_at: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    
    Ok(dirs)
}

#[tauri::command]
async fn set_active_directory(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    
    // Set all to inactive
    db.execute("UPDATE indexed_directories SET is_active = 0", [])
        .map_err(|e| e.to_string())?;
    
    // Set the selected one to active
    db.execute("UPDATE indexed_directories SET is_active = 1 WHERE path = ?1", [&path])
        .map_err(|e| e.to_string())?;
    
    Ok(())
}

#[derive(Serialize)]
struct IndexStatus {
    total_files: i64,
    last_indexed: Option<i64>,
}

#[tauri::command]
async fn simple_search(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>
) -> Result<Vec<(String, String, Option<i64>)>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let limit = limit.unwrap_or(20);
    
    // Load files into simple engine if not already loaded
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let mut simple_engine = state.simple_engine.lock().map_err(|e| e.to_string())?;
        
        // Load files if engine is empty
        if simple_engine.files.is_empty() {
            println!("Loading files into simple search engine...");
            
            let mut stmt = db.prepare("SELECT name, path, modified_at FROM files ORDER BY length(name), name")
                .map_err(|e| e.to_string())?;
            
            let mapped_rows = stmt.query_map([], |row| {
                let name: String = row.get(0)?;
                let path: String = row.get(1)?;
                let modified_at: Option<i64> = row.get(2)?;
                
                Ok(fzf_search::FileIndex {
                    name_lower: name.to_lowercase(),
                    name,
                    path,
                    modified_at,
                })
            }).map_err(|e| e.to_string())?;
            
            simple_engine.files = mapped_rows.filter_map(|r| r.ok()).collect();
            println!("Simple Search: Loaded {} files", simple_engine.files.len());
        }
    }
    
    // Perform the search
    let results = {
        let mut simple_engine = state.simple_engine.lock().map_err(|e| e.to_string())?;
        
        // Use multi-word search for better results
        if query.contains(' ') {
            simple_engine.multi_word_search(&query, limit)
        } else {
            simple_engine.search(&query, limit)
        }
    };
    
    // Apply the same filtering as FZF search to remove junk folders
    let filtered_results: Vec<_> = if !query.chars().all(|c| c.is_ascii_digit()) && query != "0" && query != "r" {
        results.into_iter()
            .filter(|(score, file)| {
                let is_junk_folder = 
                    (file.name == "0" && file.path.contains("__External")) ||
                    (file.name.chars().all(|c| c.is_ascii_digit()) && 
                     (file.path.contains("Browser") || file.path.contains("cache") || file.path.contains("morgue"))) ||
                    (file.name.len() == 1 && file.name.chars().all(|c| c.is_ascii_lowercase()) && 
                     file.path.contains("WinSxS")) ||
                    (file.name.chars().all(|c| c.is_ascii_digit()) && file.path.contains("Tor Browser")) ||
                    (*score <= 1000 && 
                     (file.path.contains("UpdateInfo") || file.path.contains("profile.default") || 
                      file.path.contains("uuid+++") || (file.name.len() <= 2 && file.name.chars().all(|c| c.is_ascii_digit()))));
                
                !is_junk_folder
            })
            .collect()
    } else {
        results
    };
    
    println!("Simple Search: After filtering: {} results", filtered_results.len());
    
    // Convert to expected format
    let formatted_results: Vec<(String, String, Option<i64>)> = filtered_results
        .into_iter()
        .map(|(_score, file)| (file.path.clone(), file.name.clone(), file.modified_at))
        .collect();
    
    Ok(formatted_results)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::new().expect("Failed to initialize app state");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            start_indexing,
            index_custom_folder,
            search_files,
            fzf_search,
            simple_search,
            refresh_fzf_index,
            natural_language_search,
            get_recent_files,
            open_file,
            open_file_with,
            get_file_info,
            get_index_status,
            debug_search_scores,
            toggle_favorite,
            get_favorites,
            get_indexed_directories,
            set_active_directory
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
