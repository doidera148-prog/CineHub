# CineHub

Aplicativo de streaming compatível com os addons do Stremio, para Windows, macOS, Linux, Android e Android TV (iOS também é possível com o Tauri, sem testes ainda).

- **Backend:** Rust (Tauri 2). Fala com os addons pelo protocolo oficial do Stremio: `manifest`, `catalog` (com busca), `meta` e `stream`.
- **Interface:** HTML, CSS e JavaScript, na pasta `web/`. Roda no WebView nativo de cada sistema.
- **Player:** [Video.js](https://videojs.com/) (código aberto, MIT), com suporte a HLS (`.m3u8`) embutido. Empacotado em `web/vendor/`, funciona sem internet.
- **Metadados em pt-BR:** o Cinemeta (addon oficial do Stremio) é instalado automaticamente na primeira execução.

O app não traz fontes de conteúdo. Você instala os addons que tem autorização para usar, pela aba **Addons**.

## Estrutura

```
web/                 interface (index.html, app.js, style.css)
src-tauri/
  tauri.conf.json    configuração da janela e dos pacotes
  src/lib.rs         inicialização e registro dos comandos
  src/commands.rs    comandos chamados pela interface (catálogos, metas, fontes, legendas)
  src/addon.rs       cliente do protocolo de addons do Stremio
  src/store.rs       lista de addons instalados (JSON)
.github/workflows/   builds automáticos para desktop e Android
```

## Segurança e desempenho

- Respostas de addons são limitadas a 8 MiB, e legendas a 2 MiB.
- O comando de legenda só aceita URLs públicas. Endereços locais e privados (localhost, 10.x, 192.168.x, 169.254.x etc.) são bloqueados.
- Catálogos, metadados e fontes ficam em cache por 10 minutos.
- Metadados são consultados em paralelo em todos os addons.

## Testes

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Os testes de integração (`src-tauri/tests/addon_integration.rs`) sobem um addon falso em localhost e validam manifest, catálogo, meta, fontes, legendas, cache e os limites de tamanho.

## Desenvolver no computador

Pré-requisitos:

- Rust estável (`rustup`)
- Linux: `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev`
- Windows: WebView2 (já vem no Windows 10/11)
- macOS: Xcode Command Line Tools

```bash
cargo install tauri-cli --version "^2" --locked
cd src-tauri
cargo tauri dev        # abre a janela em modo de desenvolvimento
cargo tauri build      # gera o instalador da sua plataforma
```

Testes do backend:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Teste de fumaça da interface (abre cada rota num Chromium, em celular e desktop, com o backend simulado):

```bash
cd tests/web
npm ci
npx playwright install --with-deps chromium
npm test
```

## Android

Requer Android Studio (ou SDK + NDK), Java 17 e as variáveis `ANDROID_HOME` e `NDK_HOME`.

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cd src-tauri
cargo tauri android init
cargo tauri android dev          # roda no emulador ou celular conectado
cargo tauri android build --apk  # gera o APK
```

### Android TV

O mesmo APK roda no celular e na TV. O script `src-tauri/android-tv/patch_manifest.py` ajusta o manifesto gerado pelo `cargo tauri android init` para o Android TV:

- adiciona o `LEANBACK_LAUNCHER`, para o app aparecer na tela inicial da TV;
- declara que o toque é opcional, para o app abrir sem tela sensível ao toque;
- define o banner do launcher (`android-tv/banner.png`).

Depois de inicializar o projeto Android, rode:

```bash
cd src-tauri
python3 android-tv/patch_manifest.py gen/android/app/src/main/AndroidManifest.xml
cargo tauri android build --apk
```

Na TV, a navegação é feita pelo controle remoto: as setas movem o foco e OK ativa o item. Para fechar o player, foque o botão ✕ e pressione OK.

No Android, o botão Voltar fecha o player antes de sair do app. Na TV, o foco inicial vai para o conteúdo ao trocar de seção pelo menu.

## Builds na nuvem

O workflow `.github/workflows/build.yml` gera, a cada push na `main`:

- Windows (`.msi` / `.exe`), macOS (`.dmg` / `.app`), Linux (`.deb` / `.AppImage`)
- Android (`.apk`), que serve para celular e Android TV

Os arquivos aparecem como artefatos na aba **Actions** do GitHub e, quando todos os testes passam, são publicados em **Releases** (as 10 publicações mais recentes são mantidas). O APK traz as arquiteturas arm64, armv7, x86 e x86_64. Antes de compilar, o workflow verifica formatação, Clippy (com avisos tratados como erro) e vulnerabilidades nas dependências.

### Assinatura do APK

Android só instala APK assinado. Sem os segredos abaixo, o CI assina com uma chave temporária: o APK instala, mas cada build tem uma assinatura diferente, então é preciso desinstalar a versão anterior antes de instalar a nova.

Para ter atualizações normais, crie uma chave uma única vez e guarde-a com segurança (perdê-la impede atualizar o app):

```bash
keytool -genkeypair -v -keystore cinehub-release.jks -alias cinehub \
  -keyalg RSA -keysize 2048 -validity 10000
base64 -w0 cinehub-release.jks > cinehub-release.b64
```

Depois, em **Settings → Secrets and variables → Actions**, crie os segredos:

- `ANDROID_KEYSTORE_BASE64`: conteúdo de `cinehub-release.b64`
- `ANDROID_KEYSTORE_PASSWORD`: senha do keystore
- `ANDROID_KEY_ALIAS`: `cinehub`
- `ANDROID_KEY_PASSWORD`: senha da chave

### Instalar no celular

1. Baixe da release mais recente o APK da sua arquitetura. A maioria dos celulares atuais usa `arm64-v8a` (é o caso do Moto G8 Power Lite); `armeabi-v7a` é para aparelhos mais antigos de 32 bits.
2. Permita a instalação de fontes desconhecidas para o navegador ou gerenciador de arquivos usado.
3. Se já houver uma versão instalada com outra assinatura, desinstale-a primeiro.

## Limitações conhecidas

- **Formatos de vídeo:** o player é o do próprio sistema (WebView). MP4 e WebM funcionam em todas as plataformas; MKV depende do codec e pode não tocar no Android ou no macOS.
- **HTTP sem criptografia:** o Android bloqueia `http://` por padrão. Fontes de addon com `http://` podem não tocar no celular.
- **Legendas:** só português (SRT ou VTT), baixadas pelo backend e convertidas para VTT. O backend só baixa de endereços públicos e recusa redirecionamentos para a rede interna.
- **Catálogos:** o botão "Carregar mais" depende de o addon aceitar o parâmetro `skip`. Addons que ignoram esse parâmetro simplesmente não mostram novos itens.
- **Validação de DNS:** a checagem de URL bloqueia endereços privados escritos diretamente, mas não impede um nome de domínio que resolva para uma rede interna.
- **Torrent e magnet:** o app inclui um motor BitTorrent local (librqbit) que reproduz fontes `magnet:` e arquivos `.torrent`, servidos por um endereço local (`127.0.0.1`). Quem usa é responsável pelo conteúdo que abre; a legalidade depende do conteúdo e do país.
- **Dados da biblioteca:** biblioteca e progresso ficam no aparelho. Use **Exportar dados** na aba Biblioteca para levá-los a outro aparelho; o arquivo tem os mesmos dados para importar depois.

## Licença

MIT
