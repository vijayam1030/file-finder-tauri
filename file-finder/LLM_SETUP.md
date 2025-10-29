# File Finder LLM Version - Installation Guide

## Prerequisites

### 1. Install Ollama (for local LLM support)
- **Download**: https://ollama.ai/download
- **Install**: Run the installer for your platform
- **Verify**: Open terminal and run `ollama --version`

### 2. Install a Language Model
After installing Ollama, download a suitable model:

```bash
# Recommended: Fast and efficient for file search queries
ollama pull llama3.1:8b

# Alternative: Smaller model (faster, less accurate)
ollama pull phi3:mini

# Alternative: Larger model (slower, more accurate)  
ollama pull llama3.1:70b
```

## Installation Options

### Option A: MSI Installer (Recommended)
1. Download `File Finder LLM_1.0.0_x64_en-US.msi`
2. Run the installer
3. Ensure Ollama is running in background
4. Launch File Finder

### Option B: Portable Version
1. Download the portable ZIP
2. Extract to desired location
3. Ensure Ollama is running
4. Run `file-finder.exe`

## Usage

### Natural Language Queries
- "Find my Python files from last week"
- "Where are my React components?"
- "Show me configuration files"
- "Recent JavaScript files"

### Technical Queries (still supported)
- `*.py` - All Python files
- `config*` - Files starting with "config"
- `/react.*component/i` - Regex search

## Troubleshooting

### LLM Not Working
1. **Check Ollama**: Run `ollama list` to see installed models
2. **Start Ollama**: Run `ollama serve` if not running
3. **Fallback**: App will use rule-based parsing if LLM fails

### Performance
- **Large Models**: Use `llama3.1:8b` for best balance
- **Fast Queries**: Use `phi3:mini` for speed
- **Accuracy**: Use `llama3.1:70b` for complex queries

## Configuration

The app automatically detects:
- Ollama URL: `http://127.0.0.1:11434`
- Model: `llama3.1:8b` (configurable)

To use a different model, the app will fall back gracefully if the preferred model isn't available.