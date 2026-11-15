//! CineHub: aplicativo de streaming compatível com os addons do Stremio.
//!
//! O backend (este crate) fala com os addons pelo protocolo do Stremio e expõe comandos
//! que a interface web chama via `invoke`. Funciona no Windows, macOS, Linux, Android e iOS.

pub mod addon;
pub mod commands;
pub mod store;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::RwLock;

use addon::Installed;

pub struct AppState {
    pub http: reqwest::Client,
    pub cache: Arc<addon::Cache>,
    pub addons: RwLock<Vec<Installed>>,
    pub store_path: PathBuf,
}

/// Addon de metadados oficial do Stremio, instalado na primeira execução.
const DEFAULT_ADDON: &str = "https://v3-cinemeta.strem.io/manifest.json";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let store_path = data_dir.join("addons.json");
            let installed = store::load(&store_path);
            let needs_default = installed.is_empty();

            let http = addon::client();
            app.manage(AppState {
                http,
                cache: Arc::new(addon::Cache::default()),
                addons: RwLock::new(installed),
                store_path,
            });

            if needs_default {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<AppState>();
                    match addon::fetch_manifest(&state.http, DEFAULT_ADDON).await {
                        Ok(manifest) => {
                            let mut list = state.addons.write().await;
                            list.push(Installed {
                                transport_url: DEFAULT_ADDON.into(),
                                manifest,
                            });
                            if let Err(e) = store::save(&state.store_path, &list) {
                                eprintln!("aviso: não consegui salvar os addons: {e}");
                            }
                        }
                        Err(e) => eprintln!("Não consegui instalar o Cinemeta agora ({e})."),
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_addons,
            commands::install_addon,
            commands::remove_addon,
            commands::home,
            commands::search,
            commands::meta,
            commands::streams,
            commands::subtitle,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o CineHub");
}
