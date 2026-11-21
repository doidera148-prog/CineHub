#!/usr/bin/env python3
"""Ajusta o AndroidManifest gerado por `cargo tauri android init` para também funcionar no Google TV.

- Declara recursos de TV (leanback) e torna o toque opcional.
- Adiciona o LEANBACK_LAUNCHER, que faz o app aparecer na tela inicial da TV.
- Define o banner exibido no launcher da TV.

Uso: python3 patch_manifest.py caminho/para/AndroidManifest.xml
"""
import pathlib
import re
import sys

FEATURES = (
    "android.software.leanback",
    "android.hardware.touchscreen",
)
LAUNCHER = '<category android:name="android.intent.category.LAUNCHER" />'
LEANBACK = '<category android:name="android.intent.category.LEANBACK_LAUNCHER" />'


def fail(message: str) -> None:
    # Falha explícita: um manifesto sem as alterações de TV não pode passar como "ajustado".
    sys.exit(f"erro ao ajustar o manifesto para TV: {message}")


def ensure_feature(xml: str, feature: str) -> str:
    marker = f'android:name="{feature}"'
    if marker in xml:
        return xml

    element = (
        f'    <uses-feature android:name="{feature}" '
        'android:required="false" />'
    )
    xml, count = re.subn(
        r"(<manifest\b[^>]*>)",
        lambda match: match.group(1) + "\n" + element,
        xml,
        count=1,
    )
    if count == 0:
        fail("tag <manifest> não encontrada")
    return xml


def main(path: str) -> None:
    manifest = pathlib.Path(path)
    xml = manifest.read_text(encoding="utf-8")

    for feature in FEATURES:
        xml = ensure_feature(xml, feature)

    if LEANBACK not in xml:
        if LAUNCHER not in xml:
            fail("categoria LAUNCHER não encontrada")
        xml = xml.replace(LAUNCHER, LAUNCHER + "\n                " + LEANBACK, 1)

    application_has_banner = re.search(
        r"<application\b[^>]*\bandroid:banner=",
        xml,
    )
    if application_has_banner is None:
        xml, count = re.subn(
            r"<application\b",
            '<application android:banner="@drawable/banner"',
            xml,
            count=1,
        )
        if count == 0:
            fail("tag <application> não encontrada")

    # Confere o resultado antes de gravar.
    for needle in (
        'android:name="android.software.leanback"',
        'android:name="android.hardware.touchscreen"',
        LEANBACK,
        'android:banner="@drawable/banner"',
    ):
        if needle not in xml:
            fail(f"não foi possível inserir: {needle}")

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
