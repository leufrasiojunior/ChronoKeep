# Relatório de Code Review — Projeto ChronoKeep

**Data:** 28 de Outubro de 2026  
**Status do Repositório:** Branch `develop` (Clean working tree)  
**Escopo Analisado:** `rust-workspace/crates/` (`cr-proto`, `cr-service`, `cr-bridge-dll`, `cr-gui`)  
**Metodologia:** Análise estática, inspeção de salvaguardas de domínio Steam Cloud, revisão de concorrência, segurança, desempenho e manutenibilidade.

---

## 1. Visão Geral e Avaliação Executiva

O **ChronoKeep** é uma solução *clean-room* em Rust desenvolvida para interceptar, preservar e sincronizar saves da Steam Cloud localmente e no Google Drive.

### Principais Pontos Positivos da Arquitetura:
1. **Zero Dependências Pesadas:** As crates `cr-proto`, `cr-service`, `cr-bridge-dll` e `cr-gui` praticamente não usam crates de terceiros pesadas (`tokio`, `reqwest`, `prost`, etc.), produzindo binários ultraleves (< 1 MB), com tempo de compilação quase instantâneo e ausência de dependências de runtime C/C++.
2. **Implementações Clean-room Eficientes:** A implementação manual de decodificação/codificação Protobuf ([`wire.rs`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-proto/src/wire.rs)), descompressão DEFLATE/ZIP ([`zip_z.rs`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-proto/src/zip_z.rs)) e cálculo incremental de MD5 ([`md5.rs`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-proto/src/md5.rs)) é concisa e robusta.
3. **Adesão a Salvaguardas Críticas:** O código respeita a grande maioria das diretrizes de segurança definidas no [`GEMINI.md`](file:///home/ubuntu/projetos/cr-clone/GEMINI.md), como a filtragem de AppIDs de sistema (`7`, `760`, `241100`, `241300`), monotonicidade do Change Number (`CN`), sanitização recursiva de tokens de raiz (`%WinMyDocuments%`), sincronização com locks de concorrência no Google Drive e monitoramento do ciclo de vida via Steam PID (`WaitForSingleObject`).

No entanto, a revisão identificou **uma discrepância crítica entre o Daemon e a GUI**, oportunidades de **reforço de segurança contra crashes no processo do Steam**, **eliminação de código duplicado** e **otimizações de I/O**.

---

## 2. Matriz de Priorização de Achados

| ID | Severidade | Categoria | Descrição Resumida | Arquivo / Componente |
| :--- | :--- | :--- | :--- | :--- |
| **BUG-01** | **Crítica (P0)** | Correção Lógica | Incompatibilidade de sintaxe do `manifest.tsv` entre `cr-gui` e `cr-service` | [`cr-gui/src/main.rs#L1100-L1145`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L1100-L1145) vs [`vault.rs#L409-L450`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/vault.rs#L409-L450) |
| **SEC-01** | **Alta (P1)** | Confiabilidade / Estabilidade | Desreferenciamento de ponteiros de vtable sem SEH/VEH em processo injetado | [`vtable.rs#L156-L180`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-bridge-dll/src/vtable.rs#L156-L180) |
| **SEC-02** | **Alta (P1)** | Segurança / Rede | Servidor HTTP local sem autenticação/token de controle | [`main.rs#L344-L450`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/main.rs#L344-L450) |
| **DUP-01** | **Média (P2)** | Manutenibilidade | Duplicação de clientes WinHTTP, OAuth2 e GDrive entre Service e GUI | [`gdrive.rs`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/gdrive.rs) e [`cr-gui/src/main.rs`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs) |
| **PERF-01** | **Média (P2)** | Desempenho / I/O | I/O síncrono e repetido em disco a cada log interceptado na vtable | [`lib.rs#L239-L264`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-bridge-dll/src/lib.rs#L239-L264) |
| **PERF-02** | **Média (P2)** | Gestão de Recursos | Alocação de memória RAM integral (até 512MB) para uploads de blobs | [`main.rs#L685-L723`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/main.rs#L685-L723) |
| **BLD-01** | **Baixa (P3)** | Tooling / Build | Falha de checagem do Rust (`cargo check`) em hosts Linux | [`cr-gui/src/main.rs#L16`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L16) |

---

## 3. Análise Detalhada dos Achados

### 3.1. BUG-01 (Crítica - P0): Incompatibilidade de Formato do `manifest.tsv`

#### Contexto e Causa Raiz
No daemon ([`vault.rs:409-450`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/vault.rs#L409-L450)), o manifesto do Vault é estruturado e analisado de forma canônica com prefixos em caixa alta:
```tsv
META	<change_number>	<app_build_id>	<default_root_token>
FILE	<filename>	<root_token>	<sha1_hex>	<timestamp>	<raw_size>	<platforms_to_sync>
```
O método [`load_manifest_raw`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/vault.rs#L398) descarta qualquer linha que não comece exatamente com `META` ou `FILE`:
```rust
if parts[0] == "META" && parts.len() >= 3 { ... }
else if parts[0] == "FILE" && parts.len() >= 7 { ... }
```

No entanto, na GUI ([`cr-gui/src/main.rs:1102-1140`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L1102-L1140)), a função `on_upload_orphans` gera o manifesto usando sintaxe legada e diferente:
```rust
manifest_lines.push(format!("cn\t{}", existing_cn));
manifest_lines.push(format!("build\t{}", 0));
manifest_lines.push(format!("default_token\t"));
...
manifest_lines.push(format!(
    "file\t{}\t{}\t{}\t{}\t{}\t0\t4294967295",
    rel_name, "", sha, file_ts, bytes.len()
));
```

#### Impacto
Se o usuário usar a aba de "Saves Órfãos" na GUI para sincronizar saves locais para o Google Drive e o Vault, o `manifest.tsv` salvo em disco é escrito com `cn\t...` e `file\t...`. Quando o `chrono-daemon` é iniciado e tenta carregar esse manifesto via `load_manifest_raw`, **nenhuma linha é reconhecida**, resetando o CN para 0 e perdendo os metadados dos arquivos!

#### Recomendação de Correção
1. Em [`cr-gui/src/main.rs`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs), atualizar a escrita para o formato `META` e `FILE`:
   ```rust
   let mut manifest_lines = Vec::new();
   manifest_lines.push(format!("META\t{}\t0\t", existing_cn));
   for (rel_name, full_path) in &game.files {
       ...
       manifest_lines.push(format!(
           "FILE\t{}\t\t{}\t{}\t{}\t4294967295",
           rel_name, sha, file_ts, bytes.len()
       ));
   }
   ```
2. Em [`vault.rs`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/vault.rs), adicionar compatibilidade retroativa no `load_manifest_raw` para ler tanto `META`/`FILE` quanto o formato legado `cn`/`file`.

---

### 3.2. SEC-01 (Alta - P1): Desreferenciamento de Ponteiros sem SEH nos Hooks de Vtable

#### Contexto e Causa Raiz
Na bridge injetada ([`vtable.rs`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-bridge-dll/src/vtable.rs)), os hooks `hooked_slot4` e `hooked_slot5` interceptam chamadas RPC que trafegam estruturas internas do Steam (`steamclient64.dll`).
Em [`vtable.rs:160`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-bridge-dll/src/vtable.rs#L160) e [`vtable.rs:294`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-bridge-dll/src/vtable.rs#L294):
```rust
let vtable = *(body_obj as *const usize);
let byte_size_fn: ByteSizeFn = std::mem::transmute(*((vtable + 64) as *const usize));
let expected_size = byte_size_fn(body_obj);
...
let req_hdr = *((request_msg as usize + 40) as *const *mut std::ffi::c_void);
let req_body = *((request_msg as usize + 48) as *const *mut std::ffi::c_void);
```

#### Impacto
Se a Valve atualizar o layout do objeto `CProtoBufMsg` em uma versão futura do Steam Client ou se o ponteiro recebido no hook for nulo/inválido/corrompido por outro hook de terceiro, o desreferenciamento causará uma Violação de Acesso Win32 (`0xC0000005 - EXCEPTION_ACCESS_VIOLATION`). No Windows, sem um manipulador de exceções estruturadas (SEH), isso resulta em **crash imediato do cliente Steam (`steam.exe`)**.

#### Recomendação de Correção
Adicionar validações adicionais de ponteiros (e.g., verificar alinhamento e intervalos válidos de memória no espaço de usuário do Windows `0x10000..0x7FFFFFFEFFFF`), ou envolver desreferenciamentos críticos com uma proteção SEH nativa via wrapper C/Assembly ou `AddVectoredExceptionHandler`.

---

### 3.3. SEC-02 (Alta - P1): Servidor HTTP Local sem Autenticação / Token de Sessão

#### Contexto e Causa Raiz
O daemon [`chrono-daemon`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/main.rs#L344) escuta na interface de loopback `127.0.0.1:<porta>` e aceita requisições HTTP comuns sem nenhum cabeçalho de autenticação ou token compartilhado:
- `POST /v1/shutdown` encerra o processo do daemon imediatamente.
- `POST /v1/rpc` executa operações de cloud arbitrárias.
- `GET /download/...` e `PUT /upload/...` manipulam arquivos do Vault.

#### Impacto
Qualquer processo executado pelo mesmo usuário no Windows (como navegadores, scripts ou programas de terceiros) pode se conectar à porta `39771` e derrubar o serviço ou ler/gravar arquivos no Vault local.

#### Recomendação de Correção
Implementar um token simples gerado dinamicamente:
1. Quando o `chrono-daemon` inicia, ele gera um token aleatório de 64 bits (ex: `chrono_auth.dat` com permissão restrita na pasta do Steam).
2. O `chronokeep.dll` e a GUI leem o token e o enviam em todas as requisições HTTP: `X-ChronoKeep-Token: <segredo>`.
3. Requisições sem o token válido retornam `401 Unauthorized`.

---

### 3.4. DUP-01 (Média - P2): Duplicação Massiva de Código entre Service e GUI

#### Contexto e Causa Raiz
Existem blocos substanciais de código 100% idênticos copiados entre `cr-service` e `cr-gui`:
1. **Cliente WinHTTP Seguro:** `https_request` ([`gdrive.rs:100-202`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/gdrive.rs#L100-L202) vs [`cr-gui/src/main.rs:401-506`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L401-L506)).
2. **Parser JSON Manual:** `extract_json_str` ([`gdrive.rs:204-228`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/gdrive.rs#L204-L228) vs [`cr-gui/src/main.rs:309-333`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L309-L333)).
3. **Renovação de Tokens OAuth2:** `refresh_access_token` ([`gdrive.rs:242`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/gdrive.rs#L242) vs [`cr-gui/src/main.rs:514`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L514)).
4. **Gerenciamento de Pastas Google Drive:** `find_or_create_drive_folder` ([`gdrive.rs:292`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/gdrive.rs#L292) vs [`cr-gui/src/main.rs:533`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L533)).
5. **Varredura Recursiva de Disco:** `collect_files_recursive` ([`vault.rs:543`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/vault.rs#L543) vs [`cr-gui/src/main.rs:719`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L719)).
6. **Resolução de Porta Dinâmica:** `resolve_service_port` ([`cr-bridge-dll/src/ipc.rs:26`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-bridge-dll/src/ipc.rs#L26) vs [`cr-gui/src/main.rs:1251`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L1251)).

#### Impacto
Duplica a superfície de manutenção e causa divergências sutis de comportamento (como o **BUG-01** citado acima).

#### Recomendação de Correção
Mover os utilitários de sistema e integração do Google Drive para `cr-proto` ou criar uma crate compartilhada de utilitários `cr-common` no workspace, tornando `cr-service` e `cr-gui` meros consumidores dessa lógica centralizada.

---

### 3.5. PERF-01 (Média - P2): I/O Síncrono em Disco a Cada Linha de Log da DLL

#### Contexto e Causa Raiz
Em [`cr-bridge-dll/src/lib.rs:239-264`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-bridge-dll/src/lib.rs#L239-L264), a função `log_line` é chamada frequentemente em `CK_DispatchCloudRpc` e nos hooks `hooked_slot4` / `hooked_slot5`.
A cada invocação:
1. Faz `OpenOptions::new().create(true).append(true).open("chronokeep_bridge.log")`.
2. Executa `write_all` e `flush()`.
3. Faz `OpenOptions::new().create(true).append(true).open("cloud_redirect.log")`.
4. Executa `write_all` e `flush()`.

#### Impacto
Abre, grava, descarrega em disco e fecha **dois arquivos de log** de forma síncrona dentro da thread de transporte do Steam client. Se o disco estiver sob alta carga (ex: Steam descompactando um jogo de 50 GB), essa chamada pode introduzir latência desnecessária no processamento de pacotes RPC do Steam.

#### Recomendação de Correção
1. Manter os `File` handles abertos em uma estrutura estática (`std::sync::Mutex<Option<File>>`) inicializada uma única vez, em vez de reabrir o descritor a cada linha.
2. Ou utilizar um canal lock-free / thread de logging em segundo plano.

---

### 3.6. PERF-02 (Média - P2): Alocação Integral de Memória RAM para Blobs de Saves

#### Contexto e Causa Raiz
Em [`cr-service/src/main.rs:685-723`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/main.rs#L685-L723), no processamento de uploads HTTP `PUT /upload/...`:
```rust
let mut body = Vec::with_capacity(content_length);
while body.len() < content_length {
    ...
    body.extend_from_slice(&chunk[..n]);
}
```
Depois, [`try_unwrap_steam_zip(&body)`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/main.rs#L520) aloca outro `Vec<u8>` para os bytes descompactados.

#### Impacto
Embora o limite configurado seja de 512 MB, jogos com saves gigantescos alocam buffers múltiplos de várias centenas de megabytes simultaneamente na RAM do `chrono-daemon.exe`.

#### Recomendação de Correção
Para saves pequenos (< 16 MB), o buffer em memória é ideal. Para arquivos maiores, transmitir o fluxo diretamente para um arquivo temporário em disco (`*.cr_tmp`) e processar a descompressão via streaming.

---

### 3.7. BLD-01 (Baixa - P3): Falha de Compilação do `cargo check` em Ambientes Linux

#### Contexto e Causa Raiz
Em [`cr-gui/src/main.rs:16`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L16):
```rust
use std::os::windows::process::CommandExt;
```
Está sem a diretiva condicional `#[cfg(target_os = "windows")]`.
Além disso, nas linhas [`1648`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L1648), [`1661`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L1661) e [`1859`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L1859), `.creation_flags(CREATE_NO_WINDOW)` é chamado diretamente no `std::process::Command`.

#### Impacto
Qualquer desenvolvedor ou ferramenta CI rodando `cargo check` padrão no Linux (sem a flag `--target x86_64-pc-windows-msvc`) quebra a compilação imediatamente, prejudicando integrações de linters locais e análise de código.

#### Recomendação de Correção
Adicionar `#[cfg(target_os = "windows")]` no import e nas chamadas de `creation_flags`.

---

## 4. Conformidade com as Regras de Domínio (`GEMINI.md`)

Avaliamos a conformidade estrita da codebase com as regras arquiteturais estabelecidas:

| Regra / Salvaguarda | Status | Observações da Análise |
| :--- | :---: | :--- |
| **Exclusão de AppIDs de Sistema (241100, 760, 7)** | ✅ Conforme | Respeitado em `is_namespace_app`, `on_exit_sync`, `on_scan_orphans` e `start_background_pull_all`. Inclui também `241300` (Steam Input). |
| **Monotonicidade do Change Number (CN)** | ✅ Conforme | Saves remotos só são adotados se `cloud_cn > local_cn` ([`gdrive.rs:770`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/gdrive.rs#L770)). Alterações no disco local não incrementam CN arbitrariamente ([`vault.rs:184-185`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/vault.rs#L184-L185)). |
| **Sanitização de Tokens de Raiz (`%WinMyDocuments%`)** | ✅ Conforme | Função `split_root_token` ([`cloud_messages.rs:60-76`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-proto/src/cloud_messages.rs#L60-L76)) trata repetições e remove tokens aninhados. |
| **Controle de Concorrência de Downloads** | ✅ Conforme | Utiliza `ACTIVE_PULLS` com `HashSet<(u32, u32)>` e guard RAII para evitar downloads duplicados simultâneos. |
| **Ciclo de Vida do Daemon Atrelado ao Steam** | ✅ Conforme | Monitora Steam PID via `WaitForSingleObject` e desliga via `/v1/shutdown` no `DLL_PROCESS_DETACH`. |
| **Padrão de Interface Fluent Design Win11** | ⚠️ Pendente | Planejamento documentado em [`docs/PLANO_MODERNIZACAO_UI_WIN11.md`](file:///home/ubuntu/projetos/cr-clone/docs/PLANO_MODERNIZACAO_UI_WIN11.md), ainda aguardando implementação na branch de feature dedicada. |

---

## 5. Plano de Ação e Próximos Passos Sugeridos

1. **Correção Imediata (Hotfix no `develop`):**
   - Corrigir a gravação do `manifest.tsv` em [`cr-gui/src/main.rs`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-gui/src/main.rs#L1100-L1145) para usar as tags `META` e `FILE`, e adicionar fallback parser em [`vault.rs`](file:///home/ubuntu/projetos/cr-clone/rust-workspace/crates/cr-service/src/vault.rs).
   - Proteger os imports de `CommandExt` em `cr-gui` com `#[cfg(target_os = "windows")]`.
2. **Refatoração Estrutural (Próxima Feature):**
   - Unificar os módulos `gdrive.rs` e utilitários WinHTTP da GUI e do Service em uma crate compartilhada (`cr-common`).
   - Otimizar o sistema de log da bridge DLL para manter os file handles abertos.
3. **Interface Moderna:**
   - Iniciar o desenvolvimento da branch `feature/win11-modern-ui` seguindo o plano de [`docs/PLANO_MODERNIZACAO_UI_WIN11.md`](file:///home/ubuntu/projetos/cr-clone/docs/PLANO_MODERNIZACAO_UI_WIN11.md).

