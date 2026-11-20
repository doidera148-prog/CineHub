// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Experimental WebView2 visual-hosting path.
//!
//! This opt-in host creates the composition controller and DirectComposition visual,
//! then forwards Win32 mouse input to WebView2. The normal HWND controller remains
//! the default unless CINEHUB_WEBVIEW2_COMPOSITION_POC is explicitly set.

use std::{ptr, sync::mpsc};

use webview2_com::{
  CursorChangedEventHandler, CreateCoreWebView2CompositionControllerCompletedHandler,
  Microsoft::Web::WebView2::Win32::*,
};
use windows::{
  Win32::{
    Foundation::{E_POINTER, E_UNEXPECTED, HWND, POINT, RECT},
    Graphics::{
      DirectComposition::{
        DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionTarget,
        IDCompositionVisual,
      },
      Gdi::ScreenToClient,
    },
    UI::{
      Input::KeyboardAndMouse::{
        GetCapture, ReleaseCapture, SetCapture, TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE,
      },
      Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
      WindowsAndMessaging::{
        GetClientRect, SetCursor, WM_CAPTURECHANGED, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
        WM_MOUSELEAVE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN,
        WM_RBUTTONUP, WM_SETFOCUS, WM_SIZE, WM_XBUTTONDBLCLK, WM_XBUTTONDOWN, WM_XBUTTONUP,
        SIZE_MINIMIZED,
      },
    },
  },
  core::{Interface, IUnknown},
};

use crate::Result;

const COMPOSITION_INPUT_SUBCLASS_ID: u32 = 0x4348_0001;
const MOUSE_BUTTON_MASK: usize = 0x0073;

struct CompositionInputState {
  controller: ICoreWebView2Controller,
  composition_controller: ICoreWebView2CompositionController,
  mouse_tracking: bool,
  captured: bool,
}

pub(super) struct CompositionHost {
  pub controller: ICoreWebView2Controller,
  composition_controller: ICoreWebView2CompositionController,
  hwnd: HWND,
  input_state: *mut CompositionInputState,
  cursor_changed_token: Option<i64>,
  _device: IDCompositionDesktopDevice,
  _target: IDCompositionTarget,
  _visual: IDCompositionVisual,
}

impl CompositionHost {
  pub fn new(hwnd: HWND, environment: &ICoreWebView2Environment) -> Result<Self> {
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

    // Attach the WebView2 composition visual to the existing Wry container HWND.
    let device = unsafe {
      DCompositionCreateDevice2::<_, IDCompositionDesktopDevice>(None::<&IUnknown>)?
    };
    let target = unsafe { device.CreateTargetForHwnd(hwnd, true)? };
    let visual = unsafe { device.CreateVisual()?.cast::<IDCompositionVisual>()? };

    let visual_unknown = visual.cast::<IUnknown>()?;
    unsafe {
      target.SetRoot(&visual)?;
      composition_controller.SetRootVisualTarget(&visual_unknown)?;
      device.Commit()?;
    }

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
      mouse_tracking: false,
      captured: false,
    }));
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
        let _ = composition_controller.remove_CursorChanged(cursor_changed_token);
      }
      return Err(windows::core::Error::from_win32().into());
    }

    Ok(Self {
      controller,
      composition_controller,
      hwnd,
      input_state,
      cursor_changed_token: Some(cursor_changed_token),
      _device: device,
      _target: target,
      _visual: visual,
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
