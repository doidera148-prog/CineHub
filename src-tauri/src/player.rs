//! Player nativo de desktop (libmpv).
//!
//! Abre o vídeo numa janela do mpv (com o decodificador e os codecs do próprio mpv) e responde
//! quando a janela fecha, com a posição final, para a interface salvar o progresso.
//! Só funciona quando o app é compilado com a feature `mpv` (que exige libmpv instalada).
//! Sem a feature, o comando responde com erro e a interface usa o player do navegador.

use serde::{Deserialize, Serialize};

use crate::skip::Segment;

#[cfg(all(windows, feature = "mpv"))]
#[path = "player/d3d11.rs"]
mod mpv_d3d11;

/// Área retangular da interface web que deve permanecer visível sobre o vídeo.
/// Coordenadas relativas à caixa do player, em pixels físicos do Windows.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerCutout {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

// Payload fields are consumed by the libmpv loop, which is compiled only with feature `mpv`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
enum PlayerControl {
    TogglePause,
    SeekRelative(f64),
    SeekAbsolute(f64),
    ToggleMute,
    SetVolume(f64),
    SelectAudio(i64),
    SelectSubtitle(Option<i64>),
}

static PLAYER_CONTROL_SENDER: std::sync::Mutex<Option<std::sync::mpsc::Sender<PlayerControl>>> =
    std::sync::Mutex::new(None);

fn parse_player_control(action: &str, value: Option<f64>) -> Result<PlayerControl, String> {
    match action {
        "toggle-pause" => Ok(PlayerControl::TogglePause),
        "seek-relative" => {
            let seconds = value.ok_or_else(|| "tempo de busca ausente".to_string())?;
            if !seconds.is_finite() {
                return Err("tempo de busca inválido".into());
            }
            Ok(PlayerControl::SeekRelative(seconds.clamp(-600.0, 600.0)))
        }
        "seek-absolute" => {
            let seconds = value.ok_or_else(|| "posição de busca ausente".to_string())?;
            if !seconds.is_finite() || seconds < 0.0 {
                return Err("posição de busca inválida".into());
            }
            Ok(PlayerControl::SeekAbsolute(seconds))
        }
        "toggle-mute" => Ok(PlayerControl::ToggleMute),
        "set-volume" => {
            let volume = value.ok_or_else(|| "volume ausente".to_string())?;
            if !volume.is_finite() {
                return Err("volume inválido".into());
            }
            Ok(PlayerControl::SetVolume(volume.clamp(0.0, 100.0)))
        }
        "select-audio" => Ok(PlayerControl::SelectAudio(parse_track_id(
            value,
            "faixa de áudio",
        )?)),
        "select-subtitle" => Ok(PlayerControl::SelectSubtitle(
            value
                .map(|id| parse_track_id(Some(id), "faixa de legenda"))
                .transpose()?,
        )),
        _ => Err("comando de player desconhecido".into()),
    }
}

fn parse_track_id(value: Option<f64>, description: &str) -> Result<i64, String> {
    let id = value.ok_or_else(|| format!("{description} ausente"))?;
    if !id.is_finite() || id < 0.0 || id.fract() != 0.0 || id > i64::MAX as f64 {
        return Err(format!("{description} inválida"));
    }
    Ok(id as i64)
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
    app: tauri::AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    cutouts: Vec<PlayerCutout>,
) -> bool {
    #[cfg(all(windows, feature = "mpv"))]
    {
        if std::env::var_os("CINEHUB_WEBVIEW2_COMPOSITION_POC").is_some() {
            return embed_handle(&app)
                .is_some_and(|parent| mpv_d3d11::set_video_rect(parent, x, y, width, height));
        }
        win::set_shape(x, y, width, height, &cutouts)
    }
    #[cfg(not(all(windows, feature = "mpv")))]
    {
        let _ = (app, x, y, width, height, cutouts);
        false
    }
}

#[tauri::command]
pub fn player_composition_enabled() -> bool {
    #[cfg(all(windows, feature = "mpv"))]
    {
        std::env::var_os("CINEHUB_WEBVIEW2_COMPOSITION_POC").is_some()
    }
    #[cfg(not(all(windows, feature = "mpv")))]
    {
        false
    }
}

#[tauri::command]
pub fn player_close() {
    request_close();
}

#[tauri::command]
pub fn player_set_fullscreen(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use tauri::Manager;

        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "a janela principal do CineHub não está disponível".to_string())?;
        window
            .set_fullscreen(enabled)
            .map_err(|error| format!("não consegui alterar a tela cheia: {error}"))
    }
    #[cfg(not(windows))]
    {
        let _ = (app, enabled);
        Err("tela cheia nativa disponível somente no player Windows".into())
    }
}

#[tauri::command]
pub fn player_control(action: String, value: Option<f64>) -> Result<(), String> {
    let control = parse_player_control(&action, value)?;
    let sender = PLAYER_CONTROL_SENDER
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
        .ok_or_else(|| "o player ainda não está pronto para receber comandos".to_string())?;
    sender
        .send(control)
        .map_err(|_| "o player encerrou antes de receber o comando".to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerResult {
    pub position_ms: i64,
    pub duration_ms: i64,
}

#[cfg(feature = "mpv")]
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayerPlaybackState {
    position_ms: i64,
    duration_ms: i64,
    paused: bool,
    volume: f64,
    muted: bool,
    audio_tracks: Vec<PlayerTrack>,
    subtitle_tracks: Vec<PlayerTrack>,
}

#[cfg(feature = "mpv")]
#[derive(Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PlayerTrack {
    id: i64,
    title: Option<String>,
    language: Option<String>,
    codec: Option<String>,
    external: bool,
    selected: bool,
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
    {
        if std::env::var_os("CINEHUB_WEBVIEW2_COMPOSITION_POC").is_some() {
            imp::request_close();
        } else {
            win::request_close();
        }
    }
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
    let (control_sender, control_receiver) = std::sync::mpsc::channel();
    *PLAYER_CONTROL_SENDER
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(control_sender);
    let player_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        imp::play(
            &url,
            start,
            wid,
            subtitle.as_deref(),
            &segments,
            player_app,
            control_receiver,
        )
    })
    .await;
    *PLAYER_CONTROL_SENDER
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = None;
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
    use super::{PlayerControl, PlayerPlaybackState, PlayerResult};
    use crate::skip::{self, Segment};
    use libmpv2::events::Event;
    use libmpv2::{mpv_end_file_reason, Mpv};

    /// Tecla que pula o trecho em andamento (abertura, recapitulação ou créditos).
    const SKIP_KEY: &str = "F12";
    static COMPOSITION_CLOSE_REQUESTED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    pub fn request_close() {
        COMPOSITION_CLOSE_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn track_text(mpv: &Mpv, property: &str) -> Option<String> {
        mpv.get_property::<String>(property)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn collect_tracks(mpv: &Mpv) -> (Vec<PlayerTrack>, Vec<PlayerTrack>) {
        let count = mpv
            .get_property::<i64>("track-list/count")
            .unwrap_or(0)
            .clamp(0, 128);
        let mut audio = Vec::new();
        let mut subtitles = Vec::new();

        for index in 0..count {
            let prefix = format!("track-list/{index}");
            let Ok(id) = mpv.get_property::<i64>(&format!("{prefix}/id")) else {
                continue;
            };
            let Ok(kind) = mpv.get_property::<String>(&format!("{prefix}/type")) else {
                continue;
            };
            if kind != "audio" && kind != "sub" {
                continue;
            }

            let track = PlayerTrack {
                id,
                title: track_text(mpv, &format!("{prefix}/title")),
                language: track_text(mpv, &format!("{prefix}/lang")),
                codec: track_text(mpv, &format!("{prefix}/codec-desc"))
                    .or_else(|| track_text(mpv, &format!("{prefix}/codec"))),
                external: mpv
                    .get_property::<bool>(&format!("{prefix}/external"))
                    .unwrap_or(false),
                selected: mpv
                    .get_property::<bool>(&format!("{prefix}/selected"))
                    .unwrap_or(false),
            };

            if kind == "audio" {
                audio.push(track);
            } else {
                subtitles.push(track);
            }
        }

        (audio, subtitles)
    }

    pub fn play(
        url: &str,
        start_ms: i64,
        wid: Option<i64>,
        subtitle: Option<&str>,
        segments: &[Segment],
        app: tauri::AppHandle,
        controls: std::sync::mpsc::Receiver<PlayerControl>,
    ) -> Result<PlayerResult, String> {
        COMPOSITION_CLOSE_REQUESTED.store(false, std::sync::atomic::Ordering::SeqCst);
        #[cfg(windows)]
        let composition_mode =
            wid.is_some() && std::env::var_os("CINEHUB_WEBVIEW2_COMPOSITION_POC").is_some();
        #[cfg(not(windows))]
        let composition_mode = false;

        let mpv = Mpv::with_initializer(|init| {
            init.set_option("vo", if composition_mode { "libmpv" } else { "gpu" })?;
            init.set_option("hwdec", "auto-safe")?;
            // No modo de composição, os controles HTML do CineHub recebem o mouse.
            // O OSC nativo do mpv fica desativado para não exibir uma barra sem hit testing.
            init.set_option("osc", if composition_mode { "no" } else { "yes" })?;
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
            // Desativa texto/progresso do OSD, mas mantém as legendas do vídeo visíveis.
            init.set_option("osd-level", "0")?;
            init.set_option("border", "no")?;
            // Sem yt-dlp: a URL é sempre um vídeo direto vindo de um addon ou do torrent local.
            init.set_option("ytdl", "no")?;
            // Títulos com várias faixas: prefere áudio e legenda em português.
            init.set_option("alang", "pt,por")?;
            init.set_option("slang", "pt,por")?;
            if !composition_mode {
                match wid {
                    // No caminho padrão, o mpv cria uma janela filha dentro do app.
                    Some(handle) => init.set_option("wid", handle)?,
                    None => {
                        init.set_option("force-window", "yes")?;
                        init.set_option("title", "CineHub")?;
                    }
                }
            }
            Ok(())
        })
        .map_err(|e| format!("não consegui iniciar o mpv: {e}"))?;

        // Create the render context before loadfile so vo=libmpv never creates a
        // separate native window. Wry supplies the device used by its composition swap chain.
        #[cfg(windows)]
        let render_context = if composition_mode {
            let parent = wid.expect("composition_mode requer o HWND da janela principal");
            let device = super::mpv_d3d11::host_device(parent)?;
            Some(super::mpv_d3d11::D3d11RenderContext::new(&mpv, &device)?)
        } else {
            None
        };

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
        let mut last_state_emit = std::time::Instant::now();
        // Dentro da janela do app: esconde os controles web e traz o vídeo para frente (Windows).
        #[cfg(all(windows, feature = "mpv"))]
        let mut embedded = composition_mode;
        loop {
            match mpv.wait_event(if composition_mode { 0.016 } else { 0.25 }) {
                Some(Ok(Event::EndFile(reason))) => {
                    failed = reason == mpv_end_file_reason::Error;
                    break;
                }
                Some(Ok(Event::Shutdown)) => break,
                _ => {}
            }

            if COMPOSITION_CLOSE_REQUESTED.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = mpv.command("quit", &[]);
            }

            while let Ok(control) = controls.try_recv() {
                let result = match control {
                    PlayerControl::TogglePause => mpv.command("cycle", &["pause"]),
                    PlayerControl::SeekRelative(seconds) => {
                        let seconds = format!("{seconds:.3}");
                        mpv.command("seek", &[&seconds, "relative"])
                    }
                    PlayerControl::SeekAbsolute(seconds) => {
                        let seconds = format!("{seconds:.3}");
                        mpv.command("seek", &[&seconds, "absolute+exact"])
                    }
                    PlayerControl::ToggleMute => mpv.command("cycle", &["mute"]),
                    PlayerControl::SetVolume(volume) => {
                        let volume = format!("{volume:.1}");
                        mpv.command("set", &["volume", &volume])
                    }
                    PlayerControl::SelectAudio(id) => {
                        let id = id.to_string();
                        mpv.command("set", &["aid", &id])
                    }
                    PlayerControl::SelectSubtitle(track_id) => {
                        let id = track_id.map_or_else(|| "no".to_string(), |id| id.to_string());
                        mpv.command("set", &["sid", &id])
                    }
                };
                if let Err(error) = result {
                    eprintln!("aviso: comando do player não foi aplicado: {error}");
                }
            }

            #[cfg(windows)]
            if let (Some(parent), Some(renderer)) = (wid, render_context.as_ref()) {
                if renderer.frame_is_pending() {
                    match super::mpv_d3d11::render_to_host(parent, renderer) {
                        Ok(true) => renderer.report_swap(),
                        Ok(false) => {}
                        Err(error) => {
                            eprintln!("erro no renderizador D3D11 do CineHub: {error}");
                            failed = true;
                            break;
                        }
                    }
                }
            }

            #[cfg(all(windows, feature = "mpv"))]
            if !composition_mode {
                if let (Some(parent), false) = (wid, embedded) {
                    if let Ok(video) = mpv.get_property::<i64>("window-id") {
                        if super::win::take_over(&app, parent, video) {
                            embedded = true;
                        }
                    }
                }
            }
            // Guarda a última posição conhecida: depois do fim do arquivo as propriedades não existem mais.
            if let Ok(t) = mpv.get_property::<f64>("time-pos") {
                position = t;
            }
            if let Ok(d) = mpv.get_property::<f64>("duration") {
                duration = d;
            }
            if composition_mode
                && last_state_emit.elapsed() >= std::time::Duration::from_millis(200)
            {
                use tauri::Emitter;
                let (audio_tracks, subtitle_tracks) = collect_tracks(&mpv);
                let state = PlayerPlaybackState {
                    position_ms: (position.max(0.0) * 1000.0) as i64,
                    duration_ms: (duration.max(0.0) * 1000.0) as i64,
                    paused: mpv.get_property::<bool>("pause").unwrap_or(false),
                    volume: mpv.get_property::<f64>("volume").unwrap_or(100.0),
                    muted: mpv.get_property::<bool>("mute").unwrap_or(false),
                    audio_tracks,
                    subtitle_tracks,
                };
                let _ = app.emit("player-state", state);
                last_state_emit = std::time::Instant::now();
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
        if !composition_mode {
            super::win::restore(&app);
        }

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

    /// Executa operações de subclassificação na thread que possui as janelas Tauri/WebView2.
    /// SetWindowSubclass e RemoveWindowSubclass não aceitam janelas pertencentes a outra thread.
    fn run_on_main_thread_sync<T, F>(app: &tauri::AppHandle, task: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (sender, receiver) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let result = task();
            let _ = sender.send(result);
        })
        .map_err(|error| {
            format!("não consegui agendar operação Win32 na thread principal: {error}")
        })?;
        receiver.recv().map_err(|error| {
            format!("a thread principal encerrou antes da operação Win32: {error}")
        })
    }

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
    pub fn take_over(app: &tauri::AppHandle, parent: i64, video: i64) -> bool {
        match run_on_main_thread_sync(app, move || {
            let parent = parent as usize as Hwnd;
            let video = video as usize as Hwnd;
            if parent.is_null() || video.is_null() {
                return false;
            }
            PARENT.store(
                parent as usize as isize,
                std::sync::atomic::Ordering::SeqCst,
            );
            VIDEO.store(video as usize as isize, std::sync::atomic::Ordering::SeqCst);
            // SAFETY: esta closure roda na thread principal, proprietária da janela Tauri
            // e das janelas-filhas do WebView2 que serão subclassificadas.
            unsafe {
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
                // O WebView2 recebe WM_MOUSE* nas próprias janelas-filhas; a enumeração
                // também ocorre na thread proprietária para que SetWindowSubclass seja válida.
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
            !SUBCLASS_WINDOWS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty()
        }) {
            Ok(installed) => installed,
            Err(error) => {
                eprintln!("aviso: não consegui preparar entrada do mpv: {error}");
                false
            }
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

    /// Remove o recorte e as subclasses ao encerrar a reprodução.
    pub fn restore(app: &tauri::AppHandle) {
        if let Err(error) = run_on_main_thread_sync(app, || {
            let video = VIDEO.swap(0, std::sync::atomic::Ordering::SeqCst);
            PARENT.store(0, std::sync::atomic::Ordering::SeqCst);
            // SAFETY: esta closure roda na thread proprietária das janelas subclassificadas.
            unsafe {
                let subclass_windows = std::mem::take(
                    &mut *SUBCLASS_WINDOWS.lock().unwrap_or_else(|e| e.into_inner()),
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
        }) {
            VIDEO.store(0, std::sync::atomic::Ordering::SeqCst);
            PARENT.store(0, std::sync::atomic::Ordering::SeqCst);
            *INPUT_GEOMETRY.lock().unwrap_or_else(|e| e.into_inner()) = None;
            eprintln!("aviso: não consegui restaurar as subclasses do player: {error}");
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
        _app: tauri::AppHandle,
        _controls: std::sync::mpsc::Receiver<super::PlayerControl>,
    ) -> Result<PlayerResult, String> {
        Err("player nativo não incluído nesta versão".into())
    }
}

#[cfg(test)]
mod tests {
    use super::is_playable_url;

    #[test]
    fn player_control_actions_validate_and_clamp_values() {
        assert!(matches!(
            super::parse_player_control("toggle-pause", None),
            Ok(super::PlayerControl::TogglePause)
        ));
        match super::parse_player_control("seek-relative", Some(900.0)) {
            Ok(super::PlayerControl::SeekRelative(seconds)) => assert_eq!(seconds, 600.0),
            _ => panic!("busca relativa deve ser limitada a 600 segundos"),
        }
        assert!(super::parse_player_control("seek-absolute", Some(-1.0)).is_err());
        match super::parse_player_control("set-volume", Some(150.0)) {
            Ok(super::PlayerControl::SetVolume(volume)) => assert_eq!(volume, 100.0),
            _ => panic!("volume deve ser limitado a 100%"),
        }
        assert!(super::parse_player_control("set-volume", Some(f64::NAN)).is_err());
        assert!(matches!(
            super::parse_player_control("select-audio", Some(2.0)),
            Ok(super::PlayerControl::SelectAudio(2))
        ));
        assert!(matches!(
            super::parse_player_control("select-subtitle", None),
            Ok(super::PlayerControl::SelectSubtitle(None))
        ));
        assert!(matches!(
            super::parse_player_control("select-subtitle", Some(3.0)),
            Ok(super::PlayerControl::SelectSubtitle(Some(3)))
        ));
        assert!(super::parse_player_control("select-audio", Some(1.5)).is_err());
        assert!(super::parse_player_control("select-subtitle", Some(-1.0)).is_err());
        assert!(super::parse_player_control("select-audio", None).is_err());
        assert!(super::parse_player_control("unknown", None).is_err());
    }

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
            init.set_option("osd-level", "0")?;
            Ok(())
        });
        assert!(
            result.is_ok(),
            "libmpv rejeitou as opções do OSC: {:?}",
            result.err()
        );
        #[cfg(all(windows, feature = "mpv"))]
        {
            assert!(
                super::mpv_d3d11::abi_layout_is_valid(),
                "a ABI do render API D3D11 precisa corresponder aos headers nativos do libmpv"
            );

            // Runtime smoke test: initialize libmpv's D3D11 render backend, decode
            // a local test frame, render it into a DXGI composition backbuffer, and
            // Present it. The helper tries hardware first and WARP only as CI fallback.
            let d3d_mpv = Mpv::with_initializer(|init| {
                init.set_option("vo", "libmpv")?;
                init.set_option("ao", "null")?;
                init.set_option("hwdec", "no")?;
                init.set_option("osc", "no")?;
                Ok(())
            })
            .expect("libmpv precisa inicializar o render backend libmpv");

            let (device, swap_chain, driver) = super::mpv_d3d11::create_smoke_swap_chain().expect(
                "Windows precisa criar um dispositivo D3D11 e uma swap chain de composição",
            );
            let renderer = super::mpv_d3d11::D3d11RenderContext::new(&d3d_mpv, &device)
                .expect("libmpv precisa criar o render context D3D11");

            let frame_path = std::env::temp_dir()
                .join(format!("cinehub-d3d11-smoke-{}.ppm", std::process::id()));
            let mut ppm = b"P6\n64 64\n255\n".to_vec();
            for y in 0..64 {
                for x in 0..64 {
                    ppm.extend_from_slice(&[
                        if x < 32 { 255 } else { 0 },
                        if y < 32 { 255 } else { 0 },
                        if (x / 8 + y / 8) % 2 == 0 { 255 } else { 32 },
                    ]);
                }
            }
            std::fs::write(&frame_path, ppm).expect("quadro PPM temporário precisa ser gravado");
            let frame_path_string = frame_path.to_string_lossy().into_owned();
            d3d_mpv
                .command("loadfile", &[frame_path_string.as_str(), "replace"])
                .expect("libmpv precisa abrir o quadro PPM de teste");

            let mut rendered_frame = false;
            for _ in 0..100 {
                let _ = d3d_mpv.wait_event(0.05);
                if renderer.frame_is_pending() {
                    renderer.render_to_smoke_swap_chain(&swap_chain).expect(
                        "libmpv precisa renderizar e apresentar o quadro na swap chain DXGI",
                    );
                    rendered_frame = true;
                    break;
                }
            }
            let _ = std::fs::remove_file(&frame_path);
            assert!(
                rendered_frame,
                "libmpv não sinalizou nem renderizou o quadro de teste pelo backend D3D11"
            );
            eprintln!("D3D11 runtime smoke test aprovado usando driver {driver}");

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
