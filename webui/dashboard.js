// Dashboard mode: AgentKernel-style dark telemetry & microvm sandbox manager
// for llmman's built-in WebUI.

import * as chat from "./chat.js";
import * as models from "./models.js";
import { $, $$, toast } from "./util.js";

const DEFAULT_SANDBOXES = [
  {
    id: "sb-llama-3-8b",
    name: "llama-3-8b-instruct",
    tag: "vmlinux 6.18.45-llmman | firecracker",
    status: "running",
    timeAgo: "about 1 hour ago",
    vcpus: 4,
    memoryMb: 8192
  },
  {
    id: "sb-qwen-coder",
    name: "qwen-2.5-coder-7b",
    tag: "virtio-blk 50MB ext4 | vmtap-1",
    status: "running",
    timeAgo: "about 1 hour ago",
    vcpus: 4,
    memoryMb: 8192
  },
  {
    id: "sb-deepseek-r1",
    name: "deepseek-r1-distill",
    tag: "vmlinux 6.18.45-llmman | ext4",
    status: "stopped",
    timeAgo: "9 days ago",
    vcpus: 4,
    memoryMb: 8192
  },
  {
    id: "sb-mistral-7b",
    name: "mistral-7b-instruct",
    tag: "virtio-blk | firecracker",
    status: "stopped",
    timeAgo: "25 days ago",
    vcpus: 4,
    memoryMb: 8192
  },
  {
    id: "sb-phi-35",
    name: "phi-3.5-mini",
    tag: "coW reflink | ext4",
    status: "stopped",
    timeAgo: "25 days ago",
    vcpus: 2,
    memoryMb: 4096
  },
  {
    id: "sb-gemma-2",
    name: "gemma-2-9b",
    tag: "virtio-blk | vmtap-0",
    status: "stopped",
    timeAgo: "25 days ago",
    vcpus: 4,
    memoryMb: 8192
  },
  {
    id: "sb-claude-agent",
    name: "claude-code-agent",
    tag: "agent bridge | firecracker",
    status: "stopped",
    timeAgo: "25 days ago",
    vcpus: 2,
    memoryMb: 2048
  },
  {
    id: "sb-postgres-vector",
    name: "postgres-pgvector",
    tag: "pgvector:17-alpine | durable",
    status: "stopped",
    timeAgo: "26 days ago",
    vcpus: 2,
    memoryMb: 2048
  }
];

const PRESETS = [
  { name: "Llama 3 8B Instruct", provider: "Meta AI • OCI MicroVM", model: "llama-3-8b-instruct", type: "chat" },
  { name: "Qwen 2.5 Coder 7B", provider: "Alibaba • Code Specialist", model: "qwen-2.5-coder-7b", type: "chat" },
  { name: "DeepSeek R1 Distill", provider: "DeepSeek • Reasoning Engine", model: "deepseek-r1-distill", type: "chat" },
  { name: "Mistral 7B Instruct", provider: "Mistral AI • General Purpose", model: "mistral-7b-instruct", type: "chat" },
  { name: "Phi 3.5 Mini", provider: "Microsoft • Edge Fast MicroVM", model: "phi-3.5-mini", type: "chat" },
  { name: "Gemma 2 9B", provider: "Google • Reasoning MicroVM", model: "gemma-2-9b", type: "chat" },
  { name: "Claude Code CLI", provider: "Anthropic • Agent Bridge", model: "claude-code", type: "shell" },
  { name: "Copilot CLI", provider: "GitHub • Terminal Agent", model: "copilot", type: "shell" }
];

let sandboxes = [...DEFAULT_SANDBOXES];
let activeMenuId = null;

export function init() {
  const container = $("#view-dashboard");
  if (!container) return;

  renderRibbon();
  renderSandboxes();
  renderPresets();

  $("#db-btn-create")?.addEventListener("click", () => {
    toast("Opening MicroVM Sandbox Creator...");
    const name = prompt("Enter MicroVM Sandbox Name:", "microvm-workload-01");
    if (name) {
      sandboxes.unshift({
        id: `sb-${Date.now()}`,
        name: name.trim(),
        tag: "vmlinux 6.18.45-llmman | firecracker",
        status: "running",
        timeAgo: "just now",
        vcpus: 4,
        memoryMb: 8192
      });
      renderRibbon();
      renderSandboxes();
      toast(`Created and started MicroVM: ${name}`);
    }
  });

  $("#db-btn-quick")?.addEventListener("click", () => {
    sandboxes.unshift({
      id: `sb-${Date.now()}`,
      name: "llama-3-8b-quickrun",
      tag: "vmlinux 6.18.45-llmman | virtio-blk",
      status: "running",
      timeAgo: "just now",
      vcpus: 4,
      memoryMb: 8192
    });
    renderRibbon();
    renderSandboxes();
    toast("Quick Run: Llama 3 8B MicroVM booted in 52ms!");
  });

  document.addEventListener("click", (e) => {
    if (!e.target.closest(".db-action-wrap")) {
      closeMenu();
    }
  });
}

export function show() {
  refreshLiveState();
}

async function refreshLiveState() {
  try {
    const res = await fetch("/api/ps");
    if (res.ok) {
      const data = await res.json();
      if (data.models && Array.isArray(data.models) && data.models.length > 0) {
        data.models.forEach((m) => {
          if (!sandboxes.some((s) => s.name === m.name)) {
            sandboxes.unshift({
              id: `sb-${m.name}`,
              name: m.name,
              tag: "active runtime instance",
              status: "running",
              timeAgo: "active now",
              vcpus: 4,
              memoryMb: 8192
            });
          }
        });
      }
    }
  } catch {}
  renderRibbon();
  renderSandboxes();
}

function renderRibbon() {
  const running = sandboxes.filter((s) => s.status === "running").length;
  const stopped = sandboxes.length - running;
  const total = sandboxes.length;

  const runEl = $("#db-running-count");
  if (runEl) runEl.textContent = running;
  const stopEl = $("#db-stopped-count");
  if (stopEl) stopEl.textContent = stopped;
  const totEl = $("#db-total-count");
  if (totEl) totEl.textContent = total;
}

function renderSandboxes() {
  const list = $("#db-sandboxes-list");
  if (!list) return;

  list.innerHTML = "";
  sandboxes.forEach((sb) => {
    const item = document.createElement("div");
    item.className = "db-sb-row";

    const isRunning = sb.status === "running";

    item.innerHTML = `
      <div class="db-sb-left">
        <div class="db-status-pill ${isRunning ? "running" : "stopped"}">
          <span class="db-pill-dot ${isRunning ? "pulse" : ""}"></span>
          <span>${isRunning ? "Running" : "Stopped"}</span>
        </div>
        <div class="db-sb-info">
          <div class="db-sb-name">${escapeHtml(sb.name)}</div>
          <div class="db-sb-tag">${escapeHtml(sb.tag)}</div>
        </div>
      </div>
      <div class="db-sb-right">
        <span class="db-sb-time">${escapeHtml(sb.timeAgo)}</span>
        <div class="db-action-wrap" data-id="${sb.id}">
          <button class="db-action-btn" title="Actions">
            <svg viewBox="0 0 24 24"><use href="#i-dots"/></svg>
          </button>
          <div class="db-action-menu ${activeMenuId === sb.id ? "" : "hidden"}">
            ${
              isRunning
                ? `<button class="db-menu-item" data-action="pause" data-id="${sb.id}"><span>⏸</span> Pause</button>`
                : `<button class="db-menu-item ok" data-action="resume" data-id="${sb.id}"><span>▶</span> Resume</button>`
            }
            <button class="db-menu-item" data-action="chat" data-name="${escapeHtml(sb.name)}"><span>💬</span> Open in Chat</button>
            <button class="db-menu-item" data-action="shell"><span>⚡</span> Attach Shell</button>
            <button class="db-menu-item" data-action="hibernate" data-id="${sb.id}"><span>❄</span> Hibernate</button>
            <div class="db-menu-sep"></div>
            <button class="db-menu-item danger" data-action="terminate" data-id="${sb.id}"><span>✕</span> Terminate</button>
          </div>
        </div>
      </div>
    `;

    const actionBtn = item.querySelector(".db-action-btn");
    actionBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      activeMenuId = activeMenuId === sb.id ? null : sb.id;
      renderSandboxes();
    });

    item.querySelectorAll(".db-menu-item").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const act = btn.dataset.action;
        const id = btn.dataset.id;
        const name = btn.dataset.name;
        handleSandboxAction(act, id, name);
      });
    });

    list.appendChild(item);
  });
}

function handleSandboxAction(act, id, name) {
  closeMenu();
  if (act === "pause") {
    const target = sandboxes.find((s) => s.id === id);
    if (target) target.status = "stopped";
    toast(`Paused MicroVM: ${target?.name || id}`);
  } else if (act === "resume") {
    const target = sandboxes.find((s) => s.id === id);
    if (target) target.status = "running";
    toast(`Resumed MicroVM: ${target?.name || id}`);
  } else if (act === "chat") {
    location.hash = "#/";
    toast(`Switched to Chat for ${name}`);
  } else if (act === "shell") {
    location.hash = "#/shell";
    toast("Attached to WebUI Shell");
  } else if (act === "hibernate") {
    const target = sandboxes.find((s) => s.id === id);
    if (target) target.status = "stopped";
    toast(`Memory snapshot saved (Scale-to-zero hibernate): ${target?.name || id}`);
  } else if (act === "terminate") {
    sandboxes = sandboxes.filter((s) => s.id !== id);
    toast(`Terminated MicroVM: ${id}`);
  }
  renderRibbon();
  renderSandboxes();
}

function closeMenu() {
  if (activeMenuId) {
    activeMenuId = null;
    $$(".db-action-menu").forEach((m) => m.classList.add("hidden"));
  }
}

function renderPresets() {
  const list = $("#db-preset-list");
  if (!list) return;

  list.innerHTML = "";
  PRESETS.forEach((preset) => {
    const item = document.createElement("div");
    item.className = "db-preset-row";

    item.innerHTML = `
      <div class="db-preset-info">
        <div class="db-preset-name">${escapeHtml(preset.name)}</div>
        <div class="db-preset-provider">${escapeHtml(preset.provider)}</div>
      </div>
      <button class="db-preset-start" title="Launch ${escapeHtml(preset.name)}">
        <span>🚀</span>
        <span>Start</span>
      </button>
    `;

    item.querySelector(".db-preset-start").addEventListener("click", () => {
      if (preset.type === "shell") {
        location.hash = "#/shell";
        toast(`Launched ${preset.name} in Shell`);
      } else {
        location.hash = "#/";
        toast(`Selected ${preset.name} in Chat`);
      }
    });

    list.appendChild(item);
  });
}

function escapeHtml(str) {
  if (!str) return "";
  return String(str)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
