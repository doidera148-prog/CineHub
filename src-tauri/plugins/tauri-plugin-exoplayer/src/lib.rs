//! Player nativo (ExoPlayer/Media3) do CineHub. Só existe no Android: nas outras
//! plataformas o plugin não faz nada e a interface usa o player do navegador.
//!
//! O comando `play` é implementado em Kotlin (`android/.../ExoPlayerPlugin.kt`).
//! A interface chama `plugin:exoplayer|play` e recebe `{ positionMs, durationMs }`
//! quando o usuário fecha o vídeo.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::Runtime;

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("exoplayer")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            _api.register_android_plugin("com.cinehub.exoplayer", "ExoPlayerPlugin")?;
            Ok(())
        })
        .build()
}
