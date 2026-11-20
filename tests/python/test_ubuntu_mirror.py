import os
import pathlib
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / ".github" / "scripts" / "select-ubuntu-mirror.sh"


class UbuntuMirrorSelectionTests(unittest.TestCase):
    def test_selects_lowest_latency_global_mirror_and_configures_fallback(self):
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
                'if [[ "$url" == "https://mirrors.ubuntu.com/mirrors.txt" ]]; then\n'
                '  printf "%s\\n" "https://mirror.example.net/ubuntu" "https://fast.example.org/ubuntu"\n'
                '  exit 0\n'
                "fi\n"
                'case "$url" in\n'
                '  https://fast.example.org/*) echo 0.03 ;;\n'
                '  https://mirror.example.net/*) echo 0.08 ;;\n'
                '  https://archive.ubuntu.com/*) echo 0.40 ;;\n'
                '  https://us.archive.ubuntu.com/*) echo 0.50 ;;\n'
                '  https://ca.archive.ubuntu.com/*) echo 0.60 ;;\n'
                '  https://uk.archive.ubuntu.com/*) echo 0.70 ;;\n'
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
            mirror_list = (sources / "ubuntu-mirrors.txt").read_text(encoding="utf-8")
            self.assertIn(f"mirror+file:{sources}/ubuntu-mirrors.txt", updated)
            self.assertNotIn("http://security.ubuntu.com/ubuntu", updated)
            self.assertIn(
                "https://fast.example.org/ubuntu\tpriority:1", mirror_list
            )
            self.assertIn(
                "https://mirror.example.net/ubuntu\tpriority:2", mirror_list
            )
            self.assertIn(
                "https://archive.ubuntu.com/ubuntu\tpriority:2", mirror_list
            )
            self.assertIn("0.03s", result.stdout)
            self.assertIn("failover", result.stdout)

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
            self.assertFalse((sources / "ubuntu-mirrors.txt").exists())


if __name__ == "__main__":
    unittest.main()
