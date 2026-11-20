#!/usr/bin/env bash
# Build an APT mirror list ordered by measured latency, with automatic fallback.
set -euo pipefail

sources_dir="${APT_SOURCES_DIR:-/etc/apt}"
codename="${UBUNTU_CODENAME:-jammy}"
mirror_list="${sources_dir}/ubuntu-mirrors.txt"
candidates=(
  "https://mirror.ufscar.br/ubuntu"
  "https://ubuntu.c3sl.ufpr.br/ubuntu"
  "https://br.archive.ubuntu.com/ubuntu"
  "https://archive.ubuntu.com/ubuntu"
)

declare -a reachable=()
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

  reachable+=("$elapsed"$'\t'"$mirror")
  if [[ -z "$best_time" ]] || awk -v candidate="$elapsed" -v best="$best_time" 'BEGIN { exit !(candidate < best) }'; then
    best_mirror="$mirror"
    best_time="$elapsed"
  fi
done

if [[ -z "$best_mirror" ]]; then
  echo "::warning::Nenhum espelho Ubuntu alternativo respondeu; mantendo os endereços configurados."
  exit 0
fi

# APT's mirror transport tries another mirror automatically when a requested
# file is missing or a server fails. Priority 1 is the fastest measured mirror;
# priority 2 contains the remaining reachable mirrors as fallback.
sorted=$(printf '%s\n' "${reachable[@]}" | sort -n -k1,1)
: > "$mirror_list"
while IFS=$'\t' read -r elapsed mirror; do
  [[ -n "$mirror" ]] || continue
  if [[ "$mirror" == "$best_mirror" ]]; then
    printf '%s\tpriority:1\n' "$mirror" >> "$mirror_list"
  else
    printf '%s\tpriority:2\n' "$mirror" >> "$mirror_list"
  fi
done <<< "$sorted"

absolute_mirror_list=$(cd "$(dirname "$mirror_list")" && pwd)/$(basename "$mirror_list")
mirror_uri="mirror+file:$absolute_mirror_list"
echo "Espelho Ubuntu preferido: $best_mirror (${best_time}s); ${#reachable[@]} espelho(s) disponível(is) para failover."

shopt -s nullglob
files=("$sources_dir"/*.list "$sources_dir"/*.sources)
for file in "${files[@]}"; do
  [[ -f "$file" ]] || continue
  sed -i -E \
    -e "s#https?://(azure\\.)?archive\\.ubuntu\\.com/ubuntu#${mirror_uri}#g" \
    -e "s#https?://br\\.archive\\.ubuntu\\.com/ubuntu#${mirror_uri}#g" \
    "$file"
done
