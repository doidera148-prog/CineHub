import os
import pathlib
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / ".github" / "scripts" / "select-ubuntu-mirror.sh"


class UbuntuMirrorSelectionTests(unittest.TestCase):
    def test_selects_fastest_reachable_brazilian_mirror_and_preserves_security(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            sources = root / "sources"
            sources.mkdir()
            source_file = sources / "ubuntu.sources"
            source_file.write_text(
                "Types: deb\nURIs: http://azure.archive.ubuntu.com/ubuntu\n"
                "Suites: jammy jammy-updates\n\n"
                "Types: deb\nURIs: http://security.ubuntu.com/ubuntu\n"
                "Suites: jammy-security\n",
                encoding="utf-8",
            )
            fake_bin = root / "bin"
            fake_bin.mkdir()
            fake_curl = fake_bin / "curl"
            fake_curl.write_text(
                "#!/usr/bin/env bash\n"
                'url="${!#}"\n'
                'case "$url" in\n'
                '  https://ubuntu.c3sl.ufpr.br/*) echo 0.30 ;;\n'
                '  https://mirror.ufscar.br/*) echo 0.10 ;;\n'
                '  https://br.archive.ubuntu.com/*) echo 0.20 ;;\n'
                '  https://archive.ubuntu.com/*) echo 0.40 ;;\n'
                '  *) exit 1 ;;\n'
                "esac\n",
                encoding="utf-8",
            )
            fake_curl.chmod(0o755)
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env['PATH']}",
                    "APT_SOURCES_DIR": str(sources),
                    "UBUNTU_CODENAME": "jammy",
                }
            )
            result = subprocess.run(
                ["bash", str(SCRIPT)],
                check=True,
                capture_output=True,
                text=True,
                env=env,
            )
            updated = source_file.read_text(encoding="utf-8")
            self.assertIn("https://mirror.ufscar.br/ubuntu", updated)
            self.assertIn("http://security.ubuntu.com/ubuntu", updated)
            self.assertIn("0.10s", result.stdout)

    def test_keeps_existing_sources_when_no_mirror_responds(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            sources = root / "sources"
            sources.mkdir()
            source_file = sources / "ubuntu.list"
            original = "deb http://azure.archive.ubuntu.com/ubuntu jammy main\n"
            source_file.write_text(original, encoding="utf-8")
            fake_bin = root / "bin"
            fake_bin.mkdir()
            fake_curl = fake_bin / "curl"
            fake_curl.write_text("#!/usr/bin/env bash\nexit 7\n", encoding="utf-8")
            fake_curl.chmod(0o755)
            env = os.environ.copy()
            env.update({"PATH": f"{fake_bin}:{env['PATH']}", "APT_SOURCES_DIR": str(sources)})
            subprocess.run(["bash", str(SCRIPT)], check=True, capture_output=True, env=env)
            self.assertEqual(source_file.read_text(encoding="utf-8"), original)


if __name__ == "__main__":
    unittest.main()
