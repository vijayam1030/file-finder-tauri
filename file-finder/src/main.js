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
  filename_only: false
};

// Initialize app
window.addEventListener("DOMContentLoaded", async () => {
  searchInput = document.querySelector("#search-input");
  resultsList = document.querySelector("#results-list");
  recentList = document.querySelector("#recent-list");
  indexStatusEl = document.querySelector("#index-status");
  indexBtn = document.querySelector("#index-btn");

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
  
  // Debug button
  const debugBtn = document.querySelector("#debug-btn");
  debugBtn.addEventListener("click", async () => {
    const query = searchInput.value.trim();
    if (!query) {
      alert("Enter a search query first");
      return;
    }
    const results = await invoke("debug_search_scores", { query });
    console.log("Debug scores for:", query);
    results.forEach(([name, score, path]) => {
      console.log(`${score}: ${name} (${path})`);
    });
    alert(`Check console for debug scores (${results.length} results)`);
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

  // Load initial status
  await updateStatus();
  
  // Load recent files in the recent tab
  await loadRecentFiles();

  // Auto-refresh status (less frequent to avoid development interruptions)
  setInterval(updateStatus, 15000);
  
  // Check for scheduled reindexing every hour
  checkScheduledReindex();
  setInterval(checkScheduledReindex, 60 * 60 * 1000); // Check every hour
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
  searchTimeout = setTimeout(async () => {
    const query = searchInput.value.trim();
    await performSearch(query);
  }, 50); // Debounce 50ms for faster response
}

// Perform search
async function performSearch(query) {
  try {
    if (query) {
      // Show searching indicator
      resultsList.innerHTML = `
        <div class="searching-indicator">
          <div class="spinner"></div>
          <p>Searching...</p>
        </div>
      `;
      
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
    // Clear the searching indicator on error
    renderSearchResults([]);
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
      const ext = file.name.includes('.') ? file.name.split('.').pop().toUpperCase() : 'FILE';

      return `
        <div class="file-item ${isSelected ? 'selected' : ''}" data-index="${index}" data-path="${escapeHtml(file.path)}">
          <div class="file-info-row">
            <div class="file-name">${escapeHtml(file.name)}</div>
            <span class="file-ext-badge">${ext}</span>
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
      // Don't open if clicking the "Open with" button
      if (e.target.classList.contains('open-with-btn')) {
        return;
      }
      const path = item.dataset.path;
      openFile(path);
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
