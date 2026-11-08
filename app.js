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
      'nav.compatibility': 'Compatibilidade & Termos',
      'nav.how_it_works': 'Como Funciona',
      'nav.architecture': 'Arquitetura',
      'nav.faq': 'FAQ',
      'nav.download': 'Baixar',

      'hero.pill': 'Solução Clean-Room em Rust • Tauri v2 Fluent Design',
      'hero.title_part1': 'Seus saves e conquistas da Steam',
      'hero.title_part2': 'preservados com perfeição.',
      'hero.desc': 'O <strong>ChronoKeep</strong> intercepta eventos de salvamento em memória via ABI FFI x64, armazena snapshots atômicos em um <strong>Cofre Local</strong> versionado e sincroniza tudo com seu <strong>Google Drive</strong>. Sem modificar arquivos da Steam, sem perdas acidentais e sem risco de ban.',
      'hero.btn_download': 'Baixar ChronoKeep (x64)',
      'hero.btn_learn': 'Ver Compatibilidade & Lua',
      'hero.check1': 'Zero Binary Patching (Steam Intacta)',
      'hero.check2': 'Controle Monotônico de Versão (CN)',
      'hero.check3': '100% Open Source (MIT)',

      'mockup.title': 'ChronoKeep — Painel de Controle',
      'mockup.tab_home': 'Início',
      'mockup.tab_games': 'Jogos & Saves',
      'mockup.tab_vault': 'Cofre Local',
      'mockup.tab_cloud': 'Google Drive',
      'mockup.status_title': 'Steam Cloud Interceptada & Sincronizada',
      'mockup.status_sub': 'Google Drive: Conectado • Cofre Local: 100% Protegido',
      'mockup.status_synced': '● Sincronizado',
      'mockup.stat_games': 'Jogos Monitorados',
      'mockup.stat_snapshots': 'Snapshots no Cofre',
      'mockup.stat_cloud': 'Espaço em Nuvem',

      'features.pill': 'Funcionalidades Essenciais',
      'features.title': 'Desenvolvido com Rigor de Engenharia em Rust',
      'features.desc': 'Cada detalhe do ChronoKeep foi projetado para garantir que você nunca mais perca um savegame, mesmo após formatações, falhas de sistema ou trocas de hardware.',
      'features.cleanroom_title': 'Clean-Room & Zero Modificações',
      'features.cleanroom_desc': 'O ChronoKeep não modifica arquivos binários da Valve no disco (`steam.exe`, `steamclient64.dll`). As chamadas de IPC do Steam Cloud são interceptadas em tempo de execução via ponte FFI ABI MSVC x64 de alta fidelidade.',
      'features.vault_title': 'Cofre Local (Local Vault)',
      'features.vault_desc': 'Mesmo que a sua conexão caia ou você jogue em modo offline, o Cofre Local guarda snapshots versionados e imutáveis de cada save. O controle monotônico de Change Number (CN) impede que um arquivo antigo sobrescreva um mais recente.',
      'features.gdrive_title': 'Sincronização Atômica Google Drive',
      'features.gdrive_desc': 'Integração nativa com sua conta pessoal do Google Drive via OAuth2 seguro. Uploads e downloads são serializados com travas de concorrência (`Mutex`), garantindo integridade de manifests mesmo se o jogo salvar múltiplos arquivos em rajada.',
      'features.lifecycle_title': 'Ciclo de Vida Acoplado à Steam',
      'features.lifecycle_desc': 'O daemon em segundo plano (`chrono-daemon.exe`) monitora o processo do Steam via Win32 `WaitForSingleObject`. Quando você fecha a Steam, o daemon fecha silenciosamente sem deixar processos zumbis consumindo bateria ou memória.',
      'features.autocloud_title': 'Isolamento AutoCloud & Root Tokens',
      'features.autocloud_desc': 'Tratamento correto de caminhos de jogos que salvam fora da pasta Steam (`%WinAppDataLocal%`, `%WinMyDocuments%`, `%WinSavedGames%`). Sanitização profunda de caminhos impede duplicações e respeita saves oficiais da sua conta.',
      'features.ui_title': 'Fluent Design Windows 11 Nativo',
      'features.ui_desc': 'Uma interface desktop em Tauri v2 que parece ter vindo embutida no Windows 11. Efeitos Mica, fontes Segoe UI Variable, tema dinâmico Claro/Escuro e suporte bilingue completo em português brasileiro e inglês.',

      'compat.pill': 'Compatibilidade & Termos Técnicos',
      'compat.title': 'A Evolução Limpa de CloudRedirect, SteamTools e LuaTools',
      'compat.desc': 'Se você buscava por alternativas transparentes e auditadas para sincronização de jogos da Steam com suporte a manifests e Lua, conheça as vantagens do ChronoKeep.',
      'compat.ck_title': 'ChronoKeep: Seguro, Moderno e Aberto',
      'compat.legacy_title': 'O Cenário Anterior (CloudRedirect & LuaTools)',
      'compat.tags_title': 'Termos & Tecnologias Cobertas:',

      'how.pill': 'Fluxo Transparente',
      'how.title': 'Como o ChronoKeep Salva Seus Jogos',
      'how.desc': 'Três passos atômicos executados de forma silenciosa e sem impacto na taxa de quadros (FPS) do seu jogo.',
      'how.step1_title': '1. Interceptação Limpa',
      'how.step1_desc': 'Durante o gameplay, quando o jogo chama a API de salvamento da Steam, nossa bridge nativa FFI de 64-bit captura a chamada em tempo de execução sem modificar nenhum byte de arquivo no disco.',
      'how.step2_title': '2. Preservação no Cofre',
      'how.step2_desc': 'O daemon calcula o hash SHA-256 e o Change Number (CN). Um snapshot do save é copiado atomicamente para o Cofre Local, garantindo restauração instantânea mesmo se o disco falhar ou se você estiver offline.',
      'how.step3_title': '3. Upload Seguro na Nuvem',
      'how.step3_desc': 'Em segundo plano e sob travas de concorrência exclusivas, o arquivo e seu manifesto são sincronizados com seu Google Drive pessoal. Seus jogos estarão prontos para qualquer novo PC.',

      'arch.pill': 'Arquitetura de Software',
      'arch.title': 'Quatro Componentes Modulares',
      'arch.desc': 'O ecossistema ChronoKeep separa responsabilidades de forma estrita para atingir estabilidade industrial e conformidade com o Windows 11.',
      'arch.gui': 'Interface moderna construída em Tauri v2 com HTML/CSS puro seguindo o Windows 11 Fluent Design (Mica Material). Oferece gerenciamento visual de saves, cofre, status de nuvem e configurações.',
      'arch.daemon': 'O cérebro operacional em background. Gerencia o Google Drive, reconcilia o cofre local com a nuvem, sincroniza conquistas e monitora o ciclo de vida do Steam com encerramento automático.',
      'arch.bridge': 'Ponte de interceptação FFI de altíssima precisão. Compilada sob ABI MSVC x64, intercepta de forma segura chamadas de persistência sem afetar jogos protegidos por sistemas anti-cheat oficiais.',
      'arch.proto': 'Crates internas com estruturas de dados tipadas, serialização ultrarrápida, sanitização rigorosa de tokens de diretórios (`%AppData%`) e canais de comunicação entre processos por Named Pipes.',

      'dl.pill': 'Distribuição & Instalação',
      'dl.title': 'Pronto para Usar em Segundos',
      'dl.desc': 'Instalação em lote ou executável portátil. Sem instaladores complexos ou pacotes gigantescos.',
      'dl.official_badge': 'Versão Oficial Estável',
      'dl.summary': 'Inclui o aplicativo Fluent UI unificado, daemon de sincronização e ponte nativa de persistência.',
      'dl.btn_download_now': 'Baixar no GitHub Releases',
      'dl.btn_source': 'Código Fonte & Compilação',
      'dl.req_os': 'Sistema Operacional',
      'dl.req_steam': 'Steam',
      'dl.req_cloud': 'Nuvem Suportada',
      'dl.req_license': 'Licença',

      'faq.pill': 'Dúvidas Frequentes',
      'faq.title': 'Perguntas & Respostas',
      'faq.desc': 'Tudo o que você precisa saber sobre segurança, compatibilidade e funcionamento da sincronização.',
      'faq.q1': 'O ChronoKeep é compatível com manifests de SteamTools, Open SteamTools e LuaTools?',
      'faq.a1': 'Sim! O ChronoKeep foi projetado para ler e interpretar manifests de jogos da Steam (`manifest.tsv`), scripts Lua (`lua` e ecossistema `luatools`) e diretórios customizados. Ele atua como um gerenciador de preservação seguro que funciona perfeitamente ao lado de ferramentas comunitárias abertas como Open SteamTools (`open steam tools` / `steamtools`).',
      'faq.q2': 'Posso migrar meus saves antigos do CloudRedirect para o ChronoKeep?',
      'faq.a2': 'Sim, a transição é transparente. O ChronoKeep inclui rotinas automáticas de reconciliação que detectam arquivos históricos originados do CloudRedirect clássico, indexa-os no novo Cofre Local e sincroniza os manifests atualizados para sua conta no Google Drive sem perdas de progresso.',
      'faq.q3': 'O ChronoKeep altera arquivos da Steam ou pode causar ban no VAC?',
      'faq.a3': 'Não. O ChronoKeep segue a metodologia Clean-Room. Ele jamais modifica ou substitui arquivos da Valve no disco (`steam.exe`, `steamclient64.dll`). Além disso, os AppIDs de sistema e jogos oficiais com anti-cheat permanecem intactos por meio de regras de passthrough estritas.',
      'faq.q4': 'O que acontece se eu jogar offline sem acesso à internet?',
      'faq.a4': 'Seus dados continuam 100% preservados! O Cofre Local armazena o snapshot do save localmente com incremento monotônico de Change Number (CN). Assim que a conexão com a internet retornar, o daemon detectará a pendência e fará o upload atômico para o Google Drive sem sobrescrever versões mais recentes.',
      'faq.q5': 'Onde os saves ficam armazenados no Google Drive?',
      'faq.a5': 'Os arquivos são salvos na sua própria conta do Google Drive em uma pasta dedicada e segura (`ChronoKeep`), organizada por ID de conta e AppID de cada jogo, acompanhada do arquivo canônico de manifesto. Somente você tem acesso aos seus arquivos.',

      'footer.tagline': 'Preservação limpa e moderna de saves e conquistas para a comunidade de jogadores.',
      'footer.links_title': 'Links Rápidos',
      'footer.community_title': 'Comunidade & Código'
    },

    'en-US': {
      'nav.skip': 'Skip to content',
      'nav.features': 'Features',
      'nav.compatibility': 'Compatibility & Terms',
      'nav.how_it_works': 'How It Works',
      'nav.architecture': 'Architecture',
      'nav.faq': 'FAQ',
      'nav.download': 'Download',

      'hero.pill': 'Clean-Room Rust Solution • Tauri v2 Fluent Design',
      'hero.title_part1': 'Your Steam saves & achievements',
      'hero.title_part2': 'perfectly preserved.',
      'hero.desc': '<strong>ChronoKeep</strong> intercepts save events in memory via 64-bit native MSVC ABI FFI, stores atomic snapshots in a versioned <strong>Local Vault</strong>, and seamlessly synchronizes everything to your personal <strong>Google Drive</strong>. Zero binary patching, zero accidental loss, and zero VAC ban risk.',
      'hero.btn_download': 'Download ChronoKeep (x64)',
      'hero.btn_learn': 'View Compatibility & Lua',
      'hero.check1': 'Zero Binary Patching (Steam Intact)',
      'hero.check2': 'Monotonic Change Number (CN) Control',
      'hero.check3': '100% Open Source (MIT)',

      'mockup.title': 'ChronoKeep — Control Center',
      'mockup.tab_home': 'Home',
      'mockup.tab_games': 'Games & Saves',
      'mockup.tab_vault': 'Local Vault',
      'mockup.tab_cloud': 'Google Drive',
      'mockup.status_title': 'Steam Cloud Intercepted & Synchronized',
      'mockup.status_sub': 'Google Drive: Connected • Local Vault: 100% Protected',
      'mockup.status_synced': '● Synced',
      'mockup.stat_games': 'Monitored Games',
      'mockup.stat_snapshots': 'Snapshots in Vault',
      'mockup.stat_cloud': 'Cloud Storage',

      'features.pill': 'Core Features',
      'features.title': 'Engineered with Precision in Rust',
      'features.desc': 'Every component of ChronoKeep was designed to ensure you never lose your save data again—even across OS reinstallations, crashes, or new hardware migrations.',
      'features.cleanroom_title': 'Clean-Room & Zero Binary Mod',
      'features.cleanroom_desc': 'ChronoKeep never modifies Valve binary files on disk (`steam.exe`, `steamclient64.dll`). Steam Cloud IPC calls are cleanly intercepted at runtime through a high-precision MSVC x64 FFI bridge.',
      'features.vault_title': 'Local Vault',
      'features.vault_desc': 'Even if your internet connection drops or you play offline, the Local Vault stores immutable, versioned snapshots. Strict monotonic Change Number (CN) safeguards prevent older saves from overwriting newer ones.',
      'features.gdrive_title': 'Atomic Google Drive Sync',
      'features.gdrive_desc': 'Native integration with your personal Google Drive via secure OAuth2. Uploads and downloads are serialized using concurrency locks (`Mutex`), guaranteeing manifest integrity even if multiple files save simultaneously.',
      'features.lifecycle_title': 'Tightly Coupled to Steam Lifecycle',
      'features.lifecycle_desc': 'The background daemon (`chrono-daemon.exe`) monitors the Steam process using Win32 `WaitForSingleObject`. When you quit Steam, the daemon terminates cleanly without leaving orphan processes behind.',
      'features.autocloud_title': 'AutoCloud & Root Tokens Isolation',
      'features.autocloud_desc': 'Proper handling of games that save outside the Steam folder (`%WinAppDataLocal%`, `%WinMyDocuments%`, `%WinSavedGames%`). Path sanitization prevents duplicated tokens and respects official saves.',
      'features.ui_title': 'Native Windows 11 Fluent Design',
      'features.ui_desc': 'A Tauri v2 desktop application that feels built into Windows 11. Mica material effects, Segoe UI Variable typography, dynamic Light/Dark theme switching, and complete English/Portuguese bilingual support.',

      'compat.pill': 'Compatibility & Technical Scope',
      'compat.title': 'The Clean Evolution of CloudRedirect, SteamTools & LuaTools',
      'compat.desc': 'If you are looking for an open-source, audited replacement for Steam save sync with full manifest and Lua support, discover what ChronoKeep brings to the table.',
      'compat.ck_title': 'ChronoKeep: Safe, Modern and Open Source',
      'compat.legacy_title': 'The Legacy Era (CloudRedirect & LuaTools)',
      'compat.tags_title': 'Indexed Technologies & Concepts:',

      'how.pill': 'Transparent Pipeline',
      'how.title': 'How ChronoKeep Preserves Your Game Data',
      'how.desc': 'Three atomic steps executed silently in the background with zero performance or FPS drop during gameplay.',
      'how.step1_title': '1. Clean Interception',
      'how.step1_desc': 'During gameplay, when the game invokes Steam save APIs, our native 64-bit FFI bridge captures the IPC call in runtime memory without touching files on disk.',
      'how.step2_title': '2. Vault Snapshot',
      'how.step2_desc': 'The daemon calculates SHA-256 hashes and monotonic Change Numbers (CN). An atomic snapshot is placed into the Local Vault, ensuring instant recovery even if offline or if drives crash.',
      'how.step3_title': '3. Secure Cloud Upload',
      'how.step3_desc': 'Under exclusive concurrency locks, the save file and its manifest are uploaded to your personal Google Drive. Your progress is immediately ready for any new PC.',

      'arch.pill': 'Software Architecture',
      'arch.title': 'Four Modular Rust Components',
      'arch.desc': 'The ChronoKeep ecosystem strictly separates concerns to achieve industrial-grade reliability on Windows 11.',
      'arch.gui': 'Modern desktop app in Tauri v2 and pure HTML/CSS following Windows 11 Fluent Design (Mica Material). Offers visual save management, vault history, cloud status, and settings.',
      'arch.daemon': 'The background core service. Manages Google Drive OAuth2, reconciles local vault with cloud state, syncs achievements, and terminates gracefully when Steam closes.',
      'arch.bridge': 'High-precision FFI interception bridge. Compiled with MSVC x64 ABI, safely intercepts persistence calls while passing through official Steam games.',
      'arch.proto': 'Internal crates providing strongly-typed structures, fast binary serialization, deep path token sanitization, and Named Pipe IPC communications.',

      'dl.pill': 'Distribution & Install',
      'dl.title': 'Ready in Seconds',
      'dl.desc': 'Single installer or portable executable suite. No bloat, no heavy dependencies.',
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
      'faq.a1': 'Yes! ChronoKeep is engineered to parse Steam game manifests (`manifest.tsv`), Lua scripts (`lua` and the `luatools` ecosystem), and custom save folders. It acts as a safe preservation manager that works seamlessly alongside community tools such as Open SteamTools (`open steam tools` / `steamtools`).',
      'faq.q2': 'Can I migrate my old saves from CloudRedirect to ChronoKeep?',
      'faq.a2': 'Yes, the transition is seamless. ChronoKeep features automatic reconciliation routines that detect legacy saves from classic CloudRedirect, indexes them into the new Local Vault, and pushes updated manifests to your Google Drive without data loss.',
      'faq.q3': 'Does ChronoKeep alter Steam binaries or risk VAC bans?',
      'faq.a3': 'No. ChronoKeep strictly follows a Clean-Room approach. It never modifies or replaces Valve executable files on disk (`steam.exe`, `steamclient64.dll`). Official games and system AppIDs pass through untouched without interference.',
      'faq.q4': 'What happens if I play offline without an internet connection?',
      'faq.a4': 'Your progress is 100% safe! The Local Vault stores the snapshot with monotonic Change Numbers (CN). As soon as an internet connection is established, the daemon detects pending uploads and commits them to Google Drive without overwriting newer saves.',
      'faq.q5': 'Where are saves stored in Google Drive?',
      'faq.a5': 'Files are preserved in your private Google Drive inside a dedicated folder (`ChronoKeep`), categorized by Account ID and AppID alongside canonical manifest files. Only you have access to your personal files.',

      'footer.tagline': 'Clean, modern save and achievement preservation for gamers.',
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
      // Graceful fallback to static v1.0.4 on rate limits or offline
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
