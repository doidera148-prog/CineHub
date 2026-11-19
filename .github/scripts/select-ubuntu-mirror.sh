#!/usr/bin/env bash
# Select the fastest reachable Ubuntu mirror for the runner, preferring Brazilian mirrors.
set -euo pipefail

sources_dir="${APT_SOURCES_DIR:-/etc/apt}"
codename="${UBUNTU_CODENAME:-jammy}"
candidates=(
  "https://ubuntu.c3sl.ufpr.br/ubuntu"
  "https://mirror.ufscar.br/ubuntu"
  "https://br.archive.ubuntu.com/ubuntu"
  "https://archive.ubuntu.com/ubuntu"
)

best_mirror=""
best_time=""

for mirror in "${candidates[@]}"; do
  elapsed=$(curl --silent --show-error --fail --location \
    --connect-timeout 5 --max-time 12 \
    --output /dev/null --write-out '%{time_total}' \
    "${mirror}/dists/${codename}/InRelease" 2>/dev/null) || continue

  if [[ ! "$elapsed" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
    continue
  fi

  if [[ -z "$best_time" ]] || awk -v candidate="$elapsed" -v best="$best_time" 'BEGIN { exit !(candidate < best) }'; then
    best_mirror="$mirror"
    best_time="$elapsed"
  fi
done

if [[ -z "$best_mirror" ]]; then
  echo "::warning::Nenhum espelho Ubuntu alternativo respondeu; mantendo os endereços configurados."
  exit 0
fi

echo "Espelho Ubuntu selecionado: $best_mirror (${best_time}s)"
shopt -s nullglob
files=("$sources_dir"/*.list "$sources_dir"/*.sources)
for file in "${files[@]}"; do
  [[ -f "$file" ]] || continue
  sed -i -E \
    -e "s#https?://(azure\\.)?archive\\.ubuntu\\.com/ubuntu#${best_mirror}#g" \
    -e "s#https?://br\\.archive\\.ubuntu\\.com/ubuntu#${best_mirror}#g" \
    "$file"
done
