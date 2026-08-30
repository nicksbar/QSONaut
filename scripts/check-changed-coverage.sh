#!/usr/bin/env bash
set -euo pipefail

summary_file="${1:-coverage-summary.txt}"
base_ref="${2:-${GITHUB_BASE_SHA:-}}"
baseline_file="${QSONAUT_COVERAGE_BASELINE_FILE:-.coverage-baseline}"

if [[ ! -f "$summary_file" ]]; then
  echo "coverage summary not found: $summary_file" >&2
  exit 2
fi
if [[ -z "$base_ref" ]]; then
  echo "base commit is required; pass it as the second argument or set GITHUB_BASE_SHA" >&2
  exit 2
fi
if ! git rev-parse --verify "$base_ref^{commit}" >/dev/null 2>&1; then
  echo "base commit is not available: $base_ref" >&2
  exit 2
fi
if [[ ! -f "$baseline_file" ]]; then
  echo "coverage baseline file not found: $baseline_file" >&2
  exit 2
fi

is_baselined() {
  grep -Fqx "$1" "$baseline_file"
}

mapfile -t changed_files < <(git diff --name-only "$base_ref" -- '*.rs')
changed_files=("${changed_files[@]}")

if (( ${#changed_files[@]} == 0 )); then
  echo "No changed production Rust files; per-file coverage check skipped."
  exit 0
fi

failed=0
for file in "${changed_files[@]}"; do
  [[ -n "$file" && -f "$file" ]] || continue
  read -r total_lines missed_lines < <(
    awk -v path="$file" '$1 == path && $8 ~ /^[0-9]+$/ && $9 ~ /^[0-9]+$/ {
      print $8, $9
      found = 1
      exit
    }
    END { if (!found) print 0, 0 }' "$summary_file"
  )

  if (( total_lines == 0 )); then
    echo "$file: no executable coverage row found" >&2
    failed=1
    continue
  fi

  covered_lines=$((total_lines - missed_lines))
  printf '%s line coverage: %d/%d\n' "$file" "$covered_lines" "$total_lines"
  if (( covered_lines == 0 )); then
    if is_baselined "$file"; then
      echo "$file: known baseline debt; add focused tests before removing it"
    else
      echo "$file: changed production file has zero covered lines; add or update a test" >&2
      failed=1
    fi
  fi
done

exit "$failed"
