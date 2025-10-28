const { invoke } = window.__TAURI__.core;

let searchInput;
let resultsList;
let indexStatusEl;
let indexBtn;
let selectedIndex = 0;
let currentResults = [];
let searchTimeout;

// Initialize app
window.addEventListener("DOMContentLoaded", async () => {
  searchInput = document.querySelector("#search-input");
  resultsList = document.querySelector("#results-list");
  indexStatusEl = document.querySelector("#index-status");
  indexBtn = document.querySelector("#index-btn");

  // Setup event listeners
  searchInput.addEventListener("input", handleSearch);
  searchInput.addEventListener("keydown", handleKeyboard);
  indexBtn.addEventListener("click", startIndexing);

  // Load initial status and recent files
  await updateStatus();
  await loadRecentFiles();

  // Auto-refresh status every 5 seconds during indexing
  setInterval(updateStatus, 5000);
});

// Handle search input
function handleSearch() {
  clearTimeout(searchTimeout);
  searchTimeout = setTimeout(async () => {
    const query = searchInput.value.trim();
    await performSearch(query);
  }, 150); // Debounce 150ms
}

// Perform search
async function performSearch(query) {
  try {
    const results = await invoke("search_files", { query });
    currentResults = results;
    selectedIndex = 0;
    renderResults(results);
  } catch (error) {
    console.error("Search error:", error);
    showError("Search failed: " + error);
  }
}

// Load recent files (when search is empty)
async function loadRecentFiles() {
  try {
    const results = await invoke("get_recent_files");
    currentResults = results;
    selectedIndex = 0;
    renderResults(results, true);
  } catch (error) {
    console.error("Failed to load recent files:", error);
  }
}

// Render results
function renderResults(results, isRecent = false) {
  if (results.length === 0) {
    if (searchInput.value.trim()) {
      resultsList.innerHTML = `
        <div class="empty-state">
          <h3>No files found</h3>
          <p>Try a different search term</p>
        </div>
      `;
    } else {
      resultsList.innerHTML = `
        <div class="empty-state">
          <h3>No recent files</h3>
          <p>Start opening files to see them here</p>
        </div>
      `;
    }
    return;
  }

  const html = results
    .map((file, index) => {
      const isSelected = index === selectedIndex;
      const recentBadge = isRecent && file.access_count > 1
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

  resultsList.innerHTML = html;

  // Add click listeners
  document.querySelectorAll(".file-item").forEach((item) => {
    item.addEventListener("click", () => {
      const path = item.dataset.path;
      openFile(path);
    });
  });

  // Scroll selected item into view
  scrollToSelected();
}

// Handle keyboard navigation
function handleKeyboard(e) {
  if (currentResults.length === 0) return;

  switch (e.key) {
    case "ArrowDown":
      e.preventDefault();
      selectedIndex = Math.min(selectedIndex + 1, currentResults.length - 1);
      renderResults(currentResults, !searchInput.value.trim());
      break;

    case "ArrowUp":
      e.preventDefault();
      selectedIndex = Math.max(selectedIndex - 1, 0);
      renderResults(currentResults, !searchInput.value.trim());
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
      loadRecentFiles();
      break;
  }
}

// Open file
async function openFile(path) {
  try {
    await invoke("open_file", { path });
    // Reload recent files after opening
    if (!searchInput.value.trim()) {
      await loadRecentFiles();
    }
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

// Escape HTML to prevent XSS
function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}
