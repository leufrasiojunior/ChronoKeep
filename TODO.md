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
