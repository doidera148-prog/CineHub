# CineHub

Aplicativo de streaming compatível com os addons do Stremio, para Windows, macOS, Linux, Android e Android TV (iOS também é possível com o Tauri, sem testes ainda).

- **Backend:** Rust (Tauri 2). Fala com os addons pelo protocolo oficial do Stremio: `manifest`, `catalog` (com busca), `meta` e `stream`.
- **Interface:** HTML, CSS e JavaScript, na pasta `web/`. Roda no WebView nativo de cada sistema.
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

Limitação conhecida: o botão Voltar do controle ainda não fecha o player; ele apenas volta na navegação do WebView.

## Builds na nuvem

O workflow `.github/workflows/build.yml` gera, a cada push na `main`:

- Windows (`.msi` / `.exe`), macOS (`.dmg` / `.app`), Linux (`.deb` / `.AppImage`)
- Android (`.apk`), que serve para celular e Android TV

Os arquivos aparecem como artefatos na aba **Actions** do GitHub.

## Limitações conhecidas

- **Formatos de vídeo:** o player é o do próprio sistema (WebView). MP4 e WebM funcionam em todas as plataformas; MKV depende do codec e pode não tocar no Android ou no macOS.
- **HTTP sem criptografia:** o Android bloqueia `http://` por padrão. Fontes de addon com `http://` podem não tocar no celular.
- **Legendas:** só português (SRT ou VTT), baixadas pelo backend e convertidas para VTT.
- **Fontes em torrent ou magnet:** não são reproduzidas por este app. Aparecem na lista marcadas como não suportadas.

## Licença

MIT
