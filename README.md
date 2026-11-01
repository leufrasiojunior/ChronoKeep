# ChronoKeep

**ChronoKeep** é um sistema de redirecionamento, preservação e backup automático de Saves da Steam Cloud para Windows x64.
Construído 100% em **Rust** (arquitetura *clean-room*, código limpo e seguro), substitui soluções legadas trazendo estabilidade total, suporte a arquivos compactados (ZIP RFC 1951), timestamps legíveis e integração nativa com o Google Drive.

---

## 🚀 Como Usar (Instalação Rápida para Usuário Comum)

Não é necessário compilar nada nem usar scripts complexos. O executável **`ChronoKeep.exe`** é **100% autocontido** (todos os binários e DLLs necessários estão embutidos dentro dele).

1. Acesse a aba **[Releases](../../releases)** deste repositório e baixe o arquivo **`ChronoKeep.exe`**.
2. Execute o **`ChronoKeep.exe`**.
3. O instalador detectará sua pasta da Steam automaticamente:
   - Se o seu **OpenSteamTool** estiver com o recurso cloud desativado (`opensteamtool.toml`), o ChronoKeep avisará na hora e oferecerá para ativar com 1 clique.
4. Clique no botão **`Instalar na Steam (1-Clique)`**:
   - Ele implantará automaticamente a DLL principal (`chronokeep.dll`), a ponte (`cloud_redirect.dll`), o daemon em segundo plano (`chrono-daemon.exe`) e iniciará o serviço.
5. *(Opcional)* Clique em **`Conectar Conta Google Drive`** para fazer login com 1 clique no navegador e salvar uma cópia na nuvem.
6. Pronto! Abra sua Steam e jogue normalmente. Seus saves estarão sempre sincronizados e seguros em `%USERPROFILE%\Saved Games\ChronoKeepVault`.

> 💡 **Dica (Instalação Silenciosa)**: Você também pode rodar `ChronoKeep.exe /silent` ou clicar no botão **`Instalar Silencioso`** na interface para implantar tudo instantaneamente sem confirmações adicionais.

---

## 🏛️ Arquitetura

1. **`ChronoKeep.exe` (Aplicativo e Painel All-In-One — `crates/cr-gui`)**
   - Executável nativo Win32 construído em Tauri v2 com interface Fluent Design.
   - Contém a DLL e o Daemon embutidos via `include_bytes!`. Funciona mesmo executado isolado em um computador sem outros arquivos.
   - Suporte a modo silencioso via `/silent`, `/S` ou `/install`, e modo de desinstalação via `--uninstall`.
   - Gerenciador de conexão OAuth2 para Google Drive.

2. **`chronokeep.dll` / `cloud_redirect.dll` (Bridge DLL — `crates/cr-bridge-dll`)**
   - Injetada no processo `steam.exe` (via OpenSteamTool ou proxy DLL).
   - Hooka a vtable de `steamclient64.dll` e intercepta as chamadas de Cloud RPC dos jogos gerenciados.
   - Logs com data e hora local legível (`[YYYY-MM-DD HH:MM:SS]`).

3. **`chrono-daemon.exe` (Local Cloud Save Daemon — `crates/cr-service`)**
   - Servidor HTTP local em `127.0.0.1:39771`.
   - Suporte ao protocolo HTTP/1.1 completo (incluindo `Expect: 100-continue` para uploads grandes da Steam).
   - Descompressão automática de pacotes ZIP `"z"` da Steam e salvamento no Vault.
   - Sincronização em tempo real com o Google Drive via API REST oficial.

---

## 🛠️ Compilação Manual

### No Linux / WSL (Cross-Compile para Windows x64 MSVC):
```bash
# 1. Compilar o Daemon e a DLL
cargo build --manifest-path rust-workspace/Cargo.toml --target x86_64-pc-windows-msvc --release -p cr-service -p cr-bridge-dll

# 2. Atualizar os binários na pasta embutida do instalador
mkdir -p rust-workspace/crates/cr-gui/embedded
cp rust-workspace/target/x86_64-pc-windows-msvc/release/chrono-daemon.exe rust-workspace/crates/cr-gui/embedded/
cp rust-workspace/target/x86_64-pc-windows-msvc/release/chronokeep.dll rust-workspace/crates/cr-gui/embedded/

# 3. Compilar o Instalador GUI Autocontido
cargo build --manifest-path rust-workspace/Cargo.toml --target x86_64-pc-windows-msvc --release -p cr-gui
```

### No Windows:
Abra o PowerShell na pasta raiz e execute:
```powershell
cargo build --manifest-path rust-workspace/Cargo.toml --release
```

Ou deixe o **GitHub Actions** compilar tudo automaticamente e gerar a Release pronta para você!
