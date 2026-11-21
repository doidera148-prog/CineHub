"""Regressões para as versões e invariantes do workflow principal de CI."""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "build.yml"
WORKFLOW_DIR = ROOT / ".github" / "workflows"
CHECKOUT_V4_SHA = "fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09"


class CiWorkflowTests(unittest.TestCase):
    def test_all_checkout_steps_use_the_pinned_v4_revision(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        checkout_refs = re.findall(r"uses:\s*actions/checkout@([0-9a-f]+)", workflow)
        self.assertGreater(len(checkout_refs), 0)
        self.assertEqual(checkout_refs, [CHECKOUT_V4_SHA] * len(checkout_refs))

    def test_every_external_github_action_is_pinned_to_a_full_commit_sha(self):
        action_refs = []
        for path in sorted(WORKFLOW_DIR.glob("*.yml")):
            workflow = path.read_text(encoding="utf-8")
            matches = re.findall(
                r"(?m)^\s*(?:-\s*)?uses:\s*([^@\s]+)@([^\s#]+)",
                workflow,
            )
            for action, ref in matches:
                if action.startswith("./"):
                    continue
                action_refs.append((path.name, action, ref))
                self.assertRegex(
                    ref,
                    r"^[0-9a-f]{40}$",
                    f"{path.name} usa {action}@{ref}; fixe a Action por SHA completo",
                )
        self.assertGreater(len(action_refs), 0, "Nenhuma GitHub Action foi encontrada")

    def test_security_advisory_ignores_are_explicit_and_have_reasons(self):
        deny = (ROOT / "src-tauri/deny.toml").read_text(encoding="utf-8")
        ignores = re.findall(
            r'\{ id = "([^"]+)", reason = "([^"]+)" \}', deny
        )
        self.assertEqual(len(ignores), 5)
        self.assertEqual(len({advisory for advisory, _ in ignores}), 5)
        self.assertTrue(all(reason.strip() for _, reason in ignores))
        self.assertNotIn('ignore = ["*"]', deny)

    def test_readme_matches_blocking_security_checks_and_release_gate(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("essas verificações são bloqueantes", readme)
        self.assertIn(
            "todos os builds desktop (Linux, Windows e macOS) e Android passam",
            readme,
        )
        self.assertNotIn("etapas não bloqueantes", readme)
        self.assertNotIn("pelo menos um build de plataforma passam", readme)

    def test_web_smoke_uses_node24(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        web_smoke = workflow.split("  web-smoke:", 1)[1].split("\n  desktop:", 1)[0]
        self.assertIn('node-version: "24"', web_smoke)
        self.assertNotIn('node-version: "20"', web_smoke)


    def test_release_requires_all_platform_builds(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        release = workflow.split("  release:", 1)[1]
        condition = re.search(r"if:\s*\$\{\{(.*?)\}\}", release, re.DOTALL)
        self.assertIsNotNone(condition, "release precisa ter condição explícita")
        expression = condition.group(1)
        self.assertIn("needs.desktop.result == 'success'", expression)
        self.assertIn("needs.android.result == 'success'", expression)
        self.assertNotRegex(expression, r"needs\.desktop\.result == 'success'\s*\|\|\s*needs\.android\.result == 'success'")

    def test_android_keeps_all_four_requested_abis(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn(
            "targets: aarch64-linux-android,armv7-linux-androideabi,"
            "i686-linux-android,x86_64-linux-android",
            workflow,
        )

    def test_android_requires_an_apk_for_each_abi(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        verification = workflow.split(
            "      - name: Verificar geração dos APKs", 1
        )[1].split("      - name: Assinar APK", 1)[0]
        self.assertIn("for abi in arm arm64 x86 x86_64; do", verification)
        self.assertIn(
            'apk="$apk_root/$abi/release/app-$abi-release-unsigned.apk"',
            verification,
        )
        self.assertIn('[ ! -s "$apk" ]', verification)


if __name__ == "__main__":
    unittest.main()
