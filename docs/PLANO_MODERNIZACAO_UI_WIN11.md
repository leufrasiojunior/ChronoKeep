# Plano de Modernização: Interface Windows 11 (Fluent Design)

Este documento registra o planejamento técnico para a modernização da interface gráfica do instalador e configurador [`ChronoKeepSetup.exe`](../rust-workspace/crates/cr-gui/src/main.rs).

---

## 1. Diretrizes Visuais Windows 11

1. **Barra de Título Imersiva e Bordas**:
   - `DwmSetWindowAttribute` com atributo 20 (`DWMWA_USE_IMMERSIVE_DARK_MODE`) para escurecer a barra nativa do Windows no modo escuro.
   - `DwmSetWindowAttribute` com atributo 33 (`DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND (2)`) para cantos arredondados nativos no Windows 11.
2. **Tipografia Segoe UI Variable**:
   - Família prioritária: `"Segoe UI Variable Text"`, com fallback para `"Segoe UI"`.
   - Hierarquia clara:
     - Títulos / Headers: 12pt - 14pt (Semibold 600).
     - Corpo de texto / Rótulos: 9pt - 10pt (Regular 400).
3. **Organização em Superfícies (Cards)**:
   - Fundo com cor sutil (Light `#F3F3F3` / Dark `#202020`).
   - Seções agrupadas em cartões elevados com bordas finas e cantos arredondados:
     - **Card Conexão**: Status da conta Google Drive com indicador visual (badge verde/cinza) e botão de login/desconectar.
     - **Card Diretórios**: Pastas da Steam e do Vault com botões de procurar/abrir.
     - **Card Limite de Upload & Preferências**: Seletor de limite máximo de upload (`max_upload_mb`: 128 MB, 256 MB, 512 MB, 1024 MB) e seletor de tema.
     - **Card Ações**: Botão de Ação Primária em destaque ("Instalar na Steam"), botões secundários para "Iniciar Serviço", "Abrir Vault" e "Ver Logs".
4. **Navegação em Estilo Segmented Control**:
   - Substituição dos botões legados de aba no topo por um seletor visual segmentado moderno (Configuração vs. Saves na Nuvem/Órfãos).

---

## 2. Motor Dinâmico de Temas (Claro / Escuro / Sistema)

- **Modos Suportados**:
  1. `Seguir Sistema` (padrão)
  2. `Modo Claro`
  3. `Modo Escuro`
- **Leitura do Registro**:
  - `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize` -> `AppsUseLightTheme` (0 = Dark, 1 = Light).
- **Tempo Real**:
  - Interceptar mensagem Win32 `WM_SETTINGCHANGE` (quando `lParam` aponta para `"ImmersiveColorSet"`) para atualizar as cores instantaneamente quando o tema do Windows mudar.

### Paleta de Cores

| Elemento | Modo Escuro (Dark) | Modo Claro (Light) |
| :--- | :--- | :--- |
| **Fundo da Janela** | `#202020` | `#F3F3F3` |
| **Superfície do Card** | `#2C2C2C` | `#FFFFFF` |
| **Borda do Card** | `#383838` | `#E5E5E5` |
| **Texto Primário** | `#FFFFFF` | `#1B1B1B` |
| **Texto Secundário / Labels** | `#A0A0A0` | `#5C5C5C` |
| **Accent Color** | `#60CDFF` / `#0078D4` | `#0067C0` |
| **Botão Padrão** | Fundo `#323232`, Borda `#3D3D3D` | Fundo `#FFFFFF`, Borda `#D1D1D1` |
| **Botão Primário (Accent)** | Fundo `#0078D4`, Texto `#FFFFFF` | Fundo `#0067C0`, Texto `#FFFFFF` |
| **Inputs / ListBox** | Fundo `#1E1E1E`, Borda `#3D3D3D` | Fundo `#FFFFFF`, Borda `#D1D1D1` |

---

## 3. Roteiro de Implementação Futuro

1. Criar branch `feature/win11-modern-ui` a partir de `develop`.
2. Implementar módulo de temas em `cr-gui` com leitura de registro e gerenciamento de `HBRUSH`/`HPEN`.
3. Adicionar custom draw nos botões e cards com `DwmSetWindowAttribute`.
4. Adicionar controle de `max_upload_mb` no `chronokeep.ini`.
5. Compilar para Windows MSVC e validar nos dois temas.
