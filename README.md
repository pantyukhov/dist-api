# dist-api

A GraphQL engine over Postgres, compatible with the Hasura v2 surface
(metadata format, API shape) and developed against Hasura's own black-box
test suite (`tests/hasura/tests-py`). Rust workspace; see [PLAN.md](PLAN.md)
for the architecture and milestones.

## Layout

| Path | Purpose |
|---|---|
| `crates/metadata` | Hasura v2 metadata types + YAML directory loader (`!include`) |
| `crates/catalog` | Postgres introspection (pg_catalog) |
| `crates/schema` | Per-role GraphQL schema generation |
| `crates/ir` | Intermediate representation — the SQL-free boundary |
| `crates/sqlgen` | IR → one Postgres SQL statement |
| `crates/server` | axum HTTP server: `/v1/graphql`, `/healthz`, `/v1/version` |
| `tests/hasura` | Conformance harness (`run_suite.sh`, `triage.py`, `COVERAGE.md`); the vendored Hasura `tests-py` suite is not committed — see `tests/hasura/README.md` |

## Quick start

```sh
make build
make test
make run   # serves :8080 with the fixture metadata
```

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
