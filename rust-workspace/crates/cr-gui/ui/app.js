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
  } else if (pageId === "games") {
    loadGamesCatalog();
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

      // Load Steam Web API Key
      try {
        const apiKey = await invokeBackend("get_steam_api_key", { steamDir: appConfig.steam_dir });
        if (apiKey) {
          const keyInput = document.getElementById("input-settings-steam-api-key");
          if (keyInput) keyInput.value = apiKey;
        }
      } catch (_) {}
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
      // Also persist Steam Web API key if changed
      const apiKeyInput = document.getElementById("input-settings-steam-api-key");
      if (apiKeyInput) {
        try {
          await invokeBackend("set_steam_api_key", {
            steamDir: newCfg.steam_dir || appConfig.steam_dir,
            key: apiKeyInput.value.trim(),
          });
        } catch (_) {}
      }

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

  // Steam API Key helper handlers
  document.getElementById("btn-toggle-key-visibility")?.addEventListener("click", () => {
    const input = document.getElementById("input-settings-steam-api-key");
    if (input) {
      input.type = input.type === "password" ? "text" : "password";
    }
  });

  document.getElementById("link-settings-get-key")?.addEventListener("click", (e) => {
    e.preventDefault();
    invokeBackend("open_url", { url: "https://steamcommunity.com/dev/apikey" });
  });

  document.getElementById("link-open-apikey")?.addEventListener("click", (e) => {
    e.preventDefault();
    invokeBackend("open_url", { url: "https://steamcommunity.com/dev/apikey" });
  });

  // Games tab toolbar & modals setup
  setupGamesToolbarAndModals();
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
// Clipboard & Copy Helpers
// --------------------------------------------------------------------------
async function copyTextToClipboard(text, successMsg = null) {
  if (!text) return;
  try {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      await navigator.clipboard.writeText(String(text));
    } else {
      const textarea = document.createElement("textarea");
      textarea.value = String(text);
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.appendChild(textarea);
      textarea.select();
      document.execCommand("copy");
      document.body.removeChild(textarea);
    }
    if (successMsg) {
      showToast(successMsg);
    }
  } catch (err) {
    console.warn("Clipboard copy failed:", err);
  }
}
window.copyTextToClipboard = copyTextToClipboard;

// --------------------------------------------------------------------------
// Games Catalog, Details & Achievements Controller
// --------------------------------------------------------------------------
let gamesCatalog = [];
let activeGameFilter = "tools"; // "tools" | "official" | "all"
let gamesSearchQuery = "";
let currentGamesViewMode = "grid"; // "grid" | "list"
let selectedGame = null;
let currentAchievementsGame = null;

try {
  currentGamesViewMode = localStorage.getItem("chronokeep_games_view_mode") || "grid";
} catch (_) {}

function handleCoverError(img, appId, gameName) {
  if (!img) return;
  // Stage 1: Try horizontal header.jpg
  if (!img.dataset.triedHeader) {
    img.dataset.triedHeader = "1";
    img.src = `https://cdn.cloudflare.steamstatic.com/steam/apps/${appId}/header.jpg`;
    img.classList.add("fallback-header");
    return;
  }
  // Stage 2: Try capsule_616x353.jpg
  if (!img.dataset.triedCapsule) {
    img.dataset.triedCapsule = "1";
    img.src = `https://cdn.cloudflare.steamstatic.com/steam/apps/${appId}/capsule_616x353.jpg`;
    return;
  }

  // Stage 3: All CDN images failed (unreleased / private / test app)
  console.debug(`[ChronoKeep Cover] AppID ${appId} (${gameName}) has no store CDN images. Displaying Fluent Art Cover.`);
  const wrap = img.parentElement;
  if (!wrap) return;
  img.style.display = "none";

  let art = wrap.querySelector(".fluent-art-cover");
  if (!art) {
    art = document.createElement("div");
    art.className = "fluent-art-cover";
    const hue = (Number(appId) * 137) % 360;
    art.style.background = `radial-gradient(circle at 50% 30%, hsl(${hue}, 55%, 24%) 0%, hsl(${(hue + 45) % 360}, 50%, 10%) 100%)`;
    art.innerHTML = `
      <div class="art-icon-circle">🎮</div>
      <div class="art-game-title select-text">${escapeHtml(gameName)}</div>
      <div class="art-game-appid">AppID ${appId}</div>
    `;
    wrap.appendChild(art);
  } else {
    art.style.display = "flex";
  }
}
window.handleCoverError = handleCoverError;

async function loadGamesCatalog() {
  const loadingEl = document.getElementById("games-loading-state");
  const emptyEl = document.getElementById("games-empty-state");
  const gridEl = document.getElementById("games-grid");
  const listEl = document.getElementById("games-list-view");

  if (loadingEl) loadingEl.style.display = "flex";
  if (emptyEl) emptyEl.style.display = "none";
  if (gridEl) gridEl.style.display = "none";
  if (listEl) listEl.style.display = "none";

  try {
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const inputVault = document.getElementById("input-vault-dir")?.value?.trim();
    const list = await invokeBackend("get_games_catalog", {
      steamDir: inputSteam || appConfig.steam_dir || null,
      vaultDir: inputVault || appConfig.vault_dir || null,
    });
    gamesCatalog = Array.isArray(list) ? list : [];
  } catch (err) {
    console.error("Failed to load games catalog:", err);
    gamesCatalog = [];
  } finally {
    if (loadingEl) loadingEl.style.display = "none";
  }

  updateGamesCounts();
  renderGamesCatalog();
}

function updateGamesCounts() {
  const toolsCount = gamesCatalog.filter((g) => g.origin === "tools").length;
  const officialCount = gamesCatalog.filter((g) => g.origin === "official").length;
  const allCount = gamesCatalog.length;

  const countToolsEl = document.getElementById("games-count-tools");
  const countOfficialEl = document.getElementById("games-count-official");
  const countAllEl = document.getElementById("games-count-all");

  if (countToolsEl) countToolsEl.textContent = toolsCount;
  if (countOfficialEl) countOfficialEl.textContent = officialCount;
  if (countAllEl) countAllEl.textContent = allCount;
}

function renderGamesCatalog() {
  const gridEl = document.getElementById("games-grid");
  const listViewEl = document.getElementById("games-list-view");
  const tableBody = document.getElementById("games-table-tbody");
  const emptyEl = document.getElementById("games-empty-state");

  // Sync active view switcher buttons
  const btnGrid = document.getElementById("btn-view-grid");
  const btnList = document.getElementById("btn-view-list");
  if (btnGrid) btnGrid.classList.toggle("active", currentGamesViewMode === "grid");
  if (btnList) btnList.classList.toggle("active", currentGamesViewMode === "list");

  // 1. Filter by active tab (tools, official, all)
  let filtered = gamesCatalog.filter((game) => {
    if (activeGameFilter === "tools") return game.origin === "tools";
    if (activeGameFilter === "official") return game.origin === "official";
    return true;
  });

  // 2. Filter by search query
  if (gamesSearchQuery) {
    const q = gamesSearchQuery.toLowerCase();
    filtered = filtered.filter((game) => {
      const matchName = game.name && game.name.toLowerCase().includes(q);
      const matchId = String(game.app_id).includes(q);
      return matchName || matchId;
    });
  }

  if (filtered.length === 0) {
    if (emptyEl) emptyEl.style.display = "flex";
    if (gridEl) gridEl.style.display = "none";
    if (listViewEl) listViewEl.style.display = "none";
    return;
  }

  if (emptyEl) emptyEl.style.display = "none";

  if (currentGamesViewMode === "grid") {
    if (gridEl) gridEl.style.display = "grid";
    if (listViewEl) listViewEl.style.display = "none";
    renderGamesGridView(filtered, gridEl);
  } else {
    if (gridEl) gridEl.style.display = "none";
    if (listViewEl) listViewEl.style.display = "block";
    renderGamesListView(filtered, tableBody);
  }
}

function renderGamesGridView(filtered, gridEl) {
  if (!gridEl) return;
  gridEl.innerHTML = "";

  for (const game of filtered) {
    let playtimeStr = "";
    if (game.playtime_minutes > 0) {
      const h = Math.floor(game.playtime_minutes / 60);
      const m = game.playtime_minutes % 60;
      playtimeStr = h > 0
        ? t("games.playtime_hours", { hours: h, minutes: m })
        : t("games.playtime_minutes_only", { minutes: m });
    }

    let cloudBadgeHtml = "";
    if (game.cloud_status === "synced") {
      cloudBadgeHtml = `<span class="game-badge cloud-synced" title="${escapeHtml(t("games.cloud_synced"))}">
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
          <path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"></path>
        </svg> Cloud
      </span>`;
    } else if (game.cloud_status === "local_only") {
      cloudBadgeHtml = `<span class="game-badge cloud-local" title="${escapeHtml(t("games.cloud_local"))}">Local</span>`;
    }

    const originClass = game.origin === "tools" ? "origin-tools" : "origin-official";
    const originBadgeHtml = `<span class="game-badge ${originClass}">${escapeHtml(game.origin_label)}</span>`;

    const escapedName = escapeHtml(game.name);
    const rawSafeName = game.name.replace(/\\/g, "\\\\").replace(/'/g, "\\'");

    const card = document.createElement("div");
    card.className = "game-card";
    card.innerHTML = `
      <div class="game-card-cover-wrap">
        <div class="game-card-badges">
          ${originBadgeHtml}
          ${cloudBadgeHtml}
        </div>
        <img class="game-card-cover" src="${escapeHtml(game.cover_url)}" alt="${escapedName}" loading="lazy"
             onerror="handleCoverError(this, ${game.app_id}, '${rawSafeName}')" />
      </div>
      <div class="game-card-info">
        <span class="game-card-title select-text" title="${escapedName}">${escapedName}</span>
        <div class="game-card-sub">
          <button class="copy-badge-pill" style="padding: 1px 6px; font-size: 10px;"
                  onclick="event.stopPropagation(); copyTextToClipboard('${game.app_id}', t('games.toast_appid_copied', { id: '${game.app_id}' }))"
                  title="${escapeHtml(t("common.copy_appid"))}">
            AppID ${game.app_id}
          </button>
          ${playtimeStr ? `<span class="select-text">${escapeHtml(playtimeStr)}</span>` : ""}
        </div>
      </div>
    `;

    card.addEventListener("click", () => openGameDetailModal(game));
    gridEl.appendChild(card);
  }
}

function renderGamesListView(filtered, tableBody) {
  if (!tableBody) return;
  tableBody.innerHTML = "";

  for (const game of filtered) {
    let playtimeStr = "";
    if (game.playtime_minutes > 0) {
      const h = Math.floor(game.playtime_minutes / 60);
      const m = game.playtime_minutes % 60;
      playtimeStr = h > 0
        ? t("games.playtime_hours", { hours: h, minutes: m })
        : t("games.playtime_minutes_only", { minutes: m });
    }

    let cloudBadgeHtml = "";
    if (game.cloud_status === "synced") {
      cloudBadgeHtml = `<span class="status-pill active" title="${escapeHtml(t("games.cloud_synced"))}">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"></path></svg>
        ${escapeHtml(t("games.cloud_synced"))}
      </span>`;
    } else if (game.cloud_status === "local_only") {
      cloudBadgeHtml = `<span class="status-pill warning" title="${escapeHtml(t("games.cloud_local"))}">
        ${escapeHtml(t("games.cloud_local"))}
      </span>`;
    } else {
      cloudBadgeHtml = `<span class="status-pill offline">${escapeHtml(t("games.cloud_none"))}</span>`;
    }

    const originClass = game.origin === "tools" ? "badge-tools" : "badge-official";
    const originBadgeHtml = `<span class="badge ${originClass}">${escapeHtml(game.origin_label)}</span>`;

    const escapedName = escapeHtml(game.name);
    const rawSafeName = game.name.replace(/\\/g, "\\\\").replace(/'/g, "\\'");

    let syncDateStr = "-";
    if (game.last_sync_ts && game.last_sync_ts > 0) {
      const d = new Date(game.last_sync_ts * 1000);
      syncDateStr = d.toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" });
    }

    const tr = document.createElement("tr");
    tr.className = "game-list-row";
    tr.innerHTML = `
      <td>
        <div class="game-list-thumb-wrap">
          <img class="game-list-thumb" src="${escapeHtml(game.header_url)}" alt="${escapedName}" loading="lazy"
               onerror="handleCoverError(this, ${game.app_id}, '${rawSafeName}')" />
        </div>
      </td>
      <td>
        <div class="game-list-title-cell">
          <span class="game-list-name select-text">${escapedName}</span>
          <button class="copy-badge-pill btn-copy-appid-cell"
                  onclick="event.stopPropagation(); copyTextToClipboard('${game.app_id}', t('games.toast_appid_copied', { id: '${game.app_id}' }))"
                  title="${escapeHtml(t("common.copy_appid"))}">
            AppID ${game.app_id}
            <svg class="copy-pill-icon" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
              <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
              <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
            </svg>
          </button>
        </div>
      </td>
      <td>${originBadgeHtml}</td>
      <td>${cloudBadgeHtml}</td>
      <td><span class="select-text">${playtimeStr || "-"}</span></td>
      <td><span class="select-text">${game.file_count > 0 ? `${game.file_count} (${game.total_size_formatted})` : "-"}</span></td>
      <td style="text-align: right;">
        <div class="row-actions" onclick="event.stopPropagation();" style="display: flex; gap: 6px; justify-content: flex-end;">
          <button class="btn btn-secondary btn-sm" onclick="openGameDetailModal(gamesCatalog.find(g => g.app_id === ${game.app_id}))">
            ${escapeHtml(t("common.view_details"))}
          </button>
          <button class="btn btn-icon btn-sm" onclick="invokeBackend('open_steam_store', { appId: ${game.app_id} })" title="${escapeHtml(t("games.btn_steam_store"))}">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
              <path d="M12 2a10 10 0 0 0-10 10c0 4.6 3.1 8.5 7.4 9.6l3-4.3a3.5 3.5 0 0 1-1.4-2.8c0-1.9 1.6-3.5 3.5-3.5a3.5 3.5 0 0 1 3.5 3.5c0 1.9-1.6 3.5-3.5 3.5h-.4l-2.6 3.8A10 10 0 0 0 22 12c0-5.5-4.5-10-10-10z"></path>
            </svg>
          </button>
        </div>
      </td>
    `;

    tr.addEventListener("click", () => openGameDetailModal(game));
    tableBody.appendChild(tr);
  }
}

function openGameDetailModal(game) {
  if (!game) return;
  selectedGame = game;
  const modal = document.getElementById("modal-game-detail");
  if (!modal) return;

  // Hero background
  const heroBanner = document.getElementById("detail-hero-banner");
  if (heroBanner) {
    heroBanner.style.backgroundImage = `url('${game.hero_url}'), url('${game.header_url}')`;
  }

  // Titles & Badges
  document.getElementById("modal-detail-title").textContent = game.name;
  document.getElementById("detail-badge-appid").textContent = `AppID ${game.app_id}`;

  const originBadge = document.getElementById("detail-badge-origin");
  if (originBadge) {
    originBadge.textContent = game.origin_label;
    originBadge.className = `badge-pill ${game.origin === "tools" ? "badge-origin" : ""}`;
  }

  // Copy buttons in modal
  const btnCopyAppId = document.getElementById("btn-copy-detail-appid");
  if (btnCopyAppId) {
    btnCopyAppId.onclick = (e) => {
      e.stopPropagation();
      copyTextToClipboard(game.app_id, t("games.toast_appid_copied", { id: game.app_id }));
    };
  }

  const btnCopyTitle = document.getElementById("btn-copy-detail-title");
  if (btnCopyTitle) {
    btnCopyTitle.onclick = (e) => {
      e.stopPropagation();
      copyTextToClipboard(game.name, t("games.toast_title_copied"));
    };
  }

  // Cloud status & Last sync time
  const cloudStatusEl = document.getElementById("detail-cloud-status");
  const cloudSyncEl = document.getElementById("detail-cloud-last-sync");
  const cloudIconBox = document.getElementById("detail-cloud-icon-box");

  if (game.cloud_status === "synced") {
    if (cloudStatusEl) cloudStatusEl.textContent = t("games.cloud_synced");
    if (cloudIconBox) cloudIconBox.className = "detail-stat-icon-box emerald";
  } else if (game.cloud_status === "local_only") {
    if (cloudStatusEl) cloudStatusEl.textContent = t("games.cloud_local");
    if (cloudIconBox) cloudIconBox.className = "detail-stat-icon-box amber";
  } else {
    if (cloudStatusEl) cloudStatusEl.textContent = t("games.cloud_none");
    if (cloudIconBox) cloudIconBox.className = "detail-stat-icon-box";
  }

  if (game.last_sync_ts && game.last_sync_ts > 0) {
    const d = new Date(game.last_sync_ts * 1000);
    const dateStr = d.toLocaleString(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    });
    if (cloudSyncEl) cloudSyncEl.textContent = t("games.detail_last_sync", { date: dateStr });
  } else {
    if (cloudSyncEl) cloudSyncEl.textContent = t("games.detail_never_synced");
  }

  // Playtime & Last Played
  const playtimeValEl = document.getElementById("detail-playtime-val");
  const lastPlayedEl = document.getElementById("detail-last-played");

  if (game.playtime_minutes > 0) {
    const h = Math.floor(game.playtime_minutes / 60);
    const m = game.playtime_minutes % 60;
    if (playtimeValEl) {
      playtimeValEl.textContent = h > 0
        ? t("games.playtime_hours", { hours: h, minutes: m })
        : t("games.playtime_minutes_only", { minutes: m });
    }
  } else {
    if (playtimeValEl) playtimeValEl.textContent = t("games.playtime_zero");
  }

  if (game.last_played_ts && game.last_played_ts > 0) {
    const d = new Date(game.last_played_ts * 1000);
    const dateStr = d.toLocaleString(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    });
    if (lastPlayedEl) lastPlayedEl.textContent = t("games.detail_last_played", { date: dateStr });
  } else {
    if (lastPlayedEl) lastPlayedEl.textContent = t("games.detail_never_played");
  }

  // Save Files count & size
  const savesCountEl = document.getElementById("detail-saves-count");
  const savesSizeEl = document.getElementById("detail-saves-size");
  if (savesCountEl) savesCountEl.textContent = t("games.detail_files_fmt", { count: game.file_count, size: game.total_size_formatted });
  if (savesSizeEl) savesSizeEl.textContent = game.total_bytes > 0 ? `${game.total_bytes.toLocaleString()} bytes` : "-";

  // Actions
  const btnSteam = document.getElementById("btn-detail-open-steam");
  if (btnSteam) {
    btnSteam.onclick = () => {
      invokeBackend("open_steam_store", { appId: game.app_id });
    };
  }

  const btnSteamDb = document.getElementById("btn-detail-open-steamdb");
  if (btnSteamDb) {
    btnSteamDb.onclick = () => {
      invokeBackend("open_url", { url: `https://steamdb.info/app/${game.app_id}/` });
    };
  }

  const btnAch = document.getElementById("btn-detail-open-achievements");
  if (btnAch) {
    btnAch.onclick = () => {
      modal.style.display = "none";
      openAchievementsModal(game);
    };
  }

  modal.style.display = "flex";
}
window.openGameDetailModal = openGameDetailModal;

async function openAchievementsModal(game) {
  if (!game) return;
  currentAchievementsGame = game;
  const modal = document.getElementById("modal-achievements");
  if (!modal) return;

  document.getElementById("modal-ach-game-name").textContent = `${game.name}`;

  const achBadgeAppId = document.getElementById("ach-badge-appid");
  if (achBadgeAppId) {
    achBadgeAppId.textContent = `AppID ${game.app_id}`;
  }
  const btnCopyAchAppId = document.getElementById("btn-copy-ach-appid");
  if (btnCopyAchAppId) {
    btnCopyAchAppId.style.display = "inline-flex";
    btnCopyAchAppId.onclick = (e) => {
      e.stopPropagation();
      copyTextToClipboard(game.app_id, t("games.toast_appid_copied", { id: game.app_id }));
    };
  }

  // Reset sections
  const progressBox = document.getElementById("ach-progress-container");
  const keyCard = document.getElementById("ach-key-missing-card");
  const loadingState = document.getElementById("ach-loading-state");
  const errorState = document.getElementById("ach-error-state");
  const emptyState = document.getElementById("ach-empty-state");
  const listContainer = document.getElementById("ach-list-container");

  if (progressBox) progressBox.style.display = "none";
  if (keyCard) keyCard.style.display = "none";
  if (loadingState) loadingState.style.display = "flex";
  if (errorState) errorState.style.display = "none";
  if (emptyState) emptyState.style.display = "none";
  if (listContainer) listContainer.innerHTML = "";

  modal.style.display = "flex";

  try {
    const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
    const res = await invokeBackend("get_game_achievements", {
      appId: game.app_id,
      steamDir: inputSteam || appConfig.steam_dir || null,
    });

    if (loadingState) loadingState.style.display = "none";

    if (!res) {
      if (errorState) {
        errorState.style.display = "flex";
        document.getElementById("ach-error-message").textContent = t("common.unknown_error");
      }
      return;
    }

    if (!res.has_api_key) {
      if (keyCard) keyCard.style.display = "flex";
      return;
    }

    if (res.error) {
      if (errorState) {
        errorState.style.display = "flex";
        document.getElementById("ach-error-message").textContent = res.error;
      }
      return;
    }

    if (res.total === 0) {
      if (emptyState) emptyState.style.display = "flex";
      return;
    }

    // Display progress
    if (progressBox) {
      progressBox.style.display = "block";
      const percent = res.total > 0 ? Math.round((res.unlocked / res.total) * 100) : 0;
      document.getElementById("ach-progress-text").textContent = t("achievements.progress_fmt", {
        unlocked: res.unlocked,
        total: res.total,
        percent: percent,
      });
      document.getElementById("ach-progress-percent").textContent = `${percent}%`;
      document.getElementById("ach-progress-fill").style.width = `${percent}%`;
    }

    // Render achievements
    if (listContainer) {
      listContainer.innerHTML = "";
      for (const ach of res.achievements) {
        const itemEl = document.createElement("div");
        itemEl.className = `achievement-item ${ach.unlocked ? "unlocked" : "locked"}`;

        const iconUrl = ach.unlocked
          ? (ach.icon || ach.icon_gray)
          : (ach.icon_gray || ach.icon);

        let unlockDateStr = "";
        if (ach.unlocked && ach.unlock_time) {
          const d = new Date(ach.unlock_time * 1000);
          const dateStr = d.toLocaleString(undefined, {
            dateStyle: "medium",
            timeStyle: "short",
          });
          unlockDateStr = t("achievements.unlocked_at", { date: dateStr });
        }

        const isHidden = !ach.unlocked && ach.hidden;
        const displayName = isHidden ? t("achievements.hidden_title") : (ach.display_name || ach.name);
        const description = isHidden ? t("achievements.hidden_desc") : (ach.description || "");

        itemEl.innerHTML = `
          <div class="ach-icon-box">
            ${iconUrl ? `<img src="${escapeHtml(iconUrl)}" alt="${escapeHtml(displayName)}" loading="lazy" />` : `<span>🏆</span>`}
          </div>
          <div class="ach-texts">
            <span class="ach-name select-text">${escapeHtml(displayName)}</span>
            ${description ? `<span class="ach-desc select-text">${escapeHtml(description)}</span>` : ""}
            ${unlockDateStr ? `<span class="ach-unlock-date select-text">${escapeHtml(unlockDateStr)}</span>` : ""}
          </div>
          <span class="ach-status-badge ${ach.unlocked ? "unlocked" : "locked"}">
            ${ach.unlocked ? escapeHtml(t("achievements.status_unlocked")) : escapeHtml(t("achievements.status_locked"))}
          </span>
        `;
        listContainer.appendChild(itemEl);
      }
    }
  } catch (err) {
    if (loadingState) loadingState.style.display = "none";
    if (errorState) {
      errorState.style.display = "flex";
      document.getElementById("ach-error-message").textContent = String(err);
    }
  }
}
window.openAchievementsModal = openAchievementsModal;

function setupGamesToolbarAndModals() {
  // Filter chips (Tools, Official, All)
  const chips = document.querySelectorAll(".filter-chip[data-filter]");
  chips.forEach((chip) => {
    chip.addEventListener("click", () => {
      chips.forEach((c) => c.classList.remove("active"));
      chip.classList.add("active");
      activeGameFilter = chip.getAttribute("data-filter") || "tools";
      renderGamesCatalog();
    });
  });

  // View mode switcher (Grid vs List)
  const btnGrid = document.getElementById("btn-view-grid");
  const btnList = document.getElementById("btn-view-list");

  if (btnGrid) {
    btnGrid.addEventListener("click", () => {
      currentGamesViewMode = "grid";
      try {
        localStorage.setItem("chronokeep_games_view_mode", "grid");
      } catch (_) {}
      renderGamesCatalog();
    });
  }

  if (btnList) {
    btnList.addEventListener("click", () => {
      currentGamesViewMode = "list";
      try {
        localStorage.setItem("chronokeep_games_view_mode", "list");
      } catch (_) {}
      renderGamesCatalog();
    });
  }

  // Search input
  const searchInput = document.getElementById("games-search-input");
  const clearBtn = document.getElementById("btn-clear-games-search");
  if (searchInput) {
    searchInput.addEventListener("input", (e) => {
      gamesSearchQuery = e.target.value.trim();
      if (clearBtn) clearBtn.style.display = gamesSearchQuery ? "block" : "none";
      renderGamesCatalog();
    });
  }
  if (clearBtn) {
    clearBtn.addEventListener("click", () => {
      if (searchInput) searchInput.value = "";
      gamesSearchQuery = "";
      clearBtn.style.display = "none";
      renderGamesCatalog();
    });
  }

  // Refresh button
  document.getElementById("btn-refresh-games")?.addEventListener("click", () => {
    loadGamesCatalog();
  });

  // Game Detail Modal Close
  const closeDetail = () => {
    const m = document.getElementById("modal-game-detail");
    if (m) m.style.display = "none";
  };
  document.getElementById("btn-close-detail-modal")?.addEventListener("click", closeDetail);
  document.getElementById("btn-detail-close")?.addEventListener("click", closeDetail);
  document.getElementById("modal-game-detail")?.addEventListener("click", (e) => {
    if (e.target.id === "modal-game-detail") closeDetail();
  });

  // Achievements Modal Close
  const closeAch = () => {
    const m = document.getElementById("modal-achievements");
    if (m) m.style.display = "none";
  };
  document.getElementById("btn-close-ach-modal")?.addEventListener("click", closeAch);
  document.getElementById("btn-ach-close")?.addEventListener("click", closeAch);
  document.getElementById("modal-achievements")?.addEventListener("click", (e) => {
    if (e.target.id === "modal-achievements") closeAch();
  });

  // Edit Key button from error state
  document.getElementById("btn-ach-edit-key")?.addEventListener("click", () => {
    document.getElementById("ach-error-state").style.display = "none";
    document.getElementById("ach-key-missing-card").style.display = "flex";
  });

  // Save Key from Achievements Modal Tutorial
  document.getElementById("btn-save-modal-api-key")?.addEventListener("click", async () => {
    const input = document.getElementById("input-modal-api-key");
    const key = input ? input.value.trim() : "";
    if (!key) return;

    try {
      const inputSteam = document.getElementById("input-steam-dir")?.value?.trim();
      const res = await invokeBackend("set_steam_api_key", {
        steamDir: inputSteam || appConfig.steam_dir || null,
        key: key,
      });
      if (res && res.success) {
        showToast(t("achievements.key_saved"));
        // Sync settings input as well
        const settingsInput = document.getElementById("input-settings-steam-api-key");
        if (settingsInput) settingsInput.value = key;

        if (currentAchievementsGame) {
          openAchievementsModal(currentAchievementsGame);
        }
      } else {
        showToast(res ? res.message : "Error saving key");
      }
    } catch (err) {
      showToast(String(err));
    }
  });
}

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
