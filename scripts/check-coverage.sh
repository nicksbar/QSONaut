#!/usr/bin/env bash
set -euo pipefail

summary_file="${1:-coverage-summary.txt}"
minimum_percent="${2:-${QSONAUT_COVERAGE_MIN:-26.6}}"
integration_minimum_percent="${3:-${QSONAUT_RIGWRIGHT_INTEGRATION_MIN:-}}"

if [[ ! -f "$summary_file" ]]; then
  echo "coverage summary not found: $summary_file" >&2
  exit 2
fi

if [[ ! "$minimum_percent" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  echo "coverage threshold must be numeric: $minimum_percent" >&2
  exit 2
fi
if [[ -n "$integration_minimum_percent" && ! "$integration_minimum_percent" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  echo "Rigwright integration threshold must be numeric: $integration_minimum_percent" >&2
  exit 2
fi

read -r total_lines missed_lines reported_percent < <(
  awk '$1 == "TOTAL" && $8 ~ /^[0-9]+$/ && $9 ~ /^[0-9]+$/ {
    print $8, $9, $10
    found = 1
    exit
  }
  END {
    if (!found) print 0, 0, "n/a"
  }' "$summary_file"
)

if (( total_lines == 0 )); then
  echo "TOTAL line coverage row not found in $summary_file" >&2
  exit 2
fi

covered_lines=$((total_lines - missed_lines))
printf 'Workspace line coverage: %d/%d (%s), required: %s%%\n' \
  "$covered_lines" "$total_lines" "$reported_percent" "$minimum_percent"

echo "Coverage by area:"
awk '
$1 ~ /^(apps|crates)\// && $8 ~ /^[0-9]+$/ && $9 ~ /^[0-9]+$/ {
  path = $1
  if (path ~ /^apps\/qsonaut\//) area = "Application entry point"
  else if (path == "crates/qsonaut-gui/src/panels/radio_contract.rs" || path == "crates/qsonaut-gui/src/workers/radio.rs") area = "Rigwright integration"
  else if (path ~ /^crates\/qsonaut-gui\/src\/modes\//) area = "GUI modes"
  else if (path ~ /^crates\/qsonaut-gui\/src\/panels\//) area = "GUI panels"
  else if (path ~ /^crates\/qsonaut-gui\/src\/workers\//) area = "GUI workers"
  else if (path ~ /^crates\/qsonaut-gui\//) area = "GUI core"
  else {
    area = path
    sub(/^crates\//, "", area)
    sub(/\/.*$/, "", area)
  }
  total[area] += $8
  missed[area] += $9
}
END {
  for (area in total) {
    covered = total[area] - missed[area]
    printf "%s|%d|%d|%.2f\n", area, covered, total[area], covered * 100 / total[area]
  }
}' "$summary_file" | sort | awk -F'|' '{ printf "  %-24s %6d/%-6d %6.2f%%\n", $1, $2, $3, $4 }'

echo "Rigwright driver interactivity: covered by Rigwright's own driver coverage;"
echo "QSONaut coverage covers only its HAL/worker/UI integration with that dependency."

if [[ -n "$integration_minimum_percent" ]]; then
  read -r integration_total integration_missed < <(
    awk '$1 == "crates/qsonaut-gui/src/panels/radio_contract.rs" || $1 == "crates/qsonaut-gui/src/workers/radio.rs" {
      total += $8
      missed += $9
      found = 1
    }
    END { if (found) print total, missed; else print 0, 0 }' "$summary_file"
  )
  if (( integration_total == 0 )); then
    echo "Rigwright integration coverage rows not found in $summary_file" >&2
    exit 2
  fi
  integration_covered=$((integration_total - integration_missed))
  printf 'Rigwright integration line coverage: %d/%d (%s), required: %s%%\n' \
    "$integration_covered" "$integration_total" \
    "$(awk -v covered="$integration_covered" -v total="$integration_total" 'BEGIN { printf "%.2f%%", covered * 100 / total }')" \
    "$integration_minimum_percent"
fi

awk -v covered="$covered_lines" -v total="$total_lines" -v minimum="$minimum_percent" \
  'BEGIN { exit !((covered * 100) >= (total * minimum)) }'

if [[ -n "$integration_minimum_percent" ]]; then
  awk -v covered="$integration_covered" -v total="$integration_total" -v minimum="$integration_minimum_percent" \
    'BEGIN { exit !((covered * 100) >= (total * minimum)) }'
fi
