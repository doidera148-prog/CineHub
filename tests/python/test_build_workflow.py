import pathlib
import re
import subprocess
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "build.yml"


def job_block(workflow, name):
    match = re.search(
        rf"(?ms)^  {re.escape(name)}:\n(.*?)(?=^  [a-zA-Z0-9_-]+:\n|\Z)",
        workflow,
    )
    if not match:
        raise AssertionError(f"Job '{name}' não encontrado em build.yml")
    return match.group(1)


class BuildWorkflowRegressionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_workflow_has_expected_triggers_and_cancels_obsolete_builds(self):
        for trigger in ("push:", "pull_request:", "workflow_dispatch:"):
            with self.subTest(trigger=trigger):
                self.assertIn(trigger, self.workflow)
        self.assertIn("cancel-in-progress: true", self.workflow)
        self.assertIn("group: build-" + "$" + "{{ github.ref }}", self.workflow)

    def test_workflow_declares_required_jobs_without_duplicates(self):
        expected = ("test", "lint", "web-smoke", "desktop", "android", "release")
        declared = re.findall(r"(?m)^  ([a-zA-Z0-9_-]+):\s*$", self.workflow)
        for name in expected:
            with self.subTest(job=name):
                self.assertIn(name, declared, f"Job obrigatório ausente: {name}")
        self.assertEqual(len(declared), len(set(declared)), "Há nomes de jobs duplicados")

    def test_linux_jobs_select_mirror_before_apt_and_keep_retries(self):
        for name in ("test", "lint", "desktop"):
            with self.subTest(job=name):
                block = job_block(self.workflow, name)
                selection = block.find("select-ubuntu-mirror.sh")
                apt_commands = [
                    match.start()
                    for match in re.finditer(r"(?m)^\s+sudo apt-get\b", block)
                ]
                self.assertGreaterEqual(selection, 0, "Job não seleciona espelho Ubuntu")
                self.assertTrue(apt_commands, "Job não contém operações APT")
                self.assertLess(selection, min(apt_commands))
                for option in (
                    "Acquire::Retries=5",
                    "Acquire::http::Timeout=30",
                    "Acquire::https::Timeout=30",
                ):
                    with self.subTest(option=option):
                        self.assertIn(option, block)

    def test_lint_checks_workflow_and_runs_python_suite_exactly_once(self):
        lint = job_block(self.workflow, "lint")
        checks = (
            "cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check",
            "prettier@3.6.2 --check",
            "node --check",
            "eslint@10.15.0",
            "python3 -m compileall -q src-tauri/google-tv tests/python",
            "python3 -m unittest discover -s tests/python -v",
            "cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings",
            "cargo-deny-action@v2",
        )
        for command in checks:
            with self.subTest(command=command):
                self.assertIn(command, lint)
        self.assertEqual(
            lint.count("python3 -m unittest discover -s tests/python -v"),
            1,
            "A suíte Python deve ser executada uma única vez",
        )
        self.assertIn("find .github src-tauri tests", lint)

    def test_web_smoke_runs_security_and_regression_suites_once(self):
        web = job_block(self.workflow, "web-smoke")
        for command in (
            "npm ci",
            "npm ls --all",
            "npm audit --audit-level=high",
            "npx playwright install --with-deps chromium",
            "npm test",
            "node remote-navigation.test.mjs",
            "node search.test.mjs",
        ):
            with self.subTest(command=command):
                self.assertEqual(web.count(command), 1)

    def test_desktop_matrix_preserves_platforms_and_windows_mpv_build(self):
        desktop = job_block(self.workflow, "desktop")
        for platform in ("ubuntu-22.04", "windows-latest", "macos-latest"):
            with self.subTest(platform=platform):
                self.assertIn(platform, desktop)
        self.assertIn(
            "cargo tauri build --config tauri.windows.conf.json --features mpv",
            desktop,
        )
        self.assertIn(
            "cargo test --features mpv --lib mpv_osc_script_options_are_valid",
            desktop,
        )
        self.assertIn("actions/upload-artifact@", desktop)
        self.assertIn("cinehub-" + "$" + "{{ matrix.platform }}", desktop)

    def test_android_preserves_four_abis_signing_and_google_tv_smoke(self):
        android = job_block(self.workflow, "android")
        targets = (
            "aarch64-linux-android",
            "armv7-linux-androideabi",
            "i686-linux-android",
            "x86_64-linux-android",
        )
        target_line = next(
            (line for line in android.splitlines() if "targets:" in line), ""
        )
        for target in targets:
            with self.subTest(target=target):
                self.assertIn(target, target_line)
        for required in (
            "for abi in arm arm64 x86 x86_64; do",
            'app-$abi-release-unsigned.apk',
            "zipalign",
            '"$BT/apksigner" sign',
            '"$BT/apksigner" verify',
            "system-images;android-34;google-tv;x86",
            "-skip-adb-auth",
            "adb install -r",
            "adb shell monkey",
            "check_app_alive",
            "for key in 19 20 21 22 23",
        ):
            with self.subTest(required=required):
                self.assertIn(required, android)

    def test_release_is_gated_by_all_build_jobs_and_publishes_all_formats(self):
        release = job_block(self.workflow, "release")
        for dependency in ("test", "lint", "web-smoke", "desktop", "android"):
            with self.subTest(dependency=dependency):
                self.assertRegex(
                    release,
                    rf"needs:\s*\[[^\]]*\b{dependency}\b",
                )
                self.assertIn(f"needs.{dependency}.result == 'success'", release)
        for artifact in ("*.apk", "*.msi", "*.exe", "*.dmg", "*.deb", "*.AppImage"):
            with self.subTest(artifact=artifact):
                self.assertIn(artifact, release)

    def test_workflow_uses_minimal_default_and_scoped_release_permissions(self):
        self.assertRegex(
            self.workflow,
            r"(?m)^permissions:\n  contents: read\s",
        )
        release = job_block(self.workflow, "release")
        self.assertRegex(
            release,
            r"(?m)^    permissions:\n      contents: write\s",
        )
        for job in ("test", "lint", "web-smoke", "desktop", "android"):
            with self.subTest(job=job):
                self.assertNotRegex(
                    job_block(self.workflow, job),
                    r"(?m)^    permissions:\n      contents: write",
                )

    def test_build_jobs_use_expected_runners_and_checkout_source(self):
        expected_runners = {
            "test": "runs-on: ubuntu-22.04",
            "lint": "runs-on: ubuntu-22.04",
            "web-smoke": "runs-on: ubuntu-22.04",
            "desktop": "runs-on: ${{ matrix.platform }}",
            "android": "runs-on: ubuntu-24.04",
        }
        for name, runner_line in expected_runners.items():
            with self.subTest(job=name):
                block = job_block(self.workflow, name)
                self.assertIn(runner_line, block)
                self.assertIn("actions/checkout@", block)

    def test_android_signing_uses_repository_secrets_and_verifies_each_apk(self):
        android = job_block(self.workflow, "android")
        for secret in (
            "ANDROID_KEYSTORE_BASE64",
            "ANDROID_KEYSTORE_PASSWORD",
            "ANDROID_KEY_ALIAS",
            "ANDROID_KEY_PASSWORD",
        ):
            with self.subTest(secret=secret):
                self.assertIn(f"secrets.{secret}", android)
        self.assertIn('find src-tauri/gen/android/app/build/outputs/apk -name "*-unsigned.apk"', android)
        self.assertIn('while read -r apk; do', android)
        self.assertIn('"$BT/apksigner" sign', android)
        self.assertIn('"$BT/apksigner" verify --verbose "$out"', android)
        self.assertIn("path: signed/*.apk", android)

    def test_google_tv_emulator_has_timeout_and_checks_boot_and_process(self):
        android = job_block(self.workflow, "android")
        emulator = android.split("      - name: Instalar e iniciar CineHub em emulador Google TV", 1)
        self.assertEqual(len(emulator), 2, "Etapa do emulador Google TV ausente ou duplicada")
        emulator_step = emulator[1].split("      - uses: actions/upload-artifact@", 1)[0]
        self.assertRegex(emulator_step, r"(?m)^        timeout-minutes: 20$")
        for check in (
            "sys.boot_completed",
            "adb devices",
            "adb install -r",
            "adb shell monkey",
            "check_app_alive",
            "diagnose_app_exit",
            "reason=12 (DEPENDENCY DIED)",
            "com.google.android.gms/.fonts.provider.FontsProvider",
            "transient_recovery_attempted=0",
            "Aguardando estabilização",
        ):
            with self.subTest(check=check):
                self.assertIn(check, emulator_step)

    def test_windows_player_build_is_conditional_and_runs_targeted_test(self):
        desktop = job_block(self.workflow, "desktop")
        for required in (
            "if: matrix.platform == 'windows-latest'",
            "if: matrix.platform == 'windows-latest' && steps.mpv-cache.outputs.cache-hit != 'true'",
            "if: matrix.platform != 'windows-latest'",
            "if: matrix.platform == 'windows-latest'\n        working-directory: src-tauri",
            "--features mpv",
            "mpv_osc_script_options_are_valid",
        ):
            with self.subTest(required=required):
                self.assertIn(required, desktop)

    def test_release_never_runs_for_pull_requests_and_waits_for_all_jobs(self):
        release = job_block(self.workflow, "release")
        self.assertIn("github.event_name != 'pull_request'", release)
        self.assertIn("always()", release)
        self.assertIn("needs: [test, lint, web-smoke, desktop, android]", release)
        for job in ("test", "lint", "web-smoke", "desktop", "android"):
            self.assertIn(f"needs.{job}.result == 'success'", release)

    def test_release_downloads_artifacts_before_publishing_and_limits_cleanup(self):
        release = job_block(self.workflow, "release")
        download = release.find("actions/download-artifact@")
        publish = release.find("softprops/action-gh-release@")
        cleanup = release.find("gh release delete")
        self.assertGreaterEqual(download, 0)
        self.assertGreater(publish, download)
        self.assertGreater(cleanup, publish)
        self.assertIn("tail -n +11", release)
        self.assertIn("fail_on_unmatched_files: false", release)

    def test_android_and_emulator_critical_scripts_fail_fast(self):
        android = job_block(self.workflow, "android")
        for marker in (
            "      - name: Verificar geração dos APKs",
            "      - name: Assinar APK",
            "      - name: Verificar APK Google TV ARM64",
            "      - name: Habilitar aceleração KVM no Ubuntu",
            "      - name: Instalar e iniciar CineHub em emulador Google TV",
        ):
            with self.subTest(step=marker):
                self.assertIn(marker, android)
        self.assertGreaterEqual(android.count("set -euo pipefail"), 4)

    def test_windows_composition_host_selection_cleanup_and_fallback(self):
        wry = (ROOT / "src-tauri/vendor/wry/src/webview2/mod.rs").read_text(
            encoding="utf-8"
        )
        composition = (
            ROOT / "src-tauri/vendor/wry/src/webview2/composition.rs"
        ).read_text(encoding="utf-8")
        self.assertIn("CINEHUB_WEBVIEW2_COMPOSITION_POC", wry)
        self.assertIn(
            "style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,",
            wry,
            "O host WRY_WEBVIEW precisa habilitar WM_*DBLCLK para encaminhar duplo clique",
        )
        self.assertIn("Some(host) => host.controller.clone()", wry)
        self.assertIn("None => Self::create_controller(", wry)
        self.assertIn("host.shutdown();", wry)
        self.assertIn("CreateCoreWebView2CompositionController", composition)
        self.assertIn("CreateCoreWebView2CompositionControllerCompletedHandler", composition)
        self.assertIn(
            "let result: webview2_com::Result<ICoreWebView2CompositionController>",
            composition,
        )
        self.assertIn("DCompositionCreateDevice2::<_, IDCompositionDesktopDevice>", composition)
        self.assertIn(
            "let composition_controller = webview2_com::wait_with_pump(rx)??;",
            composition,
        )
        self.assertIn("SetRootVisualTarget", composition)
        self.assertIn("SetWindowSubclass", composition)
        registration = composition.split(
            "if let Err(error) = register_composition_surface(parent, &surface)", 1
        )
        self.assertEqual(len(registration), 2, "Falha de registro precisa de rollback explícito")
        registration_cleanup = registration[1].split(
            "let mut input_state = Box::new", 1
        )[0]
        for cleanup in (
            "remove_CursorChanged(cursor_changed_token)",
            "SetRootVisualTarget(None::<&IUnknown>)",
            "device.Commit()",
        ):
            with self.subTest(registration_cleanup=cleanup):
                self.assertIn(cleanup, registration_cleanup)
        self.assertLess(
            composition.index("register_composition_surface(parent, &surface)"),
            composition.index("let mut input_state = Box::new"),
            "A superfície deve ser registrada antes de publicar o estado do callback",
        )
        self.assertIn("let input_state_ptr = (&mut *input_state) as *mut CompositionInputState;", composition)
        self.assertIn("let input_state = Box::into_raw(input_state);", composition)
        self.assertIn("unregister_composition_surface(parent.0 as isize);", composition)
        self.assertIn("Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass}", composition)
        self.assertIn("KeyboardAndMouse::{", composition)
        self.assertIn("Pointer::{", composition)
        self.assertIn('"Win32_UI_Input_Pointer"', (ROOT / "src-tauri/vendor/wry/Cargo.toml").read_text(encoding="utf-8"))
        self.assertIn("GetPointerDeviceRects, GetPointerInfo, GetPointerPenInfo, GetPointerTouchInfo", composition)
        self.assertIn("POINTER_INFO, POINTER_PEN_INFO, POINTER_TOUCH_INFO", composition)
        self.assertIn("WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP | WM_POINTERENTER | WM_POINTERLEAVE", composition)
        self.assertIn("SendPointerInput(event_kind, &pointer_info)", composition)
        self.assertIn("SetPointerKind(native_info.pointerType.0 as u32)", composition)
        self.assertIn("SetTouchContact(contact)", composition)
        self.assertIn("SetTouchContactRaw(contact_raw)", composition)
        self.assertIn("SetPenPressure(pen_info.pressure)", composition)
        self.assertIn("himetric_client_origin", composition)
        self.assertIn("to_client_himetric_rect", composition)
        self.assertIn("SetPointerDeviceRect(pointer_device_rect_client)", composition)
        self.assertIn("screen_rect_to_client", composition)
        self.assertIn("native_info.pointerType != PT_TOUCH && native_info.pointerType != PT_PEN", composition)
        self.assertIn("GetCapture, ReleaseCapture, SetCapture, TrackMouseEvent", composition)
        self.assertIn("Gdi::{ClientToScreen, ScreenToClient}", composition)
        self.assertIn("SetCursor(Some(cursor))", composition)
        self.assertIn("WM_SETCURSOR if (lparam.0 as u16) == HTCLIENT as u16", composition)
        self.assertIn("state.composition_controller.Cursor(&mut cursor)", composition)
        self.assertIn("return windows::Win32::Foundation::LRESULT(1);", composition)
        self.assertIn("SetWindowSubclass(", composition)
        self.assertIn(".as_bool()", composition)
        self.assertIn("const WM_MOUSELEAVE: u32 = 0x02A3;", composition)
        self.assertNotIn("WM_MOUSELEAVE,", composition.split("WindowsAndMessaging::{", 1)[1].split("}", 1)[0])
        self.assertIn("Error::new(E_FAIL, \"SetWindowSubclass failed\")", composition)
        self.assertNotIn("Error::from_win32()", composition)
        self.assertIn("composition_controller: ICoreWebView2CompositionController", composition)
        self.assertIn("controller: ICoreWebView2Controller", composition)
        self.assertIn(".composition_controller\n        .SendMouseInput(", composition)
        self.assertNotIn("state\n        .controller\n        .SendMouseInput(", composition)
        self.assertIn("SendMouseInput", composition)
        for message in (
            "WM_MOUSEMOVE",
            "WM_MOUSELEAVE",
            "WM_LBUTTONDOWN",
            "WM_LBUTTONUP",
            "WM_RBUTTONDOWN",
            "WM_RBUTTONUP",
            "WM_MBUTTONDOWN",
            "WM_MBUTTONUP",
            "WM_XBUTTONDOWN",
            "WM_XBUTTONUP",
            "WM_MOUSEWHEEL",
            "WM_MOUSEHWHEEL",
        ):
            with self.subTest(message=message):
                self.assertIn(message, composition)
        self.assertIn("CursorChangedEventHandler", composition)
        self.assertIn("add_CursorChanged", composition)
        self.assertIn("MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC)", composition)
        self.assertIn("controller.SetBounds(bounds)", composition)
        self.assertIn("TrackMouseEvent", composition)
        self.assertIn("SetCapture(hwnd)", composition)
        self.assertIn("ReleaseCapture()", composition)
        self.assertIn("remove_CursorChanged", composition)
        self.assertIn("RemoveWindowSubclass", composition)

        d3d11 = (ROOT / "src-tauri/src/player/d3d11.rs").read_text(encoding="utf-8")
        self.assertEqual(d3d11.count("pub(super) fn abi_layout_is_valid"), 1)
        self.assertIn("MPV_RENDER_API_TYPE_D3D11 == b\"d3d11\\0\"", d3d11)
        self.assertIn("ID3D11Device, ID3D11Texture2D", d3d11)
        self.assertIn("MPV_RENDER_PARAM_D3D11_INIT_PARAMS: u32 = 21", d3d11)
        self.assertIn("MPV_RENDER_PARAM_D3D11_FBO: u32 = 22", d3d11)
        self.assertIn("mpv_render_context_create", d3d11)
        self.assertIn("mpv_render_context_update", d3d11)
        self.assertIn("mpv_render_context_render", d3d11)
        self.assertIn("mpv_render_context_report_swap", d3d11)
        self.assertIn("mpv_render_context_set_update_callback", d3d11)
        self.assertIn("notify_render_update", d3d11)
        self.assertIn("frame_pending.swap(false", d3d11)
        self.assertIn("mpv_render_context_free", d3d11)
        self.assertIn("GetDeviceRemovedReason", d3d11)
        self.assertIn("is_device_loss_hresult", d3d11)
        self.assertIn("RenderError::DeviceLost", d3d11)
        self.assertIn("recreate_host_device", d3d11)
        self.assertIn("texture.as_raw()", d3d11)
        self.assertIn("abi_layout_is_valid", d3d11)
        for feature in (
            "Win32_Graphics_Direct3D",
            "Win32_Graphics_Direct3D11",
            "Win32_Graphics_Dxgi",
            "Win32_Graphics_Dxgi_Common",
        ):
            with self.subTest(windows_feature=feature):
                self.assertIn(f'"{feature}"', (ROOT / "src-tauri/Cargo.toml").read_text(encoding="utf-8"))

        for required in (
            "create_smoke_swap_chain",
            "D3D_DRIVER_TYPE_HARDWARE",
            "D3D_DRIVER_TYPE_WARP",
            "CreateSwapChainForComposition(",
            "DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL",
            "DXGI_ALPHA_MODE_PREMULTIPLIED",
            "DXGI_FORMAT_B8G8R8A8_UNORM",
            "render_to_smoke_swap_chain",
            "swap_chain.Present(0",
            "present_result.0 < 0",
            "DXGI Present falhou no smoke test (HRESULT 0x{:08X})",
        ):
            with self.subTest(d3d11_runtime=required):
                self.assertIn(required, d3d11)
        smoke_imports = d3d11.split(
            "pub(super) fn create_smoke_swap_chain", 1
        )[1].split("let flags", 1)[0]
        for imported in (
            "DXGI_ALPHA_MODE_PREMULTIPLIED",
            "DXGI_FORMAT_B8G8R8A8_UNORM",
            "DXGI_SAMPLE_DESC",
        ):
            with self.subTest(smoke_import=imported):
                self.assertEqual(
                    smoke_imports.count(imported),
                    1,
                    f"Import DXGI duplicado no smoke test: {imported}",
                )

        self.assertIn(
            "let _ = unsafe { multithread.SetMultithreadProtected(true) };", d3d11
        )
        self.assertIn(
            "let _ = multithread.SetMultithreadProtected(true);", composition
        )

        for required in (
            "D3D11CreateDevice",
            "D3D11_CREATE_DEVICE_BGRA_SUPPORT",
            "D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS",
            "ID3D11Multithread",
            "SetMultithreadProtected(true)",
            "CreateSwapChainForComposition",
            "DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL",
            "DXGI_SCALING_STRETCH",
            "DXGI_ALPHA_MODE_PREMULTIPLIED",
            "DXGI_FORMAT_B8G8R8A8_UNORM",
            "Flags: 0,",
            "cinehub_wry_composition_get_device",
            "cinehub_wry_composition_get_backbuffer",
            "cinehub_wry_composition_present",
            "cinehub_wry_composition_set_video_rect",
            "root_visual.AddVisual(&video_visual_base, false",
            "root_visual.AddVisual(&webview_visual_base, true",
            "SetRootVisualTarget(&webview_visual_base",
            "WM_CINEHUB_SET_VIDEO_RECT",
            "WM_CINEHUB_RECOVER_DEVICE",
            "last_backbuffer_error",
            "cinehub_wry_composition_get_backbuffer_error",
            "cinehub_wry_composition_recreate_device",
            "fn recreate_device(",
            "dcomp_target.take()",
            "SetRoot(None::<&IDCompositionVisual>)",
            "SetRootVisualTarget(None::<&IUnknown>)",
            "state.video_rect",
            "ResizeBuffers(",
        ):
            with self.subTest(required=required):
                self.assertIn(required, composition)

        self.assertIn("SetDefaultBackgroundColor(COREWEBVIEW2_COLOR", composition)
        self.assertIn("A: 0", composition)

        wry_mod = (ROOT / "src-tauri/vendor/wry/src/webview2/mod.rs").read_text(encoding="utf-8")
        self.assertIn("CompositionHost::new(parent, hwnd, &env)", wry_mod)
        self.assertIn('"Win32_Graphics_Dxgi"', (ROOT / "src-tauri/vendor/wry/Cargo.toml").read_text(encoding="utf-8"))
        self.assertIn('"Win32_Graphics_Direct3D"', (ROOT / "src-tauri/vendor/wry/Cargo.toml").read_text(encoding="utf-8"))
        for required in (
            "cinehub_wry_composition_get_device",
            "cinehub_wry_composition_get_backbuffer",
            "cinehub_wry_composition_present",
            "ID3D11Device::from_raw",
            "ID3D11Texture2D::from_raw",
            "context.render(&texture)",
        ):
            with self.subTest(required=required):
                self.assertIn(required, d3d11)

        main_rs = (ROOT / "src-tauri/src/main.rs").read_text(encoding="utf-8")
        composition_enable = main_rs.index(
            'std::env::set_var("CINEHUB_WEBVIEW2_COMPOSITION_POC", "1")'
        )
        app_start = main_rs.index("cinehub_lib::run();")
        self.assertLess(
            composition_enable,
            app_start,
            "O modo WebView2 Composition precisa ser ativado antes de Tauri criar o WebView",
        )
        self.assertIn(
            '#[cfg(all(windows, feature = "mpv"))]', main_rs
        )
        self.assertIn(
            'std::env::var_os("CINEHUB_DISABLE_WEBVIEW2_COMPOSITION")', main_rs
        )
        self.assertIn(
            'std::env::remove_var("CINEHUB_WEBVIEW2_COMPOSITION_POC")', main_rs
        )

        player = (ROOT / "src-tauri/src/player.rs").read_text(encoding="utf-8")
        self.assertIn("pub fn player_composition_enabled() -> bool", player)
        self.assertIn("player::player_composition_enabled", (ROOT / "src-tauri/src/lib.rs").read_text(encoding="utf-8"))
        self.assertIn('init.set_option("vo", if composition_mode { "libmpv" } else { "gpu" })?', player)
        self.assertIn("super::mpv_d3d11::D3d11RenderContext::new(&mpv, &device)", player)
        self.assertIn("super::mpv_d3d11::render_to_host(parent, renderer)", player)
        self.assertIn("device_recovery_attempts == 0", player)
        self.assertIn("super::mpv_d3d11::recreate_host_device(parent)", player)
        self.assertIn("drop(render_context.take())", player)
        self.assertIn('init.set_option("vo", "libmpv")?', player)
        self.assertIn('command("loadfile", &[frame_path_string.as_str(), "replace"])', player)
        self.assertIn('.render_to_smoke_swap_chain(&swap_chain)', player)
        self.assertIn("rendered_frame", player)
        self.assertEqual(
            job_block(self.workflow, "desktop").count(
                "cargo test --features mpv --lib mpv_osc_script_options_are_valid"
            ),
            1,
            "O teste de integração D3D11 deve executar uma única vez no build Windows",
        )
        self.assertIn("COMPOSITION_CLOSE_REQUESTED", player)
        self.assertIn("mpv_d3d11::set_video_rect(parent, x, y, width, height)", player)

        app_js = (ROOT / "web/app.js").read_text(encoding="utf-8")
        self.assertIn("call('player_composition_enabled')", app_js)
        self.assertIn("native-composition-mode", app_js)
        css = (ROOT / "web/style.css").read_text(encoding="utf-8")
        self.assertIn("body.native-composition-mode { background: transparent !important; }", css)
        self.assertIn("body.native-composition-mode #player .video-js", css)

    def test_only_optional_ffmpeg_download_can_fail_without_stopping_android_job(self):
        android = job_block(self.workflow, "android")
        self.assertIn("name: Extensão FFmpeg do Media3 (opcional", android)
        self.assertIn("continue-on-error: true", android)
        self.assertIn("gh release download ffmpeg-ext-1.4.1", android)
        self.assertIn("|| echo \"Extensão FFmpeg ainda não publicada", android)


    def test_workflow_yaml_parses_and_has_no_duplicate_mapping_keys(self):
        # Psych faz parte do Ruby disponível nos runners Ubuntu do GitHub Actions.
        # A inspeção da AST evita o problema do YAML 1.1 converter a chave "on" em booleano.
        ruby_check = r"""
require "psych"
root = Psych.parse_file(ARGV.fetch(0)).root
abort "build.yml está vazio" unless root
def inspect_node(node, path = "$")
  return unless node
  if node.is_a?(Psych::Nodes::Mapping)
    seen = {}
    node.children.each_slice(2) do |key, value|
      if key.is_a?(Psych::Nodes::Scalar)
        name = key.value
        abort "Chave YAML duplicada em #{path}: #{name}" if seen.key?(name)
        seen[name] = true
      end
      inspect_node(value, "#{path}.#{key.respond_to?(:value) ? key.value : "?"}")
    end
  elsif node.respond_to?(:children)
    Array(node.children).each_with_index { |child, index| inspect_node(child, "#{path}[#{index}]") }
  end
end
inspect_node(root)
"""
        result = subprocess.run(
            ["ruby", "-e", ruby_check, str(WORKFLOW)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(
            result.returncode,
            0,
            f"YAML inválido ou com chaves duplicadas:\n{result.stderr}",
        )

    def test_job_dependencies_reference_existing_jobs_and_form_acyclic_graph(self):
        declared = set(re.findall(r"(?m)^  ([a-zA-Z0-9_-]+):\\s*$", self.workflow))
        dependencies = {}
        for name in declared:
            block = job_block(self.workflow, name)
            match = re.search(r"(?m)^    needs:\\s*(?:\\[([^\\]]*)\\]|([a-zA-Z0-9_-]+))", block)
            values = []
            if match:
                raw = match.group(1) if match.group(1) is not None else match.group(2)
                values = re.findall(r"[a-zA-Z0-9_-]+", raw)
            missing = set(values) - declared
            self.assertFalse(missing, f"Job {name} depende de jobs inexistentes: {sorted(missing)}")
            dependencies[name] = values

        visiting = set()
        visited = set()

        def visit(name):
            self.assertNotIn(name, visiting, f"Ciclo de dependências envolvendo o job {name}")
            if name in visited:
                return
            visiting.add(name)
            for dependency in dependencies[name]:
                visit(dependency)
            visiting.remove(name)
            visited.add(name)

        for name in declared:
            visit(name)

    def test_step_ids_are_unique_and_every_steps_reference_exists(self):
        for name in re.findall(r"(?m)^  ([a-zA-Z0-9_-]+):\\s*$", self.workflow):
            block = job_block(self.workflow, name)
            ids = re.findall(r"(?m)^        id: ([a-zA-Z0-9_-]+)\\s*$", block)
            self.assertEqual(len(ids), len(set(ids)), f"IDs de steps duplicados no job {name}")
            referenced = set(re.findall(r"steps\\.([a-zA-Z0-9_-]+)\\.", block))
            missing = referenced - set(ids)
            self.assertFalse(
                missing,
                f"Job {name} referencia outputs de steps sem ID: {sorted(missing)}",
            )

    def test_github_expression_delimiters_are_balanced(self):
        # Evita expressões truncadas que podem passar por verificações de substring.
        self.assertEqual(
            self.workflow.count("${{"),
            self.workflow.count("}}"),
            "Há expressão GitHub Actions sem delimitador de abertura/fechamento correspondente",
        )

    def test_each_job_has_steps_and_each_step_has_one_execution_or_action(self):
        for name in re.findall(r"(?m)^  ([a-zA-Z0-9_-]+):\\s*$", self.workflow):
            block = job_block(self.workflow, name)
            self.assertRegex(block, r"(?m)^    steps:\\s*$", f"Job {name} não declara steps")
            steps = re.findall(r"(?m)^      - (?:name:|uses:)", block)
            self.assertTrue(steps, f"Job {name} não contém etapas executáveis")
            self.assertEqual(
                len(steps),
                block.count("\\n      - "),
                f"Há etapa sem name/uses no job {name}",
            )


    def test_vendored_wry_webview2_source_is_complete_and_keeps_windows_version_use(self):
        wry_root = ROOT / "src-tauri/vendor/wry"
        webview2 = (wry_root / "src/webview2/mod.rs").read_text(encoding="utf-8")
        manifest = (wry_root / "Cargo.toml").read_text(encoding="utf-8")
        self.assertGreater(
            len(webview2.splitlines()),
            1900,
            "O módulo WebView2 do Wry parece truncado",
        )
        self.assertIn("fn is_windows_7()", webview2)
        self.assertIn("windows_version::OsVersion::current()", webview2)
        self.assertIn('windows-version = "0.1"', manifest)
        self.assertTrue(webview2.rstrip().endswith("}"))


if __name__ == "__main__":
    unittest.main()
