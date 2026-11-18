//! Protótipo isolado: verifica se o libmpv cria uma swapchain D3D11
//! no modo composition do Windows, sem criar uma janela de vídeo.
//!
//! Execute em um Windows com libmpv compilado com D3D11:
//! cargo run --manifest-path src-tauri/Cargo.toml --features mpv --example windows_mpv_composition_probe -- "https://servidor/video.mkv"
//!
//! Este probe NÃO integra a imagem à WebView. Valida apenas a primeira etapa
//! da arquitetura: obter a swapchain que uma camada nativa consumiria.

#[cfg(not(windows))]
fn main() {
    eprintln!("Este protótipo só pode ser executado no Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use libmpv2::{events::Event, Mpv};

    let url = std::env::args()
        .nth(1)
        .ok_or("informe uma URL HTTP/HTTPS de vídeo como primeiro argumento")?;

    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("somente URLs HTTP/HTTPS são aceitas".into());
    }

    let mpv = Mpv::with_initializer(|init| {
        init.set_option("vo", "gpu-next")?;
        init.set_option("gpu-context", "d3d11")?;
        init.set_option("d3d11-output-mode", "composition")?;
        init.set_option("d3d11-composition-size", "1280x720")?;
        init.set_option("hwdec", "auto-safe")?;
        init.set_option("ytdl", "no")?;
        init.set_option("force-window", "yes")?;
        init.set_option("terminal", "no")?;
        Ok(())
    })?;

    mpv.command("loadfile", &[url.as_str(), "replace"])?;

    // A swapchain pode só existir depois que o VO/GPU for inicializado.
    let mut started = false;
    for _ in 0..100 {
        match mpv.wait_event(0.1) {
            Some(Ok(Event::FileLoaded)) => started = true,
            Some(Ok(Event::EndFile(_))) => break,
            _ => {}
        }

        if let Ok(handle) = mpv.get_property::<i64>("display-swapchain") {
            if handle != 0 {
                println!("MPV_D3D11_COMPOSITION_SWAPCHAIN=0x{handle:x}");
                println!("Arquivo carregado: {started}");
                println!("Probe concluído: swapchain D3D11 disponível.");
                return Ok(());
            }
        }
    }

    Err("libmpv não expôs display-swapchain; confira o build D3D11 e o suporte a d3d11-output-mode=composition".into())
}
