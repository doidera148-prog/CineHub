// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Experimental WebView2 visual-hosting path.
//!
//! This opt-in host creates the composition controller and DirectComposition visual,
//! then forwards Win32 mouse input to WebView2. The normal HWND controller remains
//! the default unless CINEHUB_WEBVIEW2_COMPOSITION_POC is explicitly set.

use std::{
  collections::HashMap,
  ffi::c_void,
  ptr,
  sync::{mpsc, Arc, Mutex, OnceLock},
};

use webview2_com::{
  CursorChangedEventHandler, CreateCoreWebView2CompositionControllerCompletedHandler,
  Microsoft::Web::WebView2::Win32::*,
};
use windows::{
  Win32::{
    Foundation::{E_FAIL, E_POINTER, E_UNEXPECTED, HMODULE, HWND, LPARAM, POINT, RECT, WPARAM},
    Graphics::{
      Direct3D::D3D_DRIVER_TYPE_HARDWARE,
      Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11Multithread, ID3D11Texture2D,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS,
        D3D11_SDK_VERSION,
      },
      DirectComposition::{
        DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionTarget,
        IDCompositionVisual, IDCompositionVisual2, IDCompositionVisual3,
      },
      Dxgi::{
        IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGIOutput, IDXGISwapChain1,
        DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_FLAG,
        DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
      },
      Dxgi::Common::{
        DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN,
        DXGI_SAMPLE_DESC,
      },
      Gdi::{ClientToScreen, ScreenToClient},
    },
    UI::{
      Input::{
        KeyboardAndMouse::{
          GetCapture, ReleaseCapture, SetCapture, TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE,
        },
        Pointer::{
          GetPointerDeviceRects, GetPointerInfo, GetPointerPenInfo, GetPointerTouchInfo,
          POINTER_INFO, POINTER_PEN_INFO, POINTER_TOUCH_INFO,
        },
      },
      Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
      WindowsAndMessaging::{
        GetClientRect, SendMessageW, SetCursor, WM_CAPTURECHANGED, WM_LBUTTONDBLCLK,
        WM_LBUTTONDOWN, PT_PEN, PT_TOUCH, WM_LBUTTONUP, WM_MBUTTONDBLCLK, WM_MBUTTONDOWN,
        WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_POINTERDOWN,
        WM_POINTERENTER, WM_POINTERLEAVE, WM_POINTERUP, WM_POINTERUPDATE, WM_RBUTTONDBLCLK,
        WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETFOCUS, WM_SIZE, WM_XBUTTONDBLCLK,
        WM_XBUTTONDOWN, WM_XBUTTONUP, SIZE_MINIMIZED,
      },
    },
  },
  core::{Interface, IUnknown},
};

use crate::Result;

const COMPOSITION_INPUT_SUBCLASS_ID: u32 = 0x4348_0001;
const MOUSE_BUTTON_MASK: usize = 0x0073;
// WM_MOUSELEAVE (winuser.h); windows 0.62.2 does not expose this binding
// through the enabled WindowsAndMessaging feature set.
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_CINEHUB_SET_VIDEO_RECT: u32 = 0x8000 + 0x4C1;

static COMPOSITION_SURFACES: OnceLock<Mutex<HashMap<isize, usize>>> = OnceLock::new();

fn composition_surfaces() -> &'static Mutex<HashMap<isize, usize>> {
  COMPOSITION_SURFACES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Only D3D11/DXGI objects are shared with the mpv worker. DirectComposition
/// interfaces stay in CompositionInputState and are touched on the HWND owner thread.
struct RenderSurface {
  device: ID3D11Device,
  swap_chain: IDXGISwapChain1,
  host_hwnd: HWND,
  pending_size: Option<(u32, u32)>,
  pending_rect: Option<(i32, i32, u32, u32)>,
}

impl RenderSurface {
  fn acquire_backbuffer(&mut self) -> *mut c_void {
    if let Some((width, height)) = self.pending_size {
      if width == 0 || height == 0 {
        return ptr::null_mut();
      }
      if unsafe {
        self.swap_chain.ResizeBuffers(
          0,
          width,
          height,
          DXGI_FORMAT_UNKNOWN,
          DXGI_SWAP_CHAIN_FLAG(0),
        )
      }
      .is_err()
      {
        return ptr::null_mut();
      }
      self.pending_size = None;
    }

    match unsafe { self.swap_chain.GetBuffer::<ID3D11Texture2D>(0) } {
      Ok(texture) => texture.into_raw(),
      Err(_) => ptr::null_mut(),
    }
  }

  fn present(&self) -> i32 {
    unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)).0 }
  }
}

fn create_composition_d3d11_device() -> Result<ID3D11Device> {
  let mut device = None;
  unsafe {
    D3D11CreateDevice(
      None::<&IDXGIAdapter>,
      D3D_DRIVER_TYPE_HARDWARE,
      HMODULE::default(),
      D3D11_CREATE_DEVICE_BGRA_SUPPORT
        | D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS,
      None,
      D3D11_SDK_VERSION,
      Some(&mut device),
      None,
      None,
    )?;
  }
  let device = device.ok_or_else(|| {
    windows::core::Error::new(E_FAIL, "D3D11CreateDevice não retornou um dispositivo")
  })?;
  // libmpv renders on its player thread while the swap chain is presented there
  // and DirectComposition visual updates originate on the WebView thread.
  let multithread = device.cast::<ID3D11Multithread>()?;
  unsafe {
    multithread.SetMultithreadProtected(true);
  }
  Ok(device)
}

fn register_composition_surface(
  parent: HWND,
  surface: &Arc<Mutex<RenderSurface>>,
) -> Result<()> {
  let mut registry = composition_surfaces()
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  if registry.contains_key(&(parent.0 as isize)) {
    return Err(windows::core::Error::new(
      E_FAIL,
      "já existe uma superfície de composição para esta janela",
    )
    .into());
  }
  let raw = Arc::into_raw(surface.clone()) as usize;
  registry.insert(parent.0 as isize, raw);
  Ok(())
}

fn find_composition_surface(parent: isize) -> Option<Arc<Mutex<RenderSurface>>> {
  let registry = composition_surfaces()
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  let raw = *registry.get(&parent)?;
  let pointer = raw as *const Mutex<RenderSurface>;
  unsafe {
    Arc::increment_strong_count(pointer);
  }
  drop(registry);
  Some(unsafe { Arc::from_raw(pointer) })
}

fn unregister_composition_surface(parent: isize) {
  let raw = composition_surfaces()
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
    .remove(&parent);
  if let Some(raw) = raw {
    unsafe {
      drop(Arc::from_raw(raw as *const Mutex<RenderSurface>));
    }
  }
}

#[unsafe(no_mangle)]
pub extern "C" fn cinehub_wry_composition_get_device(parent: isize) -> *mut c_void {
  let Some(surface) = find_composition_surface(parent) else {
    return ptr::null_mut();
  };
  surface
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
    .device
    .clone()
    .into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn cinehub_wry_composition_get_backbuffer(parent: isize) -> *mut c_void {
  let Some(surface) = find_composition_surface(parent) else {
    return ptr::null_mut();
  };
  surface
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
    .acquire_backbuffer()
}

#[unsafe(no_mangle)]
pub extern "C" fn cinehub_wry_composition_present(parent: isize) -> i32 {
  let Some(surface) = find_composition_surface(parent) else {
    return E_POINTER.0;
  };
  surface
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
    .present()
}

#[unsafe(no_mangle)]
pub extern "C" fn cinehub_wry_composition_set_video_rect(
  parent: isize,
  x: i32,
  y: i32,
  width: u32,
  height: u32,
) -> i32 {
  let Some(surface) = find_composition_surface(parent) else {
    return E_POINTER.0;
  };
  let host_hwnd = {
    let mut surface = surface
      .lock()
      .unwrap_or_else(|poisoned| poisoned.into_inner());
    surface.pending_size = Some((width, height));
    surface.pending_rect = Some((x, y, width, height));
    surface.host_hwnd
  };
  let result = unsafe {
    SendMessageW(
      host_hwnd,
      WM_CINEHUB_SET_VIDEO_RECT,
      Some(WPARAM(0)),
      Some(LPARAM(0)),
    )
  };
  result.0 as i32
}

struct CompositionInputState {
  controller: ICoreWebView2Controller,
  composition_controller: ICoreWebView2CompositionController,
  environment: ICoreWebView2Environment3,
  mouse_tracking: bool,
  captured: bool,
  surface: Arc<Mutex<RenderSurface>>,
  dcomp_device: IDCompositionDesktopDevice,
  video_visual: IDCompositionVisual2,
  video_visual3: IDCompositionVisual3,
}

pub(super) struct CompositionHost {
  pub controller: ICoreWebView2Controller,
  composition_controller: ICoreWebView2CompositionController,
  hwnd: HWND,
  parent: HWND,
  input_state: *mut CompositionInputState,
  cursor_changed_token: Option<i64>,
  _device: IDCompositionDesktopDevice,
  _target: IDCompositionTarget,
  _root_visual: IDCompositionVisual2,
  _video_visual: IDCompositionVisual2,
  _webview_visual: IDCompositionVisual2,
  _surface: Arc<Mutex<RenderSurface>>,
}

impl CompositionHost {
  pub fn new(
    parent: HWND,
    hwnd: HWND,
    environment: &ICoreWebView2Environment,
  ) -> Result<Self> {
    let environment = environment.cast::<ICoreWebView2Environment3>()?;
    let (tx, rx) = mpsc::channel();

    let handler = CreateCoreWebView2CompositionControllerCompletedHandler::create(Box::new(
      move |error_code, composition_controller| {
        let result: webview2_com::Result<ICoreWebView2CompositionController> = (|| {
          error_code?;
          composition_controller
            .ok_or_else(|| windows::core::Error::from(E_POINTER).into())
        })();
        tx.send(result)
          .map_err(|_| windows::core::Error::from(E_UNEXPECTED))
      },
    ));

    unsafe {
      environment.CreateCoreWebView2CompositionController(hwnd, &handler)?;
    }

    let composition_controller = webview2_com::wait_with_pump(rx)??;
    let controller = composition_controller.cast::<ICoreWebView2Controller>()?;
    let controller2 = controller.cast::<ICoreWebView2Controller2>()?;
    unsafe {
      controller2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
        R: 0,
        G: 0,
        B: 0,
        A: 0,
      })?;
    }

    // One hardware D3D11 device backs both libmpv's render target and the
    // composition swap chain. DComp is created from that same DXGI device.
    let d3d_device = create_composition_d3d11_device()?;
    let dxgi_device = d3d_device.cast::<IDXGIDevice>()?;
    let adapter = unsafe { dxgi_device.GetAdapter()? };
    let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };

    let mut client_bounds = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client_bounds)? };
    let initial_width = (client_bounds.right - client_bounds.left).max(1) as u32;
    let initial_height = (client_bounds.bottom - client_bounds.top).max(1) as u32;
    let swap_chain_desc = windows::Win32::Graphics::Dxgi::DXGI_SWAP_CHAIN_DESC1 {
      Width: initial_width,
      Height: initial_height,
      Format: DXGI_FORMAT_B8G8R8A8_UNORM,
      Stereo: Default::default(),
      SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
      BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
      BufferCount: 2,
      Scaling: DXGI_SCALING_STRETCH,
      SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
      AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
      Flags: 0,
    };
    let swap_chain = unsafe {
      factory.CreateSwapChainForComposition(
        &d3d_device,
        &swap_chain_desc,
        None::<&IDXGIOutput>,
      )?
    };

    let dcomp_rendering_device = dxgi_device.cast::<IUnknown>()?;
    let device = unsafe {
      DCompositionCreateDevice2::<_, IDCompositionDesktopDevice>(Some(&dcomp_rendering_device))?
    };
    let target = unsafe { device.CreateTargetForHwnd(parent, true)? };
    let root_visual = unsafe { device.CreateVisual()? };
    let video_visual = unsafe { device.CreateVisual()? };
    let webview_visual = unsafe { device.CreateVisual()? };
    let video_visual_base = video_visual.cast::<IDCompositionVisual>()?;
    let webview_visual_base = webview_visual.cast::<IDCompositionVisual>()?;
    let video_visual3 = video_visual.cast::<IDCompositionVisual3>()?;

    unsafe {
      video_visual.SetContent(&swap_chain)?;
      // Video is the lower sibling; the WebView2 subtree is inserted above it.
      root_visual.AddVisual(&video_visual_base, false, None::<&IDCompositionVisual>)?;
      root_visual.AddVisual(&webview_visual_base, true, Some(&video_visual_base))?;
      video_visual3.SetVisible(false)?;
      target.SetRoot(&root_visual.cast::<IDCompositionVisual>()?)?;
      composition_controller.SetRootVisualTarget(&webview_visual_base.cast::<IUnknown>()?)?;
      device.Commit()?;
    }

    let surface = Arc::new(Mutex::new(RenderSurface {
      device: d3d_device,
      swap_chain,
      host_hwnd: hwnd,
      pending_size: None,
      pending_rect: None,
    }));

    // Cursor changes are raised by WebView2 when the pointer crosses elements with
    // different CSS/system cursors. Apply the WebView-provided cursor on the UI thread.
    let cursor_controller = composition_controller.clone();
    let cursor_handler = CursorChangedEventHandler::create(Box::new(move |_, _| {
      let mut cursor = Default::default();
      unsafe {
        cursor_controller.Cursor(&mut cursor)?;
        SetCursor(Some(cursor));
      }
      Ok(())
    }));
    let mut cursor_changed_token = 0;
    unsafe {
      composition_controller.add_CursorChanged(&cursor_handler, &mut cursor_changed_token)?;
    }

    // Composition mode has no child WebView HWND to receive input. Subclass the
    // Wry container and translate its mouse messages to WebView2 SendMouseInput.
    let input_state = Box::into_raw(Box::new(CompositionInputState {
      controller: controller.clone(),
      composition_controller: composition_controller.clone(),
      environment: environment.clone(),
      mouse_tracking: false,
      captured: false,
      surface: surface.clone(),
      dcomp_device: device.clone(),
      video_visual: video_visual.clone(),
      video_visual3,
    }));
    register_composition_surface(parent, &surface)?;
    if !unsafe {
      SetWindowSubclass(
        hwnd,
        Some(Self::input_subclass_proc),
        COMPOSITION_INPUT_SUBCLASS_ID as _,
        input_state as usize,
      )
    }
    .as_bool()
    {
      unsafe {
        drop(Box::from_raw(input_state));
        unregister_composition_surface(parent.0 as isize);
        let _ = composition_controller.remove_CursorChanged(cursor_changed_token);
      }
      return Err(
        windows::core::Error::new(E_FAIL, "SetWindowSubclass failed").into(),
      );
    }

    Ok(Self {
      controller,
      composition_controller,
      hwnd,
      parent,
      input_state,
      cursor_changed_token: Some(cursor_changed_token),
      _device: device,
      _target: target,
      _root_visual: root_visual,
      _video_visual: video_visual,
      _webview_visual: webview_visual,
      _surface: surface,
    })
  }

  unsafe extern "system" fn input_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _subclass_id: usize,
    ref_data: usize,
  ) -> windows::Win32::Foundation::LRESULT {
    unsafe {
      let Some(state) = (ref_data as *mut CompositionInputState).as_mut() else {
        return DefSubclassProc(hwnd, msg, wparam, lparam);
      };

      match msg {
        WM_CINEHUB_SET_VIDEO_RECT => {
          let rect = state
            .surface
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending_rect
            .take();
          let Some((x, y, width, height)) = rect else {
            return windows::Win32::Foundation::LRESULT(0);
          };
          let result: windows::core::Result<()> = (|| {
            state.video_visual.SetOffsetX2(x as f32)?;
            state.video_visual.SetOffsetY2(y as f32)?;
            state.video_visual3.SetVisible(width > 0 && height > 0)?;
            state.dcomp_device.Commit()
          })();
          return windows::Win32::Foundation::LRESULT(
            result.map_or_else(|error| error.code().0 as isize, |_| 0),
          );
        }
        WM_SETFOCUS => {
          let _ = state
            .controller
            .MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
        WM_SIZE if wparam.0 != SIZE_MINIMIZED as usize => {
          // Keep the composition controller bounds in sync with the host client area.
          let mut bounds = RECT::default();
          if GetClientRect(hwnd, &mut bounds).is_ok() {
            let _ = state.controller.SetBounds(bounds);
          }
        }
        WM_CAPTURECHANGED => {
          state.captured = false;
        }
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP | WM_POINTERENTER | WM_POINTERLEAVE => {
          let pointer_id = (wparam.0 & 0xffff) as u32;
          if Self::send_pointer_message(hwnd, msg, pointer_id, state) {
            // Consuming recognized touch/pen pointer messages prevents Windows from
            // synthesizing a second mouse stream for the same physical contact.
            return windows::Win32::Foundation::LRESULT(0);
          }
        }
        WM_MOUSEMOVE => {
          if !state.mouse_tracking {
            let mut tracking = TRACKMOUSEEVENT {
              cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
              dwFlags: TME_LEAVE,
              hwndTrack: hwnd,
              dwHoverTime: 0,
            };
            if TrackMouseEvent(&mut tracking).is_ok() {
              state.mouse_tracking = true;
            }
          }
          Self::send_mouse_message(hwnd, msg, wparam.0, lparam.0, state);
          return windows::Win32::Foundation::LRESULT(0);
        }
        WM_MOUSELEAVE => {
          state.mouse_tracking = false;
          let _ = state.composition_controller.SendMouseInput(
            COREWEBVIEW2_MOUSE_EVENT_KIND_LEAVE,
            COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS(0),
            0,
            POINT::default(),
          );
          return windows::Win32::Foundation::LRESULT(0);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONDBLCLK
        | WM_MBUTTONDOWN | WM_MBUTTONDBLCLK | WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => {
          if !state.captured {
            SetCapture(hwnd);
            state.captured = GetCapture() == hwnd;
          }
          Self::send_mouse_message(hwnd, msg, wparam.0, lparam.0, state);
          return windows::Win32::Foundation::LRESULT(0);
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP | WM_MOUSEWHEEL
        | WM_MOUSEHWHEEL => {
          Self::send_mouse_message(hwnd, msg, wparam.0, lparam.0, state);
          if msg != WM_MOUSEWHEEL
            && msg != WM_MOUSEHWHEEL
            && state.captured
            && (wparam.0 & MOUSE_BUTTON_MASK) == 0
            && GetCapture() == hwnd
          {
            let _ = ReleaseCapture();
            state.captured = false;
          }
          return windows::Win32::Foundation::LRESULT(0);
        }
        _ => {}
      }

      DefSubclassProc(hwnd, msg, wparam, lparam)
    }
  }

  unsafe fn send_mouse_message(
    hwnd: HWND,
    msg: u32,
    wparam: usize,
    lparam: isize,
    state: &CompositionInputState,
  ) {
    let (event_kind, mouse_data) = match msg {
      WM_MOUSEMOVE => (COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE, 0),
      WM_LBUTTONDOWN => (COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_DOWN, 0),
      WM_LBUTTONUP => (COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_UP, 0),
      WM_LBUTTONDBLCLK => (COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_DOUBLE_CLICK, 0),
      WM_RBUTTONDOWN => (COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_DOWN, 0),
      WM_RBUTTONUP => (COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_UP, 0),
      WM_RBUTTONDBLCLK => (COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_DOUBLE_CLICK, 0),
      WM_MBUTTONDOWN => (COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_DOWN, 0),
      WM_MBUTTONUP => (COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_UP, 0),
      WM_MBUTTONDBLCLK => (COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_DOUBLE_CLICK, 0),
      WM_XBUTTONDOWN => (
        COREWEBVIEW2_MOUSE_EVENT_KIND_X_BUTTON_DOWN,
        ((wparam >> 16) & 0xffff) as u32,
      ),
      WM_XBUTTONUP => (
        COREWEBVIEW2_MOUSE_EVENT_KIND_X_BUTTON_UP,
        ((wparam >> 16) & 0xffff) as u32,
      ),
      WM_XBUTTONDBLCLK => (
        COREWEBVIEW2_MOUSE_EVENT_KIND_X_BUTTON_DOUBLE_CLICK,
        ((wparam >> 16) & 0xffff) as u32,
      ),
      WM_MOUSEWHEEL => (
        COREWEBVIEW2_MOUSE_EVENT_KIND_WHEEL,
        (((wparam >> 16) as u16 as i16) as i32) as u32,
      ),
      WM_MOUSEHWHEEL => (
        COREWEBVIEW2_MOUSE_EVENT_KIND_HORIZONTAL_WHEEL,
        (((wparam >> 16) as u16 as i16) as i32) as u32,
      ),
      _ => return,
    };

    let mut point = POINT {
      x: (lparam as u16 as i16) as i32,
      y: ((lparam >> 16) as u16 as i16) as i32,
    };
    if msg == WM_MOUSEWHEEL || msg == WM_MOUSEHWHEEL {
      let _ = unsafe { ScreenToClient(hwnd, &mut point) };
    }

    let virtual_keys = COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS((wparam & 0x007f) as i32);
    let _ = unsafe {
      state
        .composition_controller
        .SendMouseInput(event_kind, virtual_keys, mouse_data, point)
    };
  }

  /// Convert a screen-space Win32 rectangle into WebView-relative physical pixels.
  unsafe fn screen_rect_to_client(hwnd: HWND, rect: RECT) -> Option<RECT> {
    let mut top_left = POINT {
      x: rect.left,
      y: rect.top,
    };
    let mut bottom_right = POINT {
      x: rect.right,
      y: rect.bottom,
    };
    if !unsafe { ScreenToClient(hwnd, &mut top_left) }.as_bool()
      || !unsafe { ScreenToClient(hwnd, &mut bottom_right) }.as_bool()
    {
      return None;
    }
    Some(RECT {
      left: top_left.x,
      top: top_left.y,
      right: bottom_right.x,
      bottom: bottom_right.y,
    })
  }

  /// Map the host window's screen origin into the pointer device's HIMETRIC space.
  /// The device rectangle is retained as the digitizer's calibration range; only
  /// screen-relative pointer locations are translated into WebView-local coordinates.
  fn himetric_client_origin(
    pointer_device_rect: RECT,
    display_rect: RECT,
    client_origin_screen: POINT,
  ) -> Option<POINT> {
    let display_width = i64::from(display_rect.right) - i64::from(display_rect.left);
    let display_height = i64::from(display_rect.bottom) - i64::from(display_rect.top);
    let device_width = i64::from(pointer_device_rect.right) - i64::from(pointer_device_rect.left);
    let device_height = i64::from(pointer_device_rect.bottom) - i64::from(pointer_device_rect.top);
    if display_width <= 0 || display_height <= 0 || device_width <= 0 || device_height <= 0 {
      return None;
    }

    let x = i64::from(pointer_device_rect.left)
      + (i64::from(client_origin_screen.x) - i64::from(display_rect.left)) * device_width
        / display_width;
    let y = i64::from(pointer_device_rect.top)
      + (i64::from(client_origin_screen.y) - i64::from(display_rect.top)) * device_height
        / display_height;
    Some(POINT {
      x: i32::try_from(x).ok()?,
      y: i32::try_from(y).ok()?,
    })
  }

  unsafe fn send_pointer_message(
    hwnd: HWND,
    msg: u32,
    pointer_id: u32,
    state: &CompositionInputState,
  ) -> bool {
    let event_kind = match msg {
      WM_POINTERDOWN => COREWEBVIEW2_POINTER_EVENT_KIND_DOWN,
      WM_POINTERUPDATE => COREWEBVIEW2_POINTER_EVENT_KIND_UPDATE,
      WM_POINTERUP => COREWEBVIEW2_POINTER_EVENT_KIND_UP,
      WM_POINTERENTER => COREWEBVIEW2_POINTER_EVENT_KIND_ENTER,
      WM_POINTERLEAVE => COREWEBVIEW2_POINTER_EVENT_KIND_LEAVE,
      _ => return false,
    };

    let mut native_info = POINTER_INFO::default();
    if unsafe { GetPointerInfo(pointer_id, &mut native_info) }.is_err()
      || (native_info.pointerType != PT_TOUCH && native_info.pointerType != PT_PEN)
    {
      // WebView2 only accepts PT_TOUCH and PT_PEN. Leave mouse/touchpad and
      // unavailable pointer data to the normal Win32 procedure.
      return false;
    }

    let mut pointer_device_rect = RECT::default();
    let mut display_rect = RECT::default();
    if unsafe {
      GetPointerDeviceRects(
        native_info.sourceDevice,
        &mut pointer_device_rect,
        &mut display_rect,
      )
    }
    .is_err()
    {
      return false;
    }

    let Some(display_rect_client) =
      (unsafe { Self::screen_rect_to_client(hwnd, display_rect) })
    else {
      return false;
    };
    let mut client_origin_screen = POINT::default();
    if !unsafe { ClientToScreen(hwnd, &mut client_origin_screen) }.as_bool() {
      return false;
    }
    let Some(himetric_origin) =
      Self::himetric_client_origin(pointer_device_rect, display_rect, client_origin_screen)
    else {
      return false;
    };

    let to_client_himetric = |point: POINT| -> Option<POINT> {
      Some(POINT {
        x: i32::try_from(i64::from(point.x) - i64::from(himetric_origin.x)).ok()?,
        y: i32::try_from(i64::from(point.y) - i64::from(himetric_origin.y)).ok()?,
      })
    };
    let to_client_himetric_rect = |rect: RECT| -> Option<RECT> {
      let top_left = to_client_himetric(POINT {
        x: rect.left,
        y: rect.top,
      })?;
      let bottom_right = to_client_himetric(POINT {
        x: rect.right,
        y: rect.bottom,
      })?;
      Some(RECT {
        left: top_left.x,
        top: top_left.y,
        right: bottom_right.x,
        bottom: bottom_right.y,
      })
    };
    let Some(pointer_device_rect_client) = to_client_himetric_rect(pointer_device_rect) else {
      return false;
    };
    let Some(himetric_location) = to_client_himetric(native_info.ptHimetricLocation) else {
      return false;
    };
    let Some(himetric_location_raw) = to_client_himetric(native_info.ptHimetricLocationRaw) else {
      return false;
    };
    let Some(pixel_location) =
      (unsafe { Self::screen_rect_to_client(hwnd, RECT {
        left: native_info.ptPixelLocation.x,
        top: native_info.ptPixelLocation.y,
        right: native_info.ptPixelLocation.x,
        bottom: native_info.ptPixelLocation.y,
      }) })
    else {
      return false;
    };
    let Some(pixel_location_raw) =
      (unsafe { Self::screen_rect_to_client(hwnd, RECT {
        left: native_info.ptPixelLocationRaw.x,
        top: native_info.ptPixelLocationRaw.y,
        right: native_info.ptPixelLocationRaw.x,
        bottom: native_info.ptPixelLocationRaw.y,
      }) })
    else {
      return false;
    };

    let pointer_info = match unsafe { state.environment.CreateCoreWebView2PointerInfo() } {
      Ok(pointer_info) => pointer_info,
      Err(_) => return false,
    };

    let populate_result: windows::core::Result<()> = (|| unsafe {
      pointer_info.SetPointerKind(native_info.pointerType.0 as u32)?;
      pointer_info.SetPointerId(native_info.pointerId)?;
      pointer_info.SetFrameId(native_info.frameId)?;
      pointer_info.SetPointerFlags(native_info.pointerFlags.0)?;
      pointer_info.SetPointerDeviceRect(pointer_device_rect_client)?;
      pointer_info.SetDisplayRect(display_rect_client)?;
      pointer_info.SetPixelLocation(POINT {
        x: pixel_location.left,
        y: pixel_location.top,
      })?;
      pointer_info.SetPixelLocationRaw(POINT {
        x: pixel_location_raw.left,
        y: pixel_location_raw.top,
      })?;
      pointer_info.SetHimetricLocation(himetric_location)?;
      pointer_info.SetHimetricLocationRaw(himetric_location_raw)?;
      pointer_info.SetTime(native_info.dwTime)?;
      pointer_info.SetHistoryCount(native_info.historyCount)?;
      pointer_info.SetInputData(native_info.InputData)?;
      pointer_info.SetKeyStates(native_info.dwKeyStates)?;
      pointer_info.SetPerformanceCount(native_info.PerformanceCount)?;
      pointer_info.SetButtonChangeKind(native_info.ButtonChangeType.0)?;

      if native_info.pointerType == PT_TOUCH {
        let mut touch_info = POINTER_TOUCH_INFO::default();
        GetPointerTouchInfo(pointer_id, &mut touch_info)?;
        let Some(contact) = Self::screen_rect_to_client(hwnd, touch_info.rcContact) else {
          return Err(windows::core::Error::new(E_FAIL, "Touch contact conversion failed"));
        };
        let Some(contact_raw) = Self::screen_rect_to_client(hwnd, touch_info.rcContactRaw) else {
          return Err(windows::core::Error::new(E_FAIL, "Raw touch contact conversion failed"));
        };
        pointer_info.SetTouchFlags(touch_info.touchFlags)?;
        pointer_info.SetTouchMask(touch_info.touchMask)?;
        pointer_info.SetTouchContact(contact)?;
        pointer_info.SetTouchContactRaw(contact_raw)?;
        pointer_info.SetTouchOrientation(touch_info.orientation)?;
        pointer_info.SetTouchPressure(touch_info.pressure)?;
      } else {
        let mut pen_info = POINTER_PEN_INFO::default();
        GetPointerPenInfo(pointer_id, &mut pen_info)?;
        pointer_info.SetPenFlags(pen_info.penFlags)?;
        pointer_info.SetPenMask(pen_info.penMask)?;
        pointer_info.SetPenPressure(pen_info.pressure)?;
        pointer_info.SetPenRotation(pen_info.rotation)?;
        pointer_info.SetPenTiltX(pen_info.tiltX)?;
        pointer_info.SetPenTiltY(pen_info.tiltY)?;
      }

      Ok(())
    })();
    if populate_result.is_err() {
      return false;
    }

    unsafe {
      state
        .composition_controller
        .SendPointerInput(event_kind, &pointer_info)
        .is_ok()
    }
  }

  /// Detach callbacks and the HWND subclass before the common controller is closed.
  pub fn shutdown(&mut self) {
    if !self.input_state.is_null() {
      unsafe {
        let state = &mut *self.input_state;
        let _ = state.composition_controller.SendMouseInput(
          COREWEBVIEW2_MOUSE_EVENT_KIND_LEAVE,
          COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS(0),
          0,
          POINT::default(),
        );
        if state.captured && GetCapture() == self.hwnd {
          let _ = ReleaseCapture();
        }
        let _ = RemoveWindowSubclass(
          self.hwnd,
          Some(Self::input_subclass_proc),
          COMPOSITION_INPUT_SUBCLASS_ID as _,
        );
        drop(Box::from_raw(self.input_state));
      }
      self.input_state = ptr::null_mut();
    }

    if let Some(token) = self.cursor_changed_token.take() {
      unsafe {
        let _ = self.composition_controller.remove_CursorChanged(token);
        let _ = self.composition_controller.SetRootVisualTarget(None::<&IUnknown>);
      }
    }
    unregister_composition_surface(self.parent.0 as isize);
  }

  #[allow(dead_code)]
  pub fn composition_controller(&self) -> &ICoreWebView2CompositionController {
    &self.composition_controller
  }
}

impl Drop for CompositionHost {
  fn drop(&mut self) {
    self.shutdown();
  }
}
