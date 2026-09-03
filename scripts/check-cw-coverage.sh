#!/usr/bin/env bash
set -euo pipefail

summary_file="${1:-coverage-summary.txt}"
minimum="${2:-80}"

if [[ ! -f "$summary_file" ]]; then
  echo "coverage summary not found: $summary_file" >&2
  exit 2
fi

read -r total_lines missed_lines < <(
  awk '$1 == "crates/qsonaut-gui/src/modes/cw.rs" && $8 ~ /^[0-9]+$/ && $9 ~ /^[0-9]+$/ {
    print $8, $9
    found = 1
    exit
  }
  END { if (!found) print 0, 0 }' "$summary_file"
)

if (( total_lines == 0 )); then
  echo "CW coverage row not found" >&2
  exit 2
fi

covered_lines=$((total_lines - missed_lines))
coverage=$((covered_lines * 100 / total_lines))
printf 'CW panel line coverage: %d/%d (%d%%); minimum %d%%\n' \
  "$covered_lines" "$total_lines" "$coverage" "$minimum"

if (( coverage < minimum )); then
  echo "CW panel coverage is below the required minimum" >&2
  exit 1
fi
