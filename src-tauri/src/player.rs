//! Player nativo de desktop (libmpv).
//!
//! Abre o vídeo numa janela do mpv (com o decodificador e os codecs do próprio mpv) e responde
//! quando a janela fecha, com a posição final, para a interface salvar o progresso.
//! Só funciona quando o app é compilado com a feature `mpv` (que exige libmpv instalada).
//! Sem a feature, o comando responde com erro e a interface usa o player do navegador.

use serde::{Deserialize, Serialize};

use crate::skip::Segment;

/// Área retangular da interface web que deve permanecer visível sobre o vídeo.
/// Coordenadas relativas à caixa do player, em pixels físicos do Windows.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerCutout {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[cfg(any(all(windows, feature = "mpv"), test))]
/// Limites e recortes da região Win32 em coordenadas locais da janela filha.
/// SetWindowRgn interpreta (0, 0) como o canto superior esquerdo da própria janela,
/// não como a origem da WebView.
struct WindowRegion {
    bounds: (i32, i32, i32, i32),
    holes: Vec<(i32, i32, i32, i32)>,
}

#[cfg(any(all(windows, feature = "mpv"), test))]
fn window_region(width: u32, height: u32, cutouts: &[PlayerCutout]) -> WindowRegion {
    let right = width.min(i32::MAX as u32) as i32;
    let bottom = height.min(i32::MAX as u32) as i32;
    WindowRegion {
        bounds: (0, 0, right, bottom),
        holes: clipped_cutouts(width, height, cutouts),
    }
}

#[cfg(any(all(windows, feature = "mpv"), test))]
/// Recorta retângulos de controle aos limites da caixa do vídeo, evitando
/// regiões Win32 negativas, vazias ou fora da superfície nativa.
fn clipped_cutouts(width: u32, height: u32, cutouts: &[PlayerCutout]) -> Vec<(i32, i32, i32, i32)> {
    let max_x = width.min(i32::MAX as u32) as i32;
    let max_y = height.min(i32::MAX as u32) as i32;
    cutouts
        .iter()
        .filter_map(|cutout| {
            let left = cutout.x.clamp(0, max_x);
            let top = cutout.y.clamp(0, max_y);
            let right = cutout
                .x
                .saturating_add(cutout.width.min(i32::MAX as u32) as i32)
                .clamp(0, max_x);
            let bottom = cutout
                .y
                .saturating_add(cutout.height.min(i32::MAX as u32) as i32)
                .clamp(0, max_y);
            (right > left && bottom > top).then_some((left, top, right, bottom))
        })
        .collect()
}

#[tauri::command]
pub fn player_set_geometry(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    cutouts: Vec<PlayerCutout>,
) -> bool {
    #[cfg(all(windows, feature = "mpv"))]
    {
        win::set_shape(x, y, width, height, &cutouts)
    }
    #[cfg(not(all(windows, feature = "mpv")))]
    {
        let _ = (x, y, width, height, cutouts);
        false
    }
}

#[tauri::command]
pub fn player_close() {
    request_close();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerResult {
    pub position_ms: i64,
    pub duration_ms: i64,
}

/// Só aceita http/https: o mpv não pode abrir arquivos locais nem opções de linha de comando.
pub fn is_playable_url(raw: &str) -> bool {
    let raw = raw.trim();
    let Some((scheme, remainder)) = raw.split_once("://") else {
        return false;
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return false;
    }
    // Url::parse normaliza barras extras; rejeitar autoridade vazia antes do parse.
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
}

/// Handle da janela principal, para o mpv desenhar o vídeo dentro dela (Windows).
/// Em outras plataformas (ou se não der certo) o mpv abre a própria janela.
fn embed_handle(app: &tauri::AppHandle) -> Option<i64> {
    #[cfg(windows)]
    {
        use tauri::Manager;
        let window = app.get_webview_window("main")?;
        let hwnd = window.hwnd().ok()?;
        Some(hwnd.0 as isize as i64)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
}

/// Pede para fechar o vídeo embutido (Windows): envia WM_CLOSE para a janela do mpv.
pub fn request_close() {
    #[cfg(all(windows, feature = "mpv"))]
    win::request_close();
}

/// Esc global durante o vídeo (desktop). No Android não existe atalho global.
#[cfg(not(target_os = "android"))]
fn set_escape_shortcut(app: &tauri::AppHandle, on: bool) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let shortcuts = app.global_shortcut();
    // Atalho global: enquanto o vídeo toca, o Esc é capturado pelo sistema inteiro.
    // Se o registro falhar, o vídeo segue e o motivo fica no log (o Esc da janela ainda funciona).
    let result = if on {
        shortcuts.register("Escape")
    } else {
        shortcuts.unregister("Escape")
    };
    if let Err(e) = result {
        let acao = if on { "registrar" } else { "liberar" };
        eprintln!("aviso: não consegui {acao} o atalho Esc: {e}");
    }
}

#[cfg(target_os = "android")]
fn set_escape_shortcut(_app: &tauri::AppHandle, _on: bool) {}

#[tauri::command]
pub async fn player_play(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    url: String,
    start_ms: Option<i64>,
    subtitle_url: Option<String>,
    segments: Option<Vec<Segment>>,
) -> Result<PlayerResult, String> {
    if !is_playable_url(&url) {
        return Err("endereço de vídeo inválido".into());
    }
    let start = start_ms.unwrap_or(0).max(0);
    // A legenda é baixada aqui (com as mesmas validações do resto do app) e entregue ao mpv
    // como arquivo local. Se falhar, o vídeo toca sem legenda.
    let subtitle = match subtitle_url.filter(|u| is_playable_url(u)) {
        Some(sub) => fetch_subtitle_file(&app, &state, &sub).await,
        None => None,
    };
    let wid = embed_handle(&app);
    let segments = segments.unwrap_or_default();
    set_escape_shortcut(&app, true);
    let result = tauri::async_runtime::spawn_blocking(move || {
        imp::play(&url, start, wid, subtitle.as_deref(), &segments)
    })
    .await;
    set_escape_shortcut(&app, false);
    result.map_err(|e| e.to_string())?
}

/// Baixa a legenda (até 2 MiB, só endereços públicos), converte para VTT e grava na pasta
/// de dados do app. Devolve o caminho do arquivo, ou None se não der certo.
async fn fetch_subtitle_file(
    app: &tauri::AppHandle,
    state: &crate::AppState,
    url: &str,
) -> Option<String> {
    use tauri::Manager;
    const MAX_SUBTITLE: usize = 2 * 1024 * 1024;
    let bytes = match crate::addon::fetch_public_capped(&state.sub_http, url, MAX_SUBTITLE).await {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("aviso: legenda indisponível para o player: {e}");
            return None;
        }
    };
    let dir = app.path().app_data_dir().ok()?.join("subtitles");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("legenda.vtt");
    let vtt = crate::commands::srt_to_vtt(&String::from_utf8_lossy(&bytes));
    std::fs::write(&path, vtt).ok()?;
    Some(path.display().to_string())
}

#[cfg(feature = "mpv")]
mod imp {
    use super::PlayerResult;
    use crate::skip::{self, Segment};
    use libmpv2::events::Event;
    use libmpv2::{mpv_end_file_reason, Mpv};

    /// Tecla que pula o trecho em andamento (abertura, recapitulação ou créditos).
    const SKIP_KEY: &str = "F12";

    pub fn play(
        url: &str,
        start_ms: i64,
        wid: Option<i64>,
        subtitle: Option<&str>,
        segments: &[Segment],
    ) -> Result<PlayerResult, String> {
        let mpv = Mpv::with_initializer(|init| {
            init.set_option("vo", "gpu")?;
            init.set_option("hwdec", "auto-safe")?;
            // O OSC do mpv é a interface de controles no Windows: a janela de vídeo
            // nativa cobre a WebView, portanto os controles HTML do CineHub não ficam visíveis.
            // Não deixar em "auto": o usuário precisa ter controles acessíveis durante o vídeo.
            init.set_option("osc", "yes")?;
            // osc-visibility e osc-idle são opções do script OSC, não opções globais do mpv.
            // Passá-las diretamente a set_option causa MPV_ERROR_OPTION_NOT_FOUND (-5).
            init.set_option("script-opts", "osc-visibility=always,osc-idlescreen=yes")?;
            init.set_option("input-default-bindings", "yes")?;
            // O libmpv desativa input-cursor por padrão; sem isso o OSC não recebe mouse.
            init.set_option("input-cursor", "yes")?;
            // O player está embutido sobre a WebView: impedir que o backend encaminhe
            // os cliques do OSC para a janela subjacente em vez de tratá-los no mpv.
            init.set_option("input-cursor-passthrough", "no")?;
            init.set_option("input-vo-keyboard", "yes")?;
            init.set_option("osd-level", "3")?;
            init.set_option("border", "no")?;
            // Sem yt-dlp: a URL é sempre um vídeo direto vindo de um addon ou do torrent local.
            init.set_option("ytdl", "no")?;
            // Títulos com várias faixas: prefere áudio e legenda em português.
            init.set_option("alang", "pt,por")?;
            init.set_option("slang", "pt,por")?;
            match wid {
                // Dentro da janela do CineHub: o mpv cria uma janela filha que cobre a área do app.
                Some(handle) => init.set_option("wid", handle)?,
                None => {
                    init.set_option("force-window", "yes")?;
                    init.set_option("title", "CineHub")?;
                }
            }
            Ok(())
        })
        .map_err(|e| format!("não consegui iniciar o mpv: {e}"))?;

        // Controles de teclado explícitos, além do OSC clicável exibido pelo mpv.
        let _ = mpv.command("keybind", &["SPACE", "cycle pause"]);
        let _ = mpv.command("keybind", &["LEFT", "seek -10"]);
        let _ = mpv.command("keybind", &["RIGHT", "seek 10"]);
        let _ = mpv.command("keybind", &["UP", "add volume 5"]);
        let _ = mpv.command("keybind", &["DOWN", "add volume -5"]);
        let _ = mpv.command("keybind", &["M", "cycle mute"]);
        let _ = mpv.command("keybind", &["F", "cycle fullscreen"]);
        // Atalhos explícitos para as opções de faixa e legenda, inclusive desativar legendas.
        let _ = mpv.command("keybind", &["V", "cycle sub"]);
        let _ = mpv.command("keybind", &["I", "cycle sub-visibility"]);
        let _ = mpv.command("keybind", &["A", "cycle audio"]);
        let _ = mpv.command("keybind", &["ESC", "quit"]);
        let _ = mpv.command("keybind", &["MBTN_RIGHT", "quit"]);

        if start_ms > 0 {
            let opts = format!("start={:.3}", start_ms as f64 / 1000.0);
            mpv.command("loadfile", &[url, "replace", "-1", &opts])
        } else {
            mpv.command("loadfile", &[url, "replace"])
        }
        .map_err(|e| format!("o mpv não abriu o vídeo: {e}"))?;

        if let Some(path) = subtitle {
            // Legenda em português, já convertida para VTT pelo backend. Falha aqui não impede o vídeo.
            if let Err(e) = mpv.command("sub-add", &[path, "select", "CineHub", "pt"]) {
                eprintln!("aviso: o mpv não carregou a legenda: {e}");
            }
        }

        let mut position = 0f64;
        let mut duration = 0f64;
        // Fim do trecho cuja tecla está ligada (None: nenhum trecho agora)
        let mut bound_end: Option<f64> = None;
        let mut failed = false;
        // Dentro da janela do app: esconde os controles web e traz o vídeo para frente (Windows).
        #[cfg(all(windows, feature = "mpv"))]
        let mut embedded = false;
        loop {
            match mpv.wait_event(0.25) {
                Some(Ok(Event::EndFile(reason))) => {
                    failed = reason == mpv_end_file_reason::Error;
                    break;
                }
                Some(Ok(Event::Shutdown)) => break,
                _ => {}
            }
            #[cfg(all(windows, feature = "mpv"))]
            if let (Some(parent), false) = (wid, embedded) {
                if let Ok(video) = mpv.get_property::<i64>("window-id") {
                    embedded = true;
                    super::win::take_over(parent, video);
                }
            }
            // Guarda a última posição conhecida: depois do fim do arquivo as propriedades não existem mais.
            if let Ok(t) = mpv.get_property::<f64>("time-pos") {
                position = t;
            }
            if let Ok(d) = mpv.get_property::<f64>("duration") {
                duration = d;
            }
            // Pular: ao entrar num trecho, avisa na tela e liga SKIP_KEY a um pulo para o fim dele.
            // Ao sair, a tecla volta a não fazer nada.
            let active = skip::active_segment(segments, position);
            let end = active.map(|seg| seg.end);
            if end != bound_end {
                bound_end = end;
                match active {
                    Some(seg) => {
                        let _ = mpv.command(
                            "keybind",
                            &[SKIP_KEY, &format!("seek {:.3} absolute", seg.end)],
                        );
                        let text = format!("{}: tecle {SKIP_KEY}", skip::label(&seg.kind));
                        let _ = mpv.command("show-text", &[&text, "4000"]);
                    }
                    None => {
                        let _ = mpv.command("keybind", &[SKIP_KEY, "ignore"]);
                    }
                }
            }
        }

        #[cfg(all(windows, feature = "mpv"))]
        super::win::restore();

        if failed {
            return Err("o mpv não conseguiu tocar este vídeo".into());
        }
        Ok(PlayerResult {
            position_ms: (position * 1000.0) as i64,
            duration_ms: (duration * 1000.0) as i64,
        })
    }
}

#[cfg(all(windows, feature = "mpv"))]
mod win {
    //! Ajustes de janela (Win32) para o vídeo aparecer por cima da interface web.
    use std::ffi::c_void;

    type Hwnd = *mut c_void;
    type Bool = i32;

    #[link(name = "gdi32")]
    extern "system" {
        fn CreateRectRgn(left: i32, top: i32, right: i32, bottom: i32) -> Hwnd;
        fn CombineRgn(dest: Hwnd, src1: Hwnd, src2: Hwnd, mode: i32) -> i32;
        fn DeleteObject(object: Hwnd) -> Bool;
    }

    #[link(name = "user32")]
    extern "system" {
        fn SetWindowRgn(hwnd: Hwnd, region: Hwnd, redraw: Bool) -> i32;
        fn SetWindowPos(
            hwnd: Hwnd,
            after: Hwnd,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> Bool;
        fn SetForegroundWindow(hwnd: Hwnd) -> Bool;
        fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> Bool;
        fn SendMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn MapWindowPoints(from: Hwnd, to: Hwnd, points: *mut Point, count: i32) -> i32;
        fn GetCursorPos(point: *mut Point) -> Bool;
        fn ScreenToClient(hwnd: Hwnd, point: *mut Point) -> Bool;
        fn EnumChildWindows(
            parent: Hwnd,
            callback: Option<unsafe extern "system" fn(Hwnd, isize) -> Bool>,
            parameter: isize,
        ) -> Bool;
        fn IsChild(parent: Hwnd, child: Hwnd) -> Bool;
    }

    #[link(name = "comctl32")]
    extern "system" {
        fn SetWindowSubclass(
            hwnd: Hwnd,
            callback: Option<
                unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize,
            >,
            subclass_id: usize,
            reference: usize,
        ) -> Bool;
        fn RemoveWindowSubclass(
            hwnd: Hwnd,
            callback: Option<
                unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize,
            >,
            subclass_id: usize,
        ) -> Bool;
        fn DefSubclassProc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    const WM_CLOSE: u32 = 0x0010;
    const WM_SETFOCUS: u32 = 0x0007;
    const WM_KILLFOCUS: u32 = 0x0008;
    const WM_MOUSEACTIVATE: u32 = 0x0021;
    const WM_KEYDOWN: u32 = 0x0100;
    const WM_KEYUP: u32 = 0x0101;
    const WM_SYSKEYDOWN: u32 = 0x0104;
    const WM_SYSKEYUP: u32 = 0x0105;
    const WM_MOUSEMOVE: u32 = 0x0200;
    const WM_LBUTTONDOWN: u32 = 0x0201;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONDOWN: u32 = 0x0204;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_RBUTTONDBLCLK: u32 = 0x0206;
    const WM_MBUTTONDOWN: u32 = 0x0207;
    const WM_MBUTTONUP: u32 = 0x0208;
    const WM_MBUTTONDBLCLK: u32 = 0x0209;
    const WM_MOUSEWHEEL: u32 = 0x020a;
    const WM_XBUTTONDOWN: u32 = 0x020b;
    const WM_XBUTTONUP: u32 = 0x020c;
    const WM_MOUSEHWHEEL: u32 = 0x020e;
    const WM_MOUSELEAVE: u32 = 0x02a3;
    const WM_IME_FIRST: u32 = 0x0280;
    const WM_IME_LAST: u32 = 0x0291;
    const SUBCLASS_ID: usize = 0x43494e45;
    const HWND_TOP: Hwnd = std::ptr::null_mut();
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const RGN_DIFF: i32 = 4;

    /// HWNDs das janelas principal e filha enquanto o player está ativo.
    pub static VIDEO: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
    static PARENT: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
    static INPUT_GEOMETRY: std::sync::Mutex<
        Option<(i32, i32, u32, u32, Vec<super::PlayerCutout>)>,
    > = std::sync::Mutex::new(None);
    /// Todas as janelas que recebem mensagens de entrada da WebView ou da janela principal.
    static SUBCLASS_WINDOWS: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(Vec::new());

    pub(super) fn point_is_in_video(
        parent_x: i32,
        parent_y: i32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        cutouts: &[super::PlayerCutout],
    ) -> bool {
        let local_x = parent_x - x;
        let local_y = parent_y - y;
        local_x >= 0
            && local_y >= 0
            && local_x < width.min(i32::MAX as u32) as i32
            && local_y < height.min(i32::MAX as u32) as i32
            && !cutouts.iter().any(|cutout| {
                local_x >= cutout.x
                    && local_y >= cutout.y
                    && local_x
                        < cutout
                            .x
                            .saturating_add(cutout.width.min(i32::MAX as u32) as i32)
                    && local_y
                        < cutout
                            .y
                            .saturating_add(cutout.height.min(i32::MAX as u32) as i32)
            })
    }

    pub(super) fn forwards_input_message(msg: u32) -> bool {
        matches!(
            msg,
            WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_MOUSEACTIVATE
                | WM_KEYDOWN
                | WM_KEYUP
                | WM_SYSKEYDOWN
                | WM_SYSKEYUP
                | WM_MOUSEMOVE
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_LBUTTONDBLCLK
                | WM_RBUTTONDOWN
                | WM_RBUTTONUP
                | WM_RBUTTONDBLCLK
                | WM_MBUTTONDOWN
                | WM_MBUTTONUP
                | WM_MBUTTONDBLCLK
                | WM_MOUSEWHEEL
                | WM_XBUTTONDOWN
                | WM_XBUTTONUP
                | WM_MOUSEHWHEEL
                | WM_MOUSELEAVE
        ) || (WM_IME_FIRST..=WM_IME_LAST).contains(&msg)
    }

    unsafe extern "system" fn install_child_input_subclass(child: Hwnd, _parameter: isize) -> Bool {
        let video = VIDEO.load(std::sync::atomic::Ordering::SeqCst) as usize as Hwnd;
        // Nunca subclassificar a janela do mpv nem seus descendentes: isso causaria
        // encaminhamento recursivo de uma mensagem para a própria janela.
        if child == video || (!video.is_null() && IsChild(video, child) != 0) {
            return 1;
        }
        if SetWindowSubclass(child, Some(parent_subclass_proc), SUBCLASS_ID, 0) != 0 {
            SUBCLASS_WINDOWS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(child as usize as isize);
        }
        1
    }

    unsafe extern "system" fn parent_subclass_proc(
        hwnd: Hwnd,
        msg: u32,
        wparam: usize,
        lparam: isize,
        _subclass_id: usize,
        _reference: usize,
    ) -> isize {
        let video = VIDEO.load(std::sync::atomic::Ordering::SeqCst);
        if video != 0 && forwards_input_message(msg) {
            let is_mouse_message = matches!(
                msg,
                WM_MOUSEACTIVATE
                    | WM_MOUSEMOVE
                    | WM_LBUTTONDOWN
                    | WM_LBUTTONUP
                    | WM_LBUTTONDBLCLK
                    | WM_RBUTTONDOWN
                    | WM_RBUTTONUP
                    | WM_RBUTTONDBLCLK
                    | WM_MBUTTONDOWN
                    | WM_MBUTTONUP
                    | WM_MBUTTONDBLCLK
                    | WM_MOUSEWHEEL
                    | WM_XBUTTONDOWN
                    | WM_XBUTTONUP
                    | WM_MOUSEHWHEEL
            );
            if is_mouse_message {
                let mut cursor = Point { x: 0, y: 0 };
                let parent = PARENT.load(std::sync::atomic::Ordering::SeqCst);
                if parent != 0
                    && GetCursorPos(&mut cursor) != 0
                    && ScreenToClient(parent as usize as Hwnd, &mut cursor) != 0
                {
                    let geometry = INPUT_GEOMETRY.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some((x, y, width, height, cutouts)) = geometry.as_ref() {
                        if !point_is_in_video(cursor.x, cursor.y, *x, *y, *width, *height, cutouts)
                        {
                            return DefSubclassProc(hwnd, msg, wparam, lparam);
                        }
                    }
                }
            }
            let mut forwarded_lparam = lparam;
            // Mouse button/move coordinates are client-relative. Convert from the
            // Tauri parent to the mpv child; wheel coordinates are screen-relative
            // and must remain unchanged.
            if matches!(
                msg,
                WM_MOUSEMOVE
                    | WM_LBUTTONDOWN
                    | WM_LBUTTONUP
                    | WM_LBUTTONDBLCLK
                    | WM_RBUTTONDOWN
                    | WM_RBUTTONUP
                    | WM_RBUTTONDBLCLK
                    | WM_MBUTTONDOWN
                    | WM_MBUTTONUP
                    | WM_MBUTTONDBLCLK
                    | WM_XBUTTONDOWN
                    | WM_XBUTTONUP
            ) {
                let mut point = Point {
                    x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
                    y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
                };
                MapWindowPoints(hwnd, video as usize as Hwnd, &mut point, 1);
                forwarded_lparam = ((point.y as u16 as u32) << 16 | point.x as u16 as u32) as isize;
            }
            return SendMessageW(video as usize as Hwnd, msg, wparam, forwarded_lparam);
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }

    /// Fecha o vídeo embutido: WM_CLOSE para a janela do mpv, que encerra a reprodução.
    pub fn request_close() {
        let video = VIDEO.load(std::sync::atomic::Ordering::SeqCst);
        if video != 0 {
            // SAFETY: video é o HWND da janela do mpv registrado em take_over; PostMessageW só enfileira a mensagem.
            unsafe {
                PostMessageW(video as usize as Hwnd, WM_CLOSE, 0, 0);
            }
        }
    }

    /// Mantém o mpv acima da WebView sem esconder os controles HTML.
    pub fn take_over(parent: i64, video: i64) {
        let parent = parent as usize as Hwnd;
        let video = video as usize as Hwnd;
        PARENT.store(
            parent as usize as isize,
            std::sync::atomic::Ordering::SeqCst,
        );
        VIDEO.store(video as usize as isize, std::sync::atomic::Ordering::SeqCst);
        // SAFETY: os HWNDs vêm do Tauri e do libmpv e permanecem válidos durante a reprodução.
        unsafe {
            // O mpv.net encaminha as mensagens recebidas pela janela hospedeira
            // diretamente ao HWND filho do mpv. Isso também funciona quando o filho
            // está marcado WS_DISABLED e não recebe hit-testing normal do Windows.
            SUBCLASS_WINDOWS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            if SetWindowSubclass(parent, Some(parent_subclass_proc), SUBCLASS_ID, 0) != 0 {
                SUBCLASS_WINDOWS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(parent as usize as isize);
            }
            // O HWND retornado pelo Tauri é a janela de topo. O WebView2 recebe
            // WM_MOUSE* nas próprias janelas-filhas, portanto também precisamos
            // interceptar a árvore de filhos para que o mpv.net-style forwarding
            // veja cliques e rolagem que nunca chegam ao WndProc da janela de topo.
            EnumChildWindows(parent, Some(install_child_input_subclass), 0);
            SetForegroundWindow(parent);
            SetWindowPos(
                video,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
        }
    }

    /// Recorta a janela nativa para a caixa de vídeo e subtrai as áreas dos
    /// controles HTML. A região Win32 afeta desenho e hit-testing.
    pub fn set_shape(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        cutouts: &[super::PlayerCutout],
    ) -> bool {
        let video = VIDEO.load(std::sync::atomic::Ordering::SeqCst);
        if video == 0 || width == 0 || height == 0 {
            return false;
        }
        // SAFETY: os handles de região são criados e liberados nesta função.
        unsafe {
            // O HWND do mpv é filho da janela principal: x/y são relativos à área cliente
            // da WebView. Redimensioná-lo aqui mantém o vídeo alinhado ao player-box.
            // Não roubar o foco permite clicar nos recortes HTML.
            let window_width = width.min(i32::MAX as u32) as i32;
            let window_height = height.min(i32::MAX as u32) as i32;
            if SetWindowPos(
                video as usize as Hwnd,
                HWND_TOP,
                x,
                y,
                window_width,
                window_height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            ) == 0
            {
                return false;
            }
            // SetWindowRgn usa coordenadas locais à janela filha, não coordenadas
            // absolutas da WebView. A região começa sempre em (0, 0).
            *INPUT_GEOMETRY.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((x, y, width, height, cutouts.to_vec()));
            let region_shape = super::window_region(width, height, cutouts);
            let bounds = region_shape.bounds;
            let region = CreateRectRgn(bounds.0, bounds.1, bounds.2, bounds.3);
            if region.is_null() {
                return false;
            }
            for (left, top, right, bottom) in region_shape.holes {
                let hole = CreateRectRgn(left, top, right, bottom);
                if !hole.is_null() {
                    CombineRgn(region, region, hole, RGN_DIFF);
                    DeleteObject(hole);
                }
            }
            if SetWindowRgn(video as usize as Hwnd, region, 1) == 0 {
                DeleteObject(region);
                false
            } else {
                true
            }
        }
    }

    /// Remove o recorte ao encerrar a reprodução.
    pub fn restore() {
        let video = VIDEO.swap(0, std::sync::atomic::Ordering::SeqCst);
        let parent = PARENT.swap(0, std::sync::atomic::Ordering::SeqCst);
        // SAFETY: a subclass é removida da janela hospedeira antes de liberar o HWND do mpv.
        unsafe {
            let subclass_windows = std::mem::take(
                &mut *SUBCLASS_WINDOWS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()),
            );
            for hwnd in subclass_windows {
                RemoveWindowSubclass(
                    hwnd as usize as Hwnd,
                    Some(parent_subclass_proc),
                    SUBCLASS_ID,
                );
            }
            *INPUT_GEOMETRY.lock().unwrap_or_else(|e| e.into_inner()) = None;
            if video != 0 {
                SetWindowRgn(video as usize as Hwnd, std::ptr::null_mut(), 1);
            }
        }
    }
}

#[cfg(not(feature = "mpv"))]
mod imp {
    use super::PlayerResult;

    pub fn play(
        _url: &str,
        _start_ms: i64,
        _wid: Option<i64>,
        _subtitle: Option<&str>,
        _segments: &[crate::skip::Segment],
    ) -> Result<PlayerResult, String> {
        Err("player nativo não incluído nesta versão".into())
    }
}

#[cfg(test)]
mod tests {
    use super::is_playable_url;

    #[cfg(feature = "mpv")]
    #[test]
    fn mpv_osc_script_options_are_valid() {
        use libmpv2::Mpv;

        let result = Mpv::with_initializer(|init| {
            init.set_option("vo", "null")?;
            init.set_option("ao", "null")?;
            init.set_option("osc", "yes")?;
            init.set_option("input-cursor", "yes")?;
            init.set_option("input-cursor-passthrough", "no")?;
            init.set_option("script-opts", "osc-visibility=always,osc-idlescreen=yes")?;
            Ok(())
        });
        assert!(
            result.is_ok(),
            "libmpv rejeitou as opções do OSC: {:?}",
            result.err()
        );
        #[cfg(all(windows, feature = "mpv"))]
        {
            for message in [
                0x0200, // WM_MOUSEMOVE
                0x0201, // WM_LBUTTONDOWN
                0x0202, // WM_LBUTTONUP
                0x020a, // WM_MOUSEWHEEL
                0x020e, // WM_MOUSEHWHEEL
                0x0100, // WM_KEYDOWN
                0x0101, // WM_KEYUP
            ] {
                assert!(super::win::forwards_input_message(message));
            }
            assert!(!super::win::forwards_input_message(0x000f)); // WM_PAINT
            let cuts = [super::PlayerCutout {
                x: 10,
                y: 20,
                width: 30,
                height: 15,
            }];
            // Fora do recorte: coordenadas locais (50, 30), dentro do vídeo.
            assert!(super::win::point_is_in_video(
                150, 130, 100, 100, 100, 100, &cuts
            ));
            // Dentro do recorte HTML (10..40, 20..35): não encaminhar ao mpv.
            assert!(!super::win::point_is_in_video(
                115, 125, 100, 100, 100, 100, &cuts
            ));
            // A borda direita do recorte é exclusiva; x=40 já pertence ao vídeo.
            assert!(super::win::point_is_in_video(
                140, 130, 100, 100, 100, 100, &cuts
            ));
            assert!(!super::win::point_is_in_video(
                99, 130, 100, 100, 100, 100, &cuts
            ));
        }
    }

    #[test]
    fn player_cutouts_are_clipped_to_the_video_box() {
        let cuts = [
            super::PlayerCutout {
                x: -10,
                y: 5,
                width: 30,
                height: 20,
            },
            super::PlayerCutout {
                x: 90,
                y: 90,
                width: 30,
                height: 30,
            },
            super::PlayerCutout {
                x: 110,
                y: 10,
                width: 5,
                height: 5,
            },
            super::PlayerCutout {
                x: 10,
                y: 10,
                width: 0,
                height: 20,
            },
        ];
        assert_eq!(
            super::clipped_cutouts(100, 100, &cuts),
            vec![(0, 5, 20, 25), (90, 90, 100, 100)]
        );
        // A região e seus recortes são locais à janela filha. A posição x/y
        // da WebView é aplicada separadamente por SetWindowPos.
        assert_eq!(
            {
                let region = super::window_region(100, 100, &cuts);
                (region.bounds, region.holes)
            },
            ((0, 0, 100, 100), vec![(0, 5, 20, 25), (90, 90, 100, 100)])
        );
    }

    #[test]
    fn player_cutouts_deserialize_and_keep_signed_offsets() {
        let cutout: super::PlayerCutout =
            serde_json::from_str(r#"{"x":-4,"y":12,"width":80,"height":32}"#).unwrap();
        assert_eq!(
            cutout,
            super::PlayerCutout {
                x: -4,
                y: 12,
                width: 80,
                height: 32
            }
        );
    }

    #[test]
    fn only_http_urls_reach_mpv() {
        assert!(is_playable_url("https://example.com/video.mkv"));
        assert!(is_playable_url("http://127.0.0.1:9000/stream/0"));
        assert!(is_playable_url("http://192.168.1.20:8080/video.mp4"));
        assert!(!is_playable_url("https:///missing-host/video.mp4"));
        assert!(!is_playable_url("http://"));
        assert!(!is_playable_url("file:///etc/passwd"));
        assert!(!is_playable_url("--script=/tmp/x.lua"));
        assert!(!is_playable_url("/caminho/local.mp4"));
        assert!(!is_playable_url(""));
    }
}
