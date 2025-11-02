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