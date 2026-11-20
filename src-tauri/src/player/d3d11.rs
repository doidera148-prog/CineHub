//! Ponte de renderização Direct3D 11 para a API nativa do libmpv.
//!
//! O wrapper libmpv2 7.0.1 expõe o render API OpenGL, mas não modela os
//! parâmetros D3D11. Esta camada usa a ABI C documentada pelo libmpv 0.40+.
//! O dispositivo e a textura devem pertencer ao mesmo ID3D11Device.
//!
//! Esta ponte ainda não cria o dispositivo/swap chain nem se conecta à árvore
//! DirectComposition do Wry. Ela fornece o lado libmpv da integração para a
//! próxima fatia; o player padrão continua usando a janela nativa (wid).

use std::{ffi::c_void, marker::PhantomData, mem, ptr::NonNull};

use windows::core::Interface;

use libmpv2::Mpv;
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
    fn mpv_render_context_free(context: *mut MpvRenderContextOpaque);
}

/// D3D11 render context; must be dropped before its associated Mpv.
///
/// libmpv takes its own COM reference to the device during creation. The caller
/// must keep each target texture alive for the duration of render().
pub(super) struct D3d11RenderContext<'a> {
    context: NonNull<MpvRenderContextOpaque>,
    _mpv: PhantomData<&'a Mpv>,
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
        Ok(Self {
            context,
            _mpv: PhantomData,
        })
    }

    /// Returns libmpv render-update flags. Bit 0 requests a new video frame.
    pub(super) fn update(&self) -> u64 {
        // SAFETY: the context is valid until this wrapper is dropped.
        unsafe { mpv_render_context_update(self.context.as_ptr()) }
    }

    /// Renders the current frame into a render-target texture from the same device.
    pub(super) fn render(&self, texture: &ID3D11Texture2D) -> Result<(), String> {
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
            return Err(format!(
                "libmpv falhou ao renderizar o frame D3D11 (código {result})"
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
        self.update() & MPV_RENDER_UPDATE_FRAME != 0
    }
}

impl Drop for D3d11RenderContext<'_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns the native context and drops
        // before Mpv due to the lifetime marker.
        unsafe { mpv_render_context_free(self.context.as_ptr()) };
    }
}

/// Checks the C ABI layout used by libmpv's render.h/render_d3d11.h headers.
unsafe extern "C" {
    fn cinehub_wry_composition_get_device(parent: isize) -> *mut c_void;
    fn cinehub_wry_composition_get_backbuffer(parent: isize) -> *mut c_void;
    fn cinehub_wry_composition_present(parent: isize) -> i32;
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
) -> Result<bool, String> {
    let raw = unsafe { cinehub_wry_composition_get_backbuffer(parent as isize) };
    let Some(raw) = NonNull::new(raw) else {
        // The window is minimized or its player geometry is temporarily empty.
        return Ok(false);
    };
    // SAFETY: Wry transfers one COM reference to the current backbuffer.
    let texture = unsafe { ID3D11Texture2D::from_raw(raw.as_ptr()) };
    context.render(&texture)?;
    // Release the backbuffer reference before Present or a later ResizeBuffers.
    drop(texture);
    let result = unsafe { cinehub_wry_composition_present(parent as isize) };
    if result < 0 {
        return Err(format!(
            "DXGI não conseguiu apresentar o frame do CineHub (HRESULT 0x{:08X})",
            result as u32
        ));
    }
    Ok(result == 0)
}

/// Moves/resizes the video visual on the thread that owns the WebView HWND.
pub(super) fn set_video_rect(
    parent: i64,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> bool {
    unsafe {
        cinehub_wry_composition_set_video_rect(
            parent as isize,
            x,
            y,
            width,
            height,
        ) == 0
    }
}

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
