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

// Search options
let searchOptions = {
  search_folders: true,
  enable_fuzzy: true,
  strict_mode: false,
  filename_only: false
};

// Initialize app
window.addEventListener("DOMContentLoaded", async () => {
  searchInput = document.querySelector("#search-input");
  resultsList = document.querySelector("#results-list");
  recentList = document.querySelector("#recent-list");
  indexStatusEl = document.querySelector("#index-status");
  indexBtn = document.querySelector("#index-btn");
  const debugBtn = document.querySelector("#debug-btn");

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
  
  indexBtn.addEventListener("click", startIndexing);
  debugBtn.addEventListener("click", debugJavaFiles);
  
  const testGlobBtn = document.querySelector("#test-glob-btn");
  testGlobBtn.addEventListener("click", testGlobPattern);
  
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
  
  // Setup settings checkboxes
  document.getElementById("search-folders").addEventListener("change", (e) => {
    searchOptions.search_folders = e.target.checked;
    performSearch(searchInput.value.trim());
  });
  
  document.getElementById("search-fuzzy").addEventListener("change", (e) => {
    searchOptions.enable_fuzzy = e.target.checked;
    performSearch(searchInput.value.trim());
  });
  
  document.getElementById("search-strict").addEventListener("change", (e) => {
    searchOptions.strict_mode = e.target.checked;
    performSearch(searchInput.value.trim());
  });
  
  document.getElementById("search-filename-only").addEventListener("change", (e) => {
    searchOptions.filename_only = e.target.checked;
    performSearch(searchInput.value.trim());
  });
  
  // Setup tab listeners
  document.querySelectorAll(".tab-btn").forEach(btn => {
    btn.addEventListener("click", (e) => {
      const tab = e.target.dataset.tab;
      switchTab(tab);
    });
  });

  // Load initial status
  await updateStatus();
  
  // Load recent files in the recent tab
  await loadRecentFiles();

  // Auto-refresh status (less frequent to avoid development interruptions)
  setInterval(updateStatus, 15000);
});

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
  
  // Load content for the active tab
  if (tabName === 'recent') {
    loadRecentFiles();
  } else if (tabName === 'search') {
    // Switch to search tab - if there's a query, perform search
    const query = searchInput.value.trim();
    if (query) {
      performSearch(query);
    } else {
      // Clear search results when switching to empty search
      renderSearchResults([]);
    }
  }
}

// Handle search input
function handleSearch() {
  // Auto-switch to search tab when typing
  if (activeTab !== 'search') {
    switchTab('search');
  }
  
  clearTimeout(searchTimeout);
  searchTimeout = setTimeout(async () => {
    const query = searchInput.value.trim();
    await performSearch(query);
  }, 50); // Debounce 50ms for faster response
}

// Perform search
async function performSearch(query) {
  try {
    if (query) {
      const results = await invoke("search_files", { query, options: searchOptions });
      currentResults = results;
      selectedIndex = 0;
      renderSearchResults(results);
    } else {
      currentResults = [];
      renderSearchResults([]);
    }
  } catch (error) {
    console.error("Search error:", error);
    showError("Search failed: " + error);
  }
}

// Load recent files
async function loadRecentFiles() {
  try {
    const results = await invoke("get_recent_files");
    if (activeTab === 'recent') {
      currentResults = results;
      selectedIndex = 0;
    }
    renderRecentResults(results);
  } catch (error) {
    console.error("Failed to load recent files:", error);
  }
}

// Render search results
function renderSearchResults(results) {
  if (results.length === 0) {
    if (searchInput.value.trim()) {
      resultsList.innerHTML = `
        <div class="empty-state">
          <h3>No files found</h3>
          <p>Try a different search term, glob pattern (*.js), or regex pattern</p>
        </div>
      `;
    } else {
      resultsList.innerHTML = `
        <div class="empty-state">
          <h3>Enter a search term</h3>
          <p>Search for files and folders, supports glob (*.js) and regex patterns</p>
        </div>
      `;
    }
    return;
  }

  const html = results
    .map((file, index) => {
      const isSelected = index === selectedIndex && activeTab === 'search';

      return `
        <div class="file-item ${isSelected ? 'selected' : ''}" data-index="${index}" data-path="${escapeHtml(file.path)}">
          <div class="file-name">${escapeHtml(file.name)}</div>
          <div class="file-path">${escapeHtml(file.path)}</div>
        </div>
      `;
    })
    .join("");

  resultsList.innerHTML = html;

  // Add click listeners
  resultsList.querySelectorAll(".file-item").forEach((item) => {
    item.addEventListener("click", () => {
      const path = item.dataset.path;
      openFile(path);
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
      const recentBadge = file.access_count > 1
        ? `<span class="recent-badge">Used ${file.access_count}x</span>`
        : '';

      return `
        <div class="file-item ${isSelected ? 'selected' : ''}" data-index="${index}" data-path="${escapeHtml(file.path)}">
          <div class="file-name">${escapeHtml(file.name)}</div>
          <div class="file-path">${escapeHtml(file.path)}</div>
          ${recentBadge ? `<div class="file-meta">${recentBadge}</div>` : ''}
        </div>
      `;
    })
    .join("");

  recentList.innerHTML = html;

  // Add click listeners
  recentList.querySelectorAll(".file-item").forEach((item) => {
    item.addEventListener("click", () => {
      const path = item.dataset.path;
      openFile(path);
    });
  });

  // Scroll selected item into view
  if (activeTab === 'recent') {
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

// Start indexing
async function startIndexing() {
  try {
    indexBtn.disabled = true;
    indexBtn.textContent = "Indexing...";
    await invoke("start_indexing");
    showSuccess("Indexing started! This may take a few minutes.");
  } catch (error) {
    console.error("Failed to start indexing:", error);
    showError("Failed to start indexing: " + error);
    indexBtn.disabled = false;
    indexBtn.textContent = "Start Indexing";
  }
}

// Update index status
async function updateStatus() {
  try {
    const status = await invoke("get_index_status");
    const count = status.total_files.toLocaleString();

    if (status.total_files === 0) {
      indexStatusEl.textContent = "No files indexed yet";
      indexBtn.disabled = false;
      indexBtn.textContent = "Start Indexing";
    } else {
      indexStatusEl.textContent = `${count} files indexed`;
      indexBtn.textContent = "Re-index";
      indexBtn.disabled = false;
    }
  } catch (error) {
    console.error("Failed to get status:", error);
    indexStatusEl.textContent = "Status unavailable";
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

// Show success message
function showSuccess(message) {
  // You can implement a toast notification here
  console.log(message);
  alert(message);
}

// Test glob pattern
async function testGlobPattern() {
  try {
    const result = await invoke("test_glob_pattern", { pattern: "*.java" });
    
    // Show results in a popup
    const debugDiv = document.createElement('div');
    debugDiv.style.cssText = `
      position: fixed;
      top: 20px;
      left: 20px;
      right: 20px;
      bottom: 20px;
      background: var(--bg-secondary);
      border: 2px solid var(--accent);
      border-radius: 10px;
      padding: 20px;
      color: var(--text-primary);
      font-family: monospace;
      font-size: 12px;
      white-space: pre-wrap;
      overflow-y: auto;
      z-index: 1000;
      box-shadow: 0 4px 20px rgba(0,0,0,0.5);
    `;
    
    debugDiv.innerHTML = `
      <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 15px;">
        <h3 style="margin: 0; color: var(--accent);">Glob Pattern Test</h3>
        <button onclick="this.parentElement.parentElement.remove()" style="background: var(--accent); color: white; border: none; padding: 5px 10px; border-radius: 4px; cursor: pointer;">Close</button>
      </div>
      <div>${result.replace(/\n/g, '<br>')}</div>
    `;
    
    document.body.appendChild(debugDiv);
    
  } catch (error) {
    console.error("Test failed:", error);
    alert("Test failed: " + error);
  }
}

// Debug Java files
async function debugJavaFiles() {
  try {
    // Get general info about indexed files (empty query gets extensions and samples)
    const result = await invoke("debug_search", { query: "" });
    
    // Show results in the search results area
    const debugDiv = document.createElement('div');
    debugDiv.style.cssText = `
      position: fixed;
      top: 20px;
      left: 20px;
      right: 20px;
      bottom: 20px;
      background: var(--bg-secondary);
      border: 2px solid var(--accent);
      border-radius: 10px;
      padding: 20px;
      color: var(--text-primary);
      font-family: monospace;
      font-size: 12px;
      white-space: pre-wrap;
      overflow-y: auto;
      z-index: 1000;
      box-shadow: 0 4px 20px rgba(0,0,0,0.5);
    `;
    
    debugDiv.innerHTML = `
      <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 15px;">
        <h3 style="margin: 0; color: var(--accent);">Debug Results</h3>
        <button onclick="this.parentElement.parentElement.remove()" style="background: var(--accent); color: white; border: none; padding: 5px 10px; border-radius: 4px; cursor: pointer;">Close</button>
      </div>
      <div>${result.replace(/\n/g, '<br>')}</div>
    `;
    
    document.body.appendChild(debugDiv);
    
  } catch (error) {
    console.error("Debug failed:", error);
    alert("Debug failed: " + error);
  }
}

// Escape HTML to prevent XSS
function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}
