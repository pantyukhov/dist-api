#!/usr/bin/env python3
"""Compact triage runner: replays a tests-py fixture directory against
dist-api and prints expected/actual diffs for each test yaml.

Usage: triage.py <queries-dir> [test-name ...]
Example: triage.py queries/graphql_mutation/insert/permissions
"""
import glob
import json
import os
import sys

import requests
from ruamel.yaml import YAML

HGE = os.environ.get("HGE_URL", "http://127.0.0.1:18080")
yaml = YAML()


def post(path, body, headers=None):
    return requests.post(HGE + path, json=json.loads(json.dumps(body)), headers=headers or {})


def reset():
    post("/v1/query", {"type": "run_sql", "args": {"sql":
        "drop schema public cascade; create schema public; "
        "grant all on schema public to public; create extension if not exists postgis;"}})
    post("/v1/query", {"type": "clear_metadata", "args": {}})


def replay(path):
    if not os.path.exists(path):
        return True
    with open(path) as f:
        conf = yaml.load(f)
    r = post("/v1/query", conf)
    if r.status_code != 200:
        print(f"  SETUP FAILED ({path}): {r.text[:300]}")
        return False
    return True


def run_dir(directory, only=None):
    reset()
    schema_setup = os.path.join(directory, "schema_setup.yaml")
    plain_setup = os.path.join(directory, "setup.yaml")
    has_values = os.path.exists(os.path.join(directory, "values_setup.yaml"))

    if not replay(schema_setup) or not replay(plain_setup):
        return

    passed = failed = 0
    for path in sorted(glob.glob(os.path.join(directory, "*.yaml"))):
        name = os.path.basename(path)
        if name.startswith(("setup", "teardown", "schema_setup", "schema_teardown",
                            "values_setup", "values_teardown")):
            continue
        if only and not any(o in name for o in only):
            continue
        if has_values:
            replay(os.path.join(directory, "values_setup.yaml"))
        with open(path) as f:
            conf = yaml.load(f)
        confs = conf if isinstance(conf, list) else [conf]
        for i, c in enumerate(confs):
            if not isinstance(c, dict) or "query" not in c:
                continue
            headers = {k: str(v) for k, v in (c.get("headers") or {}).items()}
            r = post(c.get("url", "/v1/graphql"), c["query"], headers)
            expected = json.loads(json.dumps(c.get("response")))
            try:
                actual = r.json()
            except Exception:
                actual = {"_raw": r.text[:150], "_status": r.status_code}
            tag = f"{name}[{i}]" if len(confs) > 1 else name
            if r.status_code == c.get("status", 200) and (expected is None or expected == actual):
                passed += 1
                continue
            failed += 1
            print(f"FAIL {tag} (status {r.status_code} vs {c.get('status', 200)})")
            print("  exp:", json.dumps(expected)[:300])
            print("  act:", json.dumps(actual)[:300])
        if has_values:
            replay(os.path.join(directory, "values_teardown.yaml"))
    print(f"\n{passed} passed, {failed} failed in {directory}")


if __name__ == "__main__":
    run_dir(sys.argv[1], sys.argv[2:] or None)
