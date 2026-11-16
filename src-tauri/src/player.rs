//! Player nativo de desktop (libmpv).
//!
//! Abre o vídeo numa janela do mpv (com o decodificador e os codecs do próprio mpv) e responde
//! quando a janela fecha, com a posição final, para a interface salvar o progresso.
//! Só funciona quando o app é compilado com a feature `mpv` (que exige libmpv instalada).
//! Sem a feature, o comando responde com erro e a interface usa o player do navegador.

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerResult {
    pub position_ms: i64,
    pub duration_ms: i64,
}

/// Só aceita http/https: o mpv não pode abrir arquivos locais nem opções de linha de comando.
pub fn is_playable_url(url: &str) -> bool {
    let u = url.trim();
    (u.starts_with("http://") || u.starts_with("https://")) && !u.starts_with('-')
}

/// Handle da janela principal, para o mpv desenhar o vídeo dentro dela (Windows).
/// Em outras plataformas (ou se não der certo) o mpv abre a própria janela.
fn embed_handle(app: &tauri::AppHandle) -> Option<i64> {
    #[cfg(windows)]
    {
        use tauri::Manager;
        let window = app.get_webview_window("main")?;
        let hwnd = window.hwnd().ok()?;
        return Some(hwnd.0 as isize as i64);
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
}

#[tauri::command]
pub async fn player_play(
    app: tauri::AppHandle,
    url: String,
    start_ms: Option<i64>,
) -> Result<PlayerResult, String> {
    if !is_playable_url(&url) {
        return Err("endereço de vídeo inválido".into());
    }
    let start = start_ms.unwrap_or(0).max(0);
    let wid = embed_handle(&app);
    tauri::async_runtime::spawn_blocking(move || imp::play(&url, start, wid))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(feature = "mpv")]
mod imp {
    use super::PlayerResult;
    use libmpv2::events::Event;
    use libmpv2::{mpv_end_file_reason, Mpv};

    pub fn play(url: &str, start_ms: i64, wid: Option<i64>) -> Result<PlayerResult, String> {
        let mpv = Mpv::with_initializer(|init| {
            init.set_option("vo", "gpu")?;
            init.set_option("hwdec", "auto-safe")?;
            // Sem yt-dlp: a URL é sempre um vídeo direto vindo de um addon ou do torrent local.
            init.set_option("ytdl", "no")?;
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

        // Esc também fecha o vídeo (além do q padrão do mpv).
        let _ = mpv.command("keybind", &["ESC", "quit"]);

        if start_ms > 0 {
            let opts = format!("start={:.3}", start_ms as f64 / 1000.0);
            mpv.command("loadfile", &[url, "replace", "-1", &opts])
        } else {
            mpv.command("loadfile", &[url, "replace"])
        }
        .map_err(|e| format!("o mpv não abriu o vídeo: {e}"))?;

        let mut position = 0f64;
        let mut duration = 0f64;
        let mut failed = false;
        // Dentro da janela do app: esconde os controles web e traz o vídeo para frente (Windows).
        #[cfg(all(windows, feature = "mpv"))]
        let mut hidden: Vec<usize> = Vec::new();
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
                    hidden = super::win::take_over(parent, video);
                }
            }
            // Guarda a última posição conhecida: depois do fim do arquivo as propriedades não existem mais.
            if let Ok(t) = mpv.get_property::<f64>("time-pos") {
                position = t;
            }
            if let Ok(d) = mpv.get_property::<f64>("duration") {
                duration = d;
            }
        }

        #[cfg(all(windows, feature = "mpv"))]
        super::win::restore(&hidden);

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
        fn EnumChildWindows(parent: Hwnd, callback: EnumProc, lparam: isize) -> Bool;
        fn ShowWindow(hwnd: Hwnd, cmd: i32) -> Bool;
        fn SetWindowPos(
            hwnd: Hwnd,
            after: Hwnd,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> Bool;
    }

    const SW_HIDE: i32 = 0;
    const SW_SHOW: i32 = 5;
    const HWND_TOP: Hwnd = std::ptr::null_mut();
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_SHOWWINDOW: u32 = 0x0040;

    unsafe extern "system" fn collect(hwnd: Hwnd, lparam: isize) -> Bool {
        // SAFETY: lparam aponta para o Vec criado em `take_over`, que vive até o retorno de EnumChildWindows.
        let list = &mut *(lparam as *mut Vec<usize>);
        list.push(hwnd as usize);
        1
    }

    /// Esconde os filhos da janela principal (a webview) exceto o vídeo, e traz o vídeo para frente.
    /// Devolve os handles escondidos, para `restore`.
    pub fn take_over(parent: i64, video: i64) -> Vec<usize> {
        let video = video as usize as Hwnd;
        let mut children: Vec<usize> = Vec::new();
        // SAFETY: parent é o HWND da janela principal; collect só lê o ponteiro passado.
        unsafe {
            EnumChildWindows(
                parent as usize as Hwnd,
                collect,
                &mut children as *mut Vec<usize> as isize,
            );
        }
        let mut hidden = Vec::new();
        for handle in children {
            if handle == video as usize {
                continue;
            }
            // SAFETY: handles vieram de EnumChildWindows nesta mesma chamada.
            unsafe {
                ShowWindow(handle as Hwnd, SW_HIDE);
            }
            hidden.push(handle);
        }
        // SAFETY: video é o HWND do mpv (propriedade window-id).
        unsafe {
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
        hidden
    }

    /// Mostra de novo os controles web escondidos por `take_over`.
    pub fn restore(hidden: &[usize]) {
        for handle in hidden {
            // SAFETY: handles escondidos por take_over; podem já ter sido destruídos, o que só faz ShowWindow falhar.
            unsafe {
                ShowWindow(*handle as Hwnd, SW_SHOW);
            }
        }
    }
}

#[cfg(not(feature = "mpv"))]
mod imp {
    use super::PlayerResult;

    pub fn play(_url: &str, _start_ms: i64, _wid: Option<i64>) -> Result<PlayerResult, String> {
        Err("player nativo não incluído nesta versão".into())
    }
}

#[cfg(test)]
mod tests {
    use super::is_playable_url;

    #[test]
    fn only_http_urls_reach_mpv() {
        assert!(is_playable_url("https://example.com/video.mkv"));
        assert!(is_playable_url("http://127.0.0.1:9000/stream/0"));
        assert!(!is_playable_url("file:///etc/passwd"));
        assert!(!is_playable_url("--script=/tmp/x.lua"));
        assert!(!is_playable_url("/caminho/local.mp4"));
        assert!(!is_playable_url(""));
    }
}
