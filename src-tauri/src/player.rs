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

#[tauri::command]
pub async fn player_play(url: String, start_ms: Option<i64>) -> Result<PlayerResult, String> {
    if !is_playable_url(&url) {
        return Err("endereço de vídeo inválido".into());
    }
    let start = start_ms.unwrap_or(0).max(0);
    tauri::async_runtime::spawn_blocking(move || imp::play(&url, start))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(feature = "mpv")]
mod imp {
    use super::PlayerResult;
    use libmpv2::events::Event;
    use libmpv2::{mpv_end_file_reason, Mpv};

    pub fn play(url: &str, start_ms: i64) -> Result<PlayerResult, String> {
        let mpv = Mpv::with_initializer(|init| {
            init.set_option("vo", "gpu")?;
            init.set_option("hwdec", "auto-safe")?;
            // Sem yt-dlp: a URL é sempre um vídeo direto vindo de um addon ou do torrent local.
            init.set_option("ytdl", "no")?;
            init.set_option("force-window", "yes")?;
            init.set_option("title", "CineHub")?;
            Ok(())
        })
        .map_err(|e| format!("não consegui iniciar o mpv: {e}"))?;

        if start_ms > 0 {
            let opts = format!("start={:.3}", start_ms as f64 / 1000.0);
            mpv.command("loadfile", &[url, "replace", "-1", &opts])
        } else {
            mpv.command("loadfile", &[url, "replace"])
        }
        .map_err(|e| format!("o mpv não abriu o vídeo: {e}"))?;

        let mut position = 0f64;
        let mut duration = 0f64;
        loop {
            match mpv.wait_event(0.25) {
                Some(Ok(Event::EndFile(reason))) => {
                    if reason == mpv_end_file_reason::Error {
                        return Err("o mpv não conseguiu tocar este vídeo".into());
                    }
                    break;
                }
                Some(Ok(Event::Shutdown)) => break,
                _ => {}
            }
            // Guarda a última posição conhecida: depois do fim do arquivo as propriedades não existem mais.
            if let Ok(t) = mpv.get_property::<f64>("time-pos") {
                position = t;
            }
            if let Ok(d) = mpv.get_property::<f64>("duration") {
                duration = d;
            }
        }

        Ok(PlayerResult {
            position_ms: (position * 1000.0) as i64,
            duration_ms: (duration * 1000.0) as i64,
        })
    }
}

#[cfg(not(feature = "mpv"))]
mod imp {
    use super::PlayerResult;

    pub fn play(_url: &str, _start_ms: i64) -> Result<PlayerResult, String> {
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
