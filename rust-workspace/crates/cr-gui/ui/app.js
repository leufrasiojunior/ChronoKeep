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
  language: "system",
  backup_provider: "gdrive",
};
let defaultVaultDir = "";
let osLocale = "";
let sysInfoCache = null;

let daemonOnline = false;
let toastTimeout = null;

function formatVersionLabel(val) {
  if (!val) return "-";
  const lower = String(val).trim().toLowerCase();
  if (lower === "não instalado" || lower === "nao instalado" || lower === "not installed") {
    return t("common.not_installed");
  }
  if (lower === "instalado" || lower === "installed") {
    return t("common.installed");
  }
  return val;
}

function formatSaveSource(source) {
  if (!source) return t("saves.source_vault_local");
  const norm = String(source).trim().toLowerCase();
  if (norm === "steam_userdata" || norm === "steam userdata") {
    return t("saves.source_steam_userdata");
  }
  if (norm === "steam_vault" || norm === "steam + cofre" || norm === "steam + vault") {
    return t("saves.source_steam_vault");
  }
  if (norm === "vault_local" || norm === "cofre local" || norm === "local vault") {
    return t("saves.source_vault_local");
  }
  return source;
}

function applyLanguage(pref, refreshDynamic = true) {
  const normalizedPref = pref || "system";
  appConfig.language = normalizedPref;
  try {
    localStorage.setItem("chronokeep_lang_pref", normalizedPref);
  } catch (_) {}

  if (window.ChronoI18n) {
    const resolved = window.ChronoI18n.resolveLocale(normalizedPref, osLocale);
    window.ChronoI18n.setLocale(resolved);
    window.ChronoI18n.applyI18nToDOM();
    const rawVer = (sysInfoCache && sysInfoCache.version) ? sysInfoCache.version : "v0.1.18";
    const vStr = rawVer.startsWith("v") ? rawVer : `v${rawVer}`;
    const heroVerEl = document.getElementById("hero-version");
    if (heroVerEl) heroVerEl.textContent = t("home.hero_version", { version: vStr });
  }

  const langSelect = document.getElementById("select-language");
  if (langSelect && langSelect.value !== normalizedPref) {
    langSelect.value = normalizedPref;
  }

  if (refreshDynamic) {
    refreshDynamicUI();
  }
}

function refreshSystemInfoUI() {
  if (!sysInfoCache) return;
  const ostPill = document.getElementById("ost-status-pill");
  if (ostPill) {
    if (sysInfoCache.opensteamtool_active) {
      ostPill.textContent = t("status.ost_active");
      ostPill.className = "status-pill active";
    } else {
      ostPill.textContent = t("status.ost_inactive");
      ostPill.className = "status-pill";
    }
  }

  if (sysInfoCache.version) {
    const vStr = sysInfoCache.version.startsWith("v") ? sysInfoCache.version : `v${sysInfoCache.version}`;
    const verEl = document.getElementById("about-version");
    if (verEl) verEl.textContent = vStr;
    const heroVerEl = document.getElementById("hero-version") || document.querySelector(".hero-version");
    if (heroVerEl) heroVerEl.textContent = t("home.hero_version", { version: vStr });
  }

  const elGui = document.getElementById("sidebar-ver-gui");
  const elDll = document.getElementById("sidebar-ver-dll");
  const elDaemon = document.getElementById("sidebar-ver-daemon");
  if (elGui && sysInfoCache.gui_version) elGui.textContent = formatVersionLabel(sysInfoCache.gui_version);
  if (elDll && sysInfoCache.dll_version) elDll.textContent = formatVersionLabel(sysInfoCache.dll_version);
  if (elDaemon && sysInfoCache.daemon_version) elDaemon.textContent = formatVersionLabel(sysInfoCache.daemon_version);
}

function refreshDynamicUI() {
  refreshSystemInfoUI();
  updateDaemonStatusUI(daemonOnline, appConfig.port);
  updateGDriveStatusUI(appConfig.gdrive_connected);
  renderSavesTable();
  if (lastUpdateCheckResult) {
    renderUpdateCheckResultUI(lastUpdateCheckResult);
  }
  const logsPage = document.getElementById("page-logs");
  if (logsPage && logsPage.classList.contains("active")) {
    loadLogs();
  }
}

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

function openUninstallModal() {
  const modal = document.getElementById("uninstall-modal");
  const btnCancel = document.getElementById("btn-uninstall-cancel");
  const btnConfirm = document.getElementById("btn-uninstall-confirm");
  const checkRestore = document.getElementById("check-restore-saves-on-uninstall");
  const vaultLocEl = document.getElementById("uninstall-vault-location");
  if (!modal || !btnCancel || !btnConfirm) return;

  if (checkRestore) {
    checkRestore.checked = false;
  }
  if (vaultLocEl) {
    const inputVault = document.getElementById("input-vault-dir")?.value?.trim();
    vaultLocEl.textContent = inputVault || appConfig.vault_dir || "—";
  }

  btnConfirm.disabled = false;
  btnConfirm.textContent = t("modal.uninstall_confirm");
  modal.style.display = "flex";

  const closeModal = () => {
    modal.style.display = "none";
    btnCancel.onclick = null;
    btnConfirm.onclick = null;
    btnConfirm.disabled = false;
    btnConfirm.textContent = t("modal.uninstall_confirm");
  };

  btnCancel.onclick = closeModal;

  btnConfirm.onclick = async () => {
    btnConfirm.disabled = true;
    btnConfirm.textContent = t("modal.uninstall_working");
    const restoreSaves = checkRestore ? checkRestore.checked : false;

    try {
      const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
      const inputVault = document.getElementById("input-vault-dir")?.value?.trim();
      const steamDir = inputSteam || appConfig.steam_dir;
      const vaultDir = inputVault || appConfig.vault_dir;

      const res = await invokeBackend("uninstall_chronokeep", {
        restoreToSteam: restoreSaves,
        steamDir: steamDir || null,
        vaultDir: vaultDir || null,
      });

      closeModal();
      if (res && res.success) {
        showToast(res.message, 8000);
        checkDaemon();
      } else {
        showToast(t("toast.uninstall_failed", { error: res ? res.message : t("common.unknown_error") }));
      }
    } catch (e) {
      closeModal();
      showToast(t("toast.uninstall_exception", { error: e }));
    }
  };
}

function setupUploadLimitHelpModal() {
  const btnHelp = document.getElementById("btn-upload-limit-help");
  const modal = document.getElementById("upload-limit-modal");
  const btnClose = document.getElementById("btn-upload-limit-close");

  if (!btnHelp || !modal || !btnClose) return;

  const openModal = () => {
    modal.style.display = "flex";
  };

  const closeModal = () => {
    modal.style.display = "none";
  };

  btnHelp.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    openModal();
  });

  btnClose.addEventListener("click", closeModal);

  modal.addEventListener("click", (e) => {
    if (e.target === modal) {
      closeModal();
    }
  });

  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && modal.style.display === "flex") {
      closeModal();
    }
  });
}

window.openExternalUrl = (url) => {
  if (url) {
    invokeBackend("open_url", { url }).catch(() => {});
  }
};

let currentDependenciesReport = null;

async function checkAndShowDependenciesModal(onProceed = null) {
  const modal = document.getElementById("dependencies-modal");
  const listEl = document.getElementById("dep-items-list");
  const btnRefresh = document.getElementById("btn-dep-refresh");
  const btnCancel = document.getElementById("btn-dep-cancel");
  const btnProceed = document.getElementById("btn-dep-proceed");
  const bannerCallout = document.getElementById("dep-banner-callout");
  const bannerIcon = document.getElementById("dep-banner-icon");
  const bannerTitle = document.getElementById("dep-banner-title");
  const bannerDesc = document.getElementById("dep-banner-desc");

  if (!modal || !listEl) return;

  const loadReport = async () => {
    listEl.innerHTML = `<div style="text-align: center; padding: 24px; color: var(--text-secondary);">${t("common.command_sent")}</div>`;
    try {
      const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
      const steamDir = inputSteam || appConfig.steam_dir;
      const report = await invokeBackend("check_system_dependencies", { steamDir: steamDir || null });
      currentDependenciesReport = report;
      renderReport(report);
    } catch (e) {
      listEl.innerHTML = `<div style="color: var(--error-color); padding: 12px;">Erro ao verificar dependências: ${e}</div>`;
    }
  };

  const renderReport = (report) => {
    if (report.has_missing_required) {
      bannerCallout.className = "dialog-callout warning";
      bannerIcon.textContent = "❌";
      bannerTitle.textContent = t("modal.dep_banner_missing_req_title");
      bannerDesc.textContent = t("modal.dep_banner_missing_req_desc");
    } else if (report.has_missing_third_party) {
      bannerCallout.className = "dialog-callout warning";
      bannerIcon.textContent = "⚠️";
      bannerTitle.textContent = t("modal.dep_banner_tp_warn_title");
      bannerDesc.textContent = t("modal.dep_banner_tp_warn_desc");
    } else {
      bannerCallout.className = "dialog-callout safe";
      bannerIcon.textContent = "✓";
      bannerTitle.textContent = t("modal.dep_banner_all_ok_title");
      bannerDesc.textContent = t("modal.dep_banner_all_ok_desc");
    }

    listEl.innerHTML = report.items.map((item) => {
      const iconColor = item.is_installed ? "green" : (item.is_required ? "orange" : "cyan");
      const pillClass = item.is_installed ? "active" : (item.is_required ? "error" : "warning");
      let actionBtn = "";

      if (item.action_type === "download" || item.action_type === "search") {
        const btnStyle = item.action_type === "download" ? "btn-primary" : "btn-secondary";
        actionBtn = `
          <button type="button" class="btn ${btnStyle} btn-sm" style="margin-top: 8px; font-size: 11.5px; padding: 5px 12px;" onclick="window.openExternalUrl('${item.action_target}')">
            ${item.action_label}
          </button>
        `;
      }

      const iconPath = item.is_installed
        ? '<polyline points="20 6 9 17 4 12"></polyline>'
        : (item.is_required
          ? '<circle cx="12" cy="12" r="10"></circle><line x1="15" y1="9" x2="9" y2="15"></line><line x1="9" y1="9" x2="15" y2="15"></line>'
          : '<circle cx="12" cy="12" r="10"></circle><line x1="12" y1="8" x2="12" y2="12"></line><line x1="12" y1="16" x2="12.01" y2="16"></line>');

      return `
        <div class="settings-card" style="padding: 12px 14px; margin: 0; background: var(--bg-subtle); border-radius: var(--radius-sm); border: 1px solid var(--card-border);">
          <div class="card-left" style="gap: 12px; align-items: flex-start; width: 100%;">
            <div class="card-icon-box ${iconColor}" style="margin-top: 2px;">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                ${iconPath}
              </svg>
            </div>
            <div class="card-text" style="flex: 1;">
              <div style="display: flex; align-items: center; justify-content: space-between; gap: 8px; flex-wrap: wrap;">
                <span class="card-name" style="font-weight: 600;">${item.name}</span>
                <span class="status-pill ${pillClass}">
                  ${item.status_text}
                </span>
              </div>
              <div class="card-desc" style="margin-top: 4px; line-height: 1.45;">${item.description}</div>
              ${actionBtn}
            </div>
          </div>
        </div>
      `;
    }).join("");

    if (onProceed) {
      btnProceed.style.display = "inline-flex";
      if (!report.can_install) {
        btnProceed.disabled = true;
        btnProceed.title = t("modal.dep_banner_missing_req_desc");
      } else {
        btnProceed.disabled = false;
        btnProceed.title = "";
      }
    } else {
      btnProceed.style.display = "none";
    }
  };

  const closeModal = () => {
    modal.style.display = "none";
    if (btnRefresh) btnRefresh.onclick = null;
    if (btnCancel) btnCancel.onclick = null;
    if (btnProceed) btnProceed.onclick = null;
  };

  modal.style.display = "flex";
  await loadReport();

  if (btnRefresh) btnRefresh.onclick = () => loadReport();
  if (btnCancel) btnCancel.onclick = closeModal;

  if (btnProceed) {
    btnProceed.onclick = () => {
      if (currentDependenciesReport && currentDependenciesReport.can_install) {
        closeModal();
        if (typeof onProceed === "function") {
          onProceed();
        }
      }
    };
  }

  modal.onclick = (e) => {
    if (e.target === modal) closeModal();
  };
}

// --------------------------------------------------------------------------
// Navigation (NavigationView)
// --------------------------------------------------------------------------
function navigateToPage(pageId, targetElementId = null) {
  const navItems = document.querySelectorAll(".nav-item[data-page]");
  const pages = document.querySelectorAll(".page");
  const targetBtn = document.querySelector(`.nav-item[data-page="${pageId}"]`);

  navItems.forEach((i) => i.classList.remove("active"));
  if (targetBtn) {
    targetBtn.classList.add("active");
    const badge = targetBtn.querySelector(".badge-pill");
    if (badge) {
      badge.remove();
      try {
        localStorage.setItem(`seen_badge_${pageId}`, "true");
      } catch (_) {}
    }
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

  if (targetElementId) {
    setTimeout(() => {
      const target = document.getElementById(targetElementId);
      if (target) {
        target.scrollIntoView({ behavior: "smooth", block: "start" });
        target.classList.add("highlight-card");
        setTimeout(() => target.classList.remove("highlight-card"), 2000);
      }
    }, 120);
  }
}

function setupNavigation() {
  const navItems = document.querySelectorAll(".nav-item[data-page]");

  const activatePage = (btn, pageId) => {
    navigateToPage(pageId);
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

  // Global search navigation (bilingual PT-BR & EN-US)
  const searchInput = document.getElementById("global-search");
  if (searchInput) {
    searchInput.addEventListener("input", (e) => {
      const q = e.target.value.toLowerCase().trim();
      if (!q) return;
      if (q.includes("log")) {
        document.querySelector("[data-page='logs']")?.click();
      } else if (
        q.includes("save") ||
        q.includes("nuvem") ||
        q.includes("cloud") ||
        q.includes("orfao") ||
        q.includes("órfao") ||
        q.includes("orphan")
      ) {
        document.querySelector("[data-page='cloud-saves']")?.click();
      } else if (
        q.includes("provedor") ||
        q.includes("provider") ||
        q.includes("backup") ||
        q.includes("drive") ||
        q.includes("gdrive") ||
        q.includes("cofre") ||
        q.includes("vault")
      ) {
        document.querySelector("[data-page='providers']")?.click();
      } else if (
        q.includes("steam") ||
        q.includes("porta") ||
        q.includes("port") ||
        q.includes("tema") ||
        q.includes("theme") ||
        q.includes("idioma") ||
        q.includes("lang") ||
        q.includes("config") ||
        q.includes("setting") ||
        q.includes("update") ||
        q.includes("atualiza")
      ) {
        document.querySelector("[data-page='settings']")?.click();
      } else if (q.includes("sobre") || q.includes("about")) {
        document.querySelector("[data-page='about']")?.click();
      } else if (q.includes("inicio") || q.includes("início") || q.includes("home") || q.includes("daemon")) {
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
  const toggle = document.getElementById("toggle-daemon");
  const tileLabel = document.getElementById("tile-daemon-label");
  const cardDesc = document.getElementById("card-desc-daemon");

  if (online) {
    if (dot) dot.className = "status-dot online";
    if (text) text.textContent = t("status.daemon_active");
    if (heroBadge) {
      heroBadge.innerHTML = `<span class="badge-dot online"></span><span class="badge-text">${escapeHtml(t("status.daemon_active"))}</span>`;
    }
    if (toggle) toggle.checked = true;
    if (tileLabel) tileLabel.textContent = t("home.tile_stop_daemon");
    if (cardDesc) cardDesc.textContent = t("home.daemon_desc_online");
  } else {
    if (dot) dot.className = "status-dot offline";
    if (text) text.textContent = t("status.daemon_inactive");
    if (heroBadge) {
      heroBadge.innerHTML = `<span class="badge-dot offline"></span><span class="badge-text">${escapeHtml(t("status.daemon_offline"))}</span>`;
    }
    if (toggle) toggle.checked = false;
    if (tileLabel) tileLabel.textContent = t("home.tile_start_daemon");
    if (cardDesc) cardDesc.textContent = t("home.daemon_desc_offline");
  }
}

function updateGDriveStatusUI(connected) {
  appConfig.gdrive_connected = connected;
  const heroBadge = document.getElementById("hero-gdrive-badge");
  const btnAction = document.getElementById("btn-gdrive-action");
  const tileLabel = document.getElementById("tile-gdrive-label");
  const cardDesc = document.getElementById("card-desc-gdrive");

  // Backup Providers page elements
  const providerPill = document.getElementById("provider-gdrive-status-pill");
  const providerBtnAction = document.getElementById("btn-provider-gdrive-action");
  const providerCardDesc = document.getElementById("provider-card-desc-gdrive");

  if (connected) {
    if (heroBadge) {
      heroBadge.innerHTML = `<span class="badge-dot online"></span><span class="badge-text">${escapeHtml(t("status.gdrive_connected_badge"))}</span>`;
    }
    if (btnAction) {
      btnAction.textContent = t("home.btn_disconnect");
      btnAction.className = "btn btn-secondary";
    }
    if (tileLabel) tileLabel.textContent = t("home.tile_drive_connected");
    if (cardDesc) cardDesc.textContent = t("home.gdrive_desc_connected");

    if (providerPill) {
      providerPill.textContent = t("status.connected");
      providerPill.className = "status-pill active";
    }
    if (providerBtnAction) {
      providerBtnAction.textContent = t("providers.btn_google_disconnect");
      providerBtnAction.className = "btn btn-secondary";
    }
    if (providerCardDesc) {
      providerCardDesc.textContent = t("providers.gdrive_auth_desc_connected");
    }
  } else {
    if (heroBadge) {
      heroBadge.innerHTML = `<span class="badge-dot offline"></span><span class="badge-text">${escapeHtml(t("status.gdrive_disconnected_badge"))}</span>`;
    }
    if (btnAction) {
      btnAction.textContent = t("home.btn_connect");
      btnAction.className = "btn btn-primary";
    }
    if (tileLabel) tileLabel.textContent = t("home.tile_connect_drive");
    if (cardDesc) cardDesc.textContent = t("home.gdrive_desc_disconnected");

    if (providerPill) {
      providerPill.textContent = t("status.disconnected");
      providerPill.className = "status-pill";
    }
    if (providerBtnAction) {
      providerBtnAction.textContent = t("providers.btn_google_login");
      providerBtnAction.className = "btn btn-primary";
    }
    if (providerCardDesc) {
      providerCardDesc.textContent = t("providers.gdrive_auth_desc_disconnected");
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
      sysInfoCache = sysInfo;
      if (sysInfo.os_locale) {
        osLocale = sysInfo.os_locale;
      }
      appConfig.steam_dir = sysInfo.steam_dir;
      defaultVaultDir = sysInfo.vault_dir;
      appConfig.vault_dir = sysInfo.vault_dir;
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

      // Apply saved or auto-detected OS language immediately
      applyLanguage(appConfig.language || "system", true);

      // Sync backup provider dropdown & panel
      const providerSelect = document.getElementById("select-backup-provider");
      if (providerSelect) {
        providerSelect.value = appConfig.backup_provider || "gdrive";
        updateProviderPanel(providerSelect.value);
      }

      // Sync log debug mode and auto-refresh switches
      const checkLogDebug = document.getElementById("check-log-debug-mode");
      if (checkLogDebug) {
        checkLogDebug.checked = !!appConfig.debug_mode;
      }
      const checkLogAuto = document.getElementById("check-log-auto-refresh");
      if (checkLogAuto) {
        let stored = null;
        try {
          stored = localStorage.getItem("chronokeep_log_autorefresh");
        } catch (_) {}
        checkLogAuto.checked = stored !== "false";
        updateLogAutoRefreshState();
      }
    } else {
      applyLanguage(appConfig.language || "system", true);
    }

    await checkDaemon();

    // Proactive background update check after UI startup
    setTimeout(() => {
      checkForUpdates(false).catch((e) => console.warn("Auto-check updates on startup:", e));
    }, 400);
  } catch (err) {
    console.error("Failed to load initial data:", err);
  }
}

async function checkDaemon() {
  try {
    const res = await invokeBackend("get_service_status", { steamDir: appConfig.steam_dir });
    if (res) {
      updateDaemonStatusUI(res.online, res.port);
    }
  } catch (e) {
    console.error("Failed to query daemon status:", e);
  }
}

function applyTheme(theme) {
  const html = document.documentElement;
  if (theme === "light") {
    html.setAttribute("data-theme", "light");
  } else if (theme === "dark") {
    html.setAttribute("data-theme", "dark");
  } else {
    const isDark = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
    html.setAttribute("data-theme", isDark ? "dark" : "light");
  }
}

// --------------------------------------------------------------------------
// Actions & Handlers
// --------------------------------------------------------------------------
function setupActionHandlers() {
  // Helper for starting install pipeline after checks
  const runInstallPipeline = () => {
    ensureDisclaimerAccepted(async () => {
      showToast(t("toast.installing_steam"));
      try {
        const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
        const steamDir = inputSteam || appConfig.steam_dir;
        const res = await invokeBackend("install_chronokeep", {
          steamDir: steamDir,
          vaultDir: appConfig.vault_dir,
        });
        if (res && res.success) {
          showToast(res.message);
          await checkDaemon();
        } else {
          showToast(res ? res.message : t("toast.install_failed"));
        }
      } catch (e) {
        showToast(t("toast.install_error", { error: e }));
      }
    });
  };

  // Tile: Install to Steam with Pre-flight Dependencies Check
  document.getElementById("tile-install")?.addEventListener("click", async () => {
    try {
      const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
      const steamDir = inputSteam || appConfig.steam_dir;
      const report = await invokeBackend("check_system_dependencies", { steamDir: steamDir || null });

      if (report && report.all_ok) {
        // All OK -> Proceed directly
        runInstallPipeline();
      } else {
        // Missing required dependencies or third-party tool warning -> Show Dependencies Dialog
        checkAndShowDependenciesModal(runInstallPipeline);
      }
    } catch (_) {
      checkAndShowDependenciesModal(runInstallPipeline);
    }
  });

  // Home Card: Check Dependencies Button
  document.getElementById("btn-home-check-deps")?.addEventListener("click", () => {
    checkAndShowDependenciesModal(runInstallPipeline);
  });

  // Toggle Daemon
  document.getElementById("toggle-daemon")?.addEventListener("change", async (e) => {
    if (e.target.checked) {
      showToast(t("toast.starting_daemon"));
      const res = await invokeBackend("start_service", { steamDir: appConfig.steam_dir });
      showToast(res ? res.message : t("common.command_sent"));
    } else {
      showToast(t("toast.stopping_daemon"));
      const res = await invokeBackend("stop_service", { steamDir: appConfig.steam_dir });
      showToast(res ? res.message : t("common.command_sent"));
    }
    await checkDaemon();
  });

  // Tile Daemon Button
  document.getElementById("tile-daemon")?.addEventListener("click", async () => {
    if (daemonOnline) {
      showToast(t("toast.stopping_daemon"));
      await invokeBackend("stop_service", { steamDir: appConfig.steam_dir });
    } else {
      showToast(t("toast.starting_daemon"));
      await invokeBackend("start_service", { steamDir: appConfig.steam_dir });
    }
    await checkDaemon();
  });

  // Restart Daemon Button
  document.getElementById("btn-restart-daemon")?.addEventListener("click", async () => {
    showToast(t("toast.restarting_daemon"));
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
      showToast(res ? res.message : t("toast.gdrive_disconnected"));
      updateGDriveStatusUI(false);
    } else {
      showToast(t("toast.gdrive_opening_browser"));
      try {
        const res = await invokeBackend("start_google_login", { steamDir: appConfig.steam_dir });
        if (res && res.success) {
          showToast(res.message);
          updateGDriveStatusUI(true);
        } else {
          showToast(t("toast.gdrive_auth_failed"));
        }
      } catch (e) {
        showToast(t("toast.gdrive_login_error", { error: e }));
      }
    }
  };
  document.getElementById("tile-gdrive-login")?.addEventListener("click", gdriveAction);
  document.getElementById("btn-gdrive-action")?.addEventListener("click", gdriveAction);
  document.getElementById("btn-provider-gdrive-action")?.addEventListener("click", gdriveAction);

  // Backup Providers Handlers
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
      showToast(t("toast.vault_default_restored", { path: defaultVaultDir }));
    }
  });

  // Save Settings Function (General & Providers)
  const saveSettings = async (feedbackElId = "config-feedback") => {
    const portEl = document.getElementById("input-port");
    const providerEl = document.getElementById("select-backup-provider");
    const maxUploadEl = document.getElementById("select-max-upload");
    const steamDirEl = document.getElementById("input-steam-dir");
    const vaultDirEl = document.getElementById("input-vault-dir");
    const themeEl = document.getElementById("select-theme");
    const langEl = document.getElementById("select-language");

    const newCfg = {
      steam_dir: steamDirEl ? steamDirEl.value.trim() : appConfig.steam_dir,
      vault_dir: vaultDirEl ? vaultDirEl.value.trim() : appConfig.vault_dir,
      port: (portEl ? parseInt(portEl.value, 10) : appConfig.port) || 39771,
      gdrive_connected: appConfig.gdrive_connected,
      max_upload_mb: maxUploadEl ? parseInt(maxUploadEl.value, 10) : 512,
      theme: themeEl ? themeEl.value : appConfig.theme,
      language: langEl ? langEl.value : (appConfig.language || "system"),
      backup_provider: providerEl ? providerEl.value : (appConfig.backup_provider || "gdrive"),
    };

    const res = await invokeBackend("save_config", { cfg: newCfg });
    if (res && res.success) {
      appConfig = { ...appConfig, ...newCfg };
      applyTheme(newCfg.theme);
      applyLanguage(newCfg.language, true);
      showToast(res.message || t("toast.settings_saved"));
      const fb = document.getElementById(feedbackElId);
      if (fb) {
        fb.textContent = t("common.saved_check");
        setTimeout(() => { fb.textContent = ""; }, 2500);
      }
    } else {
      showToast(res ? res.message : t("common.save_error"));
    }
  };

  document.getElementById("btn-save-config")?.addEventListener("click", () => saveSettings("config-feedback"));
  document.getElementById("btn-save-providers")?.addEventListener("click", () => saveSettings("providers-feedback"));

  // Language selector: immediate live GUI hot-reload + instant persistence
  document.getElementById("select-language")?.addEventListener("change", async (e) => {
    const newLangPref = e.target.value || "system";
    applyLanguage(newLangPref, true);
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const steamDir = inputSteam || appConfig.steam_dir;
    try {
      await invokeBackend("set_language", {
        language: newLangPref,
        steamDir: steamDir || null,
      });
    } catch (err) {
      console.warn("Failed to persist language preference:", err);
    }
  });

  // Theme selector live preview
  document.getElementById("select-theme")?.addEventListener("change", (e) => {
    applyTheme(e.target.value);
  });

  // Scan Saves & Batch Sync
  document.getElementById("btn-scan-saves")?.addEventListener("click", scanSaves);
  document.getElementById("btn-sync-selected")?.addEventListener("click", syncSelectedSaves);

  // Saves filter tabs
  const setupTab = (tabId, tabKey) => {
    document.getElementById(tabId)?.addEventListener("click", () => {
      document.querySelectorAll(".saves-tab-btn").forEach((b) => b.classList.remove("active"));
      document.getElementById(tabId)?.classList.add("active");
      currentSavesTab = tabKey;
      renderSavesTable();
    });
  };
  setupTab("tab-btn-tools", "tools");
  setupTab("tab-btn-official", "official");
  setupTab("tab-btn-all", "all");

  // Saves search input
  document.getElementById("input-search-saves")?.addEventListener("input", (e) => {
    savesSearchQuery = e.target.value.trim().toLowerCase();
    renderSavesTable();
  });

  // GitHub External Links (including delegated listener for dynamically translated disclaimer link)
  const openGitHub = (e) => {
    e?.preventDefault();
    invokeBackend("open_url", { url: "https://github.com/leufrasiojunior/ChronoKeep" });
  };
  document.getElementById("nav-github-link")?.addEventListener("click", openGitHub);
  document.getElementById("btn-about-github")?.addEventListener("click", openGitHub);
  document.addEventListener("click", (e) => {
    const issueLink = e.target && e.target.closest ? e.target.closest("#link-disclaimer-issue") : null;
    if (issueLink) {
      e.preventDefault();
      invokeBackend("open_url", { url: "https://github.com/leufrasiojunior/ChronoKeep/issues" });
    }
  });

  // Logs actions
  const selectLogTarget = document.getElementById("select-log-target");
  const checkLogDebug = document.getElementById("check-log-debug-mode");
  const checkLogAuto = document.getElementById("check-log-auto-refresh");
  const btnRefreshLog = document.getElementById("btn-refresh-log");
  const btnClearLog = document.getElementById("btn-clear-log");
  const btnCopyLog = document.getElementById("btn-copy-log");

  btnRefreshLog?.addEventListener("click", () => loadLogs(false));
  selectLogTarget?.addEventListener("change", () => loadLogs(false));

  checkLogDebug?.addEventListener("change", async () => {
    const enabled = checkLogDebug.checked;
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const steamDir = inputSteam || appConfig.steam_dir;
    try {
      const res = await invokeBackend("set_debug_mode", { enabled, steamDir: steamDir || null });
      appConfig.debug_mode = enabled;
      showToast(res ? res.message : (enabled ? t("logs.toast_debug_enabled") : t("logs.toast_debug_disabled")));
      loadLogs(false);
    } catch (e) {
      showToast(e.toString());
    }
  });

  checkLogAuto?.addEventListener("change", () => {
    updateLogAutoRefreshState();
  });

  btnCopyLog?.addEventListener("click", async () => {
    if (!currentRawLogText) return;
    try {
      await navigator.clipboard.writeText(currentRawLogText);
      const prevText = btnCopyLog.textContent;
      btnCopyLog.textContent = t("logs.btn_copied");
      showToast(t("logs.toast_copied"));
      setTimeout(() => {
        btnCopyLog.textContent = prevText;
      }, 2000);
    } catch (_) {}
  });

  btnClearLog?.addEventListener("click", async () => {
    const target = selectLogTarget ? selectLogTarget.value : "daemon";
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const steamDir = inputSteam || appConfig.steam_dir;
    const res = await invokeBackend("clear_log", { logName: target, steamDir: steamDir || null });
    showToast(res ? res.message : t("logs.toast_cleared"));
    loadLogs(false);
  });

  // Updates actions
  document.getElementById("btn-home-check-updates")?.addEventListener("click", () => {
    navigateToPage("settings", "card-updates");
    if (!lastUpdateCheckResult) {
      checkForUpdates(true);
    }
  });
  document.getElementById("btn-check-updates")?.addEventListener("click", () => checkForUpdates(true));
  document.getElementById("btn-update-all")?.addEventListener("click", updateAllComponents);
  document.getElementById("btn-update-gui")?.addEventListener("click", () => updateComponent("gui"));
  document.getElementById("btn-update-daemon")?.addEventListener("click", () => updateComponent("daemon"));
  document.getElementById("btn-update-bridge")?.addEventListener("click", () => updateComponent("bridge"));

  // Uninstall Action
  document.getElementById("btn-open-uninstall-modal")?.addEventListener("click", openUninstallModal);

  // Manual Upload Limit Help Modal
  setupUploadLimitHelpModal();
}

// --------------------------------------------------------------------------
// Saves Scanner & Renderer (Tools vs Official Steam vs All)
// --------------------------------------------------------------------------
let cachedSavesList = [];
let currentSavesTab = "tools";
let savesSearchQuery = "";

function getOriginLabel(item) {
  if (item.origin === "tools") {
    return item.origin_label || "Tools";
  }
  if (item.origin === "official") {
    return t("saves.origin_official");
  }
  return t("saves.origin_archived");
}

function getBadgeHtml(item) {
  const label = escapeHtml(getOriginLabel(item));
  if (item.origin === "tools") {
    return `<span class="badge badge-tools">${label}</span>`;
  }
  if (item.origin === "official") {
    return `<span class="badge badge-official">${label}</span>`;
  }
  return `<span class="badge badge-vault">${label}</span>`;
}

function updateSavesCounts() {
  const toolsCount = cachedSavesList.filter((i) => i.origin === "tools").length;
  const officialCount = cachedSavesList.filter((i) => i.origin === "official").length;
  const allCount = cachedSavesList.length;

  const countToolsEl = document.getElementById("count-tools");
  const countOfficialEl = document.getElementById("count-official");
  const countAllEl = document.getElementById("count-all");

  if (countToolsEl) countToolsEl.textContent = toolsCount;
  if (countOfficialEl) countOfficialEl.textContent = officialCount;
  if (countAllEl) countAllEl.textContent = allCount;
}

function renderSavesTable() {
  const tbody = document.getElementById("tbody-saves");
  if (!tbody) return;

  if (!cachedSavesList || cachedSavesList.length === 0) {
    tbody.innerHTML = `<tr><td colspan="9" class="empty-state">${escapeHtml(t("saves.empty_initial"))}</td></tr>`;
    bindOrphanCheckboxes();
    updateSelectionUI();
    return;
  }

  // 1. Filter by active tab
  let filtered = cachedSavesList.filter((item) => {
    if (currentSavesTab === "tools") return item.origin === "tools";
    if (currentSavesTab === "official") return item.origin === "official";
    return true; // 'all'
  });

  // 2. Filter by search query
  if (savesSearchQuery) {
    const q = savesSearchQuery.toLowerCase();
    filtered = filtered.filter((item) => {
      const originLbl = getOriginLabel(item).toLowerCase();
      return (
        (item.name && item.name.toLowerCase().includes(q)) ||
        (item.app_id && item.app_id.toString().includes(q)) ||
        originLbl.includes(q)
      );
    });
  }

  if (filtered.length === 0) {
    let msg = escapeHtml(t("saves.empty_filter"));
    if (savesSearchQuery) {
      msg = escapeHtml(t("saves.empty_search", { query: savesSearchQuery }));
    } else if (currentSavesTab === "tools") {
      msg = escapeHtml(t("saves.empty_tools"));
    } else if (currentSavesTab === "official") {
      msg = escapeHtml(t("saves.empty_official"));
    }
    tbody.innerHTML = `<tr><td colspan="9" class="empty-state">${msg}</td></tr>`;
    bindOrphanCheckboxes();
    updateSelectionUI();
    return;
  }

  tbody.innerHTML = filtered
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
        <td>${getBadgeHtml(item)}</td>
        <td>${escapeHtml(t("saves.files_count", { count: item.file_count }))}</td>
        <td>${item.total_size_formatted}</td>
        <td>
          <span class="status-pill active">✓ ${escapeHtml(formatSaveSource(item.source))}</span>
        </td>
        <td>
          ${
            item.in_cloud
              ? `<span class="status-pill active">${escapeHtml(t("saves.pill_in_cloud"))}</span>`
              : `<span class="status-pill warning">${escapeHtml(t("saves.pill_pending"))}</span>`
          }
        </td>
        <td style="text-align: center;">
          <button class="btn btn-secondary btn-sm" onclick="syncGameSave(${item.app_id}, ${item.account_id}, this, ${item.total_bytes}, '${escapeHtml(item.name).replace(/'/g, "\\'")}')">
            ${escapeHtml(t("saves.btn_sync"))}
          </button>
        </td>
      </tr>
    `
    )
    .join("");

  bindOrphanCheckboxes();
  updateSelectionUI();
}

function getScanBtnDefaultHtml() {
  return `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"></circle><line x1="21" y1="21" x2="16.65" y2="16.65"></line></svg> <span data-i18n="saves.btn_scan">${escapeHtml(t("saves.btn_scan"))}</span>`;
}

function getSyncSelectedBtnDefaultHtml() {
  return `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path><polyline points="17 8 12 3 7 8"></polyline><line x1="12" y1="3" x2="12" y2="15"></line></svg> <span data-i18n="saves.btn_sync_selected">${escapeHtml(t("saves.btn_sync_selected"))}</span> (<span id="selected-count">0</span>)`;
}

async function scanSaves() {
  if (!isDisclaimerAccepted()) {
    ensureDisclaimerAccepted(() => scanSaves());
    return;
  }

  const tbody = document.getElementById("tbody-saves");
  const scanBtn = document.getElementById("btn-scan-saves");
  if (scanBtn) {
    scanBtn.disabled = true;
    scanBtn.innerHTML = `<span class="fluent-spinner-sm"></span> ${escapeHtml(t("saves.btn_scanning"))}`;
  }
  if (!tbody) return;
  tbody.innerHTML = `<tr><td colspan="9" class="empty-state"><div class="fluent-spinner" style="margin: 0 auto 12px auto;"></div>${escapeHtml(t("saves.empty_scanning"))}</td></tr>`;

  try {
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const inputVault = document.getElementById("input-vault-dir")?.value?.trim();
    const steamDir = inputSteam || appConfig.steam_dir;
    const vaultDir = inputVault || appConfig.vault_dir;

    const items = await invokeBackend("scan_saves", {
      steamDir: steamDir || null,
      vaultDir: vaultDir || null,
    });

    cachedSavesList = items || [];
    updateSavesCounts();
    renderSavesTable();
  } catch (e) {
    tbody.innerHTML = `<tr><td colspan="9" class="empty-state" style="color:var(--error-color);">${escapeHtml(t("saves.empty_scan_error", { error: e }))}</td></tr>`;
  } finally {
    if (scanBtn) {
      scanBtn.disabled = false;
      scanBtn.innerHTML = getScanBtnDefaultHtml();
    }
  }
}

function escapeHtml(text) {
  if (!text) return "";
  return String(text)
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

  const totalGames = checked.length;
  const totalBytes = checked.reduce(
    (acc, el) => acc + (parseInt(el.getAttribute("data-bytes"), 10) || 0),
    0
  );

  let processedBytes = 0;
  let successCount = 0;
  const failedList = [];

  showSyncProgress(
    t("saves.batch_starting"),
    t("saves.batch_starting_detail", { total: totalGames, bytes: formatBytes(totalBytes) }),
    0
  );

  if (btnSyncSelected) {
    btnSyncSelected.disabled = true;
    btnSyncSelected.innerHTML = `<span class="fluent-spinner-sm"></span> ${escapeHtml(t("saves.batch_btn_progress", { current: 0, total: totalGames }))}`;
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
      t("saves.batch_item_title", { name: gameName, current: i + 1, total: totalGames }),
      t("saves.batch_item_detail", {
        doneBytes: formatBytes(processedBytes),
        totalBytes: formatBytes(totalBytes),
        current: i + 1,
        total: totalGames,
      }),
      currentPercent
    );

    if (btnSyncSelected) {
      btnSyncSelected.innerHTML = `<span class="fluent-spinner-sm"></span> ${escapeHtml(t("saves.batch_btn_progress", { current: i + 1, total: totalGames }))}`;
    }

    if (row) {
      const cells = row.querySelectorAll("td");
      if (cells.length >= 8) {
        cells[7].innerHTML = `<span class="status-pill warning"><span class="fluent-spinner-sm"></span> ${escapeHtml(t("saves.pill_uploading"))}</span>`;
      }
    }

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
        const cachedItem = cachedSavesList.find((it) => it.app_id === appId && it.account_id === accountId);
        if (cachedItem) cachedItem.in_cloud = true;
        if (row) {
          const cells = row.querySelectorAll("td");
          if (cells.length >= 8) {
            cells[7].innerHTML = `<span class="status-pill active">${escapeHtml(t("saves.pill_in_cloud"))}</span>`;
          }
        }
      } else {
        const errorMsg = (res && res.message) ? res.message : t("common.unknown_error");
        failedList.push({ appId, error: errorMsg });
        if (row) {
          const cells = row.querySelectorAll("td");
          if (cells.length >= 8) {
            cells[7].innerHTML = `<span class="status-pill offline" title="${escapeHtml(errorMsg)}">${escapeHtml(t("saves.pill_failed"))}</span>`;
          }
        }
      }
    } catch (e) {
      console.error(`Error syncing AppID ${appId}:`, e);
      failedList.push({ appId, error: e.message || String(e) });
      if (row) {
        const cells = row.querySelectorAll("td");
        if (cells.length >= 8) {
          cells[7].innerHTML = `<span class="status-pill offline">${escapeHtml(t("saves.pill_error"))}</span>`;
        }
      }
    }

    processedBytes += gameBytes;
    const progressAfterGame = totalBytes > 0 ? (processedBytes / totalBytes) * 100 : ((i + 1) / totalGames) * 100;
    showSyncProgress(
      t("saves.batch_done_item_title", { name: gameName }),
      t("saves.batch_done_item_detail", {
        doneBytes: formatBytes(processedBytes),
        totalBytes: formatBytes(totalBytes),
        current: i + 1,
        total: totalGames,
      }),
      progressAfterGame
    );

    await yieldToUI(20);
  }

  showSyncProgress(
    t("saves.batch_finished_title"),
    t("saves.batch_finished_detail", {
      success: successCount,
      total: totalGames,
      bytes: formatBytes(processedBytes),
    }),
    100
  );
  hideSyncProgress(4000);

  if (failedList.length === 0) {
    showToast(t("saves.toast_batch_all_ok", { success: successCount }), 4000);
  } else if (successCount === 0) {
    showToast(t("saves.toast_batch_all_fail", { failed: failedList.length, error: failedList[0].error }), 6000);
  } else {
    showToast(
      t("saves.toast_batch_partial", {
        success: successCount,
        failed: failedList.length,
        error: failedList[0].error,
      }),
      6000
    );
  }
  if (btnSyncSelected) {
    btnSyncSelected.disabled = false;
    btnSyncSelected.innerHTML = getSyncSelectedBtnDefaultHtml();
  }
  updateSelectionUI();
}

// --------------------------------------------------------------------------
// Manual Game Save Sync
// --------------------------------------------------------------------------
async function syncGameSave(appId, accountId, btn, totalBytes = 0, gameName = "") {
  const displayName = gameName || `AppID ${appId}`;
  const row = btn ? btn.closest("tr") : null;

  if (btn) {
    btn.disabled = true;
    btn.innerHTML = `<span class="fluent-spinner-sm"></span> ${escapeHtml(t("saves.pill_uploading"))}`;
  }
  if (row) {
    const cells = row.querySelectorAll("td");
    if (cells.length >= 8) {
      cells[7].innerHTML = `<span class="status-pill warning"><span class="fluent-spinner-sm"></span> ${escapeHtml(t("saves.pill_uploading"))}</span>`;
    }
  }

  showSyncProgress(
    t("saves.single_syncing_title", { name: displayName }),
    totalBytes > 0
      ? t("saves.single_syncing_bytes", { bytes: formatBytes(totalBytes) })
      : t("saves.single_syncing_fallback"),
    30
  );
  showToast(t("saves.toast_single_syncing", { name: displayName }));

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
      const cachedItem = cachedSavesList.find((it) => it.app_id === appId && it.account_id === accountId);
      if (cachedItem) cachedItem.in_cloud = true;

      showSyncProgress(
        t("saves.single_done_title", { name: displayName }),
        totalBytes > 0
          ? t("saves.single_done_bytes", { bytes: formatBytes(totalBytes) })
          : t("saves.single_done_fallback"),
        100
      );
      hideSyncProgress(3000);
      showToast(res.message, 4000);
      if (row) {
        const cells = row.querySelectorAll("td");
        if (cells.length >= 8) {
          cells[7].innerHTML = `<span class="status-pill active">${escapeHtml(t("saves.pill_in_cloud"))}</span>`;
        }
      }
      if (btn) {
        btn.textContent = t("saves.btn_synced");
        setTimeout(() => {
          btn.disabled = false;
          btn.textContent = t("saves.btn_sync");
        }, 4000);
      }
    } else {
      const errorMsg = res ? res.message : t("saves.single_fail_fallback");
      showSyncProgress(t("saves.single_fail_title", { name: displayName }), errorMsg, 0);
      hideSyncProgress(4000);
      showToast(errorMsg, 5000);
      if (row) {
        const cells = row.querySelectorAll("td");
        if (cells.length >= 8) {
          cells[7].innerHTML = `<span class="status-pill offline" title="${escapeHtml(errorMsg)}">${escapeHtml(t("saves.pill_failed"))}</span>`;
        }
      }
      if (btn) {
        btn.disabled = false;
        btn.textContent = t("saves.btn_sync");
      }
    }
  } catch (err) {
    showSyncProgress(t("saves.single_err_title", { name: displayName }), String(err), 0);
    hideSyncProgress(4000);
    showToast(t("saves.toast_single_err", { error: err }), 5000);
    if (row) {
      const cells = row.querySelectorAll("td");
      if (cells.length >= 8) {
        cells[7].innerHTML = `<span class="status-pill offline">${escapeHtml(t("saves.pill_error"))}</span>`;
      }
    }
    if (btn) {
      btn.disabled = false;
      btn.textContent = t("saves.btn_sync");
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
    badgeEl.textContent = t("settings.badge_up_to_date");
  } else if (status === "update_available") {
    badgeEl.classList.add("update-available");
    badgeEl.textContent = t("settings.badge_available");
  } else if (status === "not_installed") {
    badgeEl.classList.add("not-installed");
    badgeEl.textContent = t("settings.badge_not_installed");
  } else if (status === "error") {
    badgeEl.classList.add("error");
    badgeEl.textContent = t("settings.badge_error");
  } else {
    badgeEl.classList.add("neutral");
    badgeEl.textContent = t("settings.badge_not_checked");
  }
}

function renderUpdateCheckResultUI(res) {
  const banner = document.getElementById("update-status-banner");
  const btnUpdateAll = document.getElementById("btn-update-all");

  if (!res || !res.success) {
    if (banner) {
      banner.className = "update-banner error";
      banner.style.display = "flex";
      banner.innerHTML = `<span>⚠️</span><span>${escapeHtml((res && res.message) ? res.message : t("settings.banner_check_failed"))}</span>`;
    }
    return;
  }

  if (res.components && Array.isArray(res.components)) {
    for (const comp of res.components) {
      const id = comp.id;
      const curEl = document.getElementById(`meta-${id}-cur`);
      const latestEl = document.getElementById(`meta-${id}-latest`);
      const badgeEl = document.getElementById(`badge-${id}`);
      const updateBtn = document.getElementById(`btn-update-${id}`);
      const displayCurVer = formatVersionLabel(comp.current_version);

      if (curEl) curEl.textContent = displayCurVer;
      if (latestEl) latestEl.textContent = comp.latest_version || "-";
      if (badgeEl) renderUpdateBadge(badgeEl, comp.status);

      if (id === "gui") {
        const el = document.getElementById("sidebar-ver-gui");
        if (el && comp.current_version) el.textContent = displayCurVer;
      } else if (id === "bridge") {
        const el = document.getElementById("sidebar-ver-dll");
        if (el && comp.current_version) el.textContent = displayCurVer;
      } else if (id === "daemon") {
        const el = document.getElementById("sidebar-ver-daemon");
        if (el && comp.current_version) el.textContent = displayCurVer;
      }

      if (updateBtn) {
        if (comp.status === "update_available") {
          updateBtn.style.display = "inline-flex";
          updateBtn.disabled = false;
          updateBtn.textContent = t("settings.updates_btn_update");
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
      banner.innerHTML = `<span>🚀</span><span>${escapeHtml(t("settings.banner_updates_available"))}</span>`;
      if (btnUpdateAll) btnUpdateAll.style.display = "inline-flex";
    } else {
      banner.className = "update-banner success";
      banner.style.display = "flex";
      const tag = res.release_tag || t("settings.banner_latest_fallback");
      banner.innerHTML = `<span>✓</span><span>${escapeHtml(t("settings.banner_all_up_to_date", { tag }))}</span>`;
      if (btnUpdateAll) btnUpdateAll.style.display = "none";
    }
  }

  const homeBanner = document.getElementById("home-update-banner");
  const homeDesc = document.getElementById("home-update-desc");
  if (homeBanner) {
    if (res.has_updates) {
      homeBanner.style.display = "flex";
      if (homeDesc) {
        const compCount = res.components?.filter((c) => c.status === "update_available").length || 1;
        const tag = res.release_tag || t("settings.banner_latest_fallback");
        homeDesc.textContent = t("home.update_banner_desc_count", { count: compCount, tag });
      }
    } else {
      homeBanner.style.display = "none";
    }
  }
}

async function checkForUpdates(showUserToast = false) {
  const btn = document.getElementById("btn-check-updates");
  const icon = document.getElementById("icon-check-updates");
  const text = document.getElementById("text-check-updates");
  const banner = document.getElementById("update-status-banner");

  if (btn) btn.disabled = true;
  if (icon) icon.classList.add("spin-icon");
  if (text) text.textContent = t("settings.updates_btn_checking");

  try {
    const res = await invokeBackend("check_for_updates");
    lastUpdateCheckResult = res;
    renderUpdateCheckResultUI(res);

    if (showUserToast) {
      showToast((res && res.message) ? res.message : t("settings.banner_check_failed"));
    }
  } catch (err) {
    if (banner) {
      banner.className = "update-banner error";
      banner.style.display = "flex";
      banner.innerHTML = `<span>⚠️</span><span>${escapeHtml(t("settings.banner_check_error", { error: err }))}</span>`;
    }
    if (showUserToast) {
      showToast(t("settings.banner_check_error", { error: err }));
    }
  } finally {
    if (btn) btn.disabled = false;
    if (icon) icon.classList.remove("spin-icon");
    if (text) text.textContent = t("settings.updates_btn_check");
  }
}

function confirmSteamRestart(componentLabel = "") {
  return new Promise((resolve) => {
    const modal = document.getElementById("steam-restart-modal");
    const btnCancel = document.getElementById("btn-steam-restart-cancel");
    const btnConfirm = document.getElementById("btn-steam-restart-confirm");
    const compName = document.getElementById("steam-restart-component-name");

    if (!modal || !btnCancel || !btnConfirm) {
      resolve(true);
      return;
    }

    if (compName) {
      compName.textContent = componentLabel;
    }

    modal.style.display = "flex";

    const onKeydown = (e) => {
      if (e.key === "Escape") {
        cleanup(false);
      }
    };

    const onOverlayClick = (e) => {
      if (e.target === modal) {
        cleanup(false);
      }
    };

    const cleanup = (result) => {
      window.removeEventListener("keydown", onKeydown);
      modal.removeEventListener("click", onOverlayClick);
      modal.style.display = "none";
      btnCancel.onclick = null;
      btnConfirm.onclick = null;
      resolve(result);
    };

    btnCancel.onclick = () => cleanup(false);
    btnConfirm.onclick = () => cleanup(true);
    modal.addEventListener("click", onOverlayClick);
    window.addEventListener("keydown", onKeydown);
  });
}

async function updateComponent(componentId, reopenSteam = true, steamWasRunning = null) {
  if (isUpdatingComponent) return;

  const btn = document.getElementById(`btn-update-${componentId}`);

  let isSteamOpen = steamWasRunning;
  if ((componentId === "daemon" || componentId === "bridge") && steamWasRunning === null) {
    try {
      isSteamOpen = await invokeBackend("is_steam_running");
    } catch (_) {
      isSteamOpen = false;
    }

    if (isSteamOpen) {
      const compLabel =
        componentId === "bridge"
          ? t("modal.steam_restart_comp_bridge")
          : t("modal.steam_restart_comp_daemon");
      const confirmed = await confirmSteamRestart(compLabel);
      if (!confirmed) {
        showToast(t("toast.update_cancelled"), 3000);
        return;
      }
    }
  }

  isUpdatingComponent = true;
  if (btn) {
    btn.disabled = true;
    btn.textContent = t("settings.updates_btn_updating");
  }

  showToast(t("toast.updating_component", { component: componentId }), 5000);

  try {
    const res = await invokeBackend("update_component", {
      component: componentId,
      reopenSteam: reopenSteam !== false,
      steamWasRunning: isSteamOpen !== null ? isSteamOpen : null,
    });
    if (res && res.success) {
      showToast(res.message, 6000);
      if (res.restart_required && componentId === "gui") {
        const banner = document.getElementById("update-status-banner");
        if (banner) {
          banner.className = "update-banner success";
          banner.style.display = "flex";
          banner.innerHTML = `<span>🔄</span><span>${escapeHtml(t("settings.banner_restarting_gui"))}</span>`;
        }
      } else {
        await checkForUpdates(false);
      }
    } else {
      showToast(t("toast.update_error", { error: (res && res.message) ? res.message : t("common.unknown_error") }));
      if (btn) {
        btn.disabled = false;
        btn.textContent = t("settings.updates_btn_update");
      }
    }
  } catch (err) {
    showToast(t("toast.update_comp_error", { error: err }));
    if (btn) {
      btn.disabled = false;
      btn.textContent = t("settings.updates_btn_update");
    }
  } finally {
    isUpdatingComponent = false;
  }
}

async function updateAllComponents() {
  if (isUpdatingComponent || !lastUpdateCheckResult) return;
  const available = (lastUpdateCheckResult.components || []).filter((c) => c.status === "update_available");
  if (available.length === 0) {
    showToast(t("toast.no_pending_updates"));
    return;
  }

  const order = ["daemon", "bridge", "gui"];
  const sorted = available.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));

  const hasSteamComponent = sorted.some((c) => c.id === "daemon" || c.id === "bridge");
  let steamWasRunning = false;
  if (hasSteamComponent) {
    try {
      steamWasRunning = await invokeBackend("is_steam_running");
      if (steamWasRunning) {
        const confirmed = await confirmSteamRestart(t("modal.steam_restart_comp_all"));
        if (!confirmed) {
          showToast(t("toast.update_cancelled"), 3000);
          return;
        }
      }
    } catch (_) {}
  }

  const btnAll = document.getElementById("btn-update-all");
  const btnAllSpan = btnAll ? btnAll.querySelector("span") : null;
  if (btnAll) {
    btnAll.disabled = true;
    if (btnAllSpan) btnAllSpan.textContent = t("settings.updates_btn_updating_batch");
    else btnAll.textContent = t("settings.updates_btn_updating_batch");
  }

  try {
    for (let i = 0; i < sorted.length; i++) {
      const comp = sorted[i];
      const nextHasSteam = sorted.slice(i + 1).some((c) => c.id === "daemon" || c.id === "bridge");
      const shouldReopen = steamWasRunning && !nextHasSteam;
      await updateComponent(comp.id, shouldReopen, steamWasRunning);
    }
  } catch (err) {
    showToast(t("toast.batch_update_error", { error: err }));
    if (steamWasRunning) {
      try {
        const stillRunning = await invokeBackend("is_steam_running");
        if (!stillRunning) {
          await invokeBackend("launch_steam");
        }
      } catch (_) {}
    }
  } finally {
    if (btnAll) {
      btnAll.disabled = false;
      if (btnAllSpan) btnAllSpan.textContent = t("settings.updates_btn_all");
      else btnAll.textContent = t("settings.updates_btn_all");
    }
  }
}

// --------------------------------------------------------------------------
// Logs Loader & Syntax Highlighter
// --------------------------------------------------------------------------
let currentRawLogText = "";

function updateLogAutoRefreshState() {
  const checkLogAuto = document.getElementById("check-log-auto-refresh");
  const liveDot = document.getElementById("log-live-dot");
  if (!checkLogAuto) return;
  const isAuto = checkLogAuto.checked;
  try {
    localStorage.setItem("chronokeep_log_autorefresh", isAuto ? "true" : "false");
  } catch (_) {}
  if (liveDot) {
    liveDot.style.display = isAuto ? "inline-block" : "none";
  }
}

function formatLogSyntax(rawText) {
  if (!rawText) return "";
  const lines = rawText.split("\n");
  return lines
    .map((line) => {
      let escaped = escapeHtml(line);
      // Format timestamp: [YYYY-MM-DD HH:MM:SS]
      escaped = escaped.replace(
        /^(\[\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\])/,
        '<span class="log-ts">$1</span>'
      );
      // Format tags
      escaped = escaped.replace(/(\[DEBUG\])/g, '<span class="log-tag-debug">[DEBUG]</span>');
      escaped = escaped.replace(/(\[INFO\])/g, '<span class="log-tag-info">[INFO]</span>');
      escaped = escaped.replace(/(\[WARN(?:ING)?\])/g, '<span class="log-tag-warn">$1</span>');
      escaped = escaped.replace(/(\[(?:ERROR|ERRO|FALHA)\])/g, '<span class="log-tag-error">$1</span>');
      // Highlight parameters: app=123, acct=456, port=789
      escaped = escaped.replace(
        /\b(app|acct|port|req|resp)=(\d+)\b/g,
        '<span class="log-param-key">$1=</span><span class="log-param-val">$2</span>'
      );
      return `<span class="log-line">${escaped}</span>`;
    })
    .join("");
}

async function loadLogs(isSilentAutoRefresh = false) {
  const output = document.getElementById("log-output");
  const target = document.getElementById("select-log-target")?.value || "daemon";
  const badge = document.getElementById("terminal-target-badge");
  const statLines = document.getElementById("terminal-lines-count");
  const statUpdated = document.getElementById("terminal-last-updated");

  if (!output) return;

  const fileName = target === "bridge" ? "chronokeep_bridge.log" : "chrono_daemon.log";
  if (badge) badge.textContent = fileName;

  if (!isSilentAutoRefresh) {
    output.textContent = t("logs.loading_file", { file: fileName });
  }

  try {
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const steamDir = inputSteam || appConfig.steam_dir;
    const content = await invokeBackend("get_logs", {
      logName: target,
      maxLines: 400,
      steamDir: steamDir || null,
    });

    currentRawLogText = content || "";
    const lines = (content || "").split("\n").filter((l) => l.trim().length > 0);

    if (statLines) {
      statLines.textContent = t("logs.lines_count", { count: lines.length });
    }
    if (statUpdated) {
      const now = new Date();
      statUpdated.textContent = t("logs.updated_at", { time: now.toLocaleTimeString() });
    }

    if (!content || content.trim().length === 0) {
      output.textContent = t("logs.empty_file");
      return;
    }

    const isNearBottom = output.scrollHeight - output.scrollTop - output.clientHeight < 60;
    output.innerHTML = formatLogSyntax(content);

    if (!isSilentAutoRefresh || isNearBottom) {
      output.scrollTop = output.scrollHeight;
    }
  } catch (e) {
    output.textContent = t("logs.error_loading", { error: e });
  }
}

// --------------------------------------------------------------------------
// Lifecycle Startup
// --------------------------------------------------------------------------
document.addEventListener("DOMContentLoaded", () => {
  // Zero-flash initial pass using cached user preference or browser OS language
  let cachedPref = "system";
  try {
    cachedPref = localStorage.getItem("chronokeep_lang_pref") || "system";
  } catch (_) {}
  applyLanguage(cachedPref, false);

  setupNavigation();
  setupActionHandlers();
  loadInitialData();

  // If launched as uninstaller (ChronoKeepUninstall.exe or --uninstall)
  invokeBackend("get_launch_mode")
    .then((mode) => {
      if (mode === "uninstall") {
        navigateToPage("settings");
        openUninstallModal();
      }
    })
    .catch(() => {});

  // Background daemon poller every 3.5 seconds
  setInterval(() => {
    if (document.visibilityState === "visible") {
      checkDaemon();
    }
  }, 3500);

  // Background log auto-refresh poller every 2.5 seconds
  setInterval(() => {
    const checkLogAuto = document.getElementById("check-log-auto-refresh");
    const pageLogs = document.getElementById("page-logs");
    if (
      checkLogAuto &&
      checkLogAuto.checked &&
      pageLogs &&
      pageLogs.classList.contains("active") &&
      document.visibilityState === "visible"
    ) {
      loadLogs(true);
    }
  }, 2500);
});
