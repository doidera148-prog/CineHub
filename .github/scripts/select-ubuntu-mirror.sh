#!/usr/bin/env bash
# Discover official Ubuntu mirrors, rank them by latency from this runner,
# and let APT automatically fail over when a mirror cannot serve a file.
set -euo pipefail

sources_dir="${APT_SOURCES_DIR:-/etc/apt}"
codename="${UBUNTU_CODENAME:-jammy}"
mirror_list="${sources_dir}/ubuntu-mirrors.txt"

# Launchpad's published index is selected for the runner's public IP. Probe
# every returned endpoint rather than trusting geographic proximity alone.
declare -a candidates=()
mirror_index=$(curl --silent --show-error --fail --location \
  --connect-timeout 5 --max-time 12 \
  https://mirrors.ubuntu.com/mirrors.txt 2>/dev/null || true)
while IFS= read -r mirror; do
  mirror="${mirror%/}"
  [[ "$mirror" =~ ^https?:// ]] || continue
  candidates+=("$mirror")
done <<< "$mirror_index"

# Global official country endpoints and known community mirrors provide
# resilience if the dynamic index is unavailable or temporarily incomplete.
fallback_candidates=(
  "https://archive.ubuntu.com/ubuntu"
  "https://us.archive.ubuntu.com/ubuntu"
  "https://ca.archive.ubuntu.com/ubuntu"
  "https://uk.archive.ubuntu.com/ubuntu"
  "https://de.archive.ubuntu.com/ubuntu"
  "https://fr.archive.ubuntu.com/ubuntu"
  "https://nl.archive.ubuntu.com/ubuntu"
  "https://jp.archive.ubuntu.com/ubuntu"
  "https://kr.archive.ubuntu.com/ubuntu"
  "https://au.archive.ubuntu.com/ubuntu"
  "https://mirror.umd.edu/ubuntu"
  "https://mirrors.ocf.berkeley.edu/ubuntu"
  "https://mirror.math.princeton.edu/pub/ubuntu"
  "https://ftp.osuosl.org/pub/ubuntu"
  "https://mirror.ufscar.br/ubuntu"
  "https://ubuntu.c3sl.ufpr.br/ubuntu"
  "https://br.archive.ubuntu.com/ubuntu"
)
candidates+=("${fallback_candidates[@]}")

declare -A seen=()
declare -a reachable=()
best_mirror=""
best_time=""

for mirror in "${candidates[@]}"; do
  [[ -z "${seen[$mirror]+x}" ]] || continue
  seen["$mirror"]=1

  # A mirror só é elegível se servir tanto o arquivo principal quanto
  # o pocket de segurança; assim não selecionamos um endpoint incompleto.
  elapsed=$(curl --silent --show-error --fail --location \
    --connect-timeout 5 --max-time 12 \
    --output /dev/null --write-out '%{time_total}' \
    "${mirror}/dists/${codename}/InRelease" 2>/dev/null) || continue
  security_elapsed=$(curl --silent --show-error --fail --location \
    --connect-timeout 5 --max-time 12 \
    --output /dev/null --write-out '%{time_total}' \
    "${mirror}/dists/${codename}-security/InRelease" 2>/dev/null) || continue

  [[ "$elapsed" =~ ^[0-9]+([.][0-9]+)?$ ]] || continue
  [[ "$security_elapsed" =~ ^[0-9]+([.][0-9]+)?$ ]] || continue
  reachable+=("$elapsed"$'\t'"$mirror")
  if [[ -z "$best_time" ]] || awk -v candidate="$elapsed" -v best="$best_time" 'BEGIN { exit !(candidate < best) }'; then
    best_mirror="$mirror"
    best_time="$elapsed"
  fi
done

if [[ -z "$best_mirror" ]]; then
  echo "::warning::Nenhum espelho Ubuntu respondeu; mantendo os endereços configurados."
  exit 0
fi

# Priority 1 is the fastest measured endpoint; priority 2 is automatic
# failover. APT mirror transport retries another mirror if acquisition fails.
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
echo "Espelho Ubuntu mais rápido neste runner: $best_mirror (${best_time}s); ${#reachable[@]} endpoint(s) disponível(is) para failover."

shopt -s nullglob
files=("$sources_dir"/*.list "$sources_dir"/*.sources)
for file in "${files[@]}"; do
  [[ -f "$file" ]] || continue
  sed -i -E \
    -e "s#https?://([a-z]{2,3}\\.)?archive\\.ubuntu\\.com/ubuntu#${mirror_uri}#g" \
    -e "s#https?://azure\\.archive\\.ubuntu\\.com/ubuntu#${mirror_uri}#g" \
    -e "s#https?://security\\.ubuntu\\.com/ubuntu#${mirror_uri}#g" \
    "$file"
done
