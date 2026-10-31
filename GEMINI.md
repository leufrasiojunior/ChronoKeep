# Diretrizes e Regras do Projeto ChronoKeep

Este documento define os padrões arquiteturais, diretrizes de Git/branching, salvaguardas de sincronização Steam Cloud e padrões de interface para o desenvolvimento do ChronoKeep.

---

## 1. Fluxo Git e Política de Branches

1. **Branch `main` (Produção / Release)**:
   - Exclusivamente reservada para versões estáveis e releases publicadas (`v*`, ex: `v0.1.0`, `v0.1.2`).
   - Nunca commitar ou dar push diretamente na `main`.

2. **Branch `develop` (Desenvolvimento e Integração)**:
   - Branch principal de integração para o desenvolvimento contínuo.
   - Toda nova funcionalidade ou correção deve partir de `develop`.

3. **Branches de Feature / Fix**:
   - Devem ser criadas a partir de `develop` com nomenclatura descritiva (ex: `feature/win11-modern-ui`, `fix/...`).
   - **Ciclo de vida contínuo**: Não fechar nem mergear branches prematuramente a cada commit individual. O trabalho deve continuar na mesma branch até o ajuste estar completamente concluído, testado e validado pelo usuário.
   - Merges para `develop` ocorrem sob aprovação expressa do usuário ou via Pull Request revisado.

4. **Mudança de Escopo**:
   - Caso seja solicitada uma tarefa totalmente diferente do foco da branch ativa, **perguntar proativamente ao usuário** se deseja abrir uma nova branch ou se prefere manter o trabalho na mesma branch antes de iniciar modificações no código.

---

## 2. Salvaguardas de Domínio e Protocolo Steam Cloud

1. **Filtragem de AppIDs de Sistema**:
   - Ignorar ou excluir da sincronização de nuvem AppIDs internos da Valve/Steam que não correspondem a jogos de usuário:
     - `241100`: Configurações de Steam Controller
     - `760`: Screenshots locais do Steam
     - `7`: Módulos compartilhados do cliente Steam

2. **Monotonicidade de Change Number (CN)**:
   - Manifests e arquivos da nuvem só devem ser adotados localmente se `cloud_cn > local_cn`.
   - Arquivos locais em `userdata` não devem ser sobrescritos se já existirem no mesmo CN ou superior durante a inicialização do jogo ou da Steam.

3. **Higienização de Caminhos e Root Tokens**:
   - Sempre sanitizar recursivamente tokens de raiz (ex: `%WinMyDocuments%`, `%AppData%`).
   - Impedir duplicação de tokens no início do caminho (ex: `%WinMyDocuments%%WinMyDocuments%...`) para garantir caminhos válidos e canônicos no filesystem Win32.

4. **Controle de Concorrência de Downloads**:
   - Manter travas de concorrência (`AtomicBool`, `Mutex<HashSet<...>>`) nos downloads em segundo plano do Google Drive para impedir múltiplas threads simultâneas baixando ou gravando saves do mesmo jogo/sessão ao mesmo tempo.

5. **Ciclo de Vida do Daemon**:
   - O `chrono-daemon.exe` deve ter seu ciclo de vida estritamente acoplado à execução do cliente Steam (`steam.exe`).
   - Monitorar o PID do Steam via Win32 `WaitForSingleObject` para encerrar o daemon imediatamente e de forma limpa quando o Steam for fechado, além de responder ao `notify_shutdown` no `DLL_PROCESS_DETACH` da bridge DLL.

---

## 3. Padrões de Interface Windows 11 (UI/UX)

1. **Estilo Fluent Design**:
   - Aplicativos visuais (como o `ChronoKeepSetup.exe`) devem seguir a linguagem visual do Windows 11.
   - Utilizar tipografia Segoe UI Variable, cantos arredondados e espaçamento harmonioso.

2. **Detecção e Alternância de Tema**:
   - Suporte dinâmico a temas: Claro, Escuro e Seguir Sistema (lendo a chave do registro do Windows `AppsUseLightTheme` em `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`).
