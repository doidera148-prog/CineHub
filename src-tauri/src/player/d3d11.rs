//! Ponte de renderização Direct3D 11 para a API nativa do libmpv.
//!
//! O wrapper libmpv2 7.0.1 expõe o render API OpenGL, mas não modela os
//! parâmetros D3D11. Esta camada usa a ABI C documentada pelo libmpv 0.40+.
//! O dispositivo e a textura devem pertencer ao mesmo ID3D11Device.
//!
//! O Wry cria e mantém o dispositivo/swap chain compartilhado; esta ponte
//! fornece ao libmpv o mesmo dispositivo e os backbuffers apresentados pelo host.
//! O caminho experimental permanece opt-in; o player padrão usa a janela nativa.

use std::{
    ffi::c_void,
    fmt,
    marker::PhantomData,
    mem,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use libmpv2::Mpv;
#[cfg(test)]
use windows::Win32::Graphics::Dxgi::IDXGISwapChain1;
use windows::{
    core::Interface,
    Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D},
};

const MPV_RENDER_PARAM_INVALID: u32 = 0;
const MPV_RENDER_PARAM_API_TYPE: u32 = 1;
const MPV_RENDER_PARAM_D3D11_INIT_PARAMS: u32 = 21;
const MPV_RENDER_PARAM_D3D11_FBO: u32 = 22;
const MPV_RENDER_UPDATE_FRAME: u64 = 1;
const MPV_RENDER_API_TYPE_D3D11: &[u8] = b"d3d11\0";

#[repr(C)]
struct MpvRenderContextOpaque {
    _private: [u8; 0],
}

#[repr(C)]
struct MpvRenderParam {
    type_: u32,
    data: *mut c_void,
}

impl MpvRenderParam {
    fn terminator() -> Self {
        Self {
            type_: MPV_RENDER_PARAM_INVALID,
            data: std::ptr::null_mut(),
        }
    }
}

#[repr(C)]
struct MpvD3d11InitParams {
    device: *mut c_void,
}

#[repr(C)]
struct MpvD3d11Fbo {
    tex: *mut c_void,
}

// These entry points are provided by the same libmpv import library linked by
// libmpv2-sys. The ABI structs/constants are kept local because libmpv2-sys
// 4.0.1's generated bindings predate the D3D11 render header.
extern "C" {
    fn mpv_render_context_create(
        context: *mut *mut MpvRenderContextOpaque,
        mpv: *mut c_void,
        params: *mut MpvRenderParam,
    ) -> i32;
    fn mpv_render_context_update(context: *mut MpvRenderContextOpaque) -> u64;
    fn mpv_render_context_render(
        context: *mut MpvRenderContextOpaque,
        params: *mut MpvRenderParam,
    ) -> i32;
    fn mpv_render_context_report_swap(context: *mut MpvRenderContextOpaque);
    fn mpv_render_context_set_update_callback(
        context: *mut MpvRenderContextOpaque,
        callback: Option<unsafe extern "C" fn(*mut c_void)>,
        callback_ctx: *mut c_void,
    );
    fn mpv_render_context_free(context: *mut MpvRenderContextOpaque);
}

/// D3D11 render context; must be dropped before its associated Mpv.
///
/// libmpv takes its own COM reference to the device during creation. The caller
/// must keep each target texture alive for the duration of render(). The update
/// callback only sets this atomic flag; all mpv_render_* calls stay on the render
/// thread, never inside libmpv's callback thread.
pub(super) struct D3d11RenderContext<'a> {
    context: NonNull<MpvRenderContextOpaque>,
    device: ID3D11Device,
    frame_pending: Arc<AtomicBool>,
    _mpv: PhantomData<&'a Mpv>,
}

#[derive(Debug)]
pub(super) enum RenderError {
    DeviceLost {
        operation: &'static str,
        hresult: i32,
        removal_reason: i32,
    },
    Other(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceLost {
                operation,
                hresult,
                removal_reason,
            } => write!(
                formatter,
                "dispositivo D3D11 removido durante {operation} (HRESULT 0x{:08X}; motivo 0x{:08X})",
                *hresult as u32,
                *removal_reason as u32
            ),
            Self::Other(message) => formatter.write_str(message),
        }
    }
}

pub(super) fn is_device_loss_hresult(hresult: i32) -> bool {
    matches!(
        hresult as u32,
        0x887A0005 // DXGI_ERROR_DEVICE_REMOVED
            | 0x887A0006 // DXGI_ERROR_DEVICE_HUNG
            | 0x887A0007 // DXGI_ERROR_DEVICE_RESET
            | 0x887A0020 // DXGI_ERROR_DRIVER_INTERNAL_ERROR
    )
}

unsafe extern "C" fn notify_render_update(callback_ctx: *mut c_void) {
    let Some(pending) = (unsafe { (callback_ctx as *const AtomicBool).as_ref() }) else {
        return;
    };
    pending.store(true, Ordering::Release);
}

impl<'a> D3d11RenderContext<'a> {
    /// Creates libmpv's D3D11 renderer using a host-owned D3D11 device.
    ///
    /// Returns an error when the installed libmpv build lacks D3D11 support.
    pub(super) fn new(mpv: &'a Mpv, device: &ID3D11Device) -> Result<Self, String> {
        let mut context = std::ptr::null_mut();
        let mut init = MpvD3d11InitParams {
            device: device.as_raw(),
        };
        let mut params = [
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_API_TYPE,
                data: MPV_RENDER_API_TYPE_D3D11.as_ptr() as *mut c_void,
            },
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_D3D11_INIT_PARAMS,
                data: &mut init as *mut MpvD3d11InitParams as *mut c_void,
            },
            MpvRenderParam::terminator(),
        ];

        // SAFETY: mpv is initialized and alive for 'a; device is a live
        // ID3D11Device and the parameter storage remains valid for the call.
        let result = unsafe {
            mpv_render_context_create(
                &mut context,
                mpv.ctx.as_ptr().cast::<c_void>(),
                params.as_mut_ptr(),
            )
        };
        if result < 0 {
            if !context.is_null() {
                // SAFETY: a non-null context returned on failure is still owned here.
                unsafe { mpv_render_context_free(context) };
            }
            return Err(format!(
                "libmpv não conseguiu criar o renderizador D3D11 (código {result})"
            ));
        }

        let context = NonNull::new(context)
            .ok_or_else(|| "libmpv retornou um render context D3D11 nulo".to_string())?;
        let frame_pending = Arc::new(AtomicBool::new(false));
        // SAFETY: frame_pending is owned by this wrapper and remains alive until
        // after the callback is detached and mpv_render_context_free returns.
        unsafe {
            mpv_render_context_set_update_callback(
                context.as_ptr(),
                Some(notify_render_update),
                Arc::as_ptr(&frame_pending) as *mut c_void,
            );
        }
        Ok(Self {
            context,
            device: device.clone(),
            frame_pending,
            _mpv: PhantomData,
        })
    }

    /// Returns libmpv render-update flags. Bit 0 requests a new video frame.
    pub(super) fn update(&self) -> u64 {
        // SAFETY: the context is valid until this wrapper is dropped.
        unsafe { mpv_render_context_update(self.context.as_ptr()) }
    }

    fn render_failure(
        &self,
        operation: &'static str,
        hresult: i32,
        message: String,
    ) -> RenderError {
        // Capture the native removal reason before the old device is released.
        let removal_reason = unsafe { self.device.GetDeviceRemovedReason().0 };
        if is_device_loss_hresult(hresult) || is_device_loss_hresult(removal_reason) {
            RenderError::DeviceLost {
                operation,
                hresult,
                removal_reason,
            }
        } else {
            RenderError::Other(format!(
                "{message} (HRESULT 0x{:08X}; GetDeviceRemovedReason 0x{:08X})",
                hresult as u32, removal_reason as u32
            ))
        }
    }

    /// Renders the current frame into a render-target texture from the same device.
    pub(super) fn render(&self, texture: &ID3D11Texture2D) -> Result<(), RenderError> {
        let mut target = MpvD3d11Fbo {
            tex: texture.as_raw(),
        };
        let mut params = [
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_D3D11_FBO,
                data: &mut target as *mut MpvD3d11Fbo as *mut c_void,
            },
            MpvRenderParam::terminator(),
        ];

        // SAFETY: target is a live ID3D11Texture2D created on the same
        // device supplied to new(); libmpv consumes it synchronously.
        let result =
            unsafe { mpv_render_context_render(self.context.as_ptr(), params.as_mut_ptr()) };
        if result < 0 {
            return Err(self.render_failure(
                "mpv_render_context_render",
                result,
                format!("libmpv falhou ao renderizar o frame D3D11 (código {result})"),
            ));
        }
        Ok(())
    }

    /// Reports that the host presented the frame rendered by render().
    pub(super) fn report_swap(&self) {
        // SAFETY: the render context remains valid and the host has presented it.
        unsafe { mpv_render_context_report_swap(self.context.as_ptr()) };
    }

    pub(super) fn frame_is_pending(&self) -> bool {
        if !self.frame_pending.swap(false, Ordering::AcqRel) {
            return false;
        }
        self.update() & MPV_RENDER_UPDATE_FRAME != 0
    }
}

impl Drop for D3d11RenderContext<'_> {
    fn drop(&mut self) {
        // Detach the callback first. Its context pointer refers to frame_pending,
        // which remains alive until this Drop implementation has finished.
        // SAFETY: this wrapper uniquely owns the native context and drops
        // before Mpv due to the lifetime marker.
        unsafe {
            mpv_render_context_set_update_callback(
                self.context.as_ptr(),
                None,
                std::ptr::null_mut(),
            );
            mpv_render_context_free(self.context.as_ptr());
        }
    }
}

unsafe extern "C" {
    fn cinehub_wry_composition_get_device(parent: isize) -> *mut c_void;
    fn cinehub_wry_composition_get_backbuffer(parent: isize) -> *mut c_void;
    fn cinehub_wry_composition_get_backbuffer_error(parent: isize) -> i32;
    fn cinehub_wry_composition_present(parent: isize) -> i32;
    fn cinehub_wry_composition_recreate_device(parent: isize) -> i32;
    fn cinehub_wry_composition_set_video_rect(
        parent: isize,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> i32;
}

/// Gets an additional COM reference to the D3D11 device owned by the Wry host.
pub(super) fn host_device(parent: i64) -> Result<ID3D11Device, String> {
    let raw = unsafe { cinehub_wry_composition_get_device(parent as isize) };
    let raw = NonNull::new(raw)
        .ok_or_else(|| "o host WebView2 não registrou um dispositivo D3D11".to_string())?;
    // SAFETY: Wry returns an additional COM reference owned by this caller.
    Ok(unsafe { ID3D11Device::from_raw(raw.as_ptr()) })
}

/// Renders to the shared swap-chain backbuffer and presents it.
pub(super) fn render_to_host(
    parent: i64,
    context: &D3d11RenderContext<'_>,
) -> Result<bool, RenderError> {
    let raw = unsafe { cinehub_wry_composition_get_backbuffer(parent as isize) };
    let Some(raw) = NonNull::new(raw) else {
        let error = unsafe { cinehub_wry_composition_get_backbuffer_error(parent as isize) };
        if error < 0 {
            return Err(context.render_failure(
                "IDXGISwapChain::ResizeBuffers/GetBuffer",
                error,
                "DXGI não conseguiu adquirir o backbuffer do CineHub".to_string(),
            ));
        }
        // A janela está minimizada ou a geometria do player está temporariamente vazia.
        return Ok(false);
    };
    // SAFETY: Wry transfers one COM reference to the current backbuffer.
    let texture = unsafe { ID3D11Texture2D::from_raw(raw.as_ptr()) };
    context.render(&texture)?;
    // Release the backbuffer reference before Present or a later ResizeBuffers.
    drop(texture);
    let result = unsafe { cinehub_wry_composition_present(parent as isize) };
    if result < 0 {
        return Err(context.render_failure(
            "IDXGISwapChain::Present",
            result,
            "DXGI não conseguiu apresentar o frame do CineHub".to_string(),
        ));
    }
    Ok(result == 0)
}

/// Rebuilds the Wry D3D11 and DirectComposition resources on the window-owning thread.
pub(super) fn recreate_host_device(parent: i64) -> Result<(), String> {
    let result = unsafe { cinehub_wry_composition_recreate_device(parent as isize) };
    if result < 0 {
        return Err(format!(
            "Wry não conseguiu recriar o dispositivo/superfície de composição (HRESULT 0x{:08X})",
            result as u32
        ));
    }
    Ok(())
}

/// Creates the same kind of composition swap chain used by Wry for the Windows CI smoke test.
///
/// Hardware is preferred; WARP is a deterministic software fallback for hosted
/// runners without a usable D3D11 adapter. This helper is never used by playback.
#[cfg(test)]
pub(super) fn create_smoke_swap_chain(
) -> Result<(ID3D11Device, IDXGISwapChain1, &'static str), String> {
    use windows::Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP},
            Direct3D11::{
                D3D11CreateDevice, ID3D11Multithread, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS, D3D11_SDK_VERSION,
            },
            Dxgi::Common::{
                DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
            },
            Dxgi::{
                IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGIOutput, DXGI_SCALING_STRETCH,
                DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
        },
    };

    let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT
        | D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS;
    let mut failures = Vec::new();

    for (driver, label) in [
        (D3D_DRIVER_TYPE_HARDWARE, "hardware"),
        (D3D_DRIVER_TYPE_WARP, "WARP"),
    ] {
        let mut device = None;
        if let Err(error) = unsafe {
            D3D11CreateDevice(
                None::<&IDXGIAdapter>,
                driver,
                HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        } {
            failures.push(format!("{label}: D3D11CreateDevice: {error}"));
            continue;
        }

        let Some(device) = device else {
            failures.push(format!(
                "{label}: D3D11CreateDevice retornou dispositivo nulo"
            ));
            continue;
        };

        let multithread = match device.cast::<ID3D11Multithread>() {
            Ok(multithread) => multithread,
            Err(error) => {
                failures.push(format!("{label}: ID3D11Multithread: {error}"));
                continue;
            }
        };
        let _ = unsafe { multithread.SetMultithreadProtected(true) };

        let dxgi_device = match device.cast::<IDXGIDevice>() {
            Ok(dxgi_device) => dxgi_device,
            Err(error) => {
                failures.push(format!("{label}: IDXGIDevice: {error}"));
                continue;
            }
        };
        let adapter = match unsafe { dxgi_device.GetAdapter() } {
            Ok(adapter) => adapter,
            Err(error) => {
                failures.push(format!("{label}: GetAdapter: {error}"));
                continue;
            }
        };
        let factory: IDXGIFactory2 = match unsafe { adapter.GetParent() } {
            Ok(factory) => factory,
            Err(error) => {
                failures.push(format!("{label}: IDXGIFactory2: {error}"));
                continue;
            }
        };

        let desc = windows::Win32::Graphics::Dxgi::DXGI_SWAP_CHAIN_DESC1 {
            Width: 64,
            Height: 64,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: Default::default(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        let swap_chain = match unsafe {
            factory.CreateSwapChainForComposition(&device, &desc, None::<&IDXGIOutput>)
        } {
            Ok(swap_chain) => swap_chain,
            Err(error) => {
                failures.push(format!("{label}: CreateSwapChainForComposition: {error}"));
                continue;
            }
        };

        return Ok((device, swap_chain, label));
    }

    Err(format!(
        "não foi possível criar dispositivo/swap chain D3D11 para o smoke test ({})",
        failures.join("; ")
    ))
}

#[cfg(test)]
impl D3d11RenderContext<'_> {
    /// Exercises libmpv rendering into a DXGI composition backbuffer and Present.
    pub(super) fn render_to_smoke_swap_chain(
        &self,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<(), String> {
        let texture = unsafe { swap_chain.GetBuffer::<ID3D11Texture2D>(0) }
            .map_err(|error| format!("DXGI não conseguiu obter o backbuffer: {error}"))?;
        self.render(&texture).map_err(|error| error.to_string())?;
        drop(texture);

        let present_result =
            unsafe { swap_chain.Present(0, windows::Win32::Graphics::Dxgi::DXGI_PRESENT(0)) };
        if present_result.0 < 0 {
            let reason = unsafe { self.device.GetDeviceRemovedReason().0 };
            if is_device_loss_hresult(present_result.0) || is_device_loss_hresult(reason) {
                return Err(format!(
                    "DXGI Present indicou perda do dispositivo (HRESULT 0x{:08X}; motivo 0x{:08X})",
                    present_result.0 as u32, reason as u32
                ));
            }
            return Err(format!(
                "DXGI Present falhou no smoke test (HRESULT 0x{:08X})",
                present_result.0 as u32
            ));
        }
        self.report_swap();
        Ok(())
    }
}

/// Checks the C ABI layout used by libmpv's render.h/render_d3d11.h headers.
pub(super) fn abi_layout_is_valid() -> bool {
    mem::size_of::<MpvRenderParam>() == mem::size_of::<usize>() * 2
        && mem::offset_of!(MpvRenderParam, data) == mem::size_of::<usize>()
        && mem::size_of::<MpvD3d11InitParams>() == mem::size_of::<usize>()
        && mem::size_of::<MpvD3d11Fbo>() == mem::size_of::<usize>()
        && MPV_RENDER_PARAM_API_TYPE == 1
        && MPV_RENDER_PARAM_D3D11_INIT_PARAMS == 21
        && MPV_RENDER_PARAM_D3D11_FBO == 22
        && MPV_RENDER_API_TYPE_D3D11 == b"d3d11\0"
}

/// Moves/resizes the video visual on the thread that owns the WebView HWND.
pub(super) fn set_video_rect(parent: i64, x: i32, y: i32, width: u32, height: u32) -> bool {
    unsafe { cinehub_wry_composition_set_video_rect(parent as isize, x, y, width, height) == 0 }
}
