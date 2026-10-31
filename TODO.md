# ChronoKeep — Backlog & TODOs

Este documento lista tarefas, melhorias arquiteturais e recursos planejados para as próximas versões do ChronoKeep.

---

## 📌 Backlog / Planejado

### 1. Atualização Granular / Individual de Componentes
* **Contexto**: Atualmente, o comando de instalação/atualização implanta o pacote completo (`chrono-daemon.exe`, `chronokeep.dll`, `cloud_redirect.dll`) de uma só vez.
* **Proposta de Melhoria**:
  - [ ] **Atualização Individual do Daemon (`chrono-daemon.exe`)**: Permitir atualizar apenas o serviço em segundo plano sem necessidade de reiniciar a Steam caso não haja mudanças na DLL bridge.
  - [ ] **Atualização Individual da DLL Bridge (`chronokeep.dll` / `cloud_redirect.dll`)**: Permitir injetar/substituir apenas a DLL de interceptação.
  - [ ] **Auto-Atualização do Painel Executável (`ChronoKeepSetup.exe`)**: Mecanismo de verificação de novas versões (via GitHub Releases) e auto-atualização do executável portátil.
  - [ ] **UI com Controles Granulares**: Exibir na interface visual o status e versão individual de cada componente com opções:
    - *Botão*: "Atualizar Apenas o Daemon"
    - *Botão*: "Atualizar Apenas a DLL Bridge"
    - *Botão*: "Verificar Atualização do Setup"

### 2. Exclusão de Jogos da Sincronização Automática (Ignore / Blacklist)
* **Contexto**: Alguns jogos geram centenas de megabytes de saves desnecessários ou o usuário prefere não sincronizar certos títulos.
* **Proposta de Melhoria**:
  - [ ] **Configuração de AppIDs Ignorados**: Suporte à chave `ignored_appids=appid1,appid2` no `chronokeep.ini`.
  - [ ] **Interface de Gestão de Exclusões**: Campo ou lista de seleção visual onde o usuário pode marcar jogos para NÃO realizar backup automático na nuvem.

### 3. Tela de Gerenciamento "Jogos & Nuvem" (Biblioteca Visual)
* **Contexto**: A nova aba "Jogos & Nuvem" adicionada na interface como "Em construção" servirá como o catálogo central de saves do usuário.
* **Proposta de Melhoria**:
  - [ ] **Catálogo Visual de Jogos**:
    - Nome oficial do jogo e AppID.
    - Imagem de capa oficial via Steam CDN (`https://cdn.akamai.steamstatic.com/steam/apps/<appID>/header.jpg`).
    - Detalhes de tamanho, número de arquivos e data da última alteração.
  - [ ] **Indicadores de Estado (Local vs Nuvem)**:
    - Indicador se o save está presente no Cofre local.
    - Indicador se o save já foi sincronizado com o Google Drive.
  - [ ] **Ações por Título**:
    - Toggle individual para ativar/pausar backup na nuvem para o jogo selecionado.
    - Botão de upload/sincronização imediata.
    - Botão de restauração (download do save da nuvem para a pasta local da Steam).

### 4. Melhorias de Resiliência e Conectividade (Revisão de Código)
* **Contexto**: Pontos identificados na revisão automatizada para refinamento em versões futuras.
* **Proposta de Melhoria**:
  - [ ] **Reabrir Steam em caso de falha de instalação (`commands.rs`)**: Se o Steam estava em execução e a implantação de DLL falhar (ex: falta de permissão), garantir que o `steam.exe` seja relançado mesmo no caminho de erro.
  - [ ] **Ajuste de Timeouts WinHTTP para Uploads Grandes (`http.rs`)**: Elevar os timeouts de envio (`nSendTimeout`) e resposta (`nReceiveTimeout`) do cliente WinHTTP de 5s para 60s ou 120s para prevenir que conexões lentas abortem uploads de saves volumosos.
  - [ ] **Expiração de Placeholders no Cache da Steam Store (`commands.rs`)**: Não persistir fallbacks temporários (`AppID <id>`) no cache em disco quando a Steam Store estiver indisponível, permitindo tentar resolver o nome novamente na próxima varredura.



