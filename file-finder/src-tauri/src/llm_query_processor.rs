use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct ParsedQuery {
    pub file_types: Vec<String>,          // ["py", "js", "xlsx"]
    pub keywords: Vec<String>,            // ["machine learning", "quarterly"]
    pub time_range: Option<TimeRange>,    // Last week, yesterday, etc.
    pub size_constraints: Option<SizeRange>,
    pub location_hints: Vec<String>,      // ["Documents", "Desktop"]
    pub intent: QueryIntent,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum QueryIntent {
    FindByContent,      // "files containing..."
    FindByName,         // "files named..."
    FindByType,         // "all Python files"
    FindByTime,         // "recent files"
    FindByLocation,     // "files in folder..."
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimeRange {
    pub relative: Option<String>,    // "last week", "yesterday"
    pub absolute: Option<(String, String)>, // ("2024-01-01", "2024-01-31")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SizeRange {
    pub min: Option<u64>,
    pub max: Option<u64>,
}

// LLM Integration
pub async fn parse_natural_query(query: &str) -> Result<ParsedQuery, String> {
    // This would call an LLM API (OpenAI, Claude, local LLM, etc.)
    let prompt = format!(r#"
Parse this file search query into structured data:
Query: "{}"

Extract:
1. File types/extensions mentioned
2. Keywords for content or name matching
3. Time constraints (relative or absolute)
4. Size constraints
5. Location hints
6. Primary intent

Return JSON matching the ParsedQuery schema.
"#, query);

    // Example response processing (would use actual LLM API)
    parse_mock_llm_response(query)
}

fn parse_mock_llm_response(query: &str) -> Result<ParsedQuery, String> {
    // Mock implementation - replace with actual LLM call
    let query_lower = query.to_lowercase();
    
    let mut file_types = Vec::new();
    let mut keywords = Vec::new();
    let mut time_range = None;
    
    // Simple pattern matching (LLM would do this much better)
    if query_lower.contains("python") || query_lower.contains(".py") {
        file_types.push("py".to_string());
    }
    if query_lower.contains("javascript") || query_lower.contains(".js") {
        file_types.push("js".to_string());
    }
    if query_lower.contains("excel") || query_lower.contains(".xlsx") {
        file_types.push("xlsx".to_string());
    }
    
    if query_lower.contains("last week") {
        time_range = Some(TimeRange {
            relative: Some("last_week".to_string()),
            absolute: None,
        });
    }
    
    // Extract keywords (simplified)
    for word in query.split_whitespace() {
        if !word.to_lowercase().contains("find") && 
           !word.to_lowercase().contains("file") &&
           !word.to_lowercase().contains("show") {
            keywords.push(word.to_string());
        }
    }
    
    Ok(ParsedQuery {
        file_types,
        keywords,
        time_range,
        size_constraints: None,
        location_hints: vec![],
        intent: QueryIntent::FindByContent,
    })
}