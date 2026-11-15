#!/usr/bin/env python3
"""Ajusta o AndroidManifest gerado por `cargo tauri android init` para também funcionar no Android TV.

- Declara recursos de TV (leanback) e torna o toque opcional.
- Adiciona o LEANBACK_LAUNCHER, que faz o app aparecer na tela inicial da TV.
- Define o banner exibido no launcher da TV.

Uso: python3 patch_manifest.py caminho/para/AndroidManifest.xml
"""
import pathlib
import re
import sys

FEATURES = (
    '    <uses-feature android:name="android.software.leanback" android:required="false" />\n'
    '    <uses-feature android:name="android.hardware.touchscreen" android:required="false" />\n'
)
LAUNCHER = '<category android:name="android.intent.category.LAUNCHER" />'
LEANBACK = '<category android:name="android.intent.category.LEANBACK_LAUNCHER" />'


def main(path: str) -> None:
    manifest = pathlib.Path(path)
    xml = manifest.read_text(encoding="utf-8")

    if "android.software.leanback" not in xml:
        xml = re.sub(r"(<manifest[^>]*>\n)", lambda m: m.group(1) + FEATURES, xml, count=1)

    if LEANBACK not in xml and LAUNCHER in xml:
        xml = xml.replace(LAUNCHER, LAUNCHER + "\n                " + LEANBACK, 1)

    if "android:banner" not in xml:
        xml = re.sub(r"<application\b", '<application android:banner="@drawable/banner"', xml, count=1)

    manifest.write_text(xml, encoding="utf-8")

    drawable = manifest.parent / "res" / "drawable"
    drawable.mkdir(parents=True, exist_ok=True)
    banner_src = pathlib.Path(__file__).parent / "banner.png"
    (drawable / "banner.png").write_bytes(banner_src.read_bytes())
    print(f"AndroidManifest ajustado para TV: {manifest}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
