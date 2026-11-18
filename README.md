# CineHub

Aplicativo de mídia multiplataforma, escrito em Rust + Tauri 2, com interface em HTML/CSS/JavaScript. O CineHub consome addons compatíveis com o protocolo do Stremio e reúne catálogos, metadados, fontes de vídeo, legendas, reprodução por torrent e integração opcional com Kodi.

> O CineHub não fornece nem hospeda conteúdo. O usuário é responsável por instalar addons e abrir fontes que tenha autorização para acessar.

## Plataformas

- **Android e Android TV:** APK único, com player nativo Media3/ExoPlayer. A interface detecta TVs Android de tela grande e ativa layout próprio para controle remoto.
- **Windows:** pacote desktop Tauri; o workflow compila o player nativo libmpv.
- **Linux e macOS:** pacotes desktop Tauri; reprodução pelo player disponível no WebView quando o player nativo não está habilitado.
- **iOS:** não é alvo de build nem de teste.

## Recursos

### Catálogos e addons
- Compatibilidade com o protocolo de addons do Stremio: `manifest`, `catalog` (incluindo busca), `meta`, `stream` e `subtitles`.
- Cinemeta e Anime Catalogs são instalados automaticamente na primeira execução.
- Abas separadas para filmes, séries e animes; a aba Animes usa os catálogos compatíveis com os tipos publicados pelo addon.
- Catálogo de addons da comunidade, com verificação de disponibilidade do manifesto e indicadores de estado.
- Instalação e gerenciamento de addons pela interface.
- Fontes de addons podem fornecer metadados, streams, faixas de áudio e legendas, conforme o que cada addon publica.

### Reprodução
- **Android/Android TV:** player nativo baseado em AndroidX Media3/ExoPlayer, com seleção manual de áudio, seleção e desativação de legendas, preferência por português, controle por D-pad e ações para pular segmentos quando disponíveis.
- **Windows:** player nativo libmpv compilado no workflow desktop.
- **Outras plataformas:** player baseado em Video.js/WebView.
- Proxy HTTP local para streams HTTP/HTTPS, com suporte a respostas em blocos, solicitações Range e reescrita de playlists HLS e seus recursos filhos.
- A interface oferece recuperação para buscas de fontes sem resultado, incluindo tentar novamente, atualizar e limpar filtros quando essas ações estão disponíveis.

### Torrents
- Motor BitTorrent local baseado em librqbit; aceita links `magnet:` e arquivos `.torrent`.
- Servidor HTTP local em loopback para reprodução progressiva e solicitações Range.
- Identifica arquivos de vídeo de torrents com múltiplos episódios e permite selecionar o arquivo/episódio sem abrir outro magnet.
- Tokens de streaming separados por seleção de episódio para evitar que requisições HTTP antigas passem a apontar para o episódio recém-selecionado.
- Estatísticas de rede incluem estado, velocidade, peers e progresso.

### Android TV e controle remoto
- Layout de sala de estar com menu lateral, cartazes maiores e alvos de foco ampliados.
- Navegação por setas/DPAD e ativação por OK.
- Navegação direcional considera alinhamento e proximidade para reduzir saltos diagonais.
- O botão Voltar fecha primeiro o teclado de busca, diálogos ou player antes de sair da tela/aplicativo.
- O player nativo ExoPlayer oferece controles acessíveis pelo controle remoto.

### Biblioteca e integrações
- Biblioteca local, favoritos e progresso de reprodução.
- Exportação e importação de dados da biblioteca.
- Integração opcional com Kodi via JSON-RPC: conecta a uma instância configurada pelo usuário, lista addons de vídeo e solicita reprodução no próprio Kodi. Veja [docs/kodi.md](docs/kodi.md).

## Arquitetura e estrutura

Para uma visão dos fluxos internos, limites e responsabilidades dos módulos, consulte [docs/architecture.md](docs/architecture.md).

```
web/
  index.html                 estrutura da interface
  app.js                     rotas, telas, catálogos, detalhes e integração com addons
  lib.js                     utilitários compartilhados
  style.css                  estilos gerais
  android-tv.css             layout para Android TV
  remote-navigation.js       teclado, DPAD, foco e botão Voltar
  accessibility.js           recursos de acessibilidade
  tv-mode.js                 detecção do modo Android TV
  torrent-playback.js        fluxo de reprodução por torrent
  kodi.js                    interface de integração Kodi
  vendor/                    bibliotecas web empacotadas
  fonts/                     fontes locais

src-tauri/
  src/
    addon.rs                 cliente Stremio, validação de destinos e cache
    commands.rs              comandos Tauri usados pela interface
    media_proxy.rs           proxy HTTP/HLS local para fontes de vídeo
    player.rs                integração desktop com libmpv
    torrent.rs               motor BitTorrent e servidor local de streaming
    kodi.rs                  cliente Kodi JSON-RPC
    store.rs                 persistência dos addons instalados
    skip.rs                  segmentos de reprodução
    diag.rs                  diagnósticos
  plugins/tauri-plugin-exoplayer/
                             player Android/Android TV baseado em Media3
  android-tv/
    patch_manifest.py        ajustes do manifesto Android para Android TV
    banner.png               banner do launcher de TV
  capabilities/              permissões Tauri

tests/
  web/                       testes de fumaça Playwright
src-tauri/tests/
  addon_integration.rs       testes de integração do cliente de addons

docs/
  kodi.md                    configuração da integração Kodi
  architecture.md            visão técnica dos fluxos e limites
.github/
  workflows/build.yml        testes, lint e builds desktop/Android
  scripts/                   utilitários de build
```

## Segurança

- O cliente de addons e o proxy de mídia aceitam somente URLs HTTP/HTTPS com host válido e sem credenciais embutidas.
- Destinos privados, loopback, locais e reservados são bloqueados no acesso a addons e pelo proxy de mídia; a resolução DNS usada na conexão também filtra IPs não públicos.
- Redirecionamentos de addons e proxy são limitados e revalidados.
- O proxy de mídia escuta somente em `127.0.0.1`, usa tokens aleatórios e limita o tempo de vida e a quantidade de sessões.
- O proxy encaminha somente cabeçalhos selecionados de `proxyHeaders.request`: `authorization`, `cookie`, `origin`, `referer` e `user-agent`.
- O proxy suporta HTTP progressivo e HLS. **DASH (.mpd) não está habilitado.**
- O acesso a endereços privados é uma exceção separada e explícita na integração Kodi, que exige o endereço/IP informado pelo usuário e não faz descoberta da rede.
- Respostas de addons têm limite de 8 MiB; legendas têm limite de 2 MiB. O cache de catálogo/metadados/fontes tem TTL de 10 minutos e limites de entradas/tamanho.
- O armazenamento local de addons preserva dados corrompidos para diagnóstico em vez de sobrescrevê-los silenciosamente.

## Testes

Testes Rust:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Teste de fumaça web:

```bash
cd tests/web
npm ci
npx playwright install --with-deps chromium
npm test
```

Verificações de lint usadas no CI:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
node --check web/app.js
node --check tests/web/smoke.mjs
```

Os testes de integração Rust usam um addon falso e validam rotas do protocolo, cache, limites de resposta e bloqueio de destinos de rede não permitidos. O Playwright abre as rotas da interface em Chromium com backend simulado.

## Desenvolvimento

### Requisitos gerais
- Rust estável via rustup.
- Tauri CLI 2.
- Node.js 20 para os testes web.

Instale o CLI:

```bash
cargo install tauri-cli --version "^2" --locked
```

### Linux
Dependências do sistema usadas no CI:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev
```

### Windows
WebView2 (normalmente presente nas versões atuais do Windows). O build do player nativo exige a libmpv; o script `.github/scripts/prepare-libmpv.ps1` prepara a DLL usada pelo workflow.

### macOS
Xcode Command Line Tools.

### Executar desktop

```bash
cd src-tauri
cargo tauri dev
cargo tauri build
```

### Android e Android TV
Requer Java 17, Android SDK/NDK e as variáveis `ANDROID_HOME` e `NDK_HOME`.

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
cd src-tauri
cargo tauri android init
python3 android-tv/patch_manifest.py gen/android/app/src/main/AndroidManifest.xml
cargo tauri android dev
cargo tauri android build --apk
```

O patch de manifesto adiciona o launcher Leanback, declara toque opcional e configura o banner da TV. O CI executa esse patch antes de gerar os APKs.

## Builds automáticos e releases

O workflow [build.yml](.github/workflows/build.yml) roda em pushes para `main` (exceto alterações apenas em arquivos Markdown), pull requests e execução manual.

- **Testes:** suíte Rust.
- **Lint:** rustfmt, Clippy e verificação de sintaxe JavaScript; auditorias adicionais de licenças e vulnerabilidades são executadas como etapas não bloqueantes.
- **Web:** teste de fumaça Playwright.
- **Desktop:** Linux, Windows e macOS.
- **Android:** APKs separados por ABI para arm64-v8a, armeabi-v7a, x86 e x86_64.
- **Release:** publica os artefatos quando testes, lint, smoke web e pelo menos um build de plataforma passam; mantém as 10 releases automáticas mais recentes.

Os artefatos de cada execução também ficam disponíveis na aba **Actions** do GitHub.

### Assinatura Android
Se os segredos de assinatura não estiverem configurados, o CI usa uma chave temporária. O APK instala, mas builds seguintes terão assinatura diferente e exigirão desinstalar a versão anterior. Para atualizações no local, configure em **Settings → Secrets and variables → Actions**:

- `ANDROID_KEYSTORE_BASE64`
- `ANDROID_KEYSTORE_PASSWORD`
- `ANDROID_KEY_ALIAS`
- `ANDROID_KEY_PASSWORD`

Guarde o keystore original em local seguro: perder a chave impede assinar atualizações compatíveis com instalações anteriores.

## Limitações conhecidas

- A reprodução depende dos formatos, codecs, cabeçalhos e disponibilidade fornecidos pela fonte e do suporte do player/dispositivo.
- O proxy não oferece suporte a DASH (.mpd).
- O Android pode bloquear fontes HTTP sem criptografia conforme a configuração de segurança do sistema.
- A busca e paginação de catálogos dependem de cada addon implementar os parâmetros correspondentes.
- O catálogo comunitário verifica manifestos e disponibilidade, mas isso não garante que todos os streams de um addon funcionarão nem constitui auditoria de segurança do código/serviço do addon.
- A biblioteca e o progresso ficam no aparelho. No Android, os dados exportados ficam inicialmente no diretório privado do app; transferi-los para outro aparelho exige compartilhar o arquivo exportado por um meio externo.
- A compatibilidade de formatos e codecs varia entre Android, WebView, libmpv e sistema operacional.

## Licença

GPL-3.0-or-later. Consulte [LICENSE](LICENSE) e [NOTICE.md](NOTICE.md) para avisos de direitos autorais, código anterior e fontes tipográficas.
