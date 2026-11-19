"""Regressões para as versões e invariantes do workflow principal de CI."""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "build.yml"
CHECKOUT_V4_SHA = "fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09"


class CiWorkflowTests(unittest.TestCase):
    def test_all_checkout_steps_use_the_pinned_v4_revision(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        checkout_refs = re.findall(r"uses:\s*actions/checkout@([0-9a-f]+)", workflow)
        self.assertGreater(len(checkout_refs), 0)
        self.assertEqual(checkout_refs, [CHECKOUT_V4_SHA] * len(checkout_refs))

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
