#!/usr/bin/env bash
# Grade every checked-in example against its assignment, in the real sandbox, without Kafka or
# GitHub — the fastest way to prove an image (and an assignment) actually works.
#
#   ./scripts/smoke.sh
#
# Each `-good` example must come back `graded` with zero failures; each `-bad` example must come
# back `graded` with at least one failure. A `-bad` example that reports build_error means the
# assignment is broken, not the student.
set -uo pipefail

COMPOSE=(docker compose run --rm --no-deps grader)

run() { "${COMPOSE[@]}" grade-local "$1" "/opt/examples/students/$2" 2>/dev/null; }

# Pull one scalar out of grade-local's pretty-printed JSON: `  "failed": 5,` -> `5`.
field() { grep -m1 "\"$2\":" <<<"$1" | sed 's/.*: *//; s/[",]//g'; }

failures=0

check() {
  local assignment=$1 example=$2 expectation=$3
  local output status failed passed
  output=$(run "$assignment" "$example")
  status=$(field "$output" status)
  passed=$(field "$output" passed)
  failed=$(field "$output" failed)

  local ok=false
  case $expectation in
    all-pass) [[ $status == graded && $failed == 0 && $passed -gt 0 ]] && ok=true ;;
    some-fail) [[ $status == graded && $failed -gt 0 ]] && ok=true ;;
  esac

  if $ok; then
    printf '  ok   %-24s %-8s passed=%s failed=%s\n' "$example" "$status" "$passed" "$failed"
  else
    printf '  FAIL %-24s %-8s passed=%s failed=%s (expected %s)\n' \
      "$example" "${status:-?}" "${passed:-?}" "${failed:-?}" "$expectation"
    sed 's/^/       | /' <<<"$output"
    failures=$((failures + 1))
  fi
}

echo "grading the example submissions:"
check py-fibonacci  py-fibonacci-good   all-pass
check py-fibonacci  py-fibonacci-bad    some-fail
check mern-todo-api mern-todo-api-good  all-pass
check mern-todo-api mern-todo-api-bad   some-fail
check java-bank     java-bank-good      all-pass
check java-bank     java-bank-bad       some-fail

if (( failures )); then
  echo "$failures example(s) did not grade as expected"
  exit 1
fi
echo "all examples graded as expected"
