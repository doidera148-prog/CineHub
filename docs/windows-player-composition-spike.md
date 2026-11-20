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


## Exact dependency baseline and Wry patch surface

The current branch lockfile pins the relevant Windows stack to:

- Tauri `2.12.5`
- Wry `0.57.2`
- `webview2-com` `0.39.1`
- `windows` / `windows-core` `0.62.2`

Do not float these versions while implementing the spike. Wry 0.57.2 is the correct source baseline for a minimal fork; the application should continue to consume the same Tauri/Wry versions except for a pinned Wry source override.

### Where the current Wry implementation must change

The Windows backend in Wry currently stores a standard `ICoreWebView2Controller` in `InnerWebView`. Its `create_controller` helper creates that controller through `ICoreWebView2Environment::CreateCoreWebView2Controller` (or the options overload). The WebView is then obtained from the controller with `CoreWebView2()`. The normal child-HWND container, parent subclass, resize and focus paths are coupled to this controller model.

A composition implementation therefore needs an explicit Windows hosting mode and must cover all of these seams together:

1. **Controller creation:** add an opt-in composition path using `ICoreWebView2Environment3::CreateCoreWebView2CompositionController` (and the matching completion handler). Retain the returned `ICoreWebView2CompositionController` for the lifetime of the WebView; obtain the common controller/WebView interfaces through COM interface casts where supported.
2. **Visual hosting:** create and own the DirectComposition device, target and visual on the window's UI thread; set the composition controller's `RootVisualTarget` to the WebView visual; commit after changes. The visual must be attached to the actual Tauri window and participate in resize, DPI, visibility and teardown.
3. **Input:** composition mode does not receive normal child-HWND input automatically. Route host mouse/pointer events through `SendMouseInput` / `SendPointerInput`, handle cursor changes, keyboard focus and accelerator events, and preserve Tauri's normal WebView event/IPC behavior. Do not reuse the old HWND-forwarding workaround as the primary input path.
4. **Lifecycle:** disconnect `RootVisualTarget`, close the controller, release COM and DirectComposition objects, and remove callbacks/subclasses in a deterministic order on the UI thread.
5. **Compatibility:** keep the existing standard-controller path as the default and fallback. Expose composition as a Windows-only opt-in so Android, Linux and macOS compile and behave exactly as before.

### Why this is a Wry fork, not an application-only patch

Tauri creates its WebView through Wry before application setup code runs. The app's existing `with_webview` access gives access to an already-created standard controller; it does not convert that controller into a composition controller or replace the WebView's visual host. The composition choice must be made inside Wry's Windows creation path (or by replacing the entire WebView host, which would also require rebuilding Tauri navigation, IPC, permissions and lifecycle integration).

The pinned `webview2-com` 0.39.1 bindings provide the WebView2 COM API surface, including the composition-controller completion handler. Microsoft documents that composition hosting requires a composition controller, a host-owned visual tree, and explicit forwarding of spatial input. This makes a narrowly scoped Wry fork technically viable, but it is not a small `player.rs` change.

### Safe implementation decision

Proceed in this order:

- First create a minimal fork of Wry 0.57.2, pinned to an immutable commit, and add only the opt-in controller/visual-hosting seam. Do not change the default controller path.
- Compile the fork through CineHub's existing Windows job before integrating libmpv rendering.
- Add a Windows-only host test/diagnostic mode that proves composition WebView creation, resize, cursor and click delivery, teardown/recreation. CI compilation is necessary but not sufficient; the visual and physical-mouse acceptance checks still require a real Windows machine.
- Only after this passes, connect the mpv D3D11 render texture into the same DirectComposition visual tree. Keep the existing player path until the full acceptance criteria above pass.

No dependency override, player switch, or production code path is changed by this investigation.


## Source-level Wry inspection

The upstream Windows backend source inspected at `src/webview2/mod.rs` confirms the key ownership seam:

- `InnerWebView` currently owns `hwnd`, `is_child`, `controller: ICoreWebView2Controller`, `webview: ICoreWebView2`, and `env: ICoreWebView2Environment`.
- `InnerWebView::new_in_hwnd` creates the container HWND, calls `create_controller`, then passes the standard controller into `init_webview`.
- `create_controller` uses `CreateCoreWebView2Controller` or `CreateCoreWebView2ControllerWithOptions`, then waits for the asynchronous completion handler.
- `init_webview` registers navigation, IPC, permissions and other handlers against the common `ICoreWebView2` interface, then calls controller methods for visibility and focus and attaches the parent subclass.
- `Drop for InnerWebView` closes the controller and destroys/detaches the HWND host.

This means a safe fork must preserve the common `ICoreWebView2` initialization and event-registration path while making the visual host and controller ownership polymorphic. The standard HWND path cannot simply be deleted: it remains the fallback and is still required by other platforms' existing behavior.

### Required fork patch boundaries

The first fork patch should be deliberately limited to:

1. Add a Windows-only hosting-mode option with the existing HWND mode as the default.
2. Add composition-controller creation and retain the composition interface alongside the common controller interface.
3. Add a UI-thread-owned DirectComposition host object responsible for device/target/visual creation, RootVisualTarget assignment, bounds/DPI updates and deterministic teardown.
4. Route host pointer/mouse events to the composition controller and handle cursor/focus/keyboard integration.
5. Keep existing navigation, IPC, permission, custom-protocol, download, drag/drop and event registration using the same common WebView interface.
6. Add a focused host diagnostic/test target before wiring this mode into CineHub's normal window creation.

Do not begin the libmpv D3D11 renderer patch until this hosting-only change builds and its pointer/resize/recreate checks pass.

### Fork creation status

The exact upstream source seam has been identified, but no Wry fork or production source patch is claimed by this commit. The available GitHub integration for this task can create and update files/branches in accessible repositories, but does not expose the GitHub **Fork repository** operation. Creating a new repository fork therefore requires the repository owner to create the fork in GitHub first; after that, the fork can be updated with the pinned patch and referenced from CineHub. No unverified repository URL or dependency override should be added.


## In-repository vendor strategy (no user-created fork required)

To avoid requiring the repository owner to create a separate GitHub fork, the exact Wry `0.57.2` source is vendored under `src-tauri/vendor/wry` and selected with the existing Cargo `[patch.crates-io]` mechanism. This keeps the upstream source baseline, licenses and package metadata inside CineHub, while all changes remain reviewable in the CineHub branch. The upstream baseline is tag `wry-v0.57.2`, commit `bbba6b6312af92fbb969789a63768f4b4bb6d268`.

The vendor baseline is connected to Cargo and the lockfile points to the local package. The standard WebView2 controller remains the default. An experimental Windows-only composition path now exists behind `CINEHUB_WEBVIEW2_COMPOSITION_POC`; it creates `ICoreWebView2CompositionController`, attaches a DirectComposition visual to the existing Wry container HWND, and retains the COM objects for the WebView lifetime. The path is deliberately opt-in and is not yet a usable player host because mouse/pointer forwarding, cursor handling, and keyboard focus integration are not implemented.

The normal `.github/workflows/build.yml` is the validation path. The first gate is that the existing Linux, Windows and macOS Rust builds/tests continue to compile against the vendored Wry package before any composition behavior is added.


## Hosting POC progress — 2026-11-20

The opt-in Windows hosting slice now has a mouse-input implementation in the vendored Wry backend:

- Creates `ICoreWebView2CompositionController`, a DirectComposition device/target/visual, assigns `RootVisualTarget`, and commits the visual tree.
- Retains both COM interfaces with distinct responsibilities: `ICoreWebView2Controller` handles focus and bounds; `ICoreWebView2CompositionController` handles composition-specific cursor and mouse input.
- Installs a subclass on the Wry host HWND and forwards mouse move/leave, left/right/middle/X button down/up/double-click, and vertical/horizontal wheel messages through `SendMouseInput`.
- Converts wheel coordinates from screen to client space; tracks pointer leave; captures the mouse while a button is held and releases capture after the last button-up.
- Applies WebView2 cursor changes through the host `SetCursor` API.
- Synchronizes the composition controller's bounds on `WM_SIZE` and moves focus into the common controller on `WM_SETFOCUS`.
- Removes the cursor callback and host subclass, sends a final leave event, releases capture, and detaches `RootVisualTarget` before Wry closes the common controller.
- Uses the correct `windows 0.62.2` module paths: subclassing helpers in `Win32::UI::Shell`, capture/tracking in `Win32::UI::Input::KeyboardAndMouse`, and `ScreenToClient` in `Win32::Graphics::Gdi`.
- The normal standard-controller path remains unchanged and is selected by default. Composition remains opt-in through `CINEHUB_WEBVIEW2_COMPOSITION_POC`.

The existing Python regression test was expanded to guard the API module paths and ensure mouse events are sent through the composition interface, not the common controller. The existing `.github/workflows/build.yml` is still the sole CI workflow.



The `WM_MOUSELEAVE` message is declared locally as the Win32 value `0x02A3` because the pinned `windows` 0.62.2 feature set does not expose that constant in the generated `WindowsAndMessaging` module. A failed `SetWindowSubclass` call now returns an explicit `E_FAIL` with a diagnostic message through `windows::core::Error::new`, avoiding APIs absent from `windows-result` 0.4.x.

**Not complete:** this slice handles Win32 mouse messages; touch/pen `WM_POINTER` conversion through `SendPointerInput` is still pending. It also does not render libmpv video through D3D11. Do not enable this path for normal CineHub use or consider the player mouse issue fixed yet.

Next gate: compile and run the Windows job, then validate click/double-click, both wheel axes, drag outside the WebView, cursor changes, focus, resize and repeated close/reopen on a physical Windows machine. CI compilation does not prove physical mouse behavior or visual correctness.
