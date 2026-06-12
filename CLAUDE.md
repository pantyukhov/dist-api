# dist-api

A GraphQL engine over Postgres, compatible with the Hasura v2 surface
(metadata format, API shape), developed TDD-style against Hasura's own
black-box test suite (`tests/hasura/tests-py`).

## Tech Stack

Rust workspace (axum, tokio, serde, insta), Postgres 16 (postgis), vendored
pytest conformance suite.

## Layout

| Path | Purpose |
|---|---|
| `crates/metadata` | Hasura v2 metadata types + YAML directory loader (`!include`) |
| `crates/catalog` | Postgres introspection (pg_catalog) |
| `crates/schema` | Per-role GraphQL schema generation, introspection |
| `crates/ir` | Intermediate representation — the SQL-free boundary |
| `crates/sqlgen` | IR → one Postgres SQL statement (insta snapshot tests) |
| `crates/server` | axum server: `/v1/graphql`, `/v1/query`, ws, auth |
| `tests/hasura` | Vendored conformance suite + `run_suite.sh` + `triage.py` |
| `knowledgebase/` | Design notes and ADRs (Obsidian-style, see `_index.md`) |
| `PLAN.md` | Architecture, milestones, decision log |

## Commands

| Task | Command |
|---|---|
| Build | `make build` |
| Unit/snapshot tests | `make test` (or `cargo test -p <crate>`) |
| Run with fixture metadata | `make run` (serves :8080) |
| Conformance suite | `tests/hasura/run_suite.sh <pytest selector>` |
| Triage one fixture dir | `tests/hasura/triage.py queries/<dir> [test ...]` |
| Review snapshot changes | `cargo insta review` |

Conformance harness expects the server on `HGE_URL` (default
`http://127.0.0.1:18080`) and Postgres on `PG_URL` (default `:15432`).
Some suites need hge-bin mode (env-marked classes) — see COVERAGE.md notes.

## The TDD Loop (how all engine work is done)

1. Pick a failing tests-py suite/fixture (COVERAGE.md is the live status —
   PLAN.md milestones can lag behind it).
2. `triage.py` the fixture dir → read expected/actual diffs.
3. Implement; add/adjust unit + insta tests in the touched crate.
4. `run_suite.sh` the suite until green; then re-run neighbor suites you
   might have broken.
5. Update `tests/hasura/COVERAGE.md` with the new counts and notes.

Harness quirks to remember: some fixtures `!include` files as strings;
legacy `$op` permission spellings are valid input; a few fixtures expect
HTTP status codes inconsistent with their bodies (status diff with exactly
matching body = known-diff, record it in COVERAGE.md, don't chase it).

## BLOCKING RULE: No Admin Role

**This engine has no admin role — by design, forever.** Never implement
Hasura's implicit admin permission bypass. All data access goes through
explicit role permissions; admin-secret is API-level auth only. Conformance
fixtures that rely on implicit admin are out of scope (marked in
COVERAGE.md). Any diff that adds an admin bypass must be rejected.

## BLOCKING RULE: Knowledgebase First

Read relevant `knowledgebase/` files BEFORE analyzing, planning, or
implementing. `ls knowledgebase/` + `grep -ri "topic" knowledgebase/`;
check `knowledgebase/<domain>/decisions/` — ADRs explain *why*. Plans or
code written without this are invalid and must be redone. After work with
meaningful trade-offs, capture an ADR (template:
`knowledgebase/_templates/decision.md`).

## BLOCKING RULE: Judge Review After Every Commit

After EVERY `git commit`, dispatch the judge agent before starting the next
task. Not optional; "simple change" is not an excuse.

```
Agent(subagent_type="judge", run_in_background=true, prompt="REVIEW TASK: ...")
```

Input format in `.claude/agents/judge.md`. Continue only after ACCEPT; on
REJECT fix the issues first.

## Essential Rules

- **Repo content in English** (docs, comments, specs, ADRs). Chat language
  may differ; the repo never does.
- **Exact Hasura error shapes.** Error `code`/`path`/message text and HTTP
  status are part of the conformance contract — never invent error formats.
- **One SQL statement per operation** (M4 invariant): response JSON is
  assembled in Postgres (`json_build_object`/`json_agg`, correlated
  subqueries). Don't add row-by-row post-processing in Rust.
- **Always parameterized SQL.** Variables and session vars bind as
  parameters, never interpolate into SQL text.
- **insta snapshots are reviewed, never blind-accepted.** `cargo insta
  review` and read every diff; an unexplained snapshot change is a bug.
- **Full v2 metadata format** — metadata exported from existing Hasura
  projects must load without conversion.
- **Every change needs tests**: unit/insta in the crate AND the relevant
  conformance suite green via `run_suite.sh`.

## Agents

- `.claude/agents/judge.md` — two-stage quality gate (spec compliance →
  code quality → fresh verification). Mandatory after every commit.
- `.claude/agents/spec-writer.md` — researches the codebase + tests-py
  fixtures and writes specs to `specs/NNN-<slug>.md`.
