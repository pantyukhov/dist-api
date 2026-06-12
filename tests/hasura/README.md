# Hasura v2 conformance suite

`tests-py/` is a full copy of `server/tests-py` from
[hasura/graphql-engine](https://github.com/hasura/graphql-engine)
(commit `371d744e8a063fe348e291cc306f37973b11d1b8`, 2026-06-11),
licensed under Apache 2.0 (`tests-py/LICENSE.hasura`).

**`tests-py/` is not committed to this repository** (see `.gitignore`).
To restore it:

```sh
git clone --filter=blob:none https://github.com/hasura/graphql-engine /tmp/graphql-engine
git -C /tmp/graphql-engine checkout 371d744e8a063fe348e291cc306f37973b11d1b8
cp -R /tmp/graphql-engine/server/tests-py tests/hasura/tests-py
cp /tmp/graphql-engine/LICENSE tests/hasura/tests-py/LICENSE.hasura
```

What *is* committed here: `run_suite.sh` (suite runner), `triage.py`
(per-fixture diff tool), and `COVERAGE.md` (live conformance status).

These are black-box tests: pytest talks HTTP to a running server, so they
apply to our implementation without modifying the test code.

## Anatomy of a test

A typical test is a YAML file under `queries/`:

```yaml
# queries/graphql_query/basic/...yaml
description: ...
url: /v1/graphql
status: 200
response: { ... expected JSON ... }
query:
  query: |
    query { author { id name } }
```

Per-class setup/teardown is YAML too (`setup.yaml` / `teardown.yaml`): a list
of metadata API calls (`run_sql`, `pg_track_table`,
`pg_create_select_permission`, ...).

## Running against our server

```sh
cd tests/hasura/tests-py
python3 -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt

# our server must listen on :8080, postgres on 5432
pytest --hge-urls http://127.0.0.1:8080 \
       --pg-urls postgresql://postgres:postgres@127.0.0.1:5432/hge_tests \
       -v test_graphql_queries.py::TestGraphQLQueryBasicCommon
```

## Adoption order (core conformance subset)

| Milestone | Tests | What the engine needs |
|---|---|---|
| M2 | `queries/graphql_introspection` | schema-gen, introspection |
| M3–M4 | `queries/graphql_query/basic` | select, columns |
| M4 | `queries/graphql_query/{boolexp,order_by,limits,offset}` | where/order/limit |
| M5 | `queries/graphql_query/permissions`, `agg_perm` | roles, filters, session vars |
| M5 | `queries/graphql_query/aggregations` | `_aggregate` |
| M6 | `queries/graphql_mutation/{insert,update,delete}` | mutations |

Everything else (event triggers, remote schemas, actions, scheduled
triggers, subscriptions) comes after the core.

## Dependency on the metadata API

Test setup steps use the v2 **runtime** metadata API. Our server builds
everything from YAML, so for the tests we implement a minimal compatible
surface of `/v1/metadata` and `/v2/query` (`run_sql`), where track/permission
commands translate into internal metadata state (equivalent to a hot YAML
reload). This is part of M7 (test harness); `run_sql` is needed as early as
M3 — tests create their tables through it.
