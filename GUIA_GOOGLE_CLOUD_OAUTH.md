# Guia Passo a Passo: Como Criar o App OAuth2 no Google Cloud Console (Google Drive API)

Este guia ensina como criar as suas próprias credenciais (**Client ID** e **Client Secret**) no Google Cloud Console para sincronizar os seus saves diretamente na sua conta do Google Drive via API, sem limites de terceiros e de forma 100% gratuita.

---

## Passo 1: Criar um Novo Projeto no Google Cloud

1. Acesse o [Google Cloud Console](https://console.cloud.google.com/) e faça login com a sua conta Google.
2. No topo da página (ao lado do logo "Google Cloud"), clique no seletor de projetos e depois em **"Novo Projeto"** (*New Project*).
3. Preencha os dados:
   - **Nome do projeto:** `ChronoKeep`
   - **Organização:** Deixe como *"Sem organização"* (*No organization*).
4. Clique em **"Criar"** (*Create*) e aguarde alguns segundos.
5. Certifique-se de que o projeto recém-criado está selecionado no topo da página.

---

## Passo 2: Ativar a API do Google Drive

1. No menu lateral esquerdo, vá em **APIs e Serviços** (*APIs & Services*) -> **Biblioteca** (*Library*).
   - Link direto: [https://console.cloud.google.com/apis/library](https://console.cloud.google.com/apis/library)
2. Na barra de pesquisa, digite **`Google Drive API`**.
3. Clique no resultado **Google Drive API** e depois clique no botão azul **"Ativar"** (*Enable*).

---

## Passo 3: Configurar a Tela de Permissão OAuth (OAuth Consent Screen)

Antes de gerar as chaves, o Google exige configurar a tela que aparece quando você faz login.

1. No menu lateral esquerdo, vá em **APIs e Serviços** -> **Tela de permissão OAuth** (*OAuth consent screen*) — ou no novo layout **Google Auth Platform** -> **Visão geral / Público**.
2. Escolha o Tipo de Usuário (**User Type**):
   - Selecione **Externo** (*External*) e clique em **"Criar"** (*Create*).
3. **Informações do App:**
   - **Nome do app:** Digite `ChronoKeep`.
   - **E-mail de suporte do usuário:** Selecione o seu próprio e-mail.
   - **Dados de contato do desenvolvedor (no final da página):** Digite o seu próprio e-mail novamente.
   - Clique em **"Salvar e continuar"**.
4. **Escopos (*Scopes*):**
   - Clique em **"Adicionar ou remover escopos"** (*Add or remove scopes*).
   - Na lista ou no filtro, marque o escopo:
     - `.../auth/drive.file` (*Ver, editar, criar e excluir apenas os arquivos específicos do Google Drive que você usar com este app*) — **Recomendado**, pois é um escopo não-sensível na maioria dos casos e garante que o app só acesse a pasta de saves que ele mesmo criar.
     - *(Opcional)* `.../auth/drive.appdata` (para pasta oculta de configurações do aplicativo).
   - Clique em **"Atualizar"** (*Update*) e depois em **"Salvar e continuar"**.
5. **Usuários de Teste (*Test users*) — ⚠️ MUITO IMPORTANTE:**
   - Enquanto o status de publicação for *"Em teste"* (*Testing*), você não precisa mandar o app para verificação da Google, **desde que o seu e-mail esteja listado como Usuário de Teste**!
   - Clique em **"+ Add Users"** (*Adicionar usuários*).
   - Digite o(s) seu(s) e-mail(s) do Gmail que você vai usar para salvar os jogos na nuvem e clique em **"Adicionar"**.
   - Clique em **"Salvar e continuar"**.

> [!TIP]
> **Dica:** Para que o token de atualização (*Refresh Token*) **nunca expire após 7 dias**, depois que criar tudo, você pode voltar na aba **Tela de permissão OAuth (Público)** e clicar em **"Publicar aplicativo"** (*Publish App*). Mesmo que apareça um aviso dizendo que requer verificação, para uso pessoal você pode clicar em *"Avançar -> Ir para <NomeDoApp> (não seguro)"* uma única vez no navegador e o seu token ficará permanente!

---

## Passo 4: Gerar o `Client ID` e o `Client Secret`

1. No menu lateral esquerdo, vá em **APIs e Serviços** -> **Credenciais** (*Credentials*).
2. No topo, clique em **"+ Criar Credenciais"** (*+ Create Credentials*) -> **ID do cliente OAuth** (*OAuth client ID*).
3. Em **Tipo de aplicativo** (*Application type*), selecione:
   - **App para computador** (*Desktop app*).
4. Em **Nome**, digite: `Desktop Client`.
5. Clique em **"Criar"** (*Create*).
6. Uma janela vai aparecer na tela mostrando:
   - **ID do cliente** (*Client ID*): algo terminado em `.apps.googleusercontent.com`
   - **Chave secreta do cliente** (*Client secret*): começa geralmente com `GOCSPX-...`
7. Clique em **"Fazer o download do JSON"** (ou copie os dois valores) e cole na aba de configuração da nossa GUI!

---

## Problemas Comuns (Troubleshooting)

- **Erro `403: access_denied` ("O app não concluiu o processo de verificação do Google"):**
  - **Causa:** O e-mail com o qual você tentou logar não foi adicionado na lista de **Usuários de Teste** (*Test users*).
  - **Solução:** Volte em *Tela de permissão OAuth -> Público (Audience) -> Test users* e adicione o seu Gmail.
- **Aviso "O Google ainda não verificou este app" com botão "Voltar à segurança":**
  - **Solução:** Clique em **"Avançado"** (*Advanced*) na parte inferior esquerda do card e depois em **"Acessar <Nome do App> (não seguro)"**. Isso aparece apenas porque o projeto foi criado por você mesmo na sua conta pessoal.

