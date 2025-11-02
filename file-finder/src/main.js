const { invoke } = window.__TAURI__.core;

let searchInput;
let resultsList;
let recentList;
let indexStatusEl;
let indexBtn;
let selectedIndex = 0;
let currentResults = [];
let searchTimeout;
let activeTab = 'search';
let lastKeyTime = 0;
let lastKey = null;
let lastReindexDate = null; // Track last reindex date
let autoReindexEnabled = true; // Auto reindex enabled by default

// Search options
let searchOptions = {
  search_folders: true,
  enable_fuzzy: true,
  strict_mode: false,
  filename_only: false,
  applications_only: false
};

// Update the active settings display
function updateActiveSettingsDisplay() {
  const settingsTagsEl = document.getElementById('settings-tags');
  if (!settingsTagsEl) return;
  
  const activeSettings = [];
  
  // Check each setting and add to active list
  if (searchOptions.search_folders) activeSettings.push('Folders');
  if (searchOptions.enable_fuzzy) activeSettings.push('Fuzzy');
  if (searchOptions.strict_mode) activeSettings.push('Strict');
  if (searchOptions.filename_only) activeSettings.push('Name Only');
  if (searchOptions.applications_only) activeSettings.push('Apps Only');
  
  // Create the HTML
  let html = '';
  if (activeSettings.length > 0) {
    html = `<span class="settings-label">Active:</span>`;
    activeSettings.forEach(setting => {
      html += `<span class="settings-tag">${setting}</span>`;
    });
  } else {
    html = `<span class="settings-label">Settings:</span><span class="settings-tag disabled">None Active</span>`;
  }
  
  settingsTagsEl.innerHTML = html;
}

// Sort options
let currentSort = {
  search: 'relevance',  // relevance, date, usage
  recent: 'usage',      // usage, date
  favorites: 'name'     // name, date, usage
};

// Initialize app
window.addEventListener("DOMContentLoaded", async () => {
  searchInput = document.querySelector("#search-input");
  resultsList = document.querySelector("#results-list");
  recentList = document.querySelector("#recent-list");
  indexStatusEl = document.querySelector("#index-status");
  
  // Setup event listeners
  searchInput.addEventListener("input", handleSearch);
  searchInput.addEventListener("keydown", handleKeyboard);
  
  // Global keyboard listener for vim navigation when not typing in search
  document.addEventListener("keydown", (e) => {
    // Only handle global keys when search input is not focused
    if (document.activeElement !== searchInput) {
      handleKeyboard(e);
    }
  });
  
  // Re-index button - open folder dialog to select what to index
  const indexFolderBtn = document.querySelector("#index-folder-btn");
  if (indexFolderBtn) {
    indexFolderBtn.addEventListener("click", async () => {
      try {
        // Use Tauri's dialog API to open folder picker
        if (!window.__TAURI__?.dialog?.open) {
          throw new Error("Tauri dialog plugin not available.");
        }
        
        const selected = await window.__TAURI__.dialog.open({
          directory: true,
          multiple: false,
          title: 'Select folder to re-index'
        });
        
        if (selected) {
          // Start indexing
          indexStatusEl.textContent = `Indexing: ${selected}`;
          indexFolderBtn.disabled = true;
          indexFolderBtn.textContent = "Indexing...";
          
          // Get initial file count
          const initialStatus = await invoke("get_index_status");
          const initialCount = initialStatus.total_files;
          
          // Start the indexing process (non-blocking)
          invoke("index_custom_folder", { path: selected }).then(() => {
            console.log("Indexing completed");
          }).catch((error) => {
            console.error("Indexing failed:", error);
            indexStatusEl.textContent = `Indexing failed: ${error}`;
            indexFolderBtn.disabled = false;
            indexFolderBtn.textContent = "Re-index";
          });
          
          // Poll for status updates more frequently
          let pollCount = 0;
          let lastCount = initialCount;
          let stableCount = 0;
          
          const interval = setInterval(async () => {
            try {
              const status = await invoke("get_index_status");
              const currentCount = status.total_files;
              const newFiles = currentCount - initialCount;
              
              // Show progress
              if (newFiles > 0) {
                indexStatusEl.textContent = `Indexing: ${currentCount.toLocaleString()} files (+${newFiles.toLocaleString()} new)`;
              } else {
                indexStatusEl.textContent = `Indexing: ${currentCount.toLocaleString()} files`;
              }
              
              // Check if indexing is complete (file count stabilized)
              if (currentCount === lastCount) {
                stableCount++;
              } else {
                stableCount = 0;
                lastCount = currentCount;
              }
              
              // If count has been stable for 3 polls (6 seconds), assume indexing is done
              if (stableCount >= 3 || pollCount > 60) { // Max 2 minutes
                clearInterval(interval);
                indexStatusEl.textContent = `${currentCount.toLocaleString()} files indexed`;
                indexFolderBtn.disabled = false;
                indexFolderBtn.textContent = "Re-index";
                
                if (newFiles > 0) {
                  // Show completion message briefly
                  const tempMessage = indexStatusEl.textContent;
                  indexStatusEl.textContent = `Indexing complete! Added ${newFiles.toLocaleString()} files`;
                  setTimeout(() => {
                    indexStatusEl.textContent = tempMessage;
                  }, 3000);
                }
                
                // Refresh current search results if there's an active search
                const currentQuery = searchInput.value.trim();
                if (currentQuery && activeTab === 'search') {
                  console.log("Refreshing search results after indexing...");
                  await performFzfSearch(currentQuery);
                }
                
                // Reload recent files and favorites
                await loadRecentFiles();
                await loadFavorites();
              }
              
              pollCount++;
            } catch (error) {
              console.error("Failed to get status during indexing:", error);
            }
          }, 2000); // Poll every 2 seconds
        }
      } catch (error) {
        console.error("Failed to index folder:", error);
        alert("Failed to index folder: " + error);
        indexFolderBtn.disabled = false;
        indexFolderBtn.textContent = "Re-index";
      }
    });
  }
  
  // Global keyboard listener for vim navigation when not typing in search
  document.addEventListener("keydown", (e) => {
    // Only handle global keys when search input is not focused
    if (document.activeElement !== searchInput) {
      handleKeyboard(e);
    }
  });
  
  // Setup settings panel
  const settingsBtn = document.querySelector("#settings-btn");
  const settingsPanel = document.querySelector("#settings-panel");
  
  console.log("Settings button:", settingsBtn);
  console.log("Settings panel:", settingsPanel);
  
  if (settingsBtn && settingsPanel) {
    settingsBtn.addEventListener("click", (e) => {
      console.log("Settings button clicked!");
      e.stopPropagation();
      settingsPanel.classList.toggle("hidden");
      console.log("Panel hidden class:", settingsPanel.classList.contains("hidden"));
    });
    
    // Close settings when clicking outside
    document.addEventListener("click", (e) => {
      if (!settingsPanel.contains(e.target) && e.target !== settingsBtn) {
        settingsPanel.classList.add("hidden");
      }
    });
  } else {
    console.error("Settings button or panel not found!");
  }

  // Setup Regex Help Panel
  const regexHelpBtn = document.getElementById("regex-help-btn");
  const regexHelpPanel = document.getElementById("regex-help-panel");
  const closeRegexHelp = document.getElementById("close-regex-help");
  
  if (regexHelpBtn && regexHelpPanel) {
    regexHelpBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      regexHelpPanel.classList.toggle("hidden");
      // Close settings panel if open
      if (settingsPanel) {
        settingsPanel.classList.add("hidden");
      }
    });
    
    closeRegexHelp.addEventListener("click", () => {
      regexHelpPanel.classList.add("hidden");
    });
    
    // Close regex help when clicking outside
    document.addEventListener("click", (e) => {
      if (!regexHelpPanel.contains(e.target) && e.target !== regexHelpBtn) {
        regexHelpPanel.classList.add("hidden");
      }
    });

    // Make example patterns clickable
    document.querySelectorAll(".example").forEach(example => {
      example.addEventListener("click", () => {
        const pattern = example.getAttribute("data-pattern");
        if (pattern) {
          searchInput.value = pattern;
          searchInput.focus();
          regexHelpPanel.classList.add("hidden");
          performFzfSearch(pattern);
        }
      });
    });
  }
  
  // Setup settings checkboxes
  document.getElementById("search-folders").addEventListener("change", (e) => {
    searchOptions.search_folders = e.target.checked;
    updateActiveSettingsDisplay();
    performFzfSearch(searchInput.value.trim());
  });
  
  document.getElementById("search-fuzzy").addEventListener("change", (e) => {
    searchOptions.enable_fuzzy = e.target.checked;
    updateActiveSettingsDisplay();
    performFzfSearch(searchInput.value.trim());
  });
  
  document.getElementById("search-strict").addEventListener("change", (e) => {
    searchOptions.strict_mode = e.target.checked;
    updateActiveSettingsDisplay();
    performFzfSearch(searchInput.value.trim());
  });
  
  document.getElementById("search-filename-only").addEventListener("change", (e) => {
    searchOptions.filename_only = e.target.checked;
    updateActiveSettingsDisplay();
    performFzfSearch(searchInput.value.trim());
  });
  
  document.getElementById("applications-only").addEventListener("change", (e) => {
    searchOptions.applications_only = e.target.checked;
    updateActiveSettingsDisplay();
    performFzfSearch(searchInput.value.trim());
  });
  
  // Auto reindex checkbox listener
  document.getElementById("auto-reindex").addEventListener("change", (e) => {
    autoReindexEnabled = e.target.checked;
    localStorage.setItem('autoReindexEnabled', autoReindexEnabled);
    console.log(`Auto reindex ${autoReindexEnabled ? 'enabled' : 'disabled'}`);
  });
  
  // Load saved auto-reindex preference
  const savedAutoReindex = localStorage.getItem('autoReindexEnabled');
  if (savedAutoReindex !== null) {
    autoReindexEnabled = savedAutoReindex === 'true';
    document.getElementById("auto-reindex").checked = autoReindexEnabled;
  }
  
  // Setup tab listeners
  document.querySelectorAll(".tab-btn").forEach(btn => {
    btn.addEventListener("click", (e) => {
      const tab = e.target.dataset.tab;
      switchTab(tab);
    });
  });

  // Setup sort tab listeners
  document.querySelectorAll(".sort-btn").forEach(btn => {
    btn.addEventListener("click", (e) => {
      const sortType = e.target.dataset.sort;
      const tabType = e.target.dataset.tab;
      
      // Update sort option
      currentSort[tabType] = sortType;
      
      // Update active sort button
      document.querySelectorAll(`.sort-btn[data-tab="${tabType}"]`).forEach(b => {
        b.classList.toggle("active", b.dataset.sort === sortType);
      });
      
      // Re-render the current tab with new sort
      if (tabType === 'search' && activeTab === 'search') {
        renderSearchResults(currentResults);
      } else if (tabType === 'recent' && activeTab === 'recent') {
        loadRecentFiles(); // Reload with new sort
      } else if (tabType === 'favorites' && activeTab === 'favorites') {
        loadFavorites(); // Reload with new sort
      }
    });
  });

  // Load initial status
  await updateStatus();
  
  // Initialize active settings display
  updateActiveSettingsDisplay();
  
  // Load recent files and favorites
  await loadRecentFiles();
  await loadFavorites();

  // Auto-refresh status (less frequent to avoid development interruptions)
  setInterval(updateStatus, 15000);
  
  // Check for scheduled reindexing every hour
  checkScheduledReindex();
  setInterval(checkScheduledReindex, 60 * 60 * 1000); // Check every hour
});

// Update sort button states for the active tab
function updateSortButtons(tabName) {
  if (tabName === 'search' || tabName === 'recent' || tabName === 'favorites') {
    const currentSortType = currentSort[tabName];
    document.querySelectorAll(`.sort-btn[data-tab="${tabName}"]`).forEach(btn => {
      btn.classList.toggle("active", btn.dataset.sort === currentSortType);
    });
  }
}

// Switch between tabs
function switchTab(tabName) {
  // Update active tab
  activeTab = tabName;
  
  // Update tab buttons
  document.querySelectorAll(".tab-btn").forEach(btn => {
    btn.classList.toggle("active", btn.dataset.tab === tabName);
  });
  
  // Update tab content
  document.querySelectorAll(".tab-content").forEach(content => {
    content.classList.toggle("active", content.id === `${tabName}-results`);
  });
  
  // Update sort buttons for the active tab
  updateSortButtons(tabName);
  
  // Load content for the active tab
  if (tabName === 'recent') {
    loadRecentFiles();
  } else if (tabName === 'favorites') {
    loadFavorites();
  } else if (tabName === 'search') {
    // Switch to search tab - if there's a query, perform search
    const query = searchInput.value.trim();
    if (query) {
      performFzfSearch(query);
    } else {
      // Clear search results when switching to empty search
      renderSearchResults([]);
    }
  }
}

// Handle search input with debouncing
function handleSearch() {
  // Auto-switch to search tab when typing
  if (activeTab !== 'search') {
    activeTab = 'search';
    // Update tab buttons
    document.querySelectorAll(".tab-btn").forEach(btn => {
      btn.classList.toggle("active", btn.dataset.tab === 'search');
    });
    // Update tab content
    document.querySelectorAll(".tab-content").forEach(content => {
      content.classList.toggle("active", content.id === 'search-results');
    });
  }
  
  clearTimeout(searchTimeout);
  
  // FZF MODE: Much faster debouncing for real-time search
  const query = searchInput.value.trim();
  
  if (query.length === 0) {
    clearResults();
    return;
  }
  
  // Adaptive debouncing: shorter for single words, longer for multi-word
  const wordCount = query.split(/\s+/).length;
  const debounceTime = wordCount > 1 ? 200 : query.length < 3 ? 150 : 30; // Much faster for simple queries
  
  searchTimeout = setTimeout(async () => {
    await performFtsSearch(query);
  }, debounceTime);
}

// Track the current search to prevent race conditions
let currentSearchId = 0;

// FTS-style real-time search
async function performFtsSearch(query) {
  const searchId = ++currentSearchId;
  const loadingEl = document.getElementById('search-loading');
  if (loadingEl) loadingEl.classList.remove('hidden');
  try {
    const start = performance.now();
    // Use the regular search_files function which now includes FTS5 for multi-word queries
    const results = await invoke('search_files', { query, options: searchOptions });
    const duration = performance.now() - start;
    if (searchId === currentSearchId) {
      if (loadingEl) loadingEl.classList.add('hidden');
      currentResults = results;
      renderSearchResults(currentResults);
      console.log(`Enhanced search for "${query}": ${results.length} results in ${duration.toFixed(1)}ms`);
    }
  } catch (error) {
    if (searchId === currentSearchId) {
      if (loadingEl) loadingEl.classList.add('hidden');
      currentResults = [];
      renderSearchResults([]);
      console.error('Enhanced search error:', error);
    }
  }
}

// Simple search using nucleo-based engine  
async function performSimpleSearch(query) {
  if (!query || query.trim().length === 0) {
    clearResults();
    return;
  }

  console.log("Simple Search: Starting search for:", query);
  const start = performance.now();
  const searchId = ++currentSearchId;
  
  try {
    // Call the simple search backend
    const results = await invoke('simple_search', { 
      query: query,
      limit: 50
    });
    
    const duration = performance.now() - start;
    
    // Only update if this is still the current search
    if (searchId === currentSearchId) {
      console.log("Simple Search: Processing results for query:", query);
      
      // Get favorites for prioritization
      let favorites = [];
      try {
        favorites = await invoke("get_favorites");
      } catch (error) {
        console.warn("Could not load favorites:", error);
      }
      const favoritePaths = new Set(favorites);
      
      const formattedResults = results.map(([path, name, modifiedAt]) => {
        // Check if favorite (exact match or parent directory)
        let isFav = favoritePaths.has(path);
        if (!isFav) {
          for (const favPath of favoritePaths) {
            if (path.startsWith(favPath + '\\') || path.startsWith(favPath + '/')) {
              isFav = true;
              break;
            }
          }
        }
        
        return {
          path,
          name,
          modified_at: modifiedAt,
          isFavorite: isFav
        };
      });
      
      // Sort: favorites first, then by name
      const sortedResults = formattedResults.sort((a, b) => {
        if (a.isFavorite !== b.isFavorite) {
          return b.isFavorite ? 1 : -1;
        }
        return a.name.localeCompare(b.name);
      });
      
      selectedIndex = 0;
      currentResults = sortedResults;
      renderSearchResults(sortedResults);
      
      console.log(`Simple search for "${query}": ${results.length} results in ${duration.toFixed(1)}ms`);
    }
  } catch (error) {
    console.error('Simple search failed:', error);
    if (searchId === currentSearchId) {
      currentResults = [];
      renderSearchResults([]);
    }
  }
}

// Clear search results
function clearResults() {
  currentResults = [];
  selectedIndex = 0;
  if (resultsList) {
    resultsList.innerHTML = '';
  }
}

// Perform search
// OLD SEARCH FUNCTION - DISABLED TO PREVENT DUAL EXECUTION
async function performSearch(query) {
  console.warn("OLD performSearch called - this should not happen! Redirecting to FTS search.");
  return await performFtsSearch(query);
  
  // DISABLED CODE BELOW - DO NOT REMOVE (for reference)
  /*
  const searchId = ++currentSearchId;
  
  try {
    if (query) {
      // Only show searching indicator after a delay to avoid flicker for fast searches
      const searchingTimeout = setTimeout(() => {
        // Only show if this search is still current
        if (searchId === currentSearchId) {
          resultsList.innerHTML = `
            <div class="searching-indicator">
              <div class="spinner"></div>
              <p>Searching...</p>
            </div>
          `;
        }
      }, 200); // 200ms delay before showing "Searching..."
      
      const results = await invoke("search_files", { query, options: searchOptions });
      
      // Clear the searching timeout since we got results
      clearTimeout(searchingTimeout);
      
      // Debug: Log search info for troubleshooting
      console.log(`Search for "${query}" returned ${results.length} results`);
      if (results.length === 0) {
        console.log("No results found. Search options:", searchOptions);
        console.log("Running automatic debug check...");
        // Automatically run debug check for common patterns when no results found
        setTimeout(() => debugCheckFile(''), 1000);
      }
      
      // Only update if this search is still the current one (prevent race conditions)
      if (searchId === currentSearchId) {
        currentResults = results;
        selectedIndex = 0;
        renderSearchResults(results);
      }
    } else {
      // Only update if this is still the current search
      if (searchId === currentSearchId) {
        currentResults = [];
        renderSearchResults([]);
      }
    }
  } catch (error) {
    // Only handle error if this is still the current search
    if (searchId === currentSearchId) {
      console.error("Search error:", error);
      showError("Search failed: " + error);
      renderSearchResults([]);
    }
  }
  */
}

// Sort recent files based on the selected criteria
function sortRecentFiles(results, sortType) {
  switch (sortType) {
    case 'date':
      // Sort by most recent access time
      return [...results].sort((a, b) => b.last_accessed - a.last_accessed);
      
    case 'usage':
    default:
      // Sort by usage count (default behavior from backend)
      return [...results].sort((a, b) => {
        if (b.access_count !== a.access_count) {
          return b.access_count - a.access_count;
        }
        return b.last_accessed - a.last_accessed; // Tie-breaker
      });
  }
}

// Load recent files
async function loadRecentFiles() {
  try {
    const results = await invoke("get_recent_files");
    const sortedResults = sortRecentFiles(results, currentSort.recent);
    
    if (activeTab === 'recent') {
      currentResults = sortedResults;
      selectedIndex = 0;
    }
    renderRecentResults(sortedResults);
  } catch (error) {
    console.error("Failed to load recent files:", error);
  }
}

// Sort favorites based on the selected criteria
async function sortFavorites(results, sortType) {
  // Get recent files data for usage/date sorting
  let recentFilesData = [];
  try {
    recentFilesData = await invoke("get_recent_files");
  } catch (error) {
    console.error("Failed to load recent files for sorting:", error);
  }
  
  const recentFileMap = new Map(recentFilesData.map(f => [f.path, f]));
  
  switch (sortType) {
    case 'name':
      // Sort alphabetically by name
      return [...results].sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
      
    case 'date':
      // Sort by most recent access time
      return [...results].sort((a, b) => {
        const aRecent = recentFileMap.get(a.path);
        const bRecent = recentFileMap.get(b.path);
        const aTime = aRecent ? aRecent.last_accessed : 0;
        const bTime = bRecent ? bRecent.last_accessed : 0;
        return bTime - aTime; // Most recent first
      });
      
    case 'usage':
      // Sort by usage frequency
      return [...results].sort((a, b) => {
        const aRecent = recentFileMap.get(a.path);
        const bRecent = recentFileMap.get(b.path);
        const aCount = aRecent ? aRecent.access_count : 0;
        const bCount = bRecent ? bRecent.access_count : 0;
        if (bCount !== aCount) {
          return bCount - aCount; // Most used first
        }
        // Tie-breaker: alphabetical
        return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
      });
      
    default:
      return results;
  }
}

async function loadFavorites() {
  try {
    const favorites = await invoke("get_favorites");
    // Convert favorite paths to file objects with names
    const results = favorites.map(path => {
      const name = path.split(/[/\\]/).pop();
      return { path, name };
    });
    
    // Sort results based on current sort option
    const sortedResults = await sortFavorites(results, currentSort.favorites);
    
    if (activeTab === 'favorites') {
      currentResults = sortedResults;
      selectedIndex = 0;
    }
    renderFavorites(sortedResults);
  } catch (error) {
    console.error("Failed to load favorites:", error);
  }
}

// Sort search results based on the selected criteria
function sortSearchResults(results, sortType, recentFilesData) {
  const recentFileMap = new Map(recentFilesData.map(f => [f.path, f]));
  
  switch (sortType) {
    case 'date':
      // NOTE: This sorts by last accessed time, not actual file modification date
      // The backend doesn't currently store file modification timestamps
      // TODO: Add file modification time to database schema and FileEntry struct
      return [...results].sort((a, b) => {
        const aRecent = recentFileMap.get(a.path);
        const bRecent = recentFileMap.get(b.path);
        const aTime = aRecent ? aRecent.last_accessed : 0;
        const bTime = bRecent ? bRecent.last_accessed : 0;
        return bTime - aTime; // Most recently accessed first
      });
      
    case 'usage':
      // Sort by usage frequency
      return [...results].sort((a, b) => {
        const aRecent = recentFileMap.get(a.path);
        const bRecent = recentFileMap.get(b.path);
        const aCount = aRecent ? aRecent.access_count : 0;
        const bCount = bRecent ? bRecent.access_count : 0;
        return bCount - aCount; // Most used first
      });
      
    case 'relevance':
    default:
      // Keep original order (already sorted by relevance from backend)
      return results;
  }
}

// Render search results
async function renderSearchResults(results) {
  if (results.length === 0) {
    if (searchInput.value.trim()) {
      const query = searchInput.value.trim();
      const fileCount = await getIndexedFileCount();
      resultsList.innerHTML = `
        <div class="empty-state">
          <h3>No files found for "${escapeHtml(query)}"</h3>
          <p>Troubleshooting tips:</p>
          <ul style="text-align: left; margin: 10px 0;">
            <li>Try a broader search term (e.g., just the filename without extension)</li>
            <li>Use glob patterns: <code>*.java</code>, <code>*${escapeHtml(query)}*</code></li>
            <li>Check if the file's directory was indexed (C:\ index might have skipped some paths)</li>
            <li>Press <kbd>F5</kbd> or <kbd>Ctrl+R</kbd> to refresh the search</li>
            <li>Try re-indexing the specific folder containing your file</li>
            <li>Press <kbd>Ctrl+Shift+D</kbd> for debug info (check browser console)</li>
          </ul>
          <p><small>Searched ${fileCount} indexed files</small></p>
        </div>
      `;
    } else {
      resultsList.innerHTML = `
        <div class="empty-state">
          <h3>Enter a search term</h3>
          <p>Search for files and folders, supports glob (*.js) and regex patterns</p>
          <p><kbd>F5</kbd> or <kbd>Ctrl+R</kbd> to refresh • Click Re-index to add new files</p>
        </div>
      `;
    }
    return;
  }

  // Get recent files to check which results are recent
  let recentPaths = new Set();
  let favoritePaths = new Set();
  let recentFilesData = [];
  try {
    recentFilesData = await invoke("get_recent_files");
    recentPaths = new Set(recentFilesData.map(f => f.path));
    
    const favorites = await invoke("get_favorites");
    favoritePaths = new Set(favorites);
  } catch (error) {
    console.error("Failed to load recent files/favorites for badges:", error);
  }

  // Sort results based on current sort option
  const sortedResults = sortSearchResults(results, currentSort.search, recentFilesData);

  const html = sortedResults
    .map((file, index) => {
      const isSelected = index === selectedIndex && activeTab === 'search';
      
      // Determine if it's a folder or file by checking the path
      const isFolder = !file.name.includes('.') || 
                       file.path.endsWith('\\') || 
                       file.path.endsWith('/');
      
      // Get extension or set badge text
      let ext;
      if (isFolder) {
        ext = 'FOLDER';
      } else if (file.name.includes('.')) {
        ext = file.name.split('.').pop().toUpperCase();
      } else {
        ext = 'FILE';
      }
      
      const isRecent = recentPaths.has(file.path);
      // Use the isFavorite flag from FZF search (includes parent directory matching)
      const isFavorite = file.isFavorite || favoritePaths.has(file.path);

      return `
        <div class="file-item ${isSelected ? 'selected' : ''}" data-index="${index}" data-path="${escapeHtml(file.path)}">
          <div class="file-info-row">
            <button class="favorite-btn ${isFavorite ? 'favorited' : ''}" data-path="${escapeHtml(file.path)}" title="${isFavorite ? 'Remove from favorites' : 'Add to favorites'}">
              ${isFavorite ? '★' : '☆'}
            </button>
            <div class="file-name">${escapeHtml(file.name)}</div>
            ${isFavorite ? '<span class="fav-badge">FAV</span>' : ''}
            ${isRecent ? '<span class="recent-badge">RECENT</span>' : ''}
            <span class="file-ext-badge ${isFolder ? 'folder-badge' : ''}">${ext}</span>
            <button class="open-with-btn" data-path="${escapeHtml(file.path)}" title="Open with...">⚙</button>
          </div>
          <div class="file-path">${escapeHtml(file.path)}</div>
        </div>
      `;
    })
    .join("");

  resultsList.innerHTML = html;

  // Add click listeners for file items
  resultsList.querySelectorAll(".file-item").forEach((item) => {
    item.addEventListener("click", (e) => {
      // Don't open if clicking buttons or badges
      if (e.target.classList.contains('open-with-btn') || 
          e.target.classList.contains('favorite-btn') ||
          e.target.classList.contains('fav-badge') ||
          e.target.classList.contains('recent-badge') ||
          e.target.classList.contains('file-ext-badge')) {
        return;
      }
      
      const index = parseInt(item.dataset.index);
      selectedIndex = index;
      
      // Update visual selection
      resultsList.querySelectorAll('.file-item').forEach(el => el.classList.remove('selected'));
      item.classList.add('selected');
      
      const path = item.dataset.path;
      openFile(path);
    });
  });

  // Add click listeners for favorite buttons
  resultsList.querySelectorAll(".favorite-btn").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const path = btn.dataset.path;
      try {
        const isFavorited = await invoke("toggle_favorite", { path });
        btn.textContent = isFavorited ? '★' : '☆';
        btn.classList.toggle('favorited', isFavorited);
        btn.title = isFavorited ? 'Remove from favorites' : 'Add to favorites';
        
        // Update the FAV badge
        const fileItem = btn.closest('.file-item');
        const favBadge = fileItem.querySelector('.fav-badge');
        const fileName = fileItem.querySelector('.file-name');
        
        if (isFavorited && !favBadge) {
          // Add FAV badge
          const badge = document.createElement('span');
          badge.className = 'fav-badge';
          badge.textContent = 'FAV';
          fileName.after(badge);
        } else if (!isFavorited && favBadge) {
          // Remove FAV badge
          favBadge.remove();
        }
        
        // Refresh favorites tab if needed
        await loadFavorites();
      } catch (error) {
        console.error("Failed to toggle favorite:", error);
      }
    });
  });

  // Add click listeners for "Open with" buttons
  resultsList.querySelectorAll(".open-with-btn").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const path = btn.dataset.path;
      await showOpenWithDialog(path);
    });
  });

  // Scroll selected item into view
  if (activeTab === 'search') {
    scrollToSelected();
  }
}

// Render recent files
function renderRecentResults(results) {
  if (results.length === 0) {
    recentList.innerHTML = `
      <div class="empty-state">
        <h3>No recent files</h3>
        <p>Start opening files to see them here</p>
      </div>
    `;
    return;
  }

  const html = results
    .map((file, index) => {
      const isSelected = index === selectedIndex && activeTab === 'recent';
      const ext = file.name.includes('.') ? file.name.split('.').pop().toUpperCase() : 'FILE';
      const recentBadge = file.access_count > 1
        ? `<span class="recent-badge">Used ${file.access_count}x</span>`
        : '';

      return `
        <div class="file-item ${isSelected ? 'selected' : ''}" data-index="${index}" data-path="${escapeHtml(file.path)}">
          <div class="file-info-row">
            <div class="file-name">${escapeHtml(file.name)}</div>
            <span class="file-ext-badge">${ext}</span>
            <button class="open-with-btn" data-path="${escapeHtml(file.path)}" title="Open with...">⚙</button>
          </div>
          <div class="file-path">${escapeHtml(file.path)}</div>
          ${recentBadge ? `<div class="file-meta">${recentBadge}</div>` : ''}
        </div>
      `;
    })
    .join("");

  recentList.innerHTML = html;

  // Add click listeners for file items
  recentList.querySelectorAll(".file-item").forEach((item) => {
    item.addEventListener("click", (e) => {
      if (e.target.classList.contains('open-with-btn')) {
        return;
      }
      const path = item.dataset.path;
      openFile(path);
    });
  });

  // Add click listeners for "Open with" buttons
  recentList.querySelectorAll(".open-with-btn").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const path = btn.dataset.path;
      await showOpenWithDialog(path);
    });
  });

  // Scroll selected item into view
  if (activeTab === 'recent') {
    scrollToSelected();
  }
}

function renderFavorites(results) {
  const favoritesList = document.getElementById("favorites-list");
  if (!favoritesList) return;

  if (results.length === 0) {
    favoritesList.innerHTML = `
      <div class="empty-state">
        <h3>No favorite files</h3>
        <p>Star files in search results to see them here</p>
      </div>
    `;
    return;
  }

  const html = results
    .map((file, index) => {
      const isSelected = index === selectedIndex && activeTab === 'favorites';
      const ext = file.name.includes('.') ? file.name.split('.').pop().toUpperCase() : 'FILE';
      
      // Determine if it's a folder by checking if the name has no extension and path exists
      const isFolder = !file.name.includes('.') || file.path.endsWith('/') || file.path.endsWith('\\');
      const badge = isFolder ? 'FOLDER' : ext;

      return `
        <div class="file-item ${isSelected ? 'selected' : ''}" data-index="${index}" data-path="${escapeHtml(file.path)}">
          <div class="file-info-row">
            <div class="file-name">${escapeHtml(file.name)}</div>
            <span class="file-ext-badge ${isFolder ? 'folder-badge' : ''}">${badge}</span>
            <button class="favorite-btn favorited" data-path="${escapeHtml(file.path)}" title="Remove from favorites">
              ★
            </button>
            <button class="open-with-btn" data-path="${escapeHtml(file.path)}" title="Open with...">⚙</button>
          </div>
          <div class="file-path">${escapeHtml(file.path)}</div>
          <div class="file-meta"><span class="fav-badge">FAV</span></div>
        </div>
      `;
    })
    .join("");

  favoritesList.innerHTML = html;

  // Add click listeners for file items
  favoritesList.querySelectorAll(".file-item").forEach((item) => {
    item.addEventListener("click", (e) => {
      if (e.target.classList.contains('open-with-btn') || e.target.classList.contains('favorite-btn')) {
        return;
      }
      const path = item.dataset.path;
      openFile(path);
    });
  });

  // Add click listeners for favorite buttons
  favoritesList.querySelectorAll(".favorite-btn").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const path = btn.dataset.path;
      try {
        await invoke("toggle_favorite", { path });
        // Reload favorites after removing
        await loadFavorites();
      } catch (error) {
        console.error("Failed to toggle favorite:", error);
      }
    });
  });

  // Add click listeners for "Open with" buttons
  favoritesList.querySelectorAll(".open-with-btn").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const path = btn.dataset.path;
      await showOpenWithDialog(path);
    });
  });

  // Scroll selected item into view
  if (activeTab === 'favorites') {
    scrollToSelected();
  }
}

// Handle keyboard navigation
function handleKeyboard(e) {
  // Don't handle vim keys if user is typing in the search input (except for special keys)
  const isTypingInSearch = document.activeElement === searchInput;
  const isNavigationKey = ['ArrowDown', 'ArrowUp', 'Enter', 'Escape'].includes(e.key);
  const isVimKey = ['j', 'k', 'g', 'G'].includes(e.key);
  const isCtrlKey = e.ctrlKey && ['d', 'u'].includes(e.key);
  
  // Handle refresh shortcuts before other checks
  if (e.key === 'F5' || (e.ctrlKey && e.key === 'r')) {
    e.preventDefault();
    const currentQuery = searchInput.value.trim();
    if (currentQuery) {
      console.log("Force refreshing search for:", currentQuery);
      performFzfSearch(currentQuery);
    }
    return;
  }
  
  // Debug shortcut - Ctrl+Shift+D
  if (e.ctrlKey && e.shiftKey && e.key === 'D') {
    e.preventDefault();
    console.log("Running debug check for KotlinConventions.java...");
    debugCheckFile('C:\\charry\\java\\source\\spring-boot\\buildSrc\\src\\main\\java\\org\\springframework\\boot\\build\\KotlinConventions.java');
    return;
  }
  
  // Allow navigation keys and Escape even when typing
  // Only block vim keys when actively typing (not for Ctrl combinations)
  if (isTypingInSearch && isVimKey && !e.ctrlKey) {
    return; // Let the user type normally
  }
  
  if (currentResults.length === 0) return;

  const now = Date.now();
  const isDoubleG = e.key === 'g' && lastKey === 'g' && (now - lastKeyTime) < 500;
  
  // Update last key tracking
  lastKey = e.key;
  lastKeyTime = now;

  switch (e.key) {
    case "ArrowDown":
    case "j":
      e.preventDefault();
      selectedIndex = Math.min(selectedIndex + 1, currentResults.length - 1);
      renderCurrentTab();
      break;

    case "ArrowUp":
    case "k":
      e.preventDefault();
      selectedIndex = Math.max(selectedIndex - 1, 0);
      renderCurrentTab();
      break;

    case "g":
      if (isDoubleG) {
        // gg - Go to first item
        e.preventDefault();
        selectedIndex = 0;
        renderCurrentTab();
      }
      break;

    case "G":
      // G - Go to last item
      e.preventDefault();
      selectedIndex = currentResults.length - 1;
      renderCurrentTab();
      break;

    case "d":
      if (e.ctrlKey) {
        // Ctrl+d - Jump down half page
        e.preventDefault();
        const halfPage = Math.floor(10); // Roughly half page of items
        selectedIndex = Math.min(selectedIndex + halfPage, currentResults.length - 1);
        renderCurrentTab();
      }
      break;

    case "u":
      if (e.ctrlKey) {
        // Ctrl+u - Jump up half page
        e.preventDefault();
        const halfPage = Math.floor(10);
        selectedIndex = Math.max(selectedIndex - halfPage, 0);
        renderCurrentTab();
      }
      break;

    case "Enter":
      e.preventDefault();
      if (currentResults[selectedIndex]) {
        openFile(currentResults[selectedIndex].path);
      }
      break;

    case "Escape":
      e.preventDefault();
      searchInput.value = "";
      if (activeTab === 'search') {
        renderSearchResults([]);
        currentResults = [];
      }
      break;
  }
}

// Helper function to render the current active tab
function renderCurrentTab() {
  if (activeTab === 'search') {
    renderSearchResults(currentResults);
  } else if (activeTab === 'favorites') {
    renderFavorites(currentResults);
  } else {
    renderRecentResults(currentResults);
  }
}

// Open file
async function openFile(path) {
  try {
    await invoke("open_file", { path });
    // Always reload recent files after opening a file
    await loadRecentFiles();
  } catch (error) {
    console.error("Failed to open file:", error);
    showError("Failed to open file: " + error);
  }
}

// Open file with specific program
async function openFileWith(path, program) {
  try {
    await invoke("open_file_with", { path, program });
    await loadRecentFiles();
    showSuccess(`Opened with ${program}`);
  } catch (error) {
    console.error("Failed to open file:", error);
    showError("Failed to open file: " + error);
  }
}

// Show "Open with" dialog
async function showOpenWithDialog(path) {
  try {
    const fileInfo = await invoke("get_file_info", { path });
    const fileName = path.split(/[/\\]/).pop();
    
    // Create modal dialog
    const modal = document.createElement('div');
    modal.className = 'modal-overlay';
    modal.innerHTML = `
      <div class="modal-content">
        <h3>Open "${fileName}"</h3>
        <p class="modal-subtitle">File type: .${fileInfo.extension}</p>
        <div class="program-list">
          ${fileInfo.suggested_programs.map(program => `
            <button class="program-btn" data-program="${program}">
              <span class="program-icon">📝</span>
              <span class="program-name">${program}</span>
            </button>
          `).join('')}
        </div>
        <div class="modal-actions">
          <input type="text" id="custom-program" placeholder="Or enter custom program path..." class="custom-program-input" />
          <div style="display: flex; gap: 8px; margin-top: 8px;">
            <button class="btn-secondary" id="modal-cancel">Cancel</button>
            <button class="btn-primary" id="modal-custom-open">Open with Custom</button>
          </div>
        </div>
      </div>
    `;
    
    document.body.appendChild(modal);
    
    // Add click handlers
    modal.querySelectorAll('.program-btn').forEach(btn => {
      btn.addEventListener('click', async () => {
        const program = btn.dataset.program;
        document.body.removeChild(modal);
        await openFileWith(path, program);
      });
    });
    
    modal.querySelector('#modal-cancel').addEventListener('click', () => {
      document.body.removeChild(modal);
    });
    
    // Add Enter key support for custom program input
    const customInput = modal.querySelector('#custom-program');
    customInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        modal.querySelector('#modal-custom-open').click();
      }
    });
    
    modal.querySelector('#modal-custom-open').addEventListener('click', async () => {
      const customProgram = modal.querySelector('#custom-program').value.trim();
      if (customProgram) {
        document.body.removeChild(modal);
        await openFileWith(path, customProgram);
      } else {
        // Highlight the input if empty
        const input = modal.querySelector('#custom-program');
        input.style.borderColor = '#ff4444';
        input.placeholder = 'Please enter a program path (e.g., notepad.exe)';
        input.focus();
        setTimeout(() => {
          input.style.borderColor = '';
        }, 2000);
      }
    });
    
    // Close on overlay click
    modal.addEventListener('click', (e) => {
      if (e.target === modal) {
        document.body.removeChild(modal);
      }
    });
  } catch (error) {
    console.error("Failed to get file info:", error);
    showError("Failed to get file info: " + error);
  }
}

// Start indexing
async function startIndexing() {
  try {
    console.log("Start indexing button clicked");
    indexBtn.disabled = true;
    indexBtn.textContent = "Indexing...";
    indexStatusEl.textContent = "Starting indexing...";
    
    const result = await invoke("start_indexing");
    console.log("Indexing result:", result);
    
    indexStatusEl.textContent = "Indexing in progress...";
    alert("Indexing started! This may take a few minutes. Watch the status for progress.");
    
    // Poll for status updates
    const interval = setInterval(async () => {
      await updateStatus();
    }, 2000);
    
    // Stop polling after 2 minutes
    setTimeout(() => {
      clearInterval(interval);
    }, 120000);
  } catch (error) {
    console.error("Failed to start indexing:", error);
    alert("Failed to start indexing: " + error);
    indexBtn.disabled = false;
    indexBtn.textContent = "Index Home Directory";
    indexStatusEl.textContent = "Indexing failed";
  }
}

// Update index status
async function updateStatus() {
  try {
    const status = await invoke("get_index_status");
    const count = status.total_files.toLocaleString();
    
    console.log("Index status:", status);

    if (!indexStatusEl) {
      console.error("indexStatusEl is not defined");
      return;
    }

    // Don't update status if we're currently indexing (button is disabled)
    const indexFolderBtn = document.querySelector("#index-folder-btn");
    if (indexFolderBtn && indexFolderBtn.disabled) {
      return; // Skip status update while indexing is in progress
    }

    if (status.total_files === 0) {
      indexStatusEl.textContent = "No files indexed yet";
    } else {
      indexStatusEl.textContent = `${count} files indexed`;
    }
  } catch (error) {
    console.error("Failed to get status:", error);
    if (indexStatusEl) {
      indexStatusEl.textContent = "Status error: " + error;
    }
  }
}

// Check if it's time for scheduled reindexing
async function checkScheduledReindex() {
  // Check if auto-reindex is enabled
  if (!autoReindexEnabled) {
    return;
  }
  
  const now = new Date();
  const hour = now.getHours();
  const currentDate = now.toDateString();
  
  // Define nighttime as 2 AM to 5 AM (customize as needed)
  const isNightTime = hour >= 2 && hour < 5;
  
  // Check if we already reindexed today
  const alreadyReindexedToday = lastReindexDate === currentDate;
  
  if (isNightTime && !alreadyReindexedToday) {
    console.log(`Scheduled reindex triggered at ${now.toLocaleTimeString()}`);
    
    try {
      // Get current status to see if we have files indexed
      const status = await invoke("get_index_status");
      
      if (status.total_files > 0) {
        // Only reindex if we have existing files
        console.log("Starting automatic nighttime reindex...");
        await invoke("start_indexing");
        lastReindexDate = currentDate;
        
        // Update status after a delay to show the new count
        setTimeout(updateStatus, 5000);
        
        console.log("Automatic reindex completed successfully");
      }
    } catch (error) {
      console.error("Scheduled reindex failed:", error);
    }
  }
  
  // If it's past 6 AM, reset the flag for next night
  if (hour >= 6) {
    if (lastReindexDate !== currentDate) {
      // This means we didn't reindex last night, which is okay
      lastReindexDate = null;
    }
  }
}

// Scroll selected item into view
function scrollToSelected() {
  const selected = document.querySelector(".file-item.selected");
  if (selected) {
    selected.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }
}

// Show error message
function showError(message) {
  // You can implement a toast notification here
  console.error(message);
  alert(message);
}

// Load indexed directories into dropdown
async function loadIndexedDirectories() {
  try {
    const directories = await invoke("get_indexed_directories");
    const selector = document.querySelector("#directory-selector");
    
    if (directories.length === 0) {
      selector.innerHTML = '<option value="">No directories indexed</option>';
      return;
    }
    
    selector.innerHTML = directories.map(dir => {
      const label = dir.name || dir.path;
      return `<option value="${dir.path}" ${dir.is_active ? 'selected' : ''}>${label}</option>`;
    }).join('');
  } catch (error) {
    console.error("Failed to load directories:", error);
  }
}

// Show success message
function showSuccess(message) {
  // You can implement a toast notification here
  console.log(message);
  alert(message);
}

// Escape HTML to prevent XSS
function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}

async function getIndexedFileCount() {
  try {
    const status = await invoke("get_index_status");
    return status.total_files.toLocaleString();
  } catch (error) {
    return "unknown";
  }
}

// Debug function to check if a specific file exists in database
async function debugCheckFile(filePath) {
  try {
    console.log("=== DEBUG: Testing search patterns ===");
    
    // Test various search patterns that should find KotlinConventions.java
    const patterns = [
      // Exact matches
      'KotlinConventions.java',
      'KotlinConventions',
      
      // Partial matches
      'Kotlin',
      'kotlin',
      'Conventions',
      'conventions',
      
      // Glob patterns
      'kotlin*',
      'Kotlin*',
      '*Conventions*',
      '*conventions*',
      '*.java',
      
      // Path components
      'spring-boot',
      'buildSrc',
      'charry',
      
      // Very broad
      'java',
      'org'
    ];
    
    for (const pattern of patterns) {
      try {
        const results = await invoke("search_files", { query: pattern, options: searchOptions });
        console.log(`📋 Pattern "${pattern}": ${results.length} results`);
        
        // Check if our specific file is in the results
        const targetFile = results.find(r => 
          r.path.toLowerCase().includes('kotlinconventions.java') ||
          r.name.toLowerCase().includes('kotlinconventions.java')
        );
        
        if (targetFile) {
          console.log(`✅ FOUND target file with pattern "${pattern}":`, targetFile);
          return targetFile;
        }
        
        // Also check for any files in the charry directory
        const charryFiles = results.filter(r => r.path.toLowerCase().includes('charry'));
        if (charryFiles.length > 0) {
          console.log(`📁 Found ${charryFiles.length} files in charry directory with pattern "${pattern}"`);
          if (charryFiles.length <= 5) {
            charryFiles.forEach(f => console.log(`   - ${f.name}: ${f.path}`));
          }
        }
        
        // Special check for buildSrc directory contents
        if (pattern === 'buildSrc') {
          const buildSrcFiles = results.filter(r => r.path.toLowerCase().includes('buildsrc'));
          console.log(`🔧 BuildSrc directory analysis:`);
          buildSrcFiles.forEach(f => {
            console.log(`   📂 ${f.name}: ${f.path}`);
          });
          
          // Try to find any files specifically in the buildSrc/src path
          console.log(`🔍 Searching for files in buildSrc/src path...`);
          try {
            const buildSrcSrcResults = await invoke("search_files", { 
              query: "buildSrc/src", 
              options: searchOptions 
            });
            console.log(`📋 "buildSrc/src" pattern: ${buildSrcSrcResults.length} results`);
            buildSrcSrcResults.slice(0, 10).forEach(f => {
              console.log(`   📄 ${f.name}: ${f.path}`);
            });
          } catch (e) {
            console.log(`❌ Error searching buildSrc/src:`, e);
          }
        }
        
        // Check for any java files
        if (pattern === '*.java' && results.length > 0) {
          console.log(`☕ Sample Java files found:`, results.slice(0, 3).map(f => f.name));
        }
        
      } catch (error) {
        console.error(`❌ Error with pattern "${pattern}":`, error);
      }
    }
    
    // Final specific check - look for files in the exact path structure
    console.log("🎯 FINAL CHECK: Looking for files in buildSrc/src/main/java path...");
    try {
      const pathPatterns = [
        "buildSrc/src/main",
        "src/main/java",
        "main/java/org",
        "java/org/springframework",
        "org/springframework/boot",
        "springframework/boot/build",
        "boot/build"
      ];
      
      for (const pathPattern of pathPatterns) {
        const pathResults = await invoke("search_files", { 
          query: pathPattern, 
          options: searchOptions 
        });
        console.log(`📍 Path "${pathPattern}": ${pathResults.length} results`);
        
        if (pathResults.length > 0 && pathResults.length <= 10) {
          pathResults.forEach(f => {
            console.log(`   📄 ${f.name}: ${f.path}`);
          });
        }
      }
    } catch (e) {
      console.log(`❌ Error in final path check:`, e);
    }
    
    console.log("=== END DEBUG ===");
    console.log("💡 Analysis complete. If files are missing:")
    console.log("   - The 10-level depth limit has been REMOVED from indexing")
    console.log("   - All files at any depth will now be indexed")
    console.log("   - Re-index the root directory (C:\\) to pick up previously missed deep files")
    console.log("   - Or re-index specific folders for faster results");
    
  } catch (error) {
    console.error('❌ Debug check failed:', error);
  }
}
