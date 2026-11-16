//! CineHub: aplicativo de streaming compatível com os addons do Stremio.
//!
//! O backend (este crate) fala com os addons pelo protocolo do Stremio e expõe comandos
//! que a interface web chama via `invoke`. Funciona no Windows, macOS, Linux, Android e iOS.

pub mod addon;
pub mod commands;
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
    pub torrent: Arc<torrent::TorrentEngine>,
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let store_path = data_dir.join("addons.json");
            let installed = store::load(&store_path);
            let needs_default = installed.is_empty();
            let torrent_dir = data_dir.join("torrents");
            let torrent = Arc::new(torrent::TorrentEngine::new(torrent_dir).map_err(|e| {
                std::io::Error::other(format!("não foi possível iniciar o motor de torrent: {e}"))
            })?);

            let serve_torrent = torrent.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = serve_torrent.serve().await {
                    eprintln!("servidor de torrent local falhou: {e}");
                }
            });

            app.manage(AppState {
                http: addon::client(),
                sub_http: addon::client_no_redirect(),
                cache: Arc::new(addon::Cache::default()),
                addons: RwLock::new(installed),
                store_path,
                torrent,
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
            commands::torrent_open,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o CineHub");
}
