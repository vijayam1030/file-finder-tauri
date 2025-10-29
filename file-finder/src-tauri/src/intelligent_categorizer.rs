use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct FileCategory {
    pub primary: String,           // "Development", "Documents", "Media"
    pub secondary: Vec<String>,    // ["Frontend", "React", "Components"]
    pub confidence: f32,           // 0.0 - 1.0
    pub auto_tags: Vec<String>,    // Auto-generated tags
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectContext {
    pub project_type: String,      // "React App", "Python Package", "Research"
    pub main_language: String,     // "JavaScript", "Python", "TypeScript"
    pub frameworks: Vec<String>,   // ["React", "FastAPI", "SQLAlchemy"]
    pub purpose: String,           // "E-commerce website", "Data analysis"
}

// Smart file categorization
pub async fn categorize_file(file_path: &str, content_sample: &str, project_context: Option<&ProjectContext>) -> Result<FileCategory, String> {
    let context_info = match project_context {
        Some(ctx) => format!("Project context: {} ({})", ctx.project_type, ctx.main_language),
        None => "No project context available".to_string(),
    };

    let prompt = format!(r#"
Categorize this file based on its path and content:

File: {}
Content sample: {}
{}

Provide:
1. Primary category (Development/Documents/Media/Config/Data/etc.)
2. Secondary categories (more specific)
3. Confidence level (0-1)
4. Auto-generated tags for searchability

Consider the project context when categorizing.
"#, file_path, content_sample, context_info);

    // Would call LLM API
    categorize_with_llm(&prompt, file_path).await
}

async fn categorize_with_llm(prompt: &str, file_path: &str) -> Result<FileCategory, String> {
    // Mock implementation
    let extension = std::path::Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    match extension {
        "rs" => Ok(FileCategory {
            primary: "Development".to_string(),
            secondary: vec!["Rust".to_string(), "Backend".to_string()],
            confidence: 0.9,
            auto_tags: vec!["rust".to_string(), "source-code".to_string(), "backend".to_string()],
        }),
        "js" | "jsx" => Ok(FileCategory {
            primary: "Development".to_string(),
            secondary: vec!["JavaScript".to_string(), "Frontend".to_string()],
            confidence: 0.9,
            auto_tags: vec!["javascript".to_string(), "frontend".to_string(), "web".to_string()],
        }),
        _ => Ok(FileCategory {
            primary: "Unknown".to_string(),
            secondary: vec![],
            confidence: 0.5,
            auto_tags: vec![],
        }),
    }
}

// Detect project context automatically
pub async fn detect_project_context(root_path: &str) -> Result<ProjectContext, String> {
    let files_sample = get_sample_files(root_path).await?;
    
    let prompt = format!(r#"
Analyze these files to determine the project type and context:

Files: {:?}

Determine:
1. Project type (web app, library, CLI tool, etc.)
2. Main programming language
3. Frameworks/technologies used
4. Overall purpose

This helps with better file categorization.
"#, files_sample);

    // Would call LLM API
    detect_context_with_llm(&prompt).await
}

async fn get_sample_files(root_path: &str) -> Result<Vec<String>, String> {
    // Get representative files (package.json, Cargo.toml, requirements.txt, etc.)
    Ok(vec![
        "package.json".to_string(),
        "src/main.rs".to_string(),
        "README.md".to_string(),
    ])
}

async fn detect_context_with_llm(prompt: &str) -> Result<ProjectContext, String> {
    // Mock implementation
    Ok(ProjectContext {
        project_type: "Desktop Application".to_string(),
        main_language: "Rust".to_string(),
        frameworks: vec!["Tauri".to_string(), "SQLite".to_string()],
        purpose: "File finder and search tool".to_string(),
    })
}