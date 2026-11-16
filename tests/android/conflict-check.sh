#!/usr/bin/env bash
# Reproduz o caso do celular: instala a build-7 (funcionou) e depois a build-18 por cima.
set -u
adb wait-for-device
adb uninstall com.cinehub.app >/dev/null 2>&1 || true
echo "== 1) instalando build-7 (universal, funcionou no celular)"
adb install -r variants/build7.apk; echo "exit=$?"
adb shell pm list packages | grep -i cinehub || echo "pacote não instalado"
echo "== 2) instalando build-18 (x86_64) por cima da build-7"
adb install -r variants/build18-x86_64.apk; echo "exit=$?"
echo "== 3) mesma build-18, instalada do zero (sem nada antes)"
adb uninstall com.cinehub.app >/dev/null 2>&1 || true
adb install -r variants/build18-x86_64.apk; echo "exit=$?"
