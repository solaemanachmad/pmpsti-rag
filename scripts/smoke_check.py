#!/usr/bin/env python3
"""
smoke_check.py — quick check that the live site's login and /api proxy still work.

It guards against two regressions that broke production before:
  1. The auth cookie not reaching the browser as a first-party cookie (third-party
     cookie / PUBLIC_API_URL): login "succeeds" but /api/auth/me then answers
     401 "Token tidak ditemukan".
  2. The /api proxy decoding "%2F" in document ids into real slashes: no backend
     route matches and the request ends in an empty 404.

Checks (read-only; nothing is created or deleted):
  1. POST /api/auth/login        -> 200 and an `auth_token` Set-Cookie
                                    (the JSON `token` field is intentionally empty)
  2. GET  /api/auth/me + cookie  -> 200 with the user's email
  3. DELETE /api/admin/documents/<encoded id> WITHOUT the cookie
                                 -> 401 JSON from the backend (route reached).
     Sent without the cookie, so it can never delete anything.

Usage:
    SMOKE_EMAIL=you@mail.ugm.ac.id SMOKE_PASSWORD=... python smoke_check.py
    python smoke_check.py --base-url https://<preview>.vercel.app

SMOKE_EMAIL / SMOKE_PASSWORD are read from the environment or the root .env.
Never hard-code them. Note: 5 failed logins lock the login for 15 minutes.
Exit code 0 = all checks passed, 1 = a check failed.
Standard library only, so it runs anywhere with Python 3.
"""
import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

DEFAULT_BASE_URL = "https://pmpsti-rag.vercel.app"
# Render's free plan needs ~50 s to wake up; the proxy allows 60 s.
TIMEOUT = 90
# Must contain an encoded slash; it does not need to exist (the request is unauthenticated).
ENCODED_ID = "smoke-check%2Fdoes-not-exist"


def env_value(name: str) -> str:
    """Read a variable from the environment, falling back to the root .env."""
    value = os.environ.get(name, "")
    if value:
        return value
    env_file = Path(__file__).resolve().parents[1] / ".env"
    if env_file.exists():
        for line in env_file.read_text().splitlines():
            if line.startswith(f"{name}="):
                return line.split("=", 1)[1].strip().strip('"').strip("'")
    return ""


def request(method: str, url: str, body: dict | None = None, cookie: str | None = None):
    """Send a request and return (status, headers, body text). HTTP errors are not raised."""
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Accept", "application/json")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    if cookie:
        req.add_header("Cookie", cookie)
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as res:
            return res.status, res.headers, res.read().decode(errors="replace")
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read().decode(errors="replace")


def auth_cookie(headers) -> str | None:
    """Return 'auth_token=<value>' from the Set-Cookie headers, if present and non-empty."""
    for header in headers.get_all("Set-Cookie") or []:
        pair = header.split(";", 1)[0].strip()
        if pair.startswith("auth_token=") and pair != "auth_token=":
            return pair
    return None


def main() -> int:
    ap = argparse.ArgumentParser(description="Smoke-check login and the /api proxy of a deployment")
    ap.add_argument("--base-url", default=DEFAULT_BASE_URL,
                    help=f"frontend URL (default: {DEFAULT_BASE_URL})")
    args = ap.parse_args()
    base = args.base_url.rstrip("/")

    email = env_value("SMOKE_EMAIL")
    password = env_value("SMOKE_PASSWORD")
    if not email or not password:
        print("SMOKE_EMAIL and SMOKE_PASSWORD must be set (environment or root .env).")
        return 1

    print(f"Base URL: {base}\n")
    failures = 0

    def report(ok: bool, name: str, detail: str) -> None:
        nonlocal failures
        failures += 0 if ok else 1
        print(f"[{'PASS' if ok else 'FAIL'}] {name}: {detail}")

    # 1. Login: the JWT must arrive only as the httpOnly auth_token cookie.
    status, headers, text = request("POST", f"{base}/api/auth/login",
                                    {"email": email, "password": password})
    cookie = auth_cookie(headers)
    report(status == 200 and cookie is not None, "login",
           f"HTTP {status}, auth_token cookie {'set' if cookie else 'MISSING'}"
           + ("" if status == 200 else f" — {text[:200]}"))

    # 2. /me with that cookie: proves the cookie is accepted on the next request.
    if cookie:
        status, _, text = request("GET", f"{base}/api/auth/me", cookie=cookie)
        try:
            me = json.loads(text).get("data") or {}
        except ValueError:
            me = {}
        ok = status == 200 and me.get("email", "").lower() == email.lower()
        report(ok, "auth/me", f"HTTP {status}"
               + (f", role {me.get('role')}" if ok else f" — {text[:200] or '(empty body)'}"))
    else:
        report(False, "auth/me", "skipped (no auth_token cookie from login)")

    # 3. Encoded document id reaches the backend route (401 JSON, not an empty 404).
    status, _, text = request("DELETE", f"{base}/api/admin/documents/{ENCODED_ID}")
    try:
        body = json.loads(text)
    except ValueError:
        body = None
    ok = status == 401 and isinstance(body, dict) and body.get("code") == 401
    report(ok, "encoded id route", f"HTTP {status}, "
           + (f"backend says: {body.get('error')}" if isinstance(body, dict)
              else f"body: {text[:200] or '(empty)'}"))

    print(f"\n{'All checks passed.' if failures == 0 else f'{failures} check(s) failed.'}")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
