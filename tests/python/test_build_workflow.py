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

    def test_workflow_declares_required_jobs(self):
        for name in ("test", "lint", "web-smoke", "desktop", "android", "release"):
            with self.subTest(job=name):
                self.assertRegex(
                    self.workflow,
                    rf"(?m)^  {re.escape(name)}:\s*$",
                    f"Job obrigatório ausente: {name}",
                )

    def test_all_ubuntu_dependency_jobs_select_mirror_before_apt(self):
        for name in ("test", "lint", "desktop"):
            with self.subTest(job=name):
                block = job_block(self.workflow, name)
                selection = block.find("select-ubuntu-mirror.sh")
                apt_commands = [
                    match.start()
                    for match in re.finditer(r"(?m)^\s+sudo apt-get\b", block)
                ]
                self.assertGreaterEqual(
                    selection, 0, "Job não seleciona um espelho Ubuntu"
                )
                self.assertTrue(apt_commands, "Job não contém instalação/atualização APT")
                self.assertLess(
                    selection,
                    min(apt_commands),
                    "A seleção do espelho precisa ocorrer antes de qualquer apt-get",
                )
                self.assertIn(
                    "Acquire::Retries=5", block, "Faltou retry nas operações APT"
                )
                self.assertIn(
                    "Acquire::http::Timeout=30", block, "Faltou timeout HTTP do APT"
                )
                self.assertIn(
                    "Acquire::https::Timeout=30", block, "Faltou timeout HTTPS do APT"
                )

    def test_python_regression_suite_runs_in_existing_lint_job(self):
        lint = job_block(self.workflow, "lint")
        self.assertIn(
            "python3 -m unittest discover -s tests/python -v",
            lint,
            "A suíte Python que contém estas verificações não está no build.yml",
        )

    def test_android_build_preserves_all_four_rust_targets(self):
        android = job_block(self.workflow, "android")
        targets = (
            "aarch64-linux-android",
            "armv7-linux-androideabi",
            "i686-linux-android",
            "x86_64-linux-android",
        )
        target_step = next(
            (
                line
                for line in android.splitlines()
                if "targets:" in line
            ),
            "",
        )
        for target in targets:
            with self.subTest(target=target):
                self.assertIn(target, target_step)

    def test_android_job_keeps_google_tv_emulator_smoke_test(self):
        android = job_block(self.workflow, "android")
        self.assertIn("system-images;android-34;google-tv;x86", android)
        self.assertIn("adb install -r", android)
        self.assertIn("adb shell monkey", android)
        self.assertIn("check_app_alive", android)
        for keycode in ("19 20 21 22 23",):
            self.assertIn(keycode, android)

    def test_release_is_gated_by_every_required_job(self):
        release = job_block(self.workflow, "release")
        for dependency in ("test", "lint", "web-smoke", "desktop", "android"):
            with self.subTest(dependency=dependency):
                self.assertRegex(
                    release,
                    rf"needs:\s*\[[^\]]*\b{dependency}\b",
                )
                self.assertIn(
                    f"needs.{dependency}.result == 'success'",
                    release,
                )

    def test_workflow_keeps_minimal_default_permissions(self):
        self.assertRegex(
            self.workflow,
            r"(?ms)^permissions:\n  contents: read\s",
        )


if __name__ == "__main__":
    unittest.main()
