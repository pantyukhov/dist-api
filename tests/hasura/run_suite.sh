#!/usr/bin/env bash
# Reset DB + metadata, run a tests-py selection against dist-api, print a
# terse summary. Usage: ./run_suite.sh <pytest selector> [extra pytest args]
set -u
cd "$(dirname "$0")/tests-py"

HGE_URL="${HGE_URL:-http://127.0.0.1:18080}"
PG_URL="${PG_URL:-postgresql://postgres:postgres@127.0.0.1:15432/postgres}"

reset_out=$(curl -s -X POST "$HGE_URL/v1/query" -H 'content-type: application/json' \
  -d '{"type":"run_sql","args":{"sql":"drop schema public cascade; create schema public; grant all on schema public to public; create extension if not exists postgis;"}}')
case "$reset_out" in
  *CommandOk*) ;;
  *) echo "DB RESET FAILED: $reset_out" >&2; exit 1 ;;
esac
curl -sf -X POST "$HGE_URL/v1/query" -H 'content-type: application/json' \
  -d '{"type":"clear_metadata","args":{}}' > /dev/null || { echo "METADATA RESET FAILED" >&2; exit 1; }

VERSION="${VERSION:-2.40.0}" .venv/bin/pytest \
  --hge-urls "$HGE_URL" --pg-urls "$PG_URL" \
  -q --no-header --tb=line -p no:randomly \
  "$@" > /tmp/suite-run.log 2>&1

grep -E '^(FAILED|ERROR|PASSED)' /tmp/suite-run.log | head -40
tail -1 /tmp/suite-run.log
