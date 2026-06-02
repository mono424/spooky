#!/usr/bin/env bash
# Run each test file in its OWN `dart test` process.
#
# Why: the ssp-ffi Rust cdylib installs std's signal/stack-overflow handlers on
# load, which segfaults the Dart VM when the library is first loaded in a
# *secondary* test-suite isolate (the default `dart test` packs many suites into
# one process). Each file passes cleanly on its own, so we isolate them.
# Real single-process app usage is unaffected.
#
# Usage: tool/run_tests.sh [--integration]
set -uo pipefail
cd "$(dirname "$0")/.."

extra_args=("--exclude-tags" "integration")
if [[ "${1:-}" == "--integration" ]]; then
  extra_args=("--tags" "integration")
fi

fail=0
total_pass=0
while IFS= read -r file; do
  out=$(dart test "$file" "${extra_args[@]}" 2>&1)
  status=$?
  last=$(printf '%s\n' "$out" | tail -1)
  if [[ $status -eq 0 ]]; then
    n=$(printf '%s' "$last" | grep -oE '\+[0-9]+' | head -1 | tr -d '+')
    total_pass=$((total_pass + ${n:-0}))
    echo "PASS  $file  ($last)"
  else
    fail=1
    echo "FAIL  $file"
    printf '%s\n' "$out" | tail -15
  fi
done < <(find test -name '*_test.dart' -not -path 'test/integration/*' | sort)

echo "-----"
if [[ $fail -eq 0 ]]; then
  echo "All test files passed ($total_pass tests)."
else
  echo "Some test files failed."
fi
exit $fail
