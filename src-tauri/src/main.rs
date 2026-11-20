// Evita a janela de console extra no Windows em builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The Windows libmpv package is the integration build for the experimental
    // WebView2/DirectComposition path. Set this before Tauri creates its WebView.
    // Set CINEHUB_DISABLE_WEBVIEW2_COMPOSITION=1 to force the standard HWND path.
    #[cfg(all(windows, feature = "mpv"))]
    {
        if std::env::var_os("CINEHUB_DISABLE_WEBVIEW2_COMPOSITION").is_some() {
            std::env::remove_var("CINEHUB_WEBVIEW2_COMPOSITION_POC");
        } else {
            std::env::set_var("CINEHUB_WEBVIEW2_COMPOSITION_POC", "1");
        }
    }

    cinehub_lib::run();
}
