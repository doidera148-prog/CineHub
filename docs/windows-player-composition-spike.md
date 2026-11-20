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
3. Composition: place the WebView2 visual and the mpv video visual in one DirectComposition tree with the WebView above the video. The opt-in branch now has this tree and a shared-device swap chain; verify alpha/transparency, clipping, resize, DPI, fullscreen and input hit testing.
4. Integration: the opt-in Windows path now selects vo=libmpv and renders to the shared swap-chain backbuffer. Keep it experimental; switch the default Windows player from wid/child-HWND embedding and remove obsolete HWND forwarding only after real-device validation.
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

The later “In-repository vendor strategy” and “Native libmpv D3D11 render bridge” sections below supersede this initial sequencing note: the shared-device composition surface and opt-in renderer have since been implemented, but still require real-Windows validation.

### Fork creation status

The exact upstream source seam has been identified, but no Wry fork or production source patch is claimed by this commit. The available GitHub integration for this task can create and update files/branches in accessible repositories, but does not expose the GitHub **Fork repository** operation. Creating a new repository fork therefore requires the repository owner to create the fork in GitHub first; after that, the fork can be updated with the pinned patch and referenced from CineHub. No unverified repository URL or dependency override should be added.


## In-repository vendor strategy (no user-created fork required)

To avoid requiring the repository owner to create a separate GitHub fork, the exact Wry `0.57.2` source is vendored under `src-tauri/vendor/wry` and selected with the existing Cargo `[patch.crates-io]` mechanism. This keeps the upstream source baseline, licenses and package metadata inside CineHub, while all changes remain reviewable in the CineHub branch. The upstream baseline is tag `wry-v0.57.2`, commit `bbba6b6312af92fbb969789a63768f4b4bb6d268`.

The vendor baseline is connected to Cargo and the lockfile points to the local package. The standard WebView2 controller remains the default. An experimental Windows-only composition path now exists behind `CINEHUB_WEBVIEW2_COMPOSITION_POC`; it creates `ICoreWebView2CompositionController`, attaches a DirectComposition visual to the existing Wry container HWND, and retains the COM objects for the WebView lifetime. The path is deliberately opt-in. Win32 mouse, touch and pen input forwarding plus cursor/focus/bounds integration are now present, but this was the initial hosting slice; later sections below document the connected D3D11 renderer and runtime smoke test, while physical Windows input/visual validation remains outstanding.

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

- Handles `WM_POINTERDOWN`, `WM_POINTERUPDATE`, `WM_POINTERUP`, `WM_POINTERENTER` and `WM_POINTERLEAVE` for `PT_TOUCH` and `PT_PEN`; pointer-mouse and touchpad types are left to the normal Win32 procedure.
- Builds the stable `ICoreWebView2PointerInfo` from Win32 `POINTER_INFO`, `POINTER_TOUCH_INFO` or `POINTER_PEN_INFO`, including contact area, pressure, rotation and pen tilt.
- Converts screen pixel positions/contact rectangles and HIMETRIC positions to WebView-relative client coordinates before calling `SendPointerInput`; keeps the source digitizer's physical calibration rectangle intact.
- Consumes a recognized touch/pen pointer message only after WebView2 successfully accepts it, avoiding an additional synthesized mouse stream for the same handled input.

**Status at this earlier checkpoint:** video rendering was not yet connected; later sections below document the D3D11 integration and runtime validation. Touch/pen behavior still requires physical Windows validation. Do not enable this path for normal CineHub use or consider the player input issue fixed yet.

Next gate: compile and run the Windows job, then validate click/double-click, both wheel axes, drag outside the WebView, cursor changes, focus, resize and repeated close/reopen on a physical Windows machine. CI compilation does not prove physical mouse behavior or visual correctness.


The pointer path uses the existing pinned `windows 0.62.2` and `webview2-com 0.39.1` APIs. The Windows CI job validates compilation and static regression assertions only; physical touch/pen testing remains a separate gate.


## Native libmpv D3D11 render bridge — 2026-11-20

The Windows player now has an isolated Rust bridge for libmpv's native Direct3D 11 render API in src-tauri/src/player/d3d11.rs:

- Creates mpv_render_context with MPV_RENDER_API_TYPE_D3D11 and the host-owned ID3D11Device.
- Passes a host-owned ID3D11Texture2D through MPV_RENDER_PARAM_D3D11_FBO.
- Exposes render-update polling, frame rendering, swap reporting and deterministic render-context release.
- Keeps the native ABI structs/constants local because the pinned libmpv2-sys 4.0.1 pregenerated bindings include the generic render API but not the newer D3D11 parameter declarations.
- Ties the render context lifetime to Mpv so it is dropped before the mpv core.

The bridge is now connected to the opt-in Wry composition host. Wry creates one hardware D3D11 device with BGRA support and multithread protection, then creates a premultiplied BGRA flip-sequential composition swap chain on that same device. The DirectComposition target has separate sibling visuals: the video swap-chain visual is behind the WebView2 visual, and the WebView2 composition controller owns the upper visual. Wry exposes narrow C-ABI functions to hand libmpv an additional COM reference to the device/backbuffer, present the frame, and position/resize the video visual on the WebView-owning thread.

When CINEHUB_WEBVIEW2_COMPOSITION_POC is set before app startup, Windows libmpv uses vo=libmpv (without wid), disables mpv's mouse-driven OSC, creates its D3D11 render context before loadfile, polls for frames, renders into the swap-chain backbuffer, and presents it. CineHub's WebView controls remain the input surface. ResizeBuffers is deferred until the player releases its backbuffer reference. For Windows builds compiled with the `mpv` feature, `src/main.rs` now sets this variable before calling `cinehub_lib::run()`, so Wry sees it before creating the WebView. Set `CINEHUB_DISABLE_WEBVIEW2_COMPOSITION=1` in the process environment to force the standard HWND path for rollback/diagnostics. Android, Linux, macOS and Windows builds without `mpv` are unaffected. Closing and player geometry commands are routed to the composition host in this mode.

This is still a rendering integration prototype, not a verified player fix: CI validates compilation and regression guards, but does not prove actual GPU presentation, transparency of the WebView content over the video, DPI/fullscreen behavior, physical mouse/touch input, frame pacing, or repeated create/close safety on a real Windows desktop. The WebView must expose a transparent player region for the surface behind it to be visible. Do not enable this mode for normal CineHub use or remove the existing player path until the full acceptance criteria below pass.

The existing Windows CI build compiles this module with the mpv feature. The existing Rust OSC test now also checks the native ABI structure layout/constants, and the existing Python regression test checks the bridge entry points and feature gate. No additional workflow or duplicate test run was added.


## Shared D3D11 composition surface — 2026-11-20

The opt-in Windows path now has an end-to-end code connection between Wry and libmpv:

- Wry creates a hardware D3D11 device with BGRA support and multithread protection.
- The DXGI composition swap chain is created from that same D3D11 device, with BGRA8, premultiplied alpha, two buffers, stretch scaling and flip-sequential presentation.
- DirectComposition now has a root with independent sibling visuals: the video swap-chain visual is inserted below the WebView2 visual. The video visual is hidden until player geometry is supplied.
- Wry exposes narrow C-ABI calls for an additional COM reference to the shared device/backbuffer, frame presentation, and video visual geometry updates. DComp visual changes are marshalled synchronously to the thread that owns the WebView HWND.
- The Windows opt-in player selects `vo=libmpv`, omits `wid`, creates its D3D11 render context before loading the video, polls for render updates and presents each rendered frame through the host swap chain. It releases each backbuffer reference before Present and defers ResizeBuffers until a new backbuffer is acquired.
- The composition close and geometry commands are routed to the host. The normal HWND/wid path remains selected when the environment variable is absent.
- The existing Python regression method was expanded. The Windows build/test command remains the existing single targeted player test in `.github/workflows/build.yml`; no new workflow or duplicate test step was added.

**Not yet established:** a successful Windows CI build can verify Rust/API integration and the ABI guards, but cannot verify the GPU output, transparent WebView player region, physical pointer behavior, real resize/DPI/fullscreen, audio/video frame pacing or repeated open/close on a Windows desktop. This is an opt-in implementation slice, not a validated fix and not a change to the default player.

## Runtime D3D11/DXGI smoke test — 2026-11-20

The existing Windows-targeted Rust test mpv_osc_script_options_are_valid now also performs a native runtime smoke test rather than stopping at ABI layout assertions:

- Creates a D3D11 device with BGRA support, trying the hardware driver first and WARP as a software fallback for hosted CI runners.
- Creates a 64×64 DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL composition swap chain with DXGI_ALPHA_MODE_PREMULTIPLIED, matching the production surface descriptor.
- Starts a separate libmpv instance with vo=libmpv, creates the D3D11 render context before loading media, and opens a locally generated PPM test frame (no network fixture).
- Waits for libmpv's render-update callback, renders the decoded frame into the swap-chain backbuffer, releases the backbuffer reference, calls Present, and reports the swap.
- The callback only writes an atomic notification flag. mpv_render_context_update, mpv_render_context_render, and presentation remain on the test/player render thread.

This exercises the libmpv D3D11 backend at runtime, the texture/backbuffer contract, the composition swap-chain creation, and DXGI presentation. The Windows job runs it through the existing single targeted test step; no workflow or duplicate test step was added.

**What it does not prove:** when WARP is selected, this is software rasterization rather than the user's physical GPU. The test does not create the full WebView2/DirectComposition visual tree, prove alpha transparency or z-order, or simulate physical mouse/touch input. Those remain acceptance checks on a real Windows desktop. The standard player remains the default, and composition stays opt-in.


## Revisão de ciclo de vida — após o build 609

A composição WebView2/DirectComposition é selecionada nos builds Windows com a feature `mpv` antes da criação da janela Tauri. A variável `CINEHUB_DISABLE_WEBVIEW2_COMPOSITION=1` continua disponível para diagnóstico e rollback; quando definida, `main.rs` remove a flag de composição e o Wry usa o controller HWND convencional.

A inicialização do host agora é transacional nas duas últimas etapas de publicação:

- A superfície D3D11 compartilhada é registrada antes de o estado do callback Win32 ser convertido em ponteiro bruto.
- Se o registro da superfície falhar, o listener `CursorChanged` é removido, `RootVisualTarget` é desconectado e a alteração é enviada com `Commit`.
- O estado de entrada permanece sob propriedade de um `Box` até `SetWindowSubclass` confirmar sucesso. Se a instalação falhar, o `Box` é liberado automaticamente, a superfície é desregistrada, o callback é removido e o visual é desconectado.
- Após a instalação bem-sucedida, o ponteiro bruto passa a ser propriedade do host e é liberado por `CompositionHost::shutdown`, antes do fechamento do controller comum.

O caminho padrão HWND continua preservado para rollback. O smoke test de CI exercita libmpv/D3D11/DXGI, mas não simula perda real do dispositivo gráfico nem substitui a validação de transparência, escala DPI e interação física em hardware Windows. **Naquele checkpoint**, a recuperação automática após remoção do dispositivo D3D11 ainda era um item separado; a implementação posterior está descrita na seção seguinte.


## Recuperação de perda do dispositivo D3D11 — 2026-11-20

O caminho Windows agora reconhece a remoção/reset do dispositivo gráfico e tenta uma recuperação limitada, em vez de tratar todos os erros DXGI como falha genérica:

- `ID3D11Device::GetDeviceRemovedReason` é consultado antes de liberar o dispositivo antigo. Os HRESULTs `DXGI_ERROR_DEVICE_REMOVED`, `DXGI_ERROR_DEVICE_HUNG`, `DXGI_ERROR_DEVICE_RESET` e `DXGI_ERROR_DRIVER_INTERNAL_ERROR` são identificados explicitamente.
- A aquisição do backbuffer preserva os HRESULTs de `ResizeBuffers` e `GetBuffer`. Uma janela minimizada continua sendo tratada como ausência temporária de frame, mas uma falha real deixa de ser silenciosamente confundida com minimização.
- Ao detectar device loss, o worker libera primeiro o `mpv_render_context` antigo. Só então envia uma mensagem síncrona para a thread que possui o HWND.
- No thread da janela, o Wry cria um novo dispositivo D3D11, swap chain DXGI e árvore DirectComposition, reconecta `RootVisualTarget` do WebView2 e restaura a geometria atual do vídeo.
- O worker obtém o novo dispositivo e cria outro render context libmpv sem descarregar o vídeo/áudio em andamento.
- Há no máximo uma tentativa automática de recuperação por reprodução. Uma segunda perda, ou uma falha durante a reconstrução, encerra a reprodução com diagnóstico explícito em vez de reutilizar recursos inválidos.

O teste Windows existente `mpv_osc_script_options_are_valid` agora também verifica os HRESULTs classificados como device loss; a suíte Python existente verifica o contrato entre a mensagem Win32, o host Wry e o loop do player. O `.github/workflows/build.yml` continua sendo o único workflow e o teste de integração Windows continua executando uma única vez.

**Limite da validação:** o CI compila a rotina de recuperação, testa a classificação dos HRESULTs e executa o smoke test nativo de renderização/`Present`. Ele não simula uma remoção real do adaptador durante reprodução nem valida visualmente a troca da árvore DirectComposition em uma GPU física. Esse cenário precisa de aceitação em Windows real; a tentativa única e o encerramento controlado permanecem como proteção contra loop/inconsistência caso o driver continue falhando.
