// Test script to simulate the LLM query processing
use std::collections::HashMap;

#[derive(Debug)]
struct TestQuery {
    original: String,
    expected_intent: String,
    expected_constraint: Option<String>,
}

fn main() {
    let test_queries = vec![
        TestQuery {
            original: "find me all the latest files".to_string(),
            expected_intent: "FindByTime".to_string(),
            expected_constraint: Some("recent".to_string()),
        },
        TestQuery {
            original: "show me recent Python files".to_string(),
            expected_intent: "FindByType".to_string(),
            expected_constraint: Some("recent".to_string()),
        },
        TestQuery {
            original: "where are my config files".to_string(),
            expected_intent: "FindByName".to_string(),
            expected_constraint: None,
        },
    ];

    println!("Testing LLM query parsing simulation:");
    
    for test in test_queries {
        println!("\n=== Testing: '{}' ===", test.original);
        
        // Simulate what our fallback parser should detect
        let is_time_query = test.original.to_lowercase().contains("latest") ||
                           test.original.to_lowercase().contains("recent");
        
        let has_file_type = test.original.to_lowercase().contains("python") ||
                           test.original.to_lowercase().contains("config");
        
        if is_time_query && !has_file_type {
            println!("✓ Should trigger recent files logic");
            println!("  Enhanced query: \"\" (empty)");
            println!("  Action: Call get_recent_files()");
        } else {
            println!("✓ Should use normal search");
            println!("  Enhanced query: Contains search terms");
        }
    }
}