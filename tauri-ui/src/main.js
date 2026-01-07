import { invoke } from "@tauri-apps/api/tauri";

let isDownloading = false;
let pollingInterval = null;

const themeToggle = document.getElementById("theme-toggle");
const body = document.body;
const torrentInput = document.getElementById("torrent-input");
const startBtn = document.getElementById("start-btn");
const statusCard = document.getElementById("status-card");
const progressBar = document.getElementById("progress-bar");
const downloadedVal = document.getElementById("downloaded-val");
const totalVal = document.getElementById("total-val");
const progressVal = document.getElementById("progress-val");
const logs = document.getElementById("logs");

// Theme toggling
themeToggle.addEventListener("click", () => {
  body.classList.toggle("dark");
  themeToggle.textContent = body.classList.contains("dark") ? "☀️" : "🌙";
  // You could save this preference to localStorage here
});

// Logging helper
function log(msg) {
  const p = document.createElement("p");
  p.textContent = `[${new Date().toLocaleTimeString()}] ${msg}`;
  logs.prepend(p);
}

function formatBytes(bytes) {
  if (bytes === 0) return "0 MB";
  const mb = bytes / (1024 * 1024);
  return mb.toFixed(2);
}

async function updateStatus() {
  try {
    const status = await invoke("get_status");
    if (status.active) {
      if (!isDownloading) {
        isDownloading = true;
        statusCard.classList.remove("hidden");
      }

      const pct = status.progress.toFixed(1) + "%";
      progressBar.style.width = pct;
      progressVal.textContent = pct;
      downloadedVal.textContent = formatBytes(status.downloaded);
      totalVal.textContent = formatBytes(status.total);

      if (status.progress >= 100) {
        log("Download completed!");
        clearInterval(pollingInterval);
      }
    }
  } catch (e) {
    console.error(e);
  }
}

startBtn.addEventListener("click", async () => {
  const input = torrentInput.value.trim();
  if (!input) {
    alert("Please enter a magnet link or file path");
    return;
  }

  startBtn.disabled = true;
  startBtn.textContent = "Starting...";
  log(`Starting download for: ${input}`);

  try {
    await invoke("start_download", { torrentInput: input, outputPath: null });
    log("Download started successfully.");
    startBtn.textContent = "Running";

    // Start polling
    if (pollingInterval) clearInterval(pollingInterval);
    pollingInterval = setInterval(updateStatus, 1000);
  } catch (error) {
    log(`Error: ${error}`);
    startBtn.disabled = false;
    startBtn.textContent = "Download";
    alert("Failed to start download: " + error);
  }
});
