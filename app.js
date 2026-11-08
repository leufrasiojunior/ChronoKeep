/**
 * ChronoKeep Landing Page — Interactive & i18n Engine
 */

(function () {
  'use strict';

  // 1. Translations Dictionary (pt-BR / en-US)
  const i18nData = {
    'pt-BR': {
      'nav.skip': 'Pular para o conteúdo',
      'nav.features': 'Recursos',
      'nav.compatibility': 'Compatibilidade',
      'nav.how_it_works': 'Como Funciona',
      'nav.faq': 'FAQ',
      'nav.download': 'Baixar',

      'hero.pill': '100% Seguro & Não Invasivo • Sem Modificar a Steam',
      'hero.title_part1': 'Seus saves e conquistas da Steam',
      'hero.title_part2': 'preservados com perfeição.',
      'hero.desc': 'O <strong>ChronoKeep</strong> salva seus jogos e conquistas automaticamente em segundo plano. Ele guarda cópias seguras no seu <strong>Cofre Local</strong> e sincroniza tudo com o seu <strong>Google Drive</strong> — sem alterar nenhum arquivo da Steam, sem perdas acidentais e com total segurança.',
      'hero.btn_download': 'Baixar ChronoKeep (x64)',
      'hero.btn_learn': 'Ver Compatibilidade & Lua',
      'hero.check1': 'Steam 100% Intacta (Não altera nenhum arquivo)',
      'hero.check2': 'Proteção Inteligente (Saves novos nunca são perdidos)',
      'hero.check3': '100% Gratuito & Código Aberto (MIT)',

      // Real Mockup Labels
      'mockup.app_header': 'Configurações do ChronoKeep',
      'mockup.search_placeholder': 'Pesquisar por configurações',
      'mockup.nav_home': 'Início',
      'mockup.nav_games': 'Jogos',
      'mockup.nav_providers': 'Provedores de Backup',
      'mockup.nav_saves': 'Saves Órfãos',
      'mockup.nav_logs': 'Logs do Sistema',
      'mockup.nav_about': 'Sobre',
      'mockup.nav_settings': 'Configurações',
      'mockup.nav_github': 'Repositório GitHub',
      'mockup.daemon_active': 'Daemon Ativo',
      'mockup.page_home': 'Início',
      'mockup.quick_access': 'Acesso rápido',
      'mockup.tile_install': 'Instalar na Steam',
      'mockup.tile_daemon': 'Parar Daemon',
      'mockup.tile_vault': 'Abrir Cofre',
      'mockup.tile_steam': 'Pasta Steam',
      'mockup.tile_drive': 'Drive Conectado',
      'mockup.tile_logs': 'Ver Logs',
      'mockup.utilities': 'Utilitários e Serviços',
      'mockup.daemon_desc': 'Serviço de sincronização ativo em segundo plano',
      'mockup.ost_desc': 'Habilitação do recurso [cloud] no opensteamtool.toml',
      'mockup.active': 'Ativo',
      'mockup.check_deps': 'Verificar Dependências',
      'mockup.gdrive_desc': 'Conta Google vinculada — Sincronização em nuvem ativa',
      'mockup.disconnect': 'Desconectar',

      'features.pill': 'Funcionalidades Principais',
      'features.title': 'Segurança e Praticidade para Seus Jogos',
      'features.desc': 'Projetado para que você nunca mais perca horas de jogo após formatar o computador, trocar de PC ou jogar sem internet.',
      'features.cleanroom_title': 'Totalmente Seguro e Não Invasivo',
      'features.cleanroom_desc': 'O ChronoKeep não altera nem substitui nenhum arquivo da Valve ou dos seus jogos (`steam.exe`, `steamclient64.dll`). O salvamento é detectado de forma limpa na memória, sem risco de corrupção, sem conflitos com anti-cheat e sem banimentos.',
      'features.vault_title': 'Cofre Local Automático',
      'features.vault_desc': 'Mesmo se sua internet cair ou você jogar offline, o Cofre Local guarda cópias protegidas de cada save. O sistema inteligente impede que um save antigo acabe sobrescrevendo um mais novo.',
      'features.gdrive_title': 'Backup no seu Google Drive',
      'features.gdrive_desc': 'Conecte sua própria conta do Google Drive com 1 clique. Os uploads e downloads são organizados automaticamente, garantindo que tudo chegue seguro à sua nuvem pessoal.',
      'features.lifecycle_title': 'Abre e Fecha com a Steam',
      'features.lifecycle_desc': 'O serviço do ChronoKeep funciona silenciosamente em segundo plano enquanto a Steam estiver aberta. Quando você fechar a Steam, ele se encerra sozinho sem consumir memória.',
      'features.autocloud_title': 'Suporte a Saves Fora da Pasta Steam',
      'features.autocloud_desc': 'Muitos jogos salvam em pastas especiais do Windows (`Documentos`, `AppData`, `Saved Games`). O ChronoKeep localiza e preserva esses arquivos automaticamente.',
      'features.ui_title': 'Visual Moderno do Windows 11',
      'features.ui_desc': 'Interface fluida e elegante em Tauri v2 com efeito Mica, fontes nativas do Windows 11, modo Claro e Escuro, e suporte completo em português e inglês.',

      'compat.pill': 'Compatibilidade & Termos',
      'compat.title': 'Compatível com Steam, CloudRedirect, SteamTools e Lua',
      'compat.desc': 'Se você já utilizava ferramentas comunitárias ou scripts para gerenciar jogos, o ChronoKeep foi construído para ser a sua evolução moderna, transparente e sem riscos.',
      'compat.ck_title': 'ChronoKeep: Fácil, Seguro e Aberto',
      'compat.legacy_title': 'O Que Muda em Relação a Métodos Antigos?',
      'compat.tags_title': 'Termos & Tecnologias Integradas:',

      'how.pill': 'Como Funciona',
      'how.title': 'Como Seus Jogos Ficam Protegidos',
      'how.desc': 'Três passos automáticos que acontecem em segundo plano sem que você precise se preocupar com nada.',
      'how.step1_title': '1. Detecção Automática',
      'how.step1_desc': 'Enquanto você joga, o ChronoKeep percebe silenciosamente quando o jogo salva o seu progresso, sem causar travamentos, lentidão ou queda de FPS.',
      'how.step2_title': '2. Cópia no Cofre Local',
      'how.step2_desc': 'Uma cópia exata do save é guardada imediatamente no seu Cofre Local do computador. Seus dados estão seguros mesmo se o jogo travar ou se você estiver sem internet.',
      'how.step3_title': '3. Sincronização com a Nuvem',
      'how.step3_desc': 'O arquivo é enviado para o seu Google Drive pessoal. Se você formatar o PC ou trocar de máquina, seus jogos estarão intactos te esperando.',

      'arch.pill': 'Por Dentro do ChronoKeep',
      'arch.title': 'Estrutura Leve e Confiável',
      'arch.desc': 'Construído em Rust para ser rápido, estável e gastar praticamente zero de memória RAM e processador.',
      'arch.gui': 'Painel visual elegante em Tauri v2 para você acompanhar o status dos seus saves, espaço na nuvem e preferências.',
      'arch.daemon': 'Serviço em segundo plano que cuida dos backups com o Google Drive e se desliga automaticamente junto com a Steam.',
      'arch.bridge': 'Módulo de conexão de alta velocidade que detecta os salvamentos em tempo real sem tocar nos arquivos da Steam.',
      'arch.proto': 'Comunicação interna ultrarrápida que organiza as pastas e garante a integridade de cada save.',

      'dl.pill': 'Download',
      'dl.title': 'Comece a Usar Gratuitamente',
      'dl.desc': 'Baixe e use em poucos segundos. Sem instaladores pesados, sem anúncios e com código aberto.',
      'dl.official_badge': 'Versão Oficial Estável',
      'dl.summary': 'Inclui o aplicativo visual, serviço de segundo plano e sincronização com o Google Drive.',
      'dl.btn_download_now': 'Baixar no GitHub Releases',
      'dl.btn_source': 'Código Fonte & Compilação',
      'dl.req_os': 'Sistema Operacional',
      'dl.req_steam': 'Steam',
      'dl.req_cloud': 'Nuvem Suportada',
      'dl.req_license': 'Licença',

      'faq.pill': 'Dúvidas Frequentes',
      'faq.title': 'Perguntas & Respostas',
      'faq.desc': 'Tudo o que você precisa saber sobre a segurança e o funcionamento do ChronoKeep.',
      'faq.q1': 'O ChronoKeep é compatível com manifests de SteamTools, Open SteamTools e LuaTools?',
      'faq.a1': 'Sim! O ChronoKeep entende automaticamente manifests de jogos (`manifest.tsv`), scripts Lua (`lua` e ferramentas como `luatools`) e pastas personalizadas. Ele atua como um gerenciador de backup confiável que funciona perfeitamente ao lado de ferramentas comunitárias abertas como Open SteamTools (`open steam tools` / `steamtools`).',
      'faq.q2': 'Posso migrar meus saves do antigo CloudRedirect para o ChronoKeep?',
      'faq.a2': 'Sim, a migração é simples e automática. O ChronoKeep detecta as pastas criadas pelo CloudRedirect (`cloudredirect` / `cloud redirect`), importa seus saves anteriores para o Cofre Local e envia os manifests atualizados para o seu Google Drive sem perder nada.',
      'faq.q3': 'O ChronoKeep altera arquivos da Steam ou pode causar ban no VAC?',
      'faq.a3': 'Não. O ChronoKeep é 100% não invasivo. Ele jamais altera arquivos executáveis da Steam (`steam.exe`, `steamclient64.dll`) ou dos seus jogos. Jogos oficiais e proteções anti-cheat continuam funcionando normalmente sem qualquer interferência.',
      'faq.q4': 'O que acontece se eu jogar sem internet (modo offline)?',
      'faq.a4': 'Seus saves continuam 100% seguros! O Cofre Local armazena o progresso no seu próprio disco rígido. Assim que você se conectar à internet novamente, o ChronoKeep faz o envio para o Google Drive automaticamente sem sobrescrever arquivos mais novos.',
      'faq.q5': 'Onde os saves ficam guardados no Google Drive?',
      'faq.a5': 'Eles ficam salvos diretamente na sua conta particular do Google Drive, dentro de uma pasta segura chamada ChronoKeep, organizada por jogo e conta. Só você tem acesso aos seus arquivos.',

      'footer.tagline': 'Preservação limpa, segura e moderna de saves e conquistas para quem ama jogar.',
      'footer.links_title': 'Links Rápidos',
      'footer.community_title': 'Comunidade & Código'
    },

    'en-US': {
      'nav.skip': 'Skip to content',
      'nav.features': 'Features',
      'nav.compatibility': 'Compatibility',
      'nav.how_it_works': 'How It Works',
      'nav.faq': 'FAQ',
      'nav.download': 'Download',

      'hero.pill': '100% Safe & Non-Invasive • Never Modifies Steam',
      'hero.title_part1': 'Your Steam saves & achievements',
      'hero.title_part2': 'perfectly preserved.',
      'hero.desc': '<strong>ChronoKeep</strong> automatically backs up your game saves and achievements in the background. It keeps safe copies in your <strong>Local Vault</strong> and syncs everything to your personal <strong>Google Drive</strong> — without touching Steam files, with zero data loss, and complete peace of mind.',
      'hero.btn_download': 'Download ChronoKeep (x64)',
      'hero.btn_learn': 'View Compatibility & Lua',
      'hero.check1': 'Steam 100% Untouched (Never alters your game files)',
      'hero.check2': 'Smart Save Protection (Newer saves are never lost)',
      'hero.check3': '100% Free & Open Source (MIT)',

      // Real Mockup Labels (EN)
      'mockup.app_header': 'ChronoKeep Settings',
      'mockup.search_placeholder': 'Search settings',
      'mockup.nav_home': 'Home',
      'mockup.nav_games': 'Games',
      'mockup.nav_providers': 'Backup Providers',
      'mockup.nav_saves': 'Orphan Saves',
      'mockup.nav_logs': 'System Logs',
      'mockup.nav_about': 'About',
      'mockup.nav_settings': 'Settings',
      'mockup.nav_github': 'GitHub Repository',
      'mockup.daemon_active': 'Daemon Active',
      'mockup.page_home': 'Home',
      'mockup.quick_access': 'Quick Access',
      'mockup.tile_install': 'Install on Steam',
      'mockup.tile_daemon': 'Stop Daemon',
      'mockup.tile_vault': 'Open Vault',
      'mockup.tile_steam': 'Steam Folder',
      'mockup.tile_drive': 'Drive Connected',
      'mockup.tile_logs': 'View Logs',
      'mockup.utilities': 'Utilities & Services',
      'mockup.daemon_desc': 'Background sync service currently running',
      'mockup.ost_desc': 'Enabling [cloud] feature in opensteamtool.toml',
      'mockup.active': 'Active',
      'mockup.check_deps': 'Check Dependencies',
      'mockup.gdrive_desc': 'Google Account linked — Cloud sync active',
      'mockup.disconnect': 'Disconnect',

      'features.pill': 'Core Features',
      'features.title': 'Safety & Simplicity for Your Games',
      'features.desc': 'Engineered so you never lose your hard-earned progress again—even across Windows reinstalls, hardware changes, or offline gaming sessions.',
      'features.cleanroom_title': 'Completely Safe & Non-Invasive',
      'features.cleanroom_desc': 'ChronoKeep never alters or replaces Valve or game executable files (`steam.exe`, `steamclient64.dll`). Saves are cleanly detected in memory with zero file corruption, zero anti-cheat conflict, and zero ban risk.',
      'features.vault_title': 'Automatic Local Vault',
      'features.vault_desc': 'Even if your internet connection drops or you play offline, the Local Vault stores protected copies of each save. Smart versioning guarantees older saves never overwrite newer ones.',
      'features.gdrive_title': 'Personal Google Drive Backup',
      'features.gdrive_desc': 'Connect your private Google Drive account in 1 click. Uploads and downloads are synchronized automatically, keeping your save files safe and private in your cloud.',
      'features.lifecycle_title': 'Launches & Quits with Steam',
      'features.lifecycle_desc': 'The ChronoKeep background service runs silently while Steam is active. When you exit Steam, it terminates cleanly without wasting memory or battery.',
      'features.autocloud_title': 'Support for External Save Folders',
      'features.autocloud_desc': 'Many games store saves in special Windows directories (`Documents`, `AppData`, `Saved Games`). ChronoKeep detects and preserves these external files seamlessly.',
      'features.ui_title': 'Modern Windows 11 Visuals',
      'features.ui_desc': 'A sleek desktop app in Tauri v2 featuring Mica materials, native Segoe UI typography, dynamic Light and Dark mode, and full bilingual Portuguese/English support.',

      'compat.pill': 'Compatibility & Ecosystem',
      'compat.title': 'Compatible with Steam, CloudRedirect, SteamTools & Lua',
      'compat.desc': 'Whether you play on Steam, manage games with community setups, or are migrating from legacy tools, ChronoKeep is your modern and safe solution.',
      'compat.ck_title': 'ChronoKeep: Simple, Safe & Open Source',
      'compat.legacy_title': 'What Makes ChronoKeep Better Than Legacy Tools?',
      'compat.tags_title': 'Covered Concepts & Technologies:',

      'how.pill': 'How It Works',
      'how.title': 'How Your Saves Stay Protected',
      'how.desc': 'Three seamless steps happening automatically in the background without any manual effort.',
      'how.step1_title': '1. Automatic Detection',
      'how.step1_desc': 'As you play, ChronoKeep silently notices whenever your game saves progress, with zero lag, zero stutter, and zero FPS drop.',
      'how.step2_title': '2. Local Vault Backup',
      'how.step2_desc': 'An exact snapshot is saved immediately to your local PC Vault. Your data is protected even if the game crashes or your internet is down.',
      'how.step3_title': '3. Cloud Synchronization',
      'how.step3_desc': 'The save is securely pushed to your personal Google Drive. If you switch to a new PC or reinstall Windows, your progress is waiting for you.',

      'arch.pill': 'Under the Hood',
      'arch.title': 'Lightweight & Reliable Architecture',
      'arch.desc': 'Crafted in Rust to be lightning-fast, rock-solid, and virtually imperceptible in RAM and CPU usage.',
      'arch.gui': 'Modern visual dashboard built in Tauri v2 to monitor your saves, cloud quota, and settings.',
      'arch.daemon': 'Silent background service handling Google Drive syncing and auto-terminating whenever Steam closes.',
      'arch.bridge': 'High-speed native module detecting save events in real time without touching official Steam files.',
      'arch.proto': 'Fast internal communication layer ensuring folder sanitization and bulletproof data integrity.',

      'dl.pill': 'Distribution & Install',
      'dl.title': 'Get Started in Seconds',
      'dl.desc': 'Single installer or portable executable suite. Free, open source, and lightweight.',
      'dl.official_badge': 'Official Stable Release',
      'dl.summary': 'Includes unified Fluent UI, background synchronization daemon, and native persistence bridge.',
      'dl.btn_download_now': 'Download on GitHub Releases',
      'dl.btn_source': 'Source Code & Build Guide',
      'dl.req_os': 'Operating System',
      'dl.req_steam': 'Steam Client',
      'dl.req_cloud': 'Cloud Provider',
      'dl.req_license': 'License',

      'faq.pill': 'Frequently Asked Questions',
      'faq.title': 'Questions & Answers',
      'faq.desc': 'Everything you need to know regarding safety, compatibility, and cloud synchronization.',
      'faq.q1': 'Is ChronoKeep compatible with SteamTools, Open SteamTools, and LuaTools manifests?',
      'faq.a1': 'Yes! ChronoKeep automatically interprets game manifests (`manifest.tsv`), Lua scripts (`lua` and tools like `luatools`), and custom game folders. It acts as a safe preservation manager that works seamlessly alongside open community tools like Open SteamTools (`open steam tools` / `steamtools`).',
      'faq.q2': 'Can I migrate my saves from legacy CloudRedirect to ChronoKeep?',
      'faq.a2': 'Yes, migration is smooth and automatic. ChronoKeep detects folders created by legacy CloudRedirect (`cloudredirect` / `cloud redirect`), imports existing saves into the Local Vault, and syncs updated manifests to your Google Drive without data loss.',
      'faq.q3': 'Does ChronoKeep alter Steam files or risk VAC bans?',
      'faq.a3': 'No. ChronoKeep is 100% non-invasive. It never modifies or replaces Steam executable files (`steam.exe`, `steamclient64.dll`). Official games and anti-cheat protections continue operating normally without interference.',
      'faq.q4': 'What happens if I play offline without an internet connection?',
      'faq.a4': 'Your progress is 100% safe! The Local Vault stores saves directly on your drive. As soon as you go online again, ChronoKeep uploads them to Google Drive automatically without overwriting newer files.',
      'faq.q5': 'Where are saves stored in Google Drive?',
      'faq.a5': 'Files are preserved in your private Google Drive inside a dedicated folder (`ChronoKeep`), organized by game and account. Only you have access to your personal files.',

      'footer.tagline': 'Clean, safe, and modern save and achievement preservation for gamers.',
      'footer.links_title': 'Quick Links',
      'footer.community_title': 'Community & Code'
    }
  };

  // 2. Language Management
  let currentLang = localStorage.getItem('chronokeep_lang') || (navigator.language && navigator.language.startsWith('pt') ? 'pt-BR' : 'en-US');

  function applyLanguage(lang) {
    if (!i18nData[lang]) lang = 'pt-BR';
    currentLang = lang;
    localStorage.setItem('chronokeep_lang', lang);

    document.documentElement.setAttribute('lang', lang);
    const flagEl = document.getElementById('lang-current-label');
    if (flagEl) {
      flagEl.textContent = lang === 'pt-BR' ? 'PT' : 'EN';
    }

    const elements = document.querySelectorAll('[data-i18n]');
    elements.forEach(function (el) {
      const key = el.getAttribute('data-i18n');
      if (i18nData[lang][key]) {
        el.innerHTML = i18nData[lang][key];
      }
    });

    const placeholders = document.querySelectorAll('[data-i18n-placeholder]');
    placeholders.forEach(function (el) {
      const key = el.getAttribute('data-i18n-placeholder');
      if (i18nData[lang][key]) {
        el.setAttribute('placeholder', i18nData[lang][key]);
      }
    });
  }

  // 3. Theme Management
  const currentTheme = localStorage.getItem('chronokeep_theme') || 
    (window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark');

  function applyTheme(theme) {
    document.documentElement.setAttribute('data-theme', theme);
    localStorage.setItem('chronokeep_theme', theme);
  }

  // 4. Dynamic GitHub Releases Fetcher
  async function fetchLatestRelease() {
    try {
      const response = await fetch('https://api.github.com/repos/leufrasiojunior/ChronoKeep/releases/latest');
      if (!response.ok) return;
      const data = await response.json();
      
      const tagName = data.tag_name || 'v1.0.4';
      const releaseUrl = data.html_url || 'https://github.com/leufrasiojunior/ChronoKeep/releases/latest';

      const navTag = document.getElementById('nav-release-tag');
      const dlTag = document.getElementById('dl-tag-badge');
      const heroVersionText = document.getElementById('hero-version-text');
      const heroDlBtn = document.getElementById('hero-download-btn');
      const primaryDlLink = document.getElementById('primary-dl-link');

      if (navTag) navTag.textContent = tagName;
      if (dlTag) dlTag.textContent = tagName;
      if (heroDlBtn) heroDlBtn.href = releaseUrl;
      if (primaryDlLink) primaryDlLink.href = releaseUrl;

      if (heroVersionText) {
        heroVersionText.textContent = `${currentLang === 'pt-BR' ? 'Versão Estável' : 'Stable Version'} ${tagName} • Windows 10/11`;
      }
    } catch (e) {
      console.warn('Using static release fallback:', e);
    }
  }

  // 5. DOM Initialization
  document.addEventListener('DOMContentLoaded', function () {
    // Apply initial state
    applyTheme(currentTheme);
    applyLanguage(currentLang);
    fetchLatestRelease();

    // Theme Toggle Listener
    const themeBtn = document.getElementById('theme-toggle-btn');
    if (themeBtn) {
      themeBtn.addEventListener('click', function () {
        const activeTheme = document.documentElement.getAttribute('data-theme') === 'light' ? 'dark' : 'light';
        applyTheme(activeTheme);
      });
    }

    // Language Toggle Listener
    const langBtn = document.getElementById('lang-toggle-btn');
    if (langBtn) {
      langBtn.addEventListener('click', function () {
        const nextLang = currentLang === 'pt-BR' ? 'en-US' : 'pt-BR';
        applyLanguage(nextLang);
      });
    }

    // Mobile Menu Toggle
    const mobileMenuBtn = document.getElementById('mobile-menu-btn');
    const navLinks = document.getElementById('nav-links');
    if (mobileMenuBtn && navLinks) {
      mobileMenuBtn.addEventListener('click', function () {
        navLinks.classList.toggle('active');
      });

      // Close menu when clicking link
      navLinks.querySelectorAll('.nav-link').forEach(function (link) {
        link.addEventListener('click', function () {
          navLinks.classList.remove('active');
        });
      });
    }

    // FAQ Accordion
    const faqQuestions = document.querySelectorAll('.faq-question');
    faqQuestions.forEach(function (btn) {
      btn.addEventListener('click', function () {
        const item = btn.closest('.faq-item');
        const isActive = item.classList.contains('active');

        // Close other items
        document.querySelectorAll('.faq-item').forEach(function (other) {
          other.classList.remove('active');
          const q = other.querySelector('.faq-question');
          if (q) q.setAttribute('aria-expanded', 'false');
        });

        // Toggle current item
        if (!isActive) {
          item.classList.add('active');
          btn.setAttribute('aria-expanded', 'true');
        }
      });
    });
  });
})();
