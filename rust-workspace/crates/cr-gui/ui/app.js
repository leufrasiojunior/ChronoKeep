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
  backup_provider: "gdrive",
};
let defaultVaultDir = "";

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
// First-Run Disclaimer Modal
// --------------------------------------------------------------------------
function isDisclaimerAccepted() {
  try {
    return localStorage.getItem("chronokeep_disclaimer_accepted") === "true";
  } catch (_) {
    return false;
  }
}

function ensureDisclaimerAccepted(onAccepted) {
  if (isDisclaimerAccepted()) {
    onAccepted();
    return;
  }

  const modal = document.getElementById("disclaimer-modal");
  const btnCancel = document.getElementById("btn-disclaimer-cancel");
  const btnAccept = document.getElementById("btn-disclaimer-accept");
  if (!modal || !btnCancel || !btnAccept) {
    onAccepted();
    return;
  }

  modal.style.display = "flex";

  const closeModal = () => {
    modal.style.display = "none";
    btnCancel.onclick = null;
    btnAccept.onclick = null;
  };

  btnCancel.onclick = () => {
    closeModal();
  };

  btnAccept.onclick = () => {
    try {
      localStorage.setItem("chronokeep_disclaimer_accepted", "true");
    } catch (_) {}
    closeModal();
    onAccepted();
  };
}

// --------------------------------------------------------------------------
// Navigation (NavigationView)
// --------------------------------------------------------------------------
function setupNavigation() {
  const navItems = document.querySelectorAll(".nav-item[data-page]");
  const pages = document.querySelectorAll(".page");

  const activatePage = (btn, pageId) => {
    navItems.forEach((i) => i.classList.remove("active"));
    btn.classList.add("active");

    // Remove o badge de novidade ao clicar na aba correspondente
    const badge = btn.querySelector(".badge-pill");
    if (badge) {
      badge.remove();
      try {
        localStorage.setItem(`seen_badge_${pageId}`, "true");
      } catch (_) {}
    }

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
    } else if (pageId === "about") {
      if (!lastUpdateCheckResult) {
        checkForUpdates(false);
      }
    }
  };

  navItems.forEach((btn) => {
    const pageId = btn.getAttribute("data-page");
    try {
      if (localStorage.getItem(`seen_badge_${pageId}`)) {
        const badge = btn.querySelector(".badge-pill");
        if (badge) badge.remove();
      }
    } catch (_) {}

    btn.addEventListener("click", () => {
      if (pageId === "cloud-saves" && !isDisclaimerAccepted()) {
        ensureDisclaimerAccepted(() => {
          activatePage(btn, pageId);
        });
        return;
      }
      activatePage(btn, pageId);
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
      } else if (q.includes("provedor") || q.includes("provedores") || q.includes("drive") || q.includes("gdrive") || q.includes("cofre")) {
        document.querySelector("[data-page='providers']")?.click();
      } else if (q.includes("steam") || q.includes("porta") || q.includes("tema") || q.includes("config")) {
        document.querySelector("[data-page='settings']")?.click();
      } else if (q.includes("inicio") || q.includes("daemon")) {
        document.querySelector("[data-page='home']")?.click();
      }
    });
  }
}

function updateProviderPanel(provider) {
  const panelGdrive = document.getElementById("panel-provider-gdrive");
  const panelLocal = document.getElementById("panel-provider-local");
  if (provider === "local") {
    if (panelGdrive) panelGdrive.style.display = "none";
    if (panelLocal) panelLocal.style.display = "flex";
  } else {
    if (panelGdrive) panelGdrive.style.display = "flex";
    if (panelLocal) panelLocal.style.display = "none";
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

  // Provedores de Backup elements
  const providerPill = document.getElementById("provider-gdrive-status-pill");
  const providerBtnAction = document.getElementById("btn-provider-gdrive-action");
  const providerCardDesc = document.getElementById("provider-card-desc-gdrive");

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

    if (providerPill) {
      providerPill.textContent = "Conectado";
      providerPill.className = "status-pill active";
    }
    if (providerBtnAction) {
      providerBtnAction.textContent = "Desconectar Google Drive";
      providerBtnAction.className = "btn btn-secondary";
    }
    if (providerCardDesc) {
      providerCardDesc.textContent = "Conta Google vinculada — Sincronização em nuvem ativa";
    }
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

    if (providerPill) {
      providerPill.textContent = "Desconectado";
      providerPill.className = "status-pill";
    }
    if (providerBtnAction) {
      providerBtnAction.textContent = "Fazer Login no Google";
      providerBtnAction.className = "btn btn-primary";
    }
    if (providerCardDesc) {
      providerCardDesc.textContent = "Conecte sua conta para envio automático de saves e manifestos na nuvem";
    }
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
      defaultVaultDir = sysInfo.vault_dir;
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
      if (sysInfo.version) {
        const verEl = document.getElementById("about-version");
        if (verEl) verEl.textContent = `v${sysInfo.version}`;
      }
      const elGui = document.getElementById("sidebar-ver-gui");
      const elDll = document.getElementById("sidebar-ver-dll");
      const elDaemon = document.getElementById("sidebar-ver-daemon");
      if (elGui && sysInfo.gui_version) elGui.textContent = sysInfo.gui_version;
      if (elDll && sysInfo.dll_version) elDll.textContent = sysInfo.dll_version;
      if (elDaemon && sysInfo.daemon_version) elDaemon.textContent = sysInfo.daemon_version;
    }

    const cfg = await invokeBackend("get_config");
    if (cfg) {
      appConfig = { ...appConfig, ...cfg };
      if (!appConfig.vault_dir) {
        appConfig.vault_dir = defaultVaultDir;
      }
      document.getElementById("input-steam-dir").value = appConfig.steam_dir;
      document.getElementById("input-vault-dir").value = appConfig.vault_dir;
      if (document.getElementById("input-port")) {
        document.getElementById("input-port").value = appConfig.port;
      }
      document.getElementById("select-max-upload").value = String(appConfig.max_upload_mb);
      document.getElementById("select-theme").value = appConfig.theme;
      applyTheme(appConfig.theme);
      updateGDriveStatusUI(appConfig.gdrive_connected);

      // Sincroniza dropdown e painel do provedor de backup
      const providerSelect = document.getElementById("select-backup-provider");
      if (providerSelect) {
        providerSelect.value = appConfig.backup_provider || "gdrive";
        updateProviderPanel(providerSelect.value);
      }
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
  document.getElementById("tile-install")?.addEventListener("click", () => {
    ensureDisclaimerAccepted(async () => {
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
  document.getElementById("btn-provider-gdrive-action")?.addEventListener("click", gdriveAction);

  // Provedores de Backup Handlers
  const providerSelect = document.getElementById("select-backup-provider");
  if (providerSelect) {
    providerSelect.addEventListener("change", (e) => {
      updateProviderPanel(e.target.value);
    });
  }

  // Reset Vault Path to Default
  document.getElementById("btn-reset-vault-default")?.addEventListener("click", () => {
    const input = document.getElementById("input-vault-dir");
    if (input && defaultVaultDir) {
      input.value = defaultVaultDir;
      showToast("Caminho padrão restaurado: " + defaultVaultDir);
    }
  });

  // Save Settings Function (Geral & Provedores)
  const saveSettings = async (feedbackElId = "config-feedback") => {
    const portEl = document.getElementById("input-port");
    const providerEl = document.getElementById("select-backup-provider");
    const maxUploadEl = document.getElementById("select-max-upload");
    const steamDirEl = document.getElementById("input-steam-dir");
    const vaultDirEl = document.getElementById("input-vault-dir");
    const themeEl = document.getElementById("select-theme");

    const newCfg = {
      steam_dir: steamDirEl ? steamDirEl.value.trim() : appConfig.steam_dir,
      vault_dir: vaultDirEl ? vaultDirEl.value.trim() : appConfig.vault_dir,
      port: (portEl ? parseInt(portEl.value, 10) : appConfig.port) || 39771,
      gdrive_connected: appConfig.gdrive_connected,
      max_upload_mb: maxUploadEl ? parseInt(maxUploadEl.value, 10) : 512,
      theme: themeEl ? themeEl.value : appConfig.theme,
      backup_provider: providerEl ? providerEl.value : (appConfig.backup_provider || "gdrive"),
    };

    const res = await invokeBackend("save_config", { cfg: newCfg });
    if (res && res.success) {
      appConfig = { ...appConfig, ...newCfg };
      applyTheme(newCfg.theme);
      showToast("Configurações salvas com sucesso!");
      const fb = document.getElementById(feedbackElId);
      if (fb) {
        fb.textContent = "✓ Salvo!";
        setTimeout(() => { fb.textContent = ""; }, 2500);
      }
    } else {
      showToast(res ? res.message : "Erro ao salvar.");
    }
  };

  document.getElementById("btn-save-config")?.addEventListener("click", () => saveSettings("config-feedback"));
  document.getElementById("btn-save-providers")?.addEventListener("click", () => saveSettings("providers-feedback"));

  // Theme selector live preview
  document.getElementById("select-theme")?.addEventListener("change", (e) => {
    applyTheme(e.target.value);
  });

  // Scan Saves & Batch Sync
  document.getElementById("btn-scan-saves")?.addEventListener("click", scanSaves);
  document.getElementById("btn-sync-selected")?.addEventListener("click", syncSelectedSaves);

  // GitHub External Links
  const openGitHub = (e) => {
    e?.preventDefault();
    invokeBackend("open_url", { url: "https://github.com/leufrasiojunior/ChronoKeep" });
  };
  document.getElementById("nav-github-link")?.addEventListener("click", openGitHub);
  document.getElementById("btn-about-github")?.addEventListener("click", openGitHub);
  document.getElementById("link-disclaimer-issue")?.addEventListener("click", (e) => {
    e.preventDefault();
    invokeBackend("open_url", { url: "https://github.com/leufrasiojunior/ChronoKeep/issues" });
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

  // Updates actions
  document.getElementById("btn-check-updates")?.addEventListener("click", () => checkForUpdates(true));
  document.getElementById("btn-update-all")?.addEventListener("click", updateAllComponents);
  document.getElementById("btn-update-gui")?.addEventListener("click", () => updateComponent("gui"));
  document.getElementById("btn-update-daemon")?.addEventListener("click", () => updateComponent("daemon"));
  document.getElementById("btn-update-bridge")?.addEventListener("click", () => updateComponent("bridge"));
}

// --------------------------------------------------------------------------
// Saves Scanner & Renderer
// --------------------------------------------------------------------------
async function scanSaves() {
  if (!isDisclaimerAccepted()) {
    ensureDisclaimerAccepted(() => scanSaves());
    return;
  }

  const tbody = document.getElementById("tbody-saves");
  const scanBtn = document.getElementById("btn-scan-saves");
  const originalScanHtml = scanBtn ? scanBtn.innerHTML : "";
  if (scanBtn) {
    scanBtn.disabled = true;
    scanBtn.innerHTML = `<span class="fluent-spinner-sm"></span> Escaneando...`;
  }
  if (!tbody) return;
  tbody.innerHTML = `<tr><td colspan="8" class="empty-state"><div class="fluent-spinner" style="margin: 0 auto 12px auto;"></div>Varrendo pastas de saves locais no PC...</td></tr>`;

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
            <input type="checkbox" class="orphan-checkbox" 
                   data-appid="${item.app_id}" 
                   data-accountid="${item.account_id}"
                   data-name="${escapeHtml(item.name)}"
                   data-bytes="${item.total_bytes}"
                   data-files="${item.file_count}">
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
            <button class="btn btn-secondary btn-sm" onclick="syncGameSave(${item.app_id}, ${item.account_id}, this, ${item.total_bytes}, '${escapeHtml(item.name).replace(/'/g, "\\'")}')">
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
  } finally {
    if (scanBtn) {
      scanBtn.disabled = false;
      scanBtn.innerHTML = originalScanHtml;
    }
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

function formatBytes(bytes) {
  if (!bytes || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const val = (bytes / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 1);
  return `${val} ${units[i]}`;
}

let syncProgressTimeout = null;

// Permite ao navegador repintar a tela e manter spinners rodando suavemente
function yieldToUI(delayMs = 30) {
  return new Promise((resolve) => setTimeout(resolve, delayMs));
}

function showSyncProgress(title, detail, percent) {
  const card = document.getElementById("sync-progress-card");
  const titleEl = document.getElementById("progress-status-title");
  const detailEl = document.getElementById("progress-status-detail");
  const percentEl = document.getElementById("progress-percent-text");
  const barEl = document.getElementById("fluent-progress-bar");
  const spinnerEl = document.getElementById("sync-spinner");

  if (!card) return;
  card.style.display = "flex";
  if (syncProgressTimeout) {
    clearTimeout(syncProgressTimeout);
    syncProgressTimeout = null;
  }
  if (titleEl && title) titleEl.textContent = title;
  if (detailEl && detail) detailEl.textContent = detail;
  const pct = Math.min(100, Math.max(0, Math.round(percent || 0)));
  if (percentEl) percentEl.textContent = `${pct}%`;
  if (barEl) barEl.style.width = `${pct}%`;

  if (spinnerEl) {
    if (pct >= 100) {
      spinnerEl.style.display = "none";
    } else {
      spinnerEl.style.display = "inline-block";
    }
  }
}

function hideSyncProgress(delayMs = 3500) {
  if (syncProgressTimeout) clearTimeout(syncProgressTimeout);
  syncProgressTimeout = setTimeout(() => {
    const card = document.getElementById("sync-progress-card");
    if (card) card.style.display = "none";
  }, delayMs);
}

function updateSelectionUI() {
  const checked = Array.from(document.querySelectorAll(".orphan-checkbox:checked"));
  const count = checked.length;
  const btnSyncSelected = document.getElementById("btn-sync-selected");
  const countSpan = document.getElementById("selected-count");

  const totalSelectedBytes = checked.reduce(
    (acc, el) => acc + (parseInt(el.getAttribute("data-bytes"), 10) || 0),
    0
  );

  if (countSpan) {
    countSpan.textContent = count > 0 ? `${count} • ${formatBytes(totalSelectedBytes)}` : "0";
  }
  if (btnSyncSelected) {
    btnSyncSelected.disabled = count === 0;
  }
}

async function syncSelectedSaves() {
  const checked = Array.from(document.querySelectorAll(".orphan-checkbox:checked"));
  if (checked.length === 0) return;

  const btnSyncSelected = document.getElementById("btn-sync-selected");
  const originalHtml = btnSyncSelected ? btnSyncSelected.innerHTML : "";

  const totalGames = checked.length;
  const totalBytes = checked.reduce(
    (acc, el) => acc + (parseInt(el.getAttribute("data-bytes"), 10) || 0),
    0
  );

  let processedBytes = 0;
  let successCount = 0;
  const failedList = [];

  showSyncProgress(
    "Iniciando sincronização com o Google Drive...",
    `0 de ${totalGames} jogos • 0 B de ${formatBytes(totalBytes)}`,
    0
  );

  if (btnSyncSelected) {
    btnSyncSelected.disabled = true;
    btnSyncSelected.innerHTML = `<span class="fluent-spinner-sm"></span> Sincronizando (0/${totalGames})...`;
  }

  for (let i = 0; i < totalGames; i++) {
    const el = checked[i];
    const appId = parseInt(el.getAttribute("data-appid"), 10);
    const accountId = parseInt(el.getAttribute("data-accountid"), 10);
    const gameName = el.getAttribute("data-name") || `AppID ${appId}`;
    const gameBytes = parseInt(el.getAttribute("data-bytes"), 10) || 0;
    const row = el.closest("tr");

    const currentPercent = totalBytes > 0 ? (processedBytes / totalBytes) * 100 : (i / totalGames) * 100;
    showSyncProgress(
      `Sincronizando: ${gameName} (${i + 1} de ${totalGames})`,
      `${formatBytes(processedBytes)} de ${formatBytes(totalBytes)} • Jogo ${i + 1} de ${totalGames}`,
      currentPercent
    );

    if (btnSyncSelected) {
      btnSyncSelected.innerHTML = `<span class="fluent-spinner-sm"></span> Sincronizando (${i + 1}/${totalGames})...`;
    }

    if (row) {
      const cells = row.querySelectorAll("td");
      if (cells.length >= 7) {
        cells[6].innerHTML = `<span class="status-pill warning"><span class="fluent-spinner-sm"></span> Enviando...</span>`;
      }
    }

    // Dá um respiro para o navegador atualizar os spinners
    await yieldToUI(35);

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
        if (row) {
          const cells = row.querySelectorAll("td");
          if (cells.length >= 7) {
            cells[6].innerHTML = `<span class="status-pill active">✓ Na Nuvem</span>`;
          }
        }
      } else {
        const errorMsg = (res && res.message) ? res.message : "Falha desconhecida";
        failedList.push({ appId, error: errorMsg });
        if (row) {
          const cells = row.querySelectorAll("td");
          if (cells.length >= 7) {
            cells[6].innerHTML = `<span class="status-pill offline" title="${escapeHtml(errorMsg)}">Falha</span>`;
          }
        }
      }
    } catch (e) {
      console.error(`Erro ao sincronizar AppID ${appId}:`, e);
      failedList.push({ appId, error: e.message || String(e) });
      if (row) {
        const cells = row.querySelectorAll("td");
        if (cells.length >= 7) {
          cells[6].innerHTML = `<span class="status-pill offline">Erro</span>`;
        }
      }
    }

    processedBytes += gameBytes;
    const progressAfterGame = totalBytes > 0 ? (processedBytes / totalBytes) * 100 : ((i + 1) / totalGames) * 100;
    showSyncProgress(
      `Concluído: ${gameName}`,
      `${formatBytes(processedBytes)} de ${formatBytes(totalBytes)} • ${i + 1} de ${totalGames} jogos processados`,
      progressAfterGame
    );

    await yieldToUI(20);
  }

  showSyncProgress(
    "Sincronização em lote finalizada!",
    `${successCount} de ${totalGames} jogos sincronizados com sucesso • ${formatBytes(processedBytes)} enviados`,
    100
  );
  hideSyncProgress(4000);

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
async function syncGameSave(appId, accountId, btn, totalBytes = 0, gameName = "") {
  const originalHtml = btn ? btn.innerHTML : "Sincronizar";
  const displayName = gameName || `AppID ${appId}`;
  const row = btn ? btn.closest("tr") : null;

  if (btn) {
    btn.disabled = true;
    btn.innerHTML = `<span class="fluent-spinner-sm"></span> Enviando...`;
  }
  if (row) {
    const cells = row.querySelectorAll("td");
    if (cells.length >= 7) {
      cells[6].innerHTML = `<span class="status-pill warning"><span class="fluent-spinner-sm"></span> Enviando...</span>`;
    }
  }

  showSyncProgress(
    `Sincronizando ${displayName}...`,
    totalBytes > 0 ? `1 jogo • ${formatBytes(totalBytes)}` : "Enviando save para o Google Drive...",
    30
  );
  showToast(`Sincronizando ${displayName} com o Google Drive...`);

  await yieldToUI(35);

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
      showSyncProgress(
        `Sincronizado: ${displayName}!`,
        totalBytes > 0 ? `${formatBytes(totalBytes)} enviados com sucesso` : "Concluído",
        100
      );
      hideSyncProgress(3000);
      showToast(res.message, 4000);
      if (row) {
        const cells = row.querySelectorAll("td");
        if (cells.length >= 7) {
          cells[6].innerHTML = `<span class="status-pill active">✓ Na Nuvem</span>`;
        }
      }
      if (btn) {
        btn.textContent = "Sincronizado ✅";
        setTimeout(() => {
          btn.disabled = false;
          btn.innerHTML = originalHtml;
        }, 4000);
      }
    } else {
      const errorMsg = res ? res.message : "Falha na sincronização";
      showSyncProgress(
        `Falha ao sincronizar: ${displayName}`,
        errorMsg,
        0
      );
      hideSyncProgress(4000);
      showToast(errorMsg, 5000);
      if (row) {
        const cells = row.querySelectorAll("td");
        if (cells.length >= 7) {
          cells[6].innerHTML = `<span class="status-pill offline" title="${escapeHtml(errorMsg)}">Falha</span>`;
        }
      }
      if (btn) {
        btn.disabled = false;
        btn.innerHTML = originalHtml;
      }
    }
  } catch (err) {
    showSyncProgress(`Erro ao sincronizar: ${displayName}`, String(err), 0);
    hideSyncProgress(4000);
    showToast(`Erro ao sincronizar: ${err}`, 5000);
    if (row) {
      const cells = row.querySelectorAll("td");
      if (cells.length >= 7) {
        cells[6].innerHTML = `<span class="status-pill offline">Erro</span>`;
      }
    }
    if (btn) {
      btn.disabled = false;
      btn.innerHTML = originalHtml;
    }
  }
}
window.syncGameSave = syncGameSave;

// --------------------------------------------------------------------------
// Component Updater Controller
// --------------------------------------------------------------------------
let lastUpdateCheckResult = null;
let isUpdatingComponent = false;

function renderUpdateBadge(badgeEl, status) {
  if (!badgeEl) return;
  badgeEl.className = "badge-tag";
  if (status === "up_to_date") {
    badgeEl.classList.add("up-to-date");
    badgeEl.textContent = "Atualizado";
  } else if (status === "update_available") {
    badgeEl.classList.add("update-available");
    badgeEl.textContent = "Disponível";
  } else if (status === "not_installed") {
    badgeEl.classList.add("not-installed");
    badgeEl.textContent = "Não Instalado";
  } else if (status === "error") {
    badgeEl.classList.add("error");
    badgeEl.textContent = "Erro";
  } else {
    badgeEl.classList.add("neutral");
    badgeEl.textContent = "Não Verificado";
  }
}

async function checkForUpdates(showUserToast = false) {
  const btn = document.getElementById("btn-check-updates");
  const icon = document.getElementById("icon-check-updates");
  const text = document.getElementById("text-check-updates");
  const banner = document.getElementById("update-status-banner");
  const btnUpdateAll = document.getElementById("btn-update-all");

  if (btn) btn.disabled = true;
  if (icon) icon.classList.add("spin-icon");
  if (text) text.textContent = "Verificando...";

  try {
    const res = await invokeBackend("check_for_updates");
    lastUpdateCheckResult = res;

    if (!res || !res.success) {
      if (banner) {
        banner.className = "update-banner error";
        banner.style.display = "flex";
        banner.innerHTML = `<span>⚠️</span><span>${(res && res.message) ? res.message : "Falha ao verificar atualizações."}</span>`;
      }
      if (showUserToast) {
        showToast((res && res.message) ? res.message : "Falha ao verificar atualizações.");
      }
      return;
    }

    // Process components
    if (res.components && Array.isArray(res.components)) {
      for (const comp of res.components) {
        const id = comp.id;
        const curEl = document.getElementById(`meta-${id}-cur`);
        const latestEl = document.getElementById(`meta-${id}-latest`);
        const badgeEl = document.getElementById(`badge-${id}`);
        const updateBtn = document.getElementById(`btn-update-${id}`);

        if (curEl) curEl.textContent = comp.current_version || "-";
        if (latestEl) latestEl.textContent = comp.latest_version || "-";
        if (badgeEl) renderUpdateBadge(badgeEl, comp.status);

        // Atualiza os indicadores de versão na barra lateral
        if (id === "gui") {
          const el = document.getElementById("sidebar-ver-gui");
          if (el && comp.current_version) el.textContent = comp.current_version;
        } else if (id === "bridge") {
          const el = document.getElementById("sidebar-ver-dll");
          if (el && comp.current_version) el.textContent = comp.current_version;
        } else if (id === "daemon") {
          const el = document.getElementById("sidebar-ver-daemon");
          if (el && comp.current_version) el.textContent = comp.current_version;
        }

        if (updateBtn) {
          if (comp.status === "update_available") {
            updateBtn.style.display = "inline-flex";
            updateBtn.disabled = false;
            updateBtn.textContent = "Atualizar";
          } else {
            updateBtn.style.display = "none";
          }
        }
      }
    }

    if (banner) {
      if (res.has_updates) {
        banner.className = "update-banner info";
        banner.style.display = "flex";
        banner.innerHTML = `<span>🚀</span><span>Atualizações disponíveis.</span>`;
        if (btnUpdateAll) btnUpdateAll.style.display = "inline-flex";
      } else {
        banner.className = "update-banner success";
        banner.style.display = "flex";
        banner.innerHTML = `<span>✓</span><span>Todos os componentes do ChronoKeep estão atualizados (${res.release_tag || "Versão mais recente"}).</span>`;
        if (btnUpdateAll) btnUpdateAll.style.display = "none";
      }
    }

    if (showUserToast) {
      showToast(res.message);
    }
  } catch (err) {
    if (banner) {
      banner.className = "update-banner error";
      banner.style.display = "flex";
      banner.innerHTML = `<span>⚠️</span><span>Erro ao consultar atualizações: ${err}</span>`;
    }
    if (showUserToast) {
      showToast(`Erro ao verificar atualizações: ${err}`);
    }
  } finally {
    if (btn) btn.disabled = false;
    if (icon) icon.classList.remove("spin-icon");
    if (text) text.textContent = "Verificar Atualizações";
  }
}

async function updateComponent(componentId) {
  if (isUpdatingComponent) return;
  isUpdatingComponent = true;

  const btn = document.getElementById(`btn-update-${componentId}`);
  const originalText = btn ? btn.textContent : "Atualizar";
  if (btn) {
    btn.disabled = true;
    btn.textContent = "Atualizando...";
  }

  showToast(`Baixando e aplicando atualização de ${componentId}...`, 5000);

  try {
    const res = await invokeBackend("update_component", { component: componentId });
    if (res && res.success) {
      showToast(res.message, 6000);
      if (res.restart_required && componentId === "gui") {
        const banner = document.getElementById("update-status-banner");
        if (banner) {
          banner.className = "update-banner success";
          banner.style.display = "flex";
          banner.innerHTML = `<span>🔄</span><span>ChronoKeep atualizado! Reiniciando aplicativo...</span>`;
        }
      } else {
        await checkForUpdates(false);
      }
    } else {
      showToast(`Erro ao atualizar: ${(res && res.message) ? res.message : "Falha na operação."}`);
      if (btn) {
        btn.disabled = false;
        btn.textContent = originalText;
      }
    }
  } catch (err) {
    showToast(`Erro ao atualizar componente: ${err}`);
    if (btn) {
      btn.disabled = false;
      btn.textContent = originalText;
    }
  } finally {
    isUpdatingComponent = false;
  }
}

async function updateAllComponents() {
  if (isUpdatingComponent || !lastUpdateCheckResult) return;
  const available = (lastUpdateCheckResult.components || []).filter(c => c.status === "update_available");
  if (available.length === 0) {
    showToast("Nenhum componente pendente de atualização.");
    return;
  }

  // Atualiza primeiro o daemon, depois a bridge dll, e por último a interface (pois reinicia o app)
  const order = ["daemon", "bridge", "gui"];
  const sorted = available.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));

  for (const comp of sorted) {
    await updateComponent(comp.id);
  }
}

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

