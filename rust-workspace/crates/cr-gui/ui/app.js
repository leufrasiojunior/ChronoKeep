/**
 * ChronoKeep — Modern Fluent UI Frontend Controller
 * Communicates with Rust backend via Tauri v2 window.__TAURI__.core.invoke
 */

// Helper to invoke Tauri commands with safety
async function invokeBackend(cmd, args = {}) {
  if (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) {
    return await window.__TAURI__.core.invoke(cmd, args);
  } else {
    console.warn(`[ChronoKeep Mock] Command '${cmd}' called with:`, args);
    return null;
  }
}

// State
let appConfig = {
  steam_dir: "",
  vault_dir: "",
  port: 39771,
  gdrive_connected: false,
  max_upload_mb: 512,
  theme: "dark",
};

let daemonOnline = false;
let toastTimeout = null;

function showToast(message, duration = 3500) {
  const toast = document.getElementById("toast");
  if (!toast) return;
  const timeoutMs = (typeof duration === "number" && duration > 0) ? duration : 5000;
  toast.textContent = message;
  toast.classList.add("show");
  if (toastTimeout) clearTimeout(toastTimeout);
  toastTimeout = setTimeout(() => {
    toast.classList.remove("show");
  }, timeoutMs);
}

// --------------------------------------------------------------------------
// Navigation (NavigationView)
// --------------------------------------------------------------------------
function setupNavigation() {
  const navItems = document.querySelectorAll(".nav-item[data-page]");
  const pages = document.querySelectorAll(".page");

  navItems.forEach((btn) => {
    btn.addEventListener("click", () => {
      const pageId = btn.getAttribute("data-page");
      navItems.forEach((i) => i.classList.remove("active"));
      btn.classList.add("active");

      pages.forEach((p) => {
        if (p.id === `page-${pageId}`) {
          p.classList.add("active");
        } else {
          p.classList.remove("active");
        }
      });

      if (pageId === "logs") {
        loadLogs();
      } else if (pageId === "cloud-saves") {
        scanSaves();
      }
    });
  });

  // Global search navigation
  const searchInput = document.getElementById("global-search");
  if (searchInput) {
    searchInput.addEventListener("input", (e) => {
      const q = e.target.value.toLowerCase().trim();
      if (!q) return;
      if (q.includes("log")) {
        document.querySelector("[data-page='logs']")?.click();
      } else if (q.includes("save") || q.includes("nuvem") || q.includes("orfao")) {
        document.querySelector("[data-page='cloud-saves']")?.click();
      } else if (q.includes("steam") || q.includes("vault") || q.includes("porta") || q.includes("tema")) {
        document.querySelector("[data-page='settings']")?.click();
      } else if (q.includes("inicio") || q.includes("daemon")) {
        document.querySelector("[data-page='home']")?.click();
      }
    });
  }
}

// --------------------------------------------------------------------------
// Status Updates
// --------------------------------------------------------------------------
function updateDaemonStatusUI(online, port) {
  daemonOnline = online;
  const dot = document.getElementById("global-status-dot");
  const text = document.getElementById("global-status-text");
  const heroBadge = document.getElementById("hero-daemon-badge");
  const heroText = document.getElementById("hero-daemon-text");
  const toggle = document.getElementById("toggle-daemon");
  const tileLabel = document.getElementById("tile-daemon-label");
  const cardDesc = document.getElementById("card-desc-daemon");

  if (online) {
    if (dot) { dot.className = "status-dot online"; }
    if (text) text.textContent = "Daemon Ativo";
    if (heroBadge) {
      heroBadge.innerHTML = `<span class="badge-dot online"></span><span class="badge-text">Daemon Ativo</span>`;
    }
    if (toggle) toggle.checked = true;
    if (tileLabel) tileLabel.textContent = "Parar Daemon";
    if (cardDesc) cardDesc.textContent = "Serviço de sincronização ativo em segundo plano";
  } else {
    if (dot) { dot.className = "status-dot offline"; }
    if (text) text.textContent = "Daemon Inativo";
    if (heroBadge) {
      heroBadge.innerHTML = `<span class="badge-dot offline"></span><span class="badge-text">Daemon Desligado</span>`;
    }
    if (toggle) toggle.checked = false;
    if (tileLabel) tileLabel.textContent = "Iniciar Daemon";
    if (cardDesc) cardDesc.textContent = "Serviço de sincronização local desligado";
  }
}

function updateGDriveStatusUI(connected) {
  appConfig.gdrive_connected = connected;
  const heroBadge = document.getElementById("hero-gdrive-badge");
  const btnAction = document.getElementById("btn-gdrive-action");
  const tileLabel = document.getElementById("tile-gdrive-label");
  const cardDesc = document.getElementById("card-desc-gdrive");

  if (connected) {
    if (heroBadge) {
      heroBadge.innerHTML = `<span class="badge-dot online"></span><span class="badge-text">Google Drive Conectado</span>`;
    }
    if (btnAction) {
      btnAction.textContent = "Desconectar";
      btnAction.className = "btn btn-secondary";
    }
    if (tileLabel) tileLabel.textContent = "Drive Conectado";
    if (cardDesc) cardDesc.textContent = "Conta Google vinculada — Sincronização em nuvem ativa";
  } else {
    if (heroBadge) {
      heroBadge.innerHTML = `<span class="badge-dot offline"></span><span class="badge-text">Google Drive Desconectado</span>`;
    }
    if (btnAction) {
      btnAction.textContent = "Conectar";
      btnAction.className = "btn btn-primary";
    }
    if (tileLabel) tileLabel.textContent = "Conectar Drive";
    if (cardDesc) cardDesc.textContent = "Clique para autorizar upload de saves no navegador";
  }
}

// --------------------------------------------------------------------------
// Core Data Loading
// --------------------------------------------------------------------------
async function loadInitialData() {
  try {
    const sysInfo = await invokeBackend("get_system_info");
    if (sysInfo) {
      appConfig.steam_dir = sysInfo.steam_dir;
      appConfig.vault_dir = sysInfo.vault_dir;
      const ostPill = document.getElementById("ost-status-pill");
      if (ostPill) {
        if (sysInfo.opensteamtool_active) {
          ostPill.textContent = "Ativo";
          ostPill.className = "status-pill active";
        } else {
          ostPill.textContent = "Inativo / Não detectado";
          ostPill.className = "status-pill";
        }
      }
    }

    const cfg = await invokeBackend("get_config");
    if (cfg) {
      appConfig = { ...appConfig, ...cfg };
      document.getElementById("input-steam-dir").value = appConfig.steam_dir;
      document.getElementById("input-vault-dir").value = appConfig.vault_dir;
      if (document.getElementById("input-port")) {
        document.getElementById("input-port").value = appConfig.port;
      }
      document.getElementById("select-max-upload").value = String(appConfig.max_upload_mb);
      document.getElementById("select-theme").value = appConfig.theme;
      applyTheme(appConfig.theme);
      updateGDriveStatusUI(appConfig.gdrive_connected);
    }

    await checkDaemon();
  } catch (err) {
    console.error("Erro ao carregar dados iniciais:", err);
  }
}

async function checkDaemon() {
  try {
    const res = await invokeBackend("get_service_status", { steamDir: appConfig.steam_dir });
    if (res) {
      updateDaemonStatusUI(res.online, res.port);
    }
  } catch (e) {
    console.error("Falha ao consultar status do daemon:", e);
  }
}

function applyTheme(theme) {
  const html = document.documentElement;
  if (theme === "light") {
    html.setAttribute("data-theme", "light");
  } else if (theme === "dark") {
    html.setAttribute("data-theme", "dark");
  } else {
    // Seguir sistema
    const isDark = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
    html.setAttribute("data-theme", isDark ? "dark" : "light");
  }
}

// --------------------------------------------------------------------------
// Actions & Handlers
// --------------------------------------------------------------------------
function setupActionHandlers() {
  // Tile: Instalar na Steam
  document.getElementById("tile-install")?.addEventListener("click", async () => {
    showToast("Instalando componentes do ChronoKeep na Steam...");
    try {
      const res = await invokeBackend("install_chronokeep", {
        steamDir: appConfig.steam_dir,
        vaultDir: appConfig.vault_dir,
      });
      if (res && res.success) {
        showToast(res.message);
        await checkDaemon();
      } else {
        showToast(res ? res.message : "Falha na instalação.");
      }
    } catch (e) {
      showToast("Erro durante a instalação: " + e);
    }
  });

  // Toggle Daemon
  document.getElementById("toggle-daemon")?.addEventListener("change", async (e) => {
    if (e.target.checked) {
      showToast("Iniciando serviço ChronoKeep Daemon...");
      const res = await invokeBackend("start_service", { steamDir: appConfig.steam_dir });
      showToast(res ? res.message : "Comando enviado.");
    } else {
      showToast("Encerrando ChronoKeep Daemon...");
      const res = await invokeBackend("stop_service", { steamDir: appConfig.steam_dir });
      showToast(res ? res.message : "Comando enviado.");
    }
    await checkDaemon();
  });

  // Tile Daemon Button
  document.getElementById("tile-daemon")?.addEventListener("click", async () => {
    if (daemonOnline) {
      showToast("Encerrando daemon...");
      await invokeBackend("stop_service", { steamDir: appConfig.steam_dir });
    } else {
      showToast("Iniciando daemon...");
      await invokeBackend("start_service", { steamDir: appConfig.steam_dir });
    }
    await checkDaemon();
  });

  // Restart Daemon Button
  document.getElementById("btn-restart-daemon")?.addEventListener("click", async () => {
    showToast("Reiniciando serviço...");
    await invokeBackend("stop_service", { steamDir: appConfig.steam_dir });
    setTimeout(async () => {
      await invokeBackend("start_service", { steamDir: appConfig.steam_dir });
      await checkDaemon();
    }, 1000);
  });

  // Open Vault Explorer
  document.getElementById("tile-open-vault")?.addEventListener("click", async () => {
    await invokeBackend("open_directory", { path: appConfig.vault_dir });
  });
  document.getElementById("btn-explore-vault")?.addEventListener("click", async () => {
    const p = document.getElementById("input-vault-dir").value;
    await invokeBackend("open_directory", { path: p });
  });

  // Open Steam Explorer
  document.getElementById("tile-open-steam")?.addEventListener("click", async () => {
    await invokeBackend("open_directory", { path: appConfig.steam_dir });
  });
  document.getElementById("btn-explore-steam")?.addEventListener("click", async () => {
    const p = document.getElementById("input-steam-dir").value;
    await invokeBackend("open_directory", { path: p });
  });

  // Logs Quick Access Tile
  document.getElementById("tile-view-logs")?.addEventListener("click", () => {
    document.querySelector("[data-page='logs']")?.click();
  });

  // Google Drive Action (Connect / Disconnect)
  const gdriveAction = async () => {
    if (appConfig.gdrive_connected) {
      const res = await invokeBackend("disconnect_google", { steamDir: appConfig.steam_dir });
      showToast(res ? res.message : "Google Drive desconectado.");
      updateGDriveStatusUI(false);
    } else {
      showToast("Abrindo navegador para autorizar acesso ao Google Drive...");
      try {
        const res = await invokeBackend("start_google_login", { steamDir: appConfig.steam_dir });
        if (res && res.success) {
          showToast(res.message);
          updateGDriveStatusUI(true);
        } else {
          showToast("Falha na autenticação com o Google.");
        }
      } catch (e) {
        showToast("Erro no login: " + e);
      }
    }
  };
  document.getElementById("tile-gdrive-login")?.addEventListener("click", gdriveAction);
  document.getElementById("btn-gdrive-action")?.addEventListener("click", gdriveAction);

  // Save Settings
  document.getElementById("btn-save-config")?.addEventListener("click", async () => {
    const portEl = document.getElementById("input-port");
    const newCfg = {
      steam_dir: document.getElementById("input-steam-dir").value.trim(),
      vault_dir: document.getElementById("input-vault-dir").value.trim(),
      port: (portEl ? parseInt(portEl.value, 10) : appConfig.port) || 39771,
      gdrive_connected: appConfig.gdrive_connected,
      max_upload_mb: parseInt(document.getElementById("select-max-upload").value, 10) || 512,
      theme: document.getElementById("select-theme").value,
    };

    const res = await invokeBackend("save_config", { cfg: newCfg });
    if (res && res.success) {
      appConfig = { ...appConfig, ...newCfg };
      applyTheme(newCfg.theme);
      showToast("Configurações salvas com sucesso!");
      const fb = document.getElementById("config-feedback");
      if (fb) {
        fb.textContent = "✓ Salvo!";
        setTimeout(() => { fb.textContent = ""; }, 2500);
      }
    } else {
      showToast(res ? res.message : "Erro ao salvar.");
    }
  });

  // Theme selector live preview
  document.getElementById("select-theme")?.addEventListener("change", (e) => {
    applyTheme(e.target.value);
  });

  // Scan Saves & Batch Sync
  document.getElementById("btn-scan-saves")?.addEventListener("click", scanSaves);
  document.getElementById("btn-sync-selected")?.addEventListener("click", syncSelectedSaves);

  // GitHub External Link
  document.getElementById("nav-github-link")?.addEventListener("click", (e) => {
    e.preventDefault();
    invokeBackend("open_url", { url: "https://github.com/leufrasiojunior/ChronoKeep" });
  });

  // Logs actions
  document.getElementById("btn-refresh-log")?.addEventListener("click", loadLogs);
  document.getElementById("select-log-target")?.addEventListener("change", loadLogs);
  document.getElementById("btn-clear-log")?.addEventListener("click", async () => {
    const target = document.getElementById("select-log-target").value;
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const steamDir = inputSteam || appConfig.steam_dir;
    const res = await invokeBackend("clear_log", { logName: target, steamDir: steamDir || null });
    showToast(res ? res.message : "Log limpo.");
    loadLogs();
  });
}

// --------------------------------------------------------------------------
// Saves Scanner & Renderer
// --------------------------------------------------------------------------
async function scanSaves() {
  const tbody = document.getElementById("tbody-saves");
  if (!tbody) return;
  tbody.innerHTML = `<tr><td colspan="8" class="empty-state">Varrendo pastas de saves locais no PC...</td></tr>`;

  try {
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const inputVault = document.getElementById("input-vault-dir")?.value?.trim();
    const steamDir = inputSteam || appConfig.steam_dir;
    const vaultDir = inputVault || appConfig.vault_dir;

    const items = await invokeBackend("scan_saves", {
      steamDir: steamDir || null,
      vaultDir: vaultDir || null,
    });

    if (!items || items.length === 0) {
      tbody.innerHTML = `<tr><td colspan="8" class="empty-state">Nenhum save local de jogo encontrado nas pastas especificadas.</td></tr>`;
      updateSelectionUI();
      return;
    }

    tbody.innerHTML = items
      .map(
        (item) => `
        <tr data-appid="${item.app_id}" data-accountid="${item.account_id}">
          <td style="text-align: center;">
            <input type="checkbox" class="orphan-checkbox" data-appid="${item.app_id}" data-accountid="${item.account_id}">
          </td>
          <td><strong>${item.app_id}</strong></td>
          <td><span class="game-title">${escapeHtml(item.name)}</span></td>
          <td>${item.file_count} arquivos</td>
          <td>${item.total_size_formatted}</td>
          <td>
            <span class="status-pill active">✓ ${item.source.replace("Vault", "Cofre")}</span>
          </td>
          <td>
            ${
              item.in_cloud
                ? `<span class="status-pill active">✓ Na Nuvem</span>`
                : `<span class="status-pill warning">Pendente</span>`
            }
          </td>
          <td>
            <button class="btn btn-secondary btn-sm" onclick="syncGameSave(${item.app_id}, ${item.account_id}, this)">
              Sincronizar
            </button>
          </td>
        </tr>
      `
      )
      .join("");

    bindOrphanCheckboxes();
    updateSelectionUI();
  } catch (e) {
    tbody.innerHTML = `<tr><td colspan="8" class="empty-state" style="color:var(--error-color);">Erro ao escanear saves: ${e}</td></tr>`;
  }
}

function escapeHtml(text) {
  if (!text) return "";
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

function bindOrphanCheckboxes() {
  const master = document.getElementById("check-all-orphans");
  const checks = document.querySelectorAll(".orphan-checkbox");

  if (master) {
    master.checked = false;
    master.onchange = () => {
      checks.forEach((c) => (c.checked = master.checked));
      updateSelectionUI();
    };
  }

  checks.forEach((c) => {
    c.onchange = () => {
      if (master) {
        master.checked = Array.from(checks).length > 0 && Array.from(checks).every((ch) => ch.checked);
      }
      updateSelectionUI();
    };
  });
}

function updateSelectionUI() {
  const checked = document.querySelectorAll(".orphan-checkbox:checked");
  const count = checked.length;
  const btnSyncSelected = document.getElementById("btn-sync-selected");
  const countSpan = document.getElementById("selected-count");

  if (countSpan) countSpan.textContent = String(count);
  if (btnSyncSelected) {
    btnSyncSelected.disabled = count === 0;
  }
}

async function syncSelectedSaves() {
  const checked = Array.from(document.querySelectorAll(".orphan-checkbox:checked"));
  if (checked.length === 0) return;

  const btnSyncSelected = document.getElementById("btn-sync-selected");
  const originalHtml = btnSyncSelected ? btnSyncSelected.innerHTML : "";

  if (btnSyncSelected) {
    btnSyncSelected.disabled = true;
    btnSyncSelected.textContent = `Sincronizando 1 de ${checked.length}...`;
  }

  let successCount = 0;
  const failedList = [];
  for (let i = 0; i < checked.length; i++) {
    const el = checked[i];
    const appId = parseInt(el.getAttribute("data-appid"), 10);
    const accountId = parseInt(el.getAttribute("data-accountid"), 10);

    if (btnSyncSelected) {
      btnSyncSelected.textContent = `Sincronizando ${i + 1} de ${checked.length}...`;
    }

    try {
      const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
      const inputVault = document.getElementById("input-vault-dir")?.value?.trim();
      const steamDir = inputSteam || appConfig.steam_dir;
      const vaultDir = inputVault || appConfig.vault_dir;

      const res = await invokeBackend("upload_game_save", {
        appId,
        accountId,
        steamDir: steamDir || null,
        vaultDir: vaultDir || null,
      });

      if (res && res.success) {
        successCount++;
        const row = el.closest("tr");
        if (row) {
          const cells = row.querySelectorAll("td");
          if (cells.length >= 7) {
            cells[6].innerHTML = `<span class="status-pill active">✓ Na Nuvem</span>`;
          }
        }
      } else {
        const errorMsg = (res && res.message) ? res.message : "Falha desconhecida";
        failedList.push({ appId, error: errorMsg });
      }
    } catch (e) {
      console.error(`Erro ao sincronizar AppID ${appId}:`, e);
      failedList.push({ appId, error: e.message || String(e) });
    }
  }

  if (failedList.length === 0) {
    showToast(`${successCount} jogo(s) sincronizado(s) com a nuvem com sucesso!`, 4000);
  } else if (successCount === 0) {
    showToast(`Falha ao sincronizar ${failedList.length} jogo(s) selecionado(s): ${failedList[0].error}`, 6000);
  } else {
    showToast(`${successCount} jogo(s) sincronizado(s), mas ${failedList.length} falharam: ${failedList[0].error}`, 6000);
  }
  if (btnSyncSelected) {
    btnSyncSelected.disabled = false;
    btnSyncSelected.innerHTML = originalHtml || `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path><polyline points="17 8 12 3 7 8"></polyline><line x1="12" y1="3" x2="12" y2="15"></line></svg> Sincronizar Selecionados (<span id="selected-count">0</span>)`;
  }
  updateSelectionUI();
}

// --------------------------------------------------------------------------
// Manual Game Save Sync
// --------------------------------------------------------------------------
async function syncGameSave(appId, accountId, btn) {
  const originalText = btn ? btn.textContent : "Sincronizar";
  if (btn) {
    btn.disabled = true;
    btn.textContent = "Enviando...";
  }
  showToast(`Sincronizando AppID ${appId} com o Google Drive...`);

  try {
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const inputVault = document.getElementById("input-vault-dir")?.value?.trim();
    const steamDir = inputSteam || appConfig.steam_dir;
    const vaultDir = inputVault || appConfig.vault_dir;
    const res = await invokeBackend("upload_game_save", {
      appId,
      accountId,
      steamDir: steamDir || null,
      vaultDir: vaultDir || null,
    });
    if (res && res.success) {
      showToast(res.message, 4000);
      if (btn) {
        btn.textContent = "Sincronizado ✅";
        const row = btn.closest("tr");
        if (row) {
          const cells = row.querySelectorAll("td");
          if (cells.length >= 7) {
            cells[6].innerHTML = `<span class="status-pill active">✓ Na Nuvem</span>`;
          }
        }
        setTimeout(() => {
          btn.disabled = false;
          btn.textContent = originalText;
        }, 4000);
      }
    } else {
      showToast(res ? res.message : "Falha na sincronização", 5000);
      if (btn) {
        btn.disabled = false;
        btn.textContent = originalText;
      }
    }
  } catch (err) {
    showToast(`Erro ao sincronizar: ${err}`, 5000);
    if (btn) {
      btn.disabled = false;
      btn.textContent = originalText;
    }
  }
}
window.syncGameSave = syncGameSave;

// --------------------------------------------------------------------------
// Logs Loader
// --------------------------------------------------------------------------
async function loadLogs() {
  const output = document.getElementById("log-output");
  const target = document.getElementById("select-log-target")?.value || "daemon";
  if (!output) return;

  output.textContent = `Carregando ${target === "bridge" ? "chronokeep_bridge.log" : "chrono_daemon.log"}...`;

  try {
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const steamDir = inputSteam || appConfig.steam_dir;
    const content = await invokeBackend("get_logs", {
      logName: target,
      maxLines: 300,
      steamDir: steamDir || null,
    });
    output.textContent = content || "[Arquivo de log vazio]";
    output.scrollTop = output.scrollHeight;
  } catch (e) {
    output.textContent = `Erro ao carregar log: ${e}`;
  }
}

// --------------------------------------------------------------------------
// Lifecycle Startup
// --------------------------------------------------------------------------
document.addEventListener("DOMContentLoaded", () => {
  setupNavigation();
  setupActionHandlers();
  loadInitialData();

  // Background daemon poller every 3.5 seconds
  setInterval(() => {
    if (document.visibilityState === "visible") {
      checkDaemon();
    }
  }, 3500);
});

