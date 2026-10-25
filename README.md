# ChronoKeep

**ChronoKeep** é um sistema de redirecionamento e proteção de Cloud Saves para Steam em Windows x64, escrito 100% em **Rust** (`no external dependencies` / clean-room architecture), dividido em uma **Bridge DLL ultraleve (`chronokeep.dll`)**, um **Daemon Local (`chrono-daemon.exe`)** e um **Instalador/Configurador GUI nativo (`ChronoKeepSetup.exe`)**.

---

## Arquitetura

1. **`chronokeep.dll` (Bridge DLL — `crates/cr-bridge-dll`)**
   - Injetada no processo `steam.exe` (autônoma via `DllMain` ou carregada por proxy loaders).
   - Exporta a API nativa **`CK_*`** (`CK_AttachRuntime`, `CK_InstallTransportHooks`, `CK_DispatchCloudRpc`, `CK_RegisterAppList`, `CK_RegisterApp`, `CK_UnregisterApp`, `CK_QueryAppRegistered`, `CK_BindAccount`, `CK_NotifyGameState`, `CK_DetachRuntime`) além de pontes de compatibilidade para loaders existentes.
   - Localiza `CClientUnifiedServiceTransport` no `steamclient64.dll` via RTTI x64 (`.?AVCClientUnifiedServiceTransport@@`), instala hooks nos Slots `4`, `5`, `7` e `8` da vtable e encaminha apenas os AppIDs gerenciados para o `chrono-daemon.exe`.

2. **`chrono-daemon.exe` (Local Cloud Save Daemon — `crates/cr-service`)**
   - Escuta localmente em `127.0.0.1:39771` (configurável via `chronokeep.ini`).
   - Implementa todos os 12 métodos RPC `Cloud.*` da Steam e os endpoints HTTP `PUT /upload/...` e `GET /download/...`.
   - Descomprime automaticamente pacotes ZIP `"z"` da Steam (RFC 1951 `INFLATE`), mantendo os saves puros e legíveis na pasta do Vault (`%USERPROFILE%\Saved Games\ChronoKeepVault` ou Google Drive / OneDrive).

3. **`ChronoKeepSetup.exe` (Native Win32 GUI — `crates/cr-gui`)**
   - Interface gráfica nativa Win32 para instalar `chronokeep.dll` e `chrono-daemon.exe` na pasta da Steam, configurar o diretório do Vault, a porta local e as credenciais Google Drive OAuth2.

---

## Compilação (WSL / Linux -> Windows x64 MSVC)

```bash
cargo build --manifest-path rust-workspace/Cargo.toml --target x86_64-pc-windows-msvc --release
```

