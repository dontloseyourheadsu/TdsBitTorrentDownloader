import { invoke } from "@tauri-apps/api/tauri";

const themeToggle = document.getElementById("theme-toggle");
const body = document.body;
const portInput = document.getElementById("port-input");
const udpToggle = document.getElementById("udp-toggle");
const startBtn = document.getElementById("start-btn");
const stopBtn = document.getElementById("stop-btn");
const statusVal = document.getElementById("status-val");
const logs = document.getElementById("logs");

let pollingInterval = null;

// Theme toggling
themeToggle.addEventListener("click", () => {
  body.classList.toggle("dark");
  themeToggle.textContent = body.classList.contains("dark") ? "☀️" : "🌙";
});

// Logging helper
function log(msg) {
  const p = document.createElement("p");
  p.textContent = `[${new Date().toLocaleTimeString()}] ${msg}`;
  logs.prepend(p);
}

async function updateStatus() {
  try {
    const status = await invoke("get_tracker_status");
    statusVal.textContent = status;
    if (status === "Running") {
      startBtn.disabled = true;
      stopBtn.disabled = false;
    } else {
      startBtn.disabled = false;
      stopBtn.disabled = true;
    }
  } catch (e) {
    console.error(e);
  }
}

startBtn.addEventListener("click", async () => {
  const portStr = portInput.value.trim();
  const port = parseInt(portStr);
  const useUdp = udpToggle.checked;

  if (isNaN(port)) {
     alert("Invalid port");
     return;
  }

  try {
    const res = await invoke("start_tracker", { port, useUdp });
    log(res);
    updateStatus();
    
    if (!pollingInterval) {
        pollingInterval = setInterval(updateStatus, 2000);
    }
  } catch (error) {
    log(`Error: ${error}`);
    alert("Failed to start tracker: " + error);
  }
});

stopBtn.addEventListener("click", async () => {
    try {
        const res = await invoke("stop_tracker");
        log(res);
        updateStatus();
    } catch(error) {
        log(`Error: ${error}`);
    }
});

// Init
updateStatus();
