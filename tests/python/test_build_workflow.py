import pathlib
import re
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
            "apksigner sign",
            "apksigner verify",
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

    def test_only_optional_ffmpeg_download_can_fail_without_stopping_android_job(self):
        android = job_block(self.workflow, "android")
        self.assertIn("name: Extensão FFmpeg do Media3 (opcional", android)
        self.assertIn("continue-on-error: true", android)
        self.assertIn("gh release download ffmpeg-ext-1.4.1", android)
        self.assertIn("|| echo \"Extensão FFmpeg ainda não publicada", android)


if __name__ == "__main__":
    unittest.main()
