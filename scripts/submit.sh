#!/usr/bin/env bash
# Put one Submission on the queue and wait for its GradeResult — an end-to-end check of the
# deployed worker, Kafka included.
#
#   ./scripts/submit.sh <assignment_id> <stack> <github_repo_url> [git_ref]
#
# e.g. ./scripts/submit.sh py-fibonacci python https://github.com/student/hw1
#
# Needs the compose stack up (`docker compose up -d grader`) and uses kcat from a throwaway
# container on the same network, so nothing has to be installed on the host.
set -euo pipefail

if [[ $# -lt 3 ]]; then
  sed -n '2,9p' "$0" >&2
  exit 64
fi

assignment_id=$1
stack=$2
repo_url=$3
git_ref=${4:-}
submission_id="cli-$(date +%s)-$RANDOM"

brokers=${KAFKA_BROKERS:-kafka:9092}
submission_topic=${SUBMISSION_TOPIC:-assignment-submission}
result_topic=${RESULT_TOPIC:-assignment-result}
network=$(docker compose ps --format '{{.Name}}' grader | head -1 | xargs -r docker inspect -f '{{range $n,$_ := .NetworkSettings.Networks}}{{$n}}{{end}}')

payload=$(printf '{"submission_id":"%s","assignment_id":"%s","stack":"%s","repo_url":"%s"' \
  "$submission_id" "$assignment_id" "$stack" "$repo_url")
[[ -n $git_ref ]] && payload+=$(printf ',"git_ref":"%s"' "$git_ref")
payload+='}'

kcat() { docker run --rm -i --network "$network" edenhill/kcat:1.7.1 -b "$brokers" "$@"; }

echo "→ $submission_topic: $payload"
kcat -P -t "$submission_topic" -k "$submission_id" <<<"$payload"

echo "← waiting for $submission_id on $result_topic (Ctrl-C to stop)…"
kcat -C -t "$result_topic" -o end -u | while read -r line; do
  if [[ $line == *"\"$submission_id\""* ]]; then
    echo "$line"
    exit 0
  fi
done
