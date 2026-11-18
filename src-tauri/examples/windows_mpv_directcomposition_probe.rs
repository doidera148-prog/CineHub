//! Protótipo Windows: conecta a swapchain D3D11 do libmpv a uma árvore
//! DirectComposition associada a um HWND existente (por exemplo, a janela do CineHub).
//!
//! Uso:
//! cargo run --manifest-path src-tauri/Cargo.toml --features mpv --example windows_mpv_directcomposition_probe -- "https://servidor/video.mkv" 0x123456
//!
//! O HWND pode ser obtido com uma ferramenta como Spy++/Inspect. O protótipo
//! desenha a composição na camada topmost da janela; portanto, pode cobrir a
//! WebView inteira. Não é uma integração de controles HTML.

#[cfg(not(windows))]
fn main() {
    eprintln!("Este protótipo só pode ser executado no Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
mod windows_probe {
    use libmpv2::{events::Event, Mpv};
    use std::{ffi::c_void, mem::transmute, ptr};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const IID_IDXGI_DEVICE: Guid = Guid {
        data1: 0x54ec77fa,
        data2: 0x1377,
        data3: 0x44e6,
        data4: [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c],
    };
    const IID_DCOMP_DEVICE: Guid = Guid {
        data1: 0xc37ea93a,
        data2: 0xe7aa,
        data3: 0x450d,
        data4: [0xb1, 0x6f, 0x97, 0x46, 0xcb, 0x04, 0x07, 0xf3],
    };

    #[link(name = "dcomp")]
    unsafe extern "system" {
        fn DCompositionCreateDevice(
            dxgi_device: *mut c_void,
            iid: *const Guid,
            result: *mut *mut c_void,
        ) -> i32;
    }

    struct ComPtr(*mut c_void);

    impl ComPtr {
        fn new() -> Self {
            Self(ptr::null_mut())
        }

        fn as_ptr(&self) -> *mut c_void {
            self.0
        }
    }

    impl Drop for ComPtr {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let table = *(self.0 as *mut *mut *mut c_void);
                    let release: unsafe extern "system" fn(*mut c_void) -> u32 =
                        transmute(*table.add(2));
                    release(self.0);
                }
            }
        }
    }

    fn check(hr: i32, operation: &str) -> Result<(), String> {
        if hr < 0 {
            Err(format!("{operation} falhou (HRESULT 0x{:08x})", hr as u32))
        } else {
            Ok(())
        }
    }

    unsafe fn call_get_device(swapchain: *mut c_void, device: &mut ComPtr) -> Result<(), String> {
        // IDXGISwapChain herda IDXGIDeviceSubObject; GetDevice é o slot 7.
        let table = *(swapchain as *mut *mut *mut c_void);
        let get_device: unsafe extern "system" fn(
            *mut c_void,
            *const Guid,
            *mut *mut c_void,
        ) -> i32 = transmute(*table.add(7));
        check(
            get_device(swapchain, &IID_IDXGI_DEVICE, &mut device.0),
            "IDXGISwapChain::GetDevice",
        )
    }

    unsafe fn attach_to_hwnd(
        swapchain: *mut c_void,
        hwnd: isize,
    ) -> Result<(ComPtr, ComPtr, ComPtr), String> {
        let mut dxgi_device = ComPtr::new();
        call_get_device(swapchain, &mut dxgi_device)?;

        let mut dcomp_device = ComPtr::new();
        check(
            DCompositionCreateDevice(
                dxgi_device.as_ptr(),
                &IID_DCOMP_DEVICE,
                &mut dcomp_device.0,
            ),
            "DCompositionCreateDevice",
        )?;

        // IDCompositionDevice: CreateTargetForHwnd = slot 6; CreateVisual = slot 7.
        let device_table = *(dcomp_device.as_ptr() as *mut *mut *mut c_void);
        let create_target: unsafe extern "system" fn(
            *mut c_void,
            *mut c_void,
            i32,
            *mut *mut c_void,
        ) -> i32 = transmute(*device_table.add(6));
        let create_visual: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
            transmute(*device_table.add(7));

        let mut target = ComPtr::new();
        check(
            create_target(
                dcomp_device.as_ptr(),
                hwnd as usize as *mut c_void,
                1, // topmost: sobre os filhos da janela (incluindo a WebView).
                &mut target.0,
            ),
            "IDCompositionDevice::CreateTargetForHwnd",
        )?;

        let mut visual = ComPtr::new();
        check(
            create_visual(dcomp_device.as_ptr(), &mut visual.0),
            "IDCompositionDevice::CreateVisual",
        )?;

        // IDCompositionVisual::SetContent = slot 11.
        let visual_table = *(visual.as_ptr() as *mut *mut *mut c_void);
        let set_content: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 =
            transmute(*visual_table.add(11));
        check(
            set_content(visual.as_ptr(), swapchain),
            "IDCompositionVisual::SetContent",
        )?;

        // IDCompositionTarget::SetRoot = slot 3.
        let target_table = *(target.as_ptr() as *mut *mut *mut c_void);
        let set_root: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 =
            transmute(*target_table.add(3));
        check(
            set_root(target.as_ptr(), visual.as_ptr()),
            "IDCompositionTarget::SetRoot",
        )?;

        // IDCompositionDevice::Commit = slot 3.
        let commit: unsafe extern "system" fn(*mut c_void) -> i32 = transmute(*device_table.add(3));
        check(commit(dcomp_device.as_ptr()), "IDCompositionDevice::Commit")?;

        println!("DIRECTCOMPOSITION_ATTACHED=1");
        println!("DIRECTCOMPOSITION_HWND=0x{:x}", hwnd as usize);
        println!("DIRECTCOMPOSITION_TOPMOST=1");
        Ok((dcomp_device, target, visual))
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let mut args = std::env::args().skip(1);
        let url = args.next().ok_or("informe a URL HTTP/HTTPS do vídeo")?;
        let hwnd_arg = args
            .next()
            .ok_or("informe o HWND da janela do CineHub (ex.: 0x123456)")?;
        if args.next().is_some() {
            return Err("argumentos extras não são aceitos".into());
        }
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("somente URLs HTTP/HTTPS são aceitas".into());
        }
        let hwnd = isize::from_str_radix(hwnd_arg.trim_start_matches("0x"), 16)
            .or_else(|_| hwnd_arg.parse::<isize>())
            .map_err(|_| "HWND inválido")?;
        if hwnd == 0 {
            return Err("HWND não pode ser zero".into());
        }

        let mpv = Mpv::with_initializer(|init| {
            init.set_option("vo", "gpu-next")?;
            init.set_option("gpu-context", "d3d11")?;
            init.set_option("d3d11-output-mode", "composition")?;
            init.set_option("d3d11-composition-size", "1280x720")?;
            init.set_option("hwdec", "auto-safe")?;
            init.set_option("ytdl", "no")?;
            init.set_option("terminal", "no")?;
            init.set_option("keep-open", "yes")?;
            Ok(())
        })?;
        mpv.command("loadfile", &[url.as_str(), "replace"])?;

        let mut attached = false;
        let mut dcomp: Option<(ComPtr, ComPtr, ComPtr)> = None;
        for _ in 0..2400 {
            match mpv.wait_event(0.05) {
                Some(Ok(Event::EndFile(_))) => break,
                Some(Ok(Event::Shutdown)) => break,
                _ => {}
            }
            if !attached {
                if let Ok(handle) = mpv.get_property::<i64>("display-swapchain") {
                    if handle != 0 {
                        let swapchain = handle as usize as *mut c_void;
                        // A swapchain pertence ao libmpv e permanece válida enquanto Mpv vive.
                        dcomp = Some(unsafe { attach_to_hwnd(swapchain, hwnd) }?);
                        attached = true;
                        println!("Reprodução ativa. Feche o vídeo ou aguarde o fim do arquivo.");
                    }
                }
            }
        }

        if let Some((_device, _target, _visual)) = dcomp.take() {
            println!("DirectComposition será liberado antes do encerramento do libmpv.");
        }
        if !attached {
            return Err("libmpv não disponibilizou display-swapchain em 120 segundos".into());
        }
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows_probe::run()
}
