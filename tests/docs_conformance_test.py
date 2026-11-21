#!/usr/bin/env python3
"""文档对齐测试：覆盖 oauth_profiles / helpers / catalog 校验的行为。

运行：python3 tests/docs_conformance_test.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target/release/ante"
RESULTS = []


def record(name, ok, detail=""):
    RESULTS.append((name, ok, detail))
    print(f"[{'PASS' if ok else 'FAIL'}] {name}{' — ' + detail if detail else ''}")


def run_doctor(home):
    env = dict(os.environ, ANTE_HOME=str(home), HOME="/tmp/ah_tui")
    out = subprocess.run([str(BIN), "doctor"], env=env, capture_output=True, text=True, timeout=60)
    return out.stdout + out.stderr


def write_catalog(home, data):
    (home / "catalog.json").write_text(json.dumps(data, ensure_ascii=False, indent=2))


def base_provider(credential):
    return {
        "display_name": "Val",
        "base_url": "http://127.0.0.1:8893/v1",
        "wire_style": "OpenAiCompatible",
        "credential": credential,
        "preferred_models": [{"id": "mock-model", "description": "mock"}],
    }


def case_invalid_oauth():
    home = Path(tempfile.mkdtemp(prefix="ante_doc_bad_"))
    write_catalog(home, {
        "providers": {"val": base_provider("env:NOPE")},
        "oauth_profiles": {"bad": {
            "client_id": "x",
            "redirect_uri": "http://evil.example/cb",
            "authorize_url": "not-a-url",
            "token_url": "https://x/token",
            "token_request_encoding": "xml",
        }},
    })
    out = run_doctor(home)
    checks = [
        "oauth profile `bad`: `redirect_uri` must start with http://localhost or http://127.0.0.1",
        "oauth profile `bad`: `authorize_url` must be an absolute http(s) URL",
        "oauth profile `bad`: `token_request_encoding` must be `json` or `form`",
    ]
    record("oauth-profile-validation", all(c in out for c in checks), f"{sum(c in out for c in checks)}/3 warnings")
    shutil.rmtree(home)


def case_valid_oauth():
    home = Path(tempfile.mkdtemp(prefix="ante_doc_ok_"))
    write_catalog(home, {
        "providers": {"val": base_provider("env:NOPE")},
        "oauth_profiles": {"grok": {
            "client_id": "${GROK_CLIENT_ID:-demo}",
            "redirect_uri": "http://127.0.0.1:56121/callback",
            "authorize_url": "https://auth.example/authorize",
            "token_url": "https://auth.example/token",
            "scopes": ["openid", "email", "offline_access"],
            "account_claim": ["email"],
            "token_request_encoding": "form",
            "derived_headers": [{"name": "x-account", "jwt_claim": ["account_id"]}],
        }},
    })
    out = run_doctor(home)
    record("oauth-profile-valid", "oauth profile" not in out, "no oauth warnings")
    shutil.rmtree(home)


def case_helper():
    home = Path(tempfile.mkdtemp(prefix="ante_doc_helper_"))
    write_catalog(home, {
        "providers": {"val": base_provider("helper:tok")},
        "helpers": {"tok": {"command": ["sh", "-c", "echo sk-helper; echo ignored"], "ttl_secs": 600}},
    })
    out = run_doctor(home)
    cache = home / "auth/helper_cache/tok.json"
    ok = "1 authed" in out and cache.exists()
    detail = "credential resolved via helper"
    if cache.exists():
        value = json.loads(cache.read_text()).get("value")
        ok = ok and value == "sk-helper"
        detail += f", cached first line = {value!r}"
    record("helper-first-line-and-cache", ok, detail)
    shutil.rmtree(home)


def case_helper_bad():
    home = Path(tempfile.mkdtemp(prefix="ante_doc_helper_bad_"))
    write_catalog(home, {
        "providers": {"val": base_provider("helper:missing")},
        "helpers": {"missing": {"ttl_secs": 600}},
    })
    out = run_doctor(home)
    record("helper-validation", "helper `missing`: `command` must be a non-empty array of strings" in out, "command field reported")
    shutil.rmtree(home)


def main():
    if not BIN.exists():
        sys.exit(f"missing binary: {BIN}; run cargo build --release first")
    for case in (case_invalid_oauth, case_valid_oauth, case_helper, case_helper_bad):
        case()
    failed = [r for r in RESULTS if not r[1]]
    print(f"\n{len(RESULTS) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
