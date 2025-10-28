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

    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to open database: {}", e);
            return;
        }
    };

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut count = 0;

    for entry in WalkDir::new(path)
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
        .filter(|e| e.file_type().is_file())
    {
        if let Some(path_str) = entry.path().to_str() {
            if let Some(name) = entry.file_name().to_str() {
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO files (path, name, indexed_at) VALUES (?1, ?2, ?3)",
                    params![path_str, name, now],
                );

                count += 1;

                // Commit every 1000 files for better performance
                if count % 1000 == 0 {
                    println!("Indexed {} files...", count);
                }
            }
        }
    }

    println!("Indexing complete! Total files: {}", count);
}

#[tauri::command]
async fn search_files(query: String, state: State<'_, AppState>) -> Result<Vec<FileEntry>, String> {
    if query.trim().is_empty() {
        return get_recent_files(state).await;
    }

    let (files, recent) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;

        // Get all files from database
        let mut stmt = db
            .prepare("SELECT path, name FROM files")
            .map_err(|e| e.to_string())?;

        let files: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

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

    // Fuzzy search
    let matcher = SkimMatcherV2::default();
    let mut results: Vec<(i64, FileEntry)> = files
        .into_iter()
        .filter_map(|(path, name)| {
            matcher.fuzzy_match(&name, &query).map(|score| {
                // Boost score if file is in recent files
                let boosted_score = if recent.contains(&path) {
                    score * 2
                } else {
                    score
                };

                (
                    boosted_score,
                    FileEntry {
                        path: path.clone(),
                        name,
                        last_accessed: None,
                        access_count: 0,
                    },
                )
            })
        })
        .collect();

    // Sort by score (descending)
    results.sort_by(|a, b| b.0.cmp(&a.0));

    // Return top 50 results
    Ok(results.into_iter().take(50).map(|(_, entry)| entry).collect())
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
            get_index_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
