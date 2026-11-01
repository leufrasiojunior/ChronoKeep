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

5. **Versionamento Unificado e Builds (`dist/` e `embedded/`)**:
   - Manter sempre os 3 componentes principais (`cr-gui` / `ChronoKeep.exe`, `cr-service` / `chrono-daemon.exe` e `cr-bridge-dll` / `chronokeep.dll`), crates internas (`cr-common`, `cr-proto`), `tauri.conf.json`, marcadores `CK_DAEMON_VER:`/`CK_BRIDGE_VER:` e referências de versão da UI sincronizados na **mesma versão semântica**.
   - Sempre gerar binários via `./build_dist.sh` para garantir que `crates/cr-gui/embedded/` embuta o daemon e a DLL recém-compilados antes do build final do `ChronoKeep.exe`.
   - O verificador de atualizações (`updater.rs`) deve comparar versões estritamente via SemVer (`remote_semver > local_semver`), nunca sugerindo *downgrade* ou atualização falsa apenas por divergência de hash SHA-256.

---

## 2. Salvaguardas de Domínio e Protocolo Steam Cloud

1. **Filtragem de AppIDs de Sistema e Isolamento de Jogos Oficiais**:
   - Ignorar completamente em RPCs e no Google Drive AppIDs internos da Valve/Steam:
     - `241100` / `241300`: Configurações de Steam Controller / Steam Input
     - `760`: Screenshots locais do Steam
     - `7`: Módulos compartilhados do cliente Steam
   - Interceptar RPCs e executar pull/push em segundo plano no Google Drive **exclusivamente** para AppIDs gerenciados pelo ChronoKeep (`stplug-in`, `lua`, `AppList`), garantindo *passthrough* total e intocabilidade para jogos oficiais da conta Steam.

2. **Monotonicidade de Change Number (CN)**:
   - Manifests e arquivos da nuvem só devem ser adotados localmente se `cloud_cn > local_cn`.
   - Arquivos locais em `userdata` não devem ser sobrescritos se já existirem no mesmo CN ou superior durante a inicialização do jogo ou da Steam.
   - Na reconciliação local (`sync_local_saves_to_vault`), arquivos já existentes no Cofre só devem ser atualizados a partir de `userdata/remote` se `src_mtime > vault_ref_ts`, e pastas legadas de migração só devem semear arquivos inéditos.

3. **Higienização de Caminhos, Root Tokens e AutoCloud**:
   - Sempre sanitizar recursivamente tokens de raiz (ex: `%WinMyDocuments%`, `%AppData%`).
   - Impedir duplicação de tokens no início do caminho (ex: `%WinMyDocuments%%WinMyDocuments%...`) para garantir caminhos válidos e canônicos no filesystem Win32.
   - **Isolamento AutoCloud (`root_token != ""`)**: Jogos AutoCloud (ex: `%WinAppDataLocal%`, `%WinMyDocuments%`, `%WinSavedGames%`) não utilizam `Steam\userdata\<conta>\<app>\remote`. Nunca espelhar saves com `root_token` não-vazio dentro de `userdata/remote` nem importar cópias órfãs de `userdata/remote` para o Cofre.

4. **Controle de Concorrência no Google Drive (Downloads e Uploads)**:
   - Manter travas de concorrência (`AtomicBool`, `Mutex<HashSet<(u32, u32)>>`) compartilhadas entre o pull sob demanda e o pull em lote para impedir downloads simultâneos do mesmo jogo.
   - Serializar uploads por `(account_id, app_id)` via `Mutex` dedicado e enviar o `manifest.tsv` lido do disco sob trava, evitando corridas ou sobrescrita de manifestos quando múltiplos arquivos de save são comitados em lote no fechamento do jogo.

5. **Ciclo de Vida do Daemon**:
   - O `chrono-daemon.exe` deve ter seu ciclo de vida estritamente acoplado à execução do cliente Steam (`steam.exe`).
   - Monitorar o PID do Steam via Win32 `WaitForSingleObject` para encerrar o daemon imediatamente e de forma limpa quando o Steam for fechado, além de responder ao `notify_shutdown` no `DLL_PROCESS_DETACH` da bridge DLL.

6. **ABI x64 em Hooks FFI C++ (`steamclient64.dll`)**:
   - Métodos C++ na ABI MSVC x64 que retornam `bool` (ex: `MessageLite::SerializeToArray`, `MessageLite::ParseFromArray`) escrevem o resultado apenas no registrador de 8 bits `AL`, deixando lixo nos 56 bits superiores de `RAX`. Declarar sempre o retorno FFI como `u8` (ou `bool`), nunca `usize`/`u64`.

---

## 3. Padrões de Interface Windows 11 (UI/UX)

1. **Estilo Fluent Design**:
   - O aplicativo unificado (`ChronoKeep.exe` em Tauri v2 + modos Setup/Uninstall) deve seguir a linguagem visual do Windows 11.
   - Utilizar tipografia Segoe UI Variable, cantos arredondados, espaçamento harmonioso e tabelas/listas com layout flexível que redimensionem responsivamente ao maximizar a janela.

2. **Detecção e Alternância de Tema**:
   - Suporte dinâmico a temas: Claro, Escuro e Seguir Sistema (lendo a chave do registro do Windows `AppsUseLightTheme` em `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`).

3. **Internacionalização (`i18n` — `pt-BR` e `en-US`)**:
   - Toda nova chave textual, modal, toast ou mensagem de backend deve incluir tradução em `pt-BR` e `en-US` (`crates/cr-gui/ui/i18n.js` e `crates/cr-gui/src/i18n.rs`), mantendo detecção automática do idioma do sistema (com fallback `en-US`) e troca imediata na seção **"Configurações"** (**"Settings"**).
