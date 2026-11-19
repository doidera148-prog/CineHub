"""Testes do ajuste de manifesto Google TV usado no build."""
import importlib.util
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "src-tauri" / "android-tv" / "patch_manifest.py"
SPEC = importlib.util.spec_from_file_location("patch_manifest", SCRIPT)
PATCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PATCH)


class PatchManifestTests(unittest.TestCase):
    def sample_manifest(self):
        return (
            '<manifest xmlns:android="http://schemas.android.com/apk/res/android">\n'
            '    <application>\n'
            '        <activity android:name=".MainActivity">\n'
            '            <intent-filter>\n'
            '                <category android:name="android.intent.category.LAUNCHER" />\n'
            '            </intent-filter>\n'
            '        </activity>\n'
            '    </application>\n'
            '</manifest>\n'
        )

    def test_adds_tv_features_launcher_and_banner(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = pathlib.Path(directory) / "AndroidManifest.xml"
            manifest.write_text(self.sample_manifest(), encoding="utf-8")
            PATCH.main(str(manifest))
            result = manifest.read_text(encoding="utf-8")
            self.assertIn('android.software.leanback', result)
            self.assertIn('android.hardware.touchscreen', result)
            self.assertIn('android.intent.category.LEANBACK_LAUNCHER', result)
            self.assertIn('android:banner="@drawable/banner"', result)
            self.assertTrue((pathlib.Path(directory) / "res/drawable/banner.png").is_file())

    def test_is_idempotent(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = pathlib.Path(directory) / "AndroidManifest.xml"
            manifest.write_text(self.sample_manifest(), encoding="utf-8")
            PATCH.main(str(manifest))
            first = manifest.read_text(encoding="utf-8")
            PATCH.main(str(manifest))
            second = manifest.read_text(encoding="utf-8")
            self.assertEqual(first, second)
            self.assertEqual(second.count('android.intent.category.LEANBACK_LAUNCHER'), 1)
            self.assertEqual(second.count('android.software.leanback'), 1)

    def test_fails_when_launcher_category_is_missing(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = pathlib.Path(directory) / "AndroidManifest.xml"
            manifest.write_text('<manifest>\n<application>\n</application>\n</manifest>', encoding="utf-8")
            with self.assertRaises(SystemExit):
                PATCH.main(str(manifest))


if __name__ == "__main__":
    unittest.main()
