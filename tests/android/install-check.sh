#!/usr/bin/env bash
# Instala as variantes do APK no emulador e mostra o resultado de cada uma.
set -u
adb wait-for-device
echo "== Android $(adb shell getprop ro.build.version.release) (API $(adb shell getprop ro.build.version.sdk)) | ABIs: $(adb shell getprop ro.product.cpu.abilist | tr -d '\r')"
for v in original v1v2v3; do
  adb uninstall com.cinehub.app >/dev/null 2>&1 || true
  echo "== instalando variante $v"
  adb install -r "variants/$v.apk"
  echo "exit=$?"
  adb shell pm list packages | grep -i cinehub || echo "pacote não instalado"
done
