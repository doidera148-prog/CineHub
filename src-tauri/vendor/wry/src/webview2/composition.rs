// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Experimental WebView2 visual-hosting path.
//!
//! This first slice only proves composition-controller creation and DirectComposition
//! visual attachment. Pointer forwarding is deliberately not enabled yet; the normal
//! HWND controller remains the default unless the explicit POC environment switch is set.

use std::sync::mpsc;

use webview2_com::{
  CreateCoreWebView2CompositionControllerCompletedHandler,
  Microsoft::Web::WebView2::Win32::*,
};
use windows::{
  Win32::{
    Foundation::{E_POINTER, E_UNEXPECTED, HWND},
    Graphics::DirectComposition::{
      DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionTarget,
      IDCompositionVisual,
    },
  },
  core::{Interface, IUnknown},
};

use crate::Result;

pub(super) struct CompositionHost {
  pub controller: ICoreWebView2Controller,
  composition_controller: ICoreWebView2CompositionController,
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

    // The WebView2 composition visual is attached to the existing Wry container HWND.
    // The standard HWND controller is not created in this opt-in path.
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

    Ok(Self {
      controller,
      composition_controller,
      _device: device,
      _target: target,
      _visual: visual,
    })
  }

  #[allow(dead_code)]
  pub fn composition_controller(&self) -> &ICoreWebView2CompositionController {
    &self.composition_controller
  }
}
