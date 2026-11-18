//! CineHub: aplicativo de streaming compatível com os addons do Stremio.
//!
//! O backend (este crate) fala com os addons pelo protocolo do Stremio e expõe comandos
//! que a interface web chama via `invoke`. Roda no Windows, macOS, Linux, Android e Android TV.

pub mod addon;
pub mod commands;
pub mod diag;
pub mod kodi;
pub mod player;
pub mod skip;
pub mod store;
pub mod torrent;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::RwLock;

use addon::Installed;

pub struct AppState {
    pub http: reqwest::Client,
    /// Cliente para legendas: não segue redirecionamentos automáticos (validados à parte).
    pub sub_http: reqwest::Client,
    pub cache: Arc<addon::Cache>,
    pub addons: RwLock<Vec<Installed>>,
    pub store_path: PathBuf,
    /// Motor de torrent. `None` se ele não puder ser iniciado: o resto do app segue funcionando.
    pub torrent: Option<Arc<torrent::TorrentEngine>>,
    /// Garante que as gravações de addons.json aconteçam uma de cada vez.
    pub save_lock: tokio::sync::Mutex<()>,
}

/// Addon de metadados oficial do Stremio, instalado na primeira execução.
const DEFAULT_ADDON: &str = "https://v3-cinemeta.strem.io/manifest.json";

/// Instala o Cinemeta com novas tentativas (espera crescente), pois a rede pode não estar pronta
/// logo ao abrir o app.
async fn install_default_addon(handle: tauri::AppHandle) {
    let state = handle.state::<AppState>();
    let mut delay = std::time::Duration::from_secs(2);
    for attempt in 1..=5 {
        match addon::fetch_manifest(&state.http, DEFAULT_ADDON).await {
            Ok(manifest) => {
                let _save = state.save_lock.lock().await;
                let snapshot = {
                    let mut list = state.addons.write().await;
                    if list.iter().any(|a| a.transport_url == DEFAULT_ADDON) {
                        return;
                    }
                    list.push(Installed {
                        transport_url: DEFAULT_ADDON.into(),
                        manifest,
                    });
                    list.clone()
                };
                if let Err(e) = store::save(&state.store_path, &snapshot) {
                    eprintln!("aviso: não consegui salvar os addons: {e}");
                }
                return;
            }
            Err(e) => {
                eprintln!("Cinemeta: tentativa {attempt} falhou ({e}).");
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }
}

/// Esc global, só durante o vídeo (o player registra e solta o atalho). Não depende do foco.
#[cfg(not(target_os = "android"))]
fn desktop_shortcuts<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_global_shortcut::ShortcutState;
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|_app, _shortcut, event| {
            if matches!(event.state, ShortcutState::Pressed) {
                player::request_close();
            }
        })
        .build()
}

#[cfg(target_os = "android")]
fn desktop_shortcuts<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("desktop-shortcuts").build()
}

/// Diálogo nativo "Salvar como" (desktop). No Android não existe: o plugin fica de fora.
#[cfg(not(target_os = "android"))]
fn desktop_dialog<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_dialog::init()
}

#[cfg(target_os = "android")]
fn desktop_dialog<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("desktop-dialog").build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_exoplayer::init())
        .plugin(desktop_shortcuts())
        .plugin(desktop_dialog())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let store_path = data_dir.join("addons.json");
            let installed = store::load(&store_path);
            let needs_default = installed.is_empty();
            let torrent_dir = data_dir.join("torrents");
            let torrent = match torrent::TorrentEngine::new(torrent_dir) {
                Ok(engine) => {
                    let engine = Arc::new(engine);
                    let serve_torrent = engine.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = serve_torrent.serve().await {
                            eprintln!("servidor de torrent local falhou: {e}");
                        }
                    });
                    Some(engine)
                }
                Err(e) => {
                    eprintln!("aviso: motor de torrent indisponível: {e}");
                    None
                }
            };

            app.manage(AppState {
                http: addon::client(),
                sub_http: addon::client_no_redirect(),
                cache: Arc::new(addon::Cache::default()),
                addons: RwLock::new(installed),
                store_path,
                torrent,
                save_lock: tokio::sync::Mutex::new(()),
            });

            if needs_default {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(install_default_addon(handle));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_addons,
            commands::install_addon,
            commands::remove_addon,
            commands::home,
            commands::search,
            commands::catalog_page,
            commands::meta,
            commands::streams,
            commands::subtitle,
            commands::torrent_stats,
            commands::torrent_open,
            commands::torrent_select_file,
            commands::save_export,
            commands::write_export,
            kodi::kodi_addons,
            kodi::kodi_browse,
            kodi::kodi_play,
            diag::log_event,
            diag::diagnostics_text,
            diag::save_diagnostics,
            player::player_play,
            skip::skip_segments,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o CineHub");
}
