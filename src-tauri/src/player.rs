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

#[tauri::command]
pub fn player_set_geometry(x: i32, y: i32, width: u32, height: u32, cutouts: Vec<PlayerCutout>) {
    #[cfg(all(windows, feature = "mpv"))]
    win::set_shape(x, y, width, height, &cutouts);
    #[cfg(not(all(windows, feature = "mpv")))]
    let _ = (x, y, width, height, cutouts);
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
    type EnumProc = unsafe extern "system" fn(Hwnd, isize) -> Bool;

    #[link(name = "user32")]
    extern "system" {
        fn CreateRectRgn(left: i32, top: i32, right: i32, bottom: i32) -> Hwnd;
        fn CombineRgn(dest: Hwnd, src1: Hwnd, src2: Hwnd, mode: i32) -> i32;
        fn DeleteObject(object: Hwnd) -> Bool;
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
        fn SetFocus(hwnd: Hwnd) -> Hwnd;
        fn SetForegroundWindow(hwnd: Hwnd) -> Bool;
        fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> Bool;
    }

    const WM_CLOSE: u32 = 0x0010;
    const HWND_TOP: Hwnd = std::ptr::null_mut();
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const RGN_DIFF: i32 = 4;

    /// HWND do vídeo embutido enquanto ele toca (0 = nenhum).
    pub static VIDEO: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

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
    pub fn take_over(_parent: i64, video: i64) {
        let video = video as usize as Hwnd;
        VIDEO.store(video as usize as isize, std::sync::atomic::Ordering::SeqCst);
        // SAFETY: video é o HWND retornado pelo próprio libmpv.
        unsafe {
            SetForegroundWindow(video);
            SetFocus(video);
            SetWindowPos(video, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        }
    }

    /// Recorta a janela nativa para a caixa de vídeo e subtrai as áreas dos
    /// controles HTML. A região Win32 afeta desenho e hit-testing.
    pub fn set_shape(x: i32, y: i32, width: u32, height: u32, cutouts: &[super::PlayerCutout]) {
        let video = VIDEO.load(std::sync::atomic::Ordering::SeqCst);
        if video == 0 || width == 0 || height == 0 { return; }
        // SAFETY: os handles de região são criados e liberados nesta função.
        unsafe {
            let right_edge = x.saturating_add(width.min(i32::MAX as u32) as i32);
            let bottom_edge = y.saturating_add(height.min(i32::MAX as u32) as i32);
            let region = CreateRectRgn(x, y, right_edge, bottom_edge);
            if region.is_null() { return; }
            for cutout in cutouts {
                let left = x.saturating_add(cutout.x).max(x);
                let top = y.saturating_add(cutout.y).max(y);
                let right = left.saturating_add(cutout.width.min(i32::MAX as u32) as i32).min(right_edge);
                let bottom = top.saturating_add(cutout.height.min(i32::MAX as u32) as i32).min(bottom_edge);
                if right <= left || bottom <= top { continue; }
                let hole = CreateRectRgn(left, top, right, bottom);
                if !hole.is_null() {
                    CombineRgn(region, region, hole, RGN_DIFF);
                    DeleteObject(hole);
                }
            }
            if SetWindowRgn(video as usize as Hwnd, region, 1) == 0 { DeleteObject(region); }
        }
    }

    /// Remove o recorte ao encerrar a reprodução.
    pub fn restore() {
        let video = VIDEO.swap(0, std::sync::atomic::Ordering::SeqCst);
        if video != 0 {
            // SAFETY: região nula restaura a forma retangular padrão da janela.
            unsafe { SetWindowRgn(video as usize as Hwnd, std::ptr::null_mut(), 1); }
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
            init.set_option("script-opts", "osc-visibility=always,osc-idlescreen=yes")?;
            Ok(())
        });
        assert!(
            result.is_ok(),
            "libmpv rejeitou as opções do OSC: {:?}",
            result.err()
        );
    }

    #[test]
    fn player_cutouts_deserialize_and_keep_signed_offsets() {
        let cutout: super::PlayerCutout = serde_json::from_str(
            r#"{"x":-4,"y":12,"width":80,"height":32}"#,
        ).unwrap();
        assert_eq!(cutout, super::PlayerCutout { x: -4, y: 12, width: 80, height: 32 });
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
