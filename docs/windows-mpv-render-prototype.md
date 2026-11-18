# Protótipo de player nativo Windows (libmpv + D3D11)

## Objetivo

Manter a interface do CineHub em WebView2 e investigar a reprodução por libmpv com
decodificação/legendas próprias, sem a janela filha `wid` que hoje cobre a WebView.

## Etapa 1 — probe de swapchain

O exemplo `src-tauri/examples/windows_mpv_composition_probe.rs` inicia o libmpv
com `vo=gpu-next`, `gpu-api=d3d11` e
`d3d11-output-mode=composition`, carrega uma URL HTTP(S) e consulta a propriedade
`display-swapchain`.

Em um Windows com libmpv compilado com suporte D3D11:

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --features mpv --example windows_mpv_composition_probe -- "https://servidor/video.mkv"
```

O exemplo é opt-in: não ativa libmpv no build normal do Windows, não muda o player
usado pelo aplicativo e não afeta Android, macOS ou Linux.

## Limite conhecido desta etapa

Obter o handle da swapchain **não** significa que o vídeo já está composto sobre
a WebView. O modo composition do mpv não cria uma janela, mas o CineHub ainda
precisa de um consumidor nativo da swapchain e de uma superfície visual integrada
à janela Tauri. O WebView2 usado pelo Wry não expõe, pela API Tauri padrão, um
visual de composição que possa receber diretamente essa swapchain.

Portanto, este probe não esconde a WebView, não altera `player_play` e não é
selecionado pelo player em produção. Se o handle for obtido, a próxima etapa é
um protótipo Win32/DirectComposition independente que apresente a swapchain em
uma superfície filha, validando redimensionamento, DPI, fullscreen, foco e
ordem Z. Sobrepor controles HTML transparentes a essa superfície ainda precisa
ser validado; se não for possível com o WebView2 do Wry, será necessário avaliar
WebView2 CompositionController/um plugin Tauri específico ou manter os controles
nativos.

## Critérios antes de ativar em produção

- Build Windows com libmpv/D3D11 distribuído junto ao aplicativo.
- Vídeo visível sem janela externa e sem ocultar a WebView.
- Controles, menus e legendas CineHub visíveis sobre o vídeo.
- Redimensionamento, DPI misto, minimizar/restaurar e fullscreen estáveis.
- Áudio, seek, retomada, legendas e encerramento sem processos/handles órfãos.
- Fallback automático para Video.js/WebView2 se a inicialização nativa falhar.

## Referências técnicas

- [mpv render API](https://github.com/mpv-player/mpv/blob/master/include/mpv/render.h)
- [mpv manual — D3D11 output mode](https://mpv.io/manual/stable/)
- [libmpv2 Rust API](https://docs.rs/libmpv2/7.0.1/libmpv2/)


## Etapa 2 — apresentação via DirectComposition

O exemplo `src-tauri/examples/windows_mpv_directcomposition_probe.rs` vai além
do probe anterior: obtém o `IDXGIDevice` a partir da swapchain do mpv, cria
`IDCompositionDevice`, associa um visual à janela indicada por HWND e define a
swapchain como conteúdo desse visual.

Execute no Windows, com libmpv/D3D11 disponível:

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --features mpv --example windows_mpv_directcomposition_probe -- "https://servidor/video.mkv" 0x123456
```

O segundo argumento é o HWND da janela principal do CineHub, em hexadecimal
(ou decimal). Obtenha-o com Spy++ ou outra ferramenta de inspeção de janelas.
O protótipo usa um destino DirectComposition na camada superior (`topmost=true`):
o vídeo pode cobrir a WebView inteira. Isso é intencional para validar que a
swapchain é apresentada no HWND correto; não representa ainda o layout final
nem oferece controles HTML sobre o vídeo. A saída também fixa
`d3d11-composition-size=1280x720`; redimensionamento dinâmico ainda não foi
implementado.

A etapa só pode ser considerada validada depois de executar em Windows com
libmpv compilado com D3D11 e confirmar visualmente a imagem dentro da janela
informada. O CI padrão não compila esses exemplos opt-in com a feature `mpv`.

### Limites e próximo critério

- Confirmar que o HWND informado pertence ao processo do CineHub e que o vídeo
  aparece dentro dele, sem janela externa.
- Testar se o destino topmost pode ser substituído por uma composição que permita
  controles HTML clicáveis sobre o vídeo. O destino topmost tende a ficar acima
  dos filhos da janela, portanto não se deve presumir que os controles WebView2
  serão desenhados por cima.
- Implementar atualização de tamanho/posição, DPI, fullscreen, minimizar/restaurar
  e limpeza/recriação da árvore antes de qualquer ativação em produção.
- Se a camada DirectComposition cobrir a WebView2 de forma inevitável, avaliar
  WebView2 CompositionController ou controles nativos em vez de esconder isso
  atrás de uma falsa integração HTML.
