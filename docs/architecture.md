# Arquitetura do CineHub

Este documento descreve os principais componentes e o caminho dos dados na versão atual do CineHub. A implementação executável é a fonte definitiva; detalhes de protocolo e limites podem mudar junto com o código.

## Visão geral

O aplicativo é dividido em três camadas:

1. **Interface web** em `web/`: renderiza páginas, gerencia navegação, foco, catálogos e interações com o usuário.
2. **Backend Rust/Tauri** em `src-tauri/src/`: valida e consulta addons, persiste configurações, cria o proxy de mídia e controla torrents e integrações.
3. **Players nativos**: Media3/ExoPlayer no Android; libmpv no build Windows; Video.js/WebView como caminho web/fallback nas demais configurações.

A interface chama comandos Tauri. URLs de vídeo são entregues ao player nativo ou ao player web; quando necessário, o backend cria uma URL local do proxy para adaptar a conexão com a fonte.

## Addons Stremio

O módulo `addon.rs` implementa o cliente HTTP do protocolo de addons:

- Lê e valida `manifest.json`.
- Descobre recursos, tipos, prefixos de IDs e definições de catálogo.
- Consulta catálogos, metadados, streams e legendas.
- Suporta busca de catálogo quando o addon declara o parâmetro `search`.
- Usa cache limitado por TTL, número de entradas e tamanho.
- Limita respostas e redirecionamentos e bloqueia destinos de rede privados/reservados na resolução e conexão.

O estado dos addons instalados é persistido por `store.rs`. Cinemeta e Anime Catalogs são pré-instalados pela aplicação. O catálogo comunitário apresenta addons e consulta a disponibilidade de seus manifestos; esse teste não certifica a segurança nem a reprodução de cada fonte.

## Proxy de mídia

`media_proxy.rs` cria um servidor HTTP em loopback com porta efêmera e tokens aleatórios por sessão.

Fluxo simplificado:

1. A interface recebe uma URL HTTP/HTTPS de uma fonte.
2. O backend valida esquema, host e destino público; valida os cabeçalhos explicitamente permitidos.
3. Registra uma sessão com expiração e retorna uma URL local com token imprevisível.
4. O player solicita o conteúdo ao proxy.
5. Para mídia progressiva, o proxy encaminha a resposta e os cabeçalhos de Range necessários.
6. Para HLS, reescreve a playlist e os recursos filhos para passarem pelo mesmo proxy.

O proxy limita redirecionamentos e sessões. Não aceita URLs DASH com extensão `.mpd`. Os cabeçalhos encaminhados são limitados a `authorization`, `cookie`, `origin`, `referer` e `user-agent`, quando fornecidos em `proxyHeaders.request`.

O proxy de mídia é distinto do servidor de torrent. O proxy de addons/mídia bloqueia destinos privados; a reprodução local de torrent e a conexão Kodi são fluxos específicos, com regras próprias.

## Torrents

`torrent.rs` usa librqbit para abrir magnet links ou arquivos `.torrent`. O backend:

- Mantém a sessão BitTorrent e seleciona arquivos de vídeo.
- Identifica múltiplos arquivos de vídeo para seleção de episódio/arquivo.
- Expõe um servidor HTTP em loopback, com tokens aleatórios e suporte a HEAD/Range.
- Retorna estatísticas de peers, velocidade e progresso.
- Mantém o torrent ativo ao trocar o arquivo selecionado.

Cada seleção de episódio gera um token de streaming novo. O token anterior é removido do mapa de streams, evitando que novas requisições feitas com uma URL antiga sejam interpretadas como requisições para o episódio atual. A interface atualiza o token usado para reprodução e encerramento.

O servidor de torrent não é o proxy de mídia de addons: os dois servidores têm finalidades e políticas separadas.

## Players

### Android e Android TV

O plugin `tauri-plugin-exoplayer` usa AndroidX Media3/ExoPlayer em tela cheia. O player configura tempos de conexão/leitura maiores para fontes com buffering demorado, prefere áudio e legendas em português e oferece seleção manual de áudio e legendas, incluindo a opção de desativá-las. Também recebe segmentos opcionais para pular abertura, recapitulação, prévia e créditos.

### Desktop

O módulo `player.rs` integra libmpv quando a feature `mpv` é habilitada. O workflow Windows prepara a biblioteca e compila essa feature. Sem a feature nativa, a interface usa o player web.

### WebView

Video.js é empacotado em `web/vendor/`, evitando depender de baixar a biblioteca em tempo de execução. A compatibilidade real de contêineres e codecs depende do WebView e do sistema operacional.

## Android TV e foco

Os arquivos `tv-mode.js` e `android-tv.css` ativam a apresentação de sala de estar em dispositivos Android identificados como TVs ou telas grandes sem toque. `remote-navigation.js` interpreta setas, códigos DPAD, OK e Voltar, controla o foco e limita a navegação em diálogos modais.

A navegação direcional escolhe candidatos pela geometria dos elementos, priorizando alinhamento no eixo perpendicular ao movimento. Listas verticais específicas, como a seleção de arquivos de torrent, usam navegação por índice para evitar saltos.

O plugin Android encaminha o botão Voltar à função `window.__cinehubBack` antes de permitir que o sistema saia da tela. Assim, a interface pode fechar teclado, diálogo ou player na ordem apropriada.

## Kodi

A integração em `kodi.rs` e `web/kodi.js` usa JSON-RPC HTTP para conectar a uma instância informada pelo usuário. O CineHub lista addons de vídeo e navega diretórios `plugin://`, mas a execução do addon e a reprodução permanecem no próprio Kodi. Consulte [kodi.md](kodi.md).

## Testes e automação

- `src-tauri/tests/addon_integration.rs`: testes de integração com um addon HTTP falso.
- `tests/web/`: testes de fumaça da interface com Playwright/Chromium.
- `.github/workflows/build.yml`: testes Rust, lint, smoke web, builds desktop e Android e publicação de releases.

O CI não substitui testes em dispositivos físicos. Especialmente em Android TV, controle remoto, codecs, rede e desempenho precisam ser validados no aparelho.

## Limites deliberados

- O CineHub não hospeda catálogo ou vídeo próprio.
- O catálogo comunitário não é garantia de funcionamento ou confiabilidade de addons.
- O proxy de mídia não oferece DASH.
- Addons e proxy não podem acessar destinos privados/locais; Kodi e o servidor de torrent têm fluxos separados.
- Formatos e codecs disponíveis dependem do player e do dispositivo.
