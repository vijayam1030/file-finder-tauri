use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NaturalQuery {
    pub original_query: String,
    pub file_types: Vec<String>,
    pub keywords: Vec<String>,
    pub time_constraint: Option<String>,
    pub size_constraint: Option<String>,
    pub location_hints: Vec<String>,
    pub intent: QueryIntent,
    pub confidence: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum QueryIntent {
    FindByName,       // "find file named X"
    FindByContent,    // "files containing X"
    FindByType,       // "all Python files"
    FindByTime,       // "recent files"
    FindByLocation,   // "files in Downloads"
    FindByProject,    // "React components"
}

pub struct LLMProcessor {
    pub ollama_url: String,
    pub model_name: String,
}

impl LLMProcessor {
    pub fn new() -> Self {
        Self {
            ollama_url: "http://127.0.0.1:11434".to_string(),
            model_name: "llama3.1:8b".to_string(), // Perfect model available
        }
    }

    pub async fn parse_natural_query(&self, query: &str) -> Result<NaturalQuery, String> {
        // Check if Ollama is available
        if !self.is_ollama_available().await {
            return self.fallback_parse(query);
        }

        let prompt = self.create_query_parsing_prompt(query);
        
        match self.call_ollama(&prompt).await {
            Ok(response) => {
                self.parse_llm_response(&response, query)
            }
            Err(_) => {
                println!("LLM failed, using fallback parsing");
                self.fallback_parse(query)
            }
        }
    }

    async fn is_ollama_available(&self) -> bool {
        match reqwest::get(&format!("{}/api/tags", self.ollama_url)).await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    fn create_query_parsing_prompt(&self, query: &str) -> String {
        format!(r#"
You are a file search query parser. Parse this natural language query into structured search parameters.

Query: "{}"

Respond with JSON in this exact format:
{{
    "file_types": ["ext1", "ext2"],
    "keywords": ["keyword1", "keyword2"],  
    "time_constraint": "recent|today|yesterday|last_week|null",
    "size_constraint": "large|small|null",
    "location_hints": ["folder1", "folder2"],
    "intent": "FindByName|FindByContent|FindByType|FindByTime|FindByLocation|FindByProject",
    "confidence": 0.8
}}

Examples:
- "find my Python files from last week" → {{"file_types": ["py"], "time_constraint": "last_week", "intent": "FindByType"}}
- "where is my config file" → {{"keywords": ["config"], "intent": "FindByName"}}
- "React components" → {{"keywords": ["component"], "file_types": ["jsx", "tsx"], "intent": "FindByProject"}}
- "find me all the latest files" → {{"time_constraint": "recent", "intent": "FindByTime"}}
- "show me recent files" → {{"time_constraint": "recent", "intent": "FindByTime"}}

Parse the query above:
"#, query)
    }

    async fn call_ollama(&self, prompt: &str) -> Result<String, String> {
        let client = reqwest::Client::new();
        
        let request_body = serde_json::json!({
            "model": self.model_name,
            "prompt": prompt,
            "stream": false,
            "options": {
                "temperature": 0.1,
                "top_p": 0.9,
                "num_predict": 200
            }
        });

        let response = client
            .post(&format!("{}/api/generate", self.ollama_url))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        let response_text = response.text().await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        let parsed: serde_json::Value = serde_json::from_str(&response_text)
            .map_err(|e| format!("Failed to parse JSON: {}", e))?;

        parsed.get("response")
            .and_then(|r| r.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "No response field found".to_string())
    }

    fn parse_llm_response(&self, response: &str, original_query: &str) -> Result<NaturalQuery, String> {
        // Try to extract JSON from the response
        let json_start = response.find('{');
        let json_end = response.rfind('}');
        
        if let (Some(start), Some(end)) = (json_start, json_end) {
            let json_str = &response[start..=end];
            
            match serde_json::from_str::<serde_json::Value>(json_str) {
                Ok(parsed) => {
                    let file_types = parsed.get("file_types")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                        .unwrap_or_default();

                    let keywords = parsed.get("keywords")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                        .unwrap_or_default();

                    let location_hints = parsed.get("location_hints")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                        .unwrap_or_default();

                    let intent = parsed.get("intent")
                        .and_then(|v| v.as_str())
                        .and_then(|s| match s {
                            "FindByName" => Some(QueryIntent::FindByName),
                            "FindByContent" => Some(QueryIntent::FindByContent),
                            "FindByType" => Some(QueryIntent::FindByType),
                            "FindByTime" => Some(QueryIntent::FindByTime),
                            "FindByLocation" => Some(QueryIntent::FindByLocation),
                            "FindByProject" => Some(QueryIntent::FindByProject),
                            _ => None,
                        })
                        .unwrap_or(QueryIntent::FindByName);

                    let confidence = parsed.get("confidence")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.5) as f32;

                    Ok(NaturalQuery {
                        original_query: original_query.to_string(),
                        file_types,
                        keywords,
                        time_constraint: parsed.get("time_constraint").and_then(|v| v.as_str().map(|s| s.to_string())),
                        size_constraint: parsed.get("size_constraint").and_then(|v| v.as_str().map(|s| s.to_string())),
                        location_hints,
                        intent,
                        confidence,
                    })
                }
                Err(_) => self.fallback_parse(original_query)
            }
        } else {
            self.fallback_parse(original_query)
        }
    }

    fn fallback_parse(&self, query: &str) -> Result<NaturalQuery, String> {
        let query_lower = query.to_lowercase();
        let mut file_types = Vec::new();
        let mut keywords = Vec::new();
        let mut intent = QueryIntent::FindByName;
        let mut time_constraint = None;

        // Time-based queries detection
        if query_lower.contains("latest") || query_lower.contains("recent") || query_lower.contains("newest") {
            intent = QueryIntent::FindByTime;
            time_constraint = Some("recent".to_string());
        }
        if query_lower.contains("today") {
            intent = QueryIntent::FindByTime;
            time_constraint = Some("today".to_string());
        }
        if query_lower.contains("yesterday") {
            intent = QueryIntent::FindByTime;
            time_constraint = Some("yesterday".to_string());
        }
        if query_lower.contains("last week") || query_lower.contains("this week") {
            intent = QueryIntent::FindByTime;
            time_constraint = Some("last_week".to_string());
        }

        // File type detection
        if query_lower.contains("python") || query_lower.contains(".py") {
            file_types.push("py".to_string());
            if intent == QueryIntent::FindByName { intent = QueryIntent::FindByType; }
        }
        if query_lower.contains("javascript") || query_lower.contains(".js") {
            file_types.push("js".to_string());
            if intent == QueryIntent::FindByName { intent = QueryIntent::FindByType; }
        }
        if query_lower.contains("react") {
            file_types.extend(vec!["jsx".to_string(), "tsx".to_string()]);
            keywords.push("component".to_string());
            if intent == QueryIntent::FindByName { intent = QueryIntent::FindByProject; }
        }
        if query_lower.contains("config") {
            keywords.push("config".to_string());
            file_types.extend(vec!["json".to_string(), "yaml".to_string(), "ini".to_string()]);
        }

        // Extract remaining words as keywords
        for word in query.split_whitespace() {
            if !["find", "show", "where", "my", "the", "a", "an", "is", "are", "files", "file"].contains(&word.to_lowercase().as_str()) {
                keywords.push(word.to_string());
            }
        }

        Ok(NaturalQuery {
            original_query: query.to_string(),
            file_types,
            keywords,
            time_constraint,
            size_constraint: None,
            location_hints: vec![],
            intent,
            confidence: 0.7, // Higher confidence for time-based queries
        })
    }

    pub fn convert_to_search_query(&self, natural_query: &NaturalQuery) -> String {
        // For time-based queries, we need to use a special approach
        match natural_query.intent {
            QueryIntent::FindByTime => {
                // For "latest files" queries, return empty string to get all files
                // The time filtering will be handled in the database query
                if natural_query.file_types.is_empty() && natural_query.keywords.is_empty() {
                    "".to_string() // This will trigger recent files logic
                } else {
                    // Combine file types and keywords for time-filtered search
                    let mut parts = Vec::new();
                    
                    if !natural_query.file_types.is_empty() {
                        for ext in &natural_query.file_types {
                            parts.push(format!("*.{}", ext));
                        }
                    }
                    
                    parts.extend(natural_query.keywords.clone());
                    parts.join(" ")
                }
            }
            _ => {
                let mut parts = Vec::new();

                // Add file type constraints
                if !natural_query.file_types.is_empty() {
                    for ext in &natural_query.file_types {
                        parts.push(format!("*.{}", ext));
                    }
                }

                // Add keywords
                parts.extend(natural_query.keywords.clone());

                // Join with space for fuzzy matching
                if parts.is_empty() {
                    natural_query.original_query.clone()
                } else {
                    parts.join(" ")
                }
            }
        }
    }
}