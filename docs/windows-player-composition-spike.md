# Windows player composition spike

## Objective

Replace the embedded libmpv child HWND (wid) with libmpv's render API so video pixels can be composed with the CineHub WebView2 UI. This is a Windows-only stability fix for the native mpv window/input conflict. Android/Google TV (ExoPlayer), Linux and macOS playback must remain unchanged.

## Verified constraints

- libmpv exposes a Direct3D 11 render backend: create a render context with MPV_RENDER_API_TYPE_D3D11, then render into an application-owned ID3D11Texture2D render target.
- The application owns presentation and synchronization. libmpv does not present the texture itself.
- Current Tauri 2 / Wry Windows hosting creates a standard ICoreWebView2Controller using CreateCoreWebView2Controller. The Windows extension exposes that controller and the WebView HWND, but not a composition visual.
- A true shared DirectComposition visual tree therefore cannot be achieved by only changing player.rs. It requires a Wry/Tauri Windows hosting seam that creates and manages ICoreWebView2CompositionController and its visual, or a separately hosted composition-mode WebView with equivalent Tauri IPC/navigation integration.
- Do not assume a D3D11 texture can simply be placed over the existing child-HWND WebView. That recreates the z-order/input problem this work is intended to remove.

## Implementation sequence

1. Hosting proof of concept (Windows-only): prove that the app can create a WebView2 composition controller on the main UI thread, attach its visual to a DirectComposition target, resize it, and continue to receive pointer input. Keep the existing Tauri WebView active as a fallback until the composition host is functional.
2. libmpv render proof of concept: initialize a D3D11 device and libmpv D3D11 render context; render into an application-owned BGRA texture; handle update callbacks, resize, device loss, and teardown. Keep libmpv playback and command/event handling on their existing worker path.
3. Composition: place the WebView2 visual and the mpv video visual in one DirectComposition tree with the WebView above the video. Verify alpha, clipping, resize, DPI, fullscreen and input hit testing.
4. Integration: switch Windows playback from wid/child-HWND embedding to the render API only after the proof of concept passes. Remove obsolete HWND mouse-forwarding code only in the same validated change.
5. Regression validation: extend existing Windows player tests and the existing .github/workflows/build.yml Windows job. Do not create a separate workflow. Preserve the four Android ABIs and ExoPlayer path.

## Acceptance criteria

- WebView2 buttons above the video receive mouse click, double-click, wheel and pointer movement normally.
- Video renders without an independent mpv child window and remains correctly clipped/resized.
- Pause, seek, volume, subtitle/audio selection, skip segment, close and fullscreen continue to work.
- Repeated play/close cycles release render contexts, textures, COM objects and callbacks without leaks or hangs.
- Windows build, lint, existing tests and new focused tests pass in the main workflow.
- Android/Google TV, Linux and macOS behavior is unchanged.

## Safety / rollout

This is a spike on fix/windows-mpv-composition. Do not merge into main, remove the existing player path, or publish a release until the Windows proof of concept and regression checks pass on a real Windows machine. CI compilation alone does not validate visual composition or physical mouse input.
