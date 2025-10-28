# File Finder

A blazing-fast, cross-platform file finder application that helps you quickly locate and open files without navigating through folders. Built with Rust and Tauri for maximum performance.

## Features

- **Super Fast Search**: Fuzzy search with real-time results as you type
- **Smart Indexing**: Indexes your entire home directory for instant searches
- **Recent Files Priority**: Frequently used files appear first in results
- **Keyboard-Driven**: Navigate entirely with keyboard shortcuts
- **Cross-Platform**: Works on Windows, Linux, and macOS
- **Lightweight**: Small binary size (~3-5MB) thanks to Tauri
- **Privacy-First**: All data stored locally in SQLite database

## How It Works

1. **Indexing**: On first run, click "Start Indexing" to scan your home directory
   - Walks through all directories (up to 10 levels deep)
   - Skips hidden folders and common ignore patterns (node_modules, .git, etc.)
   - Stores file paths in a local SQLite database
   - Indexes in the background without blocking the UI

2. **Searching**: Type any part of a filename to see instant results
   - Uses fuzzy matching algorithm for flexible searching
   - Returns top 50 matches sorted by relevance
   - Boosts frequently accessed files in search results

3. **Opening Files**: Click or press Enter to open files with your default application

4. **Recent Files**: Leave search empty to see your most frequently used files

## Installation

### Prerequisites

- **Node.js** (v16 or higher)
- **Rust** (latest stable version)
- **Operating System**: Windows, Linux, or macOS

### Development Setup

```bash
# Navigate to the project directory
cd file-finder

# Install Node dependencies
npm install

# Run in development mode
npm run tauri dev
```

###Building Production Binaries

```bash
# Build for your current platform
npm run tauri build
```

The compiled executables will be in:
- **Windows**: `src-tauri/target/release/file-finder.exe`
- **Linux**: `src-tauri/target/release/file-finder`
- **macOS**: `src-tauri/target/release/bundle/dmg/`

## Usage

### Keyboard Shortcuts

- **Type**: Start searching immediately
- **↑/↓**: Navigate through results
- **Enter**: Open selected file
- **Esc**: Clear search and show recent files

### Tips

1. **Initial Setup**: Click "Start Indexing" on first launch. This may take a few minutes depending on your home directory size.

2. **Re-indexing**: Click "Re-index" to update the file index after adding/removing many files.

3. **Fuzzy Search**: You don't need to type exact file names. For example:
   - "mydoc" will match "my-document.txt"
   - "rmdme" will match "README.md"

4. **Recent Files**: Files you open frequently are automatically prioritized in search results.

## Technical Details

### Architecture

- **Backend (Rust)**:
  - `walkdir`: Efficient directory traversal
  - `rusqlite`: SQLite database for file index
  - `fuzzy-matcher`: Fast fuzzy string matching (Skim algorithm)
  - `tokio`: Async runtime for background indexing
  - `dirs`: Cross-platform directory detection

- **Frontend (JavaScript + HTML/CSS)**:
  - Vanilla JavaScript for minimal overhead
  - Real-time search with 150ms debouncing
  - Modern, dark-themed UI
  - Responsive keyboard navigation

### Database Schema

```sql
-- Indexed files
CREATE TABLE files (
    id INTEGER PRIMARY KEY,
    path TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    indexed_at INTEGER NOT NULL
);

-- Recent files tracking
CREATE TABLE recent_files (
    id INTEGER PRIMARY KEY,
    path TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    last_accessed INTEGER NOT NULL,
    access_count INTEGER DEFAULT 1
);
```

### File Locations

- **Windows**: `%LOCALAPPDATA%\file-finder\index.db`
- **Linux**: `~/.local/share/file-finder/index.db`
- **macOS**: `~/Library/Application Support/file-finder/index.db`

## Performance

- **Indexing Speed**: ~10,000-50,000 files per minute (depends on disk speed)
- **Search Speed**: <10ms for most queries (in-memory fuzzy matching)
- **Memory Usage**: ~50-100MB during indexing, ~30MB idle
- **Disk Usage**: ~100KB per 10,000 files indexed

## Customization

### Adjusting Indexed Paths

Edit `src-tauri/src/lib.rs:79` to change the starting directory:

```rust
let home_dir = dirs::home_dir().ok_or("Could not find home directory")?;
// Change to any path you want to index
```

### Ignore Patterns

Edit `src-tauri/src/lib.rs:115-122` to modify which folders are skipped:

```rust
.filter_entry(|e| {
    let file_name = e.file_name().to_string_lossy();
    !file_name.starts_with('.')
        && !file_name.eq("node_modules")
        && !file_name.eq("target")
        // Add more patterns here
})
```

### Max Depth

Change `src-tauri/src/lib.rs:113` to adjust directory traversal depth:

```rust
.max_depth(10)  // Increase or decrease as needed
```

## Troubleshooting

### Indexing is slow
- Reduce `max_depth` in the code
- Add more folders to the ignore list
- Ensure your antivirus isn't scanning every file access

### Files not appearing in search
- Wait for indexing to complete (check status at top)
- Click "Re-index" to refresh the database
- Check if files are in ignored directories

### App won't start
- Ensure Rust and Node.js are installed
- Try `cargo clean` in `src-tauri/` directory
- Check for permission issues with the database folder

## Future Enhancements

- [ ] Global keyboard shortcut to launch app
- [ ] File content search (not just names)
- [ ] Custom folder selection for indexing
- [ ] File type filtering
- [ ] Dark/light theme toggle
- [ ] Export search history
- [ ] Mobile version (iOS/Android via Tauri 2)

## Cross-Platform Packaging

### Windows

```bash
npm run tauri build
# Creates: file-finder.exe and file-finder.msi installer
```

### Linux

```bash
npm run tauri build
# Creates: .deb package and .AppImage
```

### macOS

```bash
npm run tauri build
# Creates: .app bundle and .dmg installer
```

## License

This project is provided as-is for personal and educational use.

## Contributing

Feel free to submit issues and enhancement requests!

## Credits

Built with:
- [Tauri](https://tauri.app/) - Lightweight desktop framework
- [Rust](https://www.rust-lang.org/) - Systems programming language
- [SQLite](https://www.sqlite.org/) - Embedded database
- [fuzzy-matcher](https://github.com/lotabout/fuzzy-matcher) - Fuzzy string matching
