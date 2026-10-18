#!/usr/bin/env python3
"""
ingest_urls.py — add a list of web pages through the backend's Add-from-URL API.

Does exactly what Admin -> Documents -> Add from URL does, page by page: preview first
(prints chunks, sections, tables and quality warnings), then ingest. So every page goes
through the same Rust pipeline (reader, sections, acronyms, embedding recipe), and the
page list in a file makes a corpus reproducible (e.g. scripts/eval/e6_pages.tsv).

Usage (backend running, from scripts/):
    python ingest_urls.py eval/e6_pages.tsv --dry-run     # preview only, saves nothing
    python ingest_urls.py eval/e6_pages.tsv               # preview + ingest
    python ingest_urls.py eval/e6_pages.tsv --replace     # re-ingest pages already present

Pages whose URL is already in the database are skipped (or deleted and re-added with
--replace), so the script can be re-run after a failure without creating duplicates.

Target: --base-url, default http://localhost:8080 (local backend -> the database in your
.env). Any other address needs --allow-remote, e.g. the live site
https://pmpsti-rag.vercel.app, which writes to PRODUCTION.
Credentials: ADMIN_EMAIL / ADMIN_PASSWORD from the environment or the root .env.
Standard library only.
"""
import argparse
import csv
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

LOCAL_HOSTS = {"localhost", "127.0.0.1", "[::1]"}
TIMEOUT = 120  # a long page plus a Render cold start can take a while


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


class Api:
    """Minimal JSON client that keeps the httpOnly auth_token cookie from login."""

    def __init__(self, base: str):
        self.base = base.rstrip("/")
        self.cookie = ""

    def call(self, method: str, path: str, body: dict | None = None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.base + path, data=data, method=method)
        req.add_header("Accept", "application/json")
        if data is not None:
            req.add_header("Content-Type", "application/json")
        if self.cookie:
            req.add_header("Cookie", self.cookie)
        try:
            with urllib.request.urlopen(req, timeout=TIMEOUT) as res:
                status, headers, text = res.status, res.headers, res.read().decode(errors="replace")
        except urllib.error.HTTPError as e:
            status, headers, text = e.code, e.headers, e.read().decode(errors="replace")
        for header in headers.get_all("Set-Cookie") or []:
            pair = header.split(";", 1)[0].strip()
            if pair.startswith("auth_token=") and pair != "auth_token=":
                self.cookie = pair
        try:
            payload = json.loads(text)
        except ValueError:
            payload = {"error": text[:300] or "(empty body)"}
        return status, payload


def read_pages(path: Path) -> list[dict]:
    lines = [l for l in path.read_text(encoding="utf-8").splitlines() if l.strip() and not l.startswith("#")]
    rows = list(csv.DictReader(lines, delimiter="\t"))
    for i, r in enumerate(rows, 1):
        if not (r.get("url") or "").startswith(("http://", "https://")):
            sys.exit(f"{path}: row {i} has no valid url: {r}")
    return rows


def norm(url: str) -> str:
    return url.strip().rstrip("/").lower()


def main() -> int:
    ap = argparse.ArgumentParser(description="Add web pages through the backend's Add-from-URL API")
    ap.add_argument("pages", type=Path, help="TSV file with columns url, title, category, subcategory")
    ap.add_argument("--base-url", default="http://localhost:8080", help="backend or frontend URL (default: local backend)")
    ap.add_argument("--allow-remote", action="store_true", help="allow a non-local --base-url (e.g. the live site)")
    ap.add_argument("--dry-run", action="store_true", help="preview only; save nothing")
    ap.add_argument("--replace", action="store_true", help="delete and re-add pages that are already present")
    ap.add_argument("--strategy", default="auto", help="chunking strategy (default: auto)")
    ap.add_argument("--pause", type=float, default=3.0, help="seconds between pages (Gemini per-minute limits)")
    args = ap.parse_args()

    host = urllib.parse.urlparse(args.base_url).hostname or ""
    if host not in LOCAL_HOSTS and f"[{host}]" not in LOCAL_HOSTS and not args.allow_remote:
        sys.exit(f"{args.base_url} is not local. Pass --allow-remote if you really mean it "
                 "(the live site writes to PRODUCTION).")

    pages = read_pages(args.pages)
    api = Api(args.base_url)
    email, password = env_value("ADMIN_EMAIL"), env_value("ADMIN_PASSWORD")
    if not email or not password:
        sys.exit("ADMIN_EMAIL and ADMIN_PASSWORD must be set (environment or root .env).")
    status, body = api.call("POST", "/api/auth/login", {"email": email, "password": password})
    if status != 200 or not api.cookie:
        sys.exit(f"Login failed: HTTP {status} {body.get('error', '')}")

    status, body = api.call("GET", "/api/admin/documents")
    if status != 200:
        sys.exit(f"Could not list documents: HTTP {status} {body.get('error', '')}")
    existing: dict[str, list[str]] = {}
    for d in body.get("data", []):
        if d.get("source_url"):
            existing.setdefault(norm(d["source_url"]), []).append(d["document_id"])

    mode = "DRY RUN (preview only)" if args.dry_run else ("replace existing" if args.replace else "skip existing")
    print(f"Backend : {args.base_url}\nPages   : {len(pages)} from {args.pages}\nMode    : {mode}\n")

    done, skipped, failed, total_chunks = 0, 0, [], 0
    for i, page in enumerate(pages, 1):
        url = page["url"].strip()
        payload = {"url": url, "chunk_strategy": args.strategy}
        for field in ("title", "category", "subcategory"):
            if (page.get(field) or "").strip():
                payload[field] = page[field].strip()
        print(f"[{i:2d}/{len(pages)}] {url}")

        present = existing.get(norm(url), [])
        if present and not args.replace and not args.dry_run:
            print(f"        skipped: already present ({len(present)} document(s))")
            skipped += 1
            continue
        if i > 1 and args.pause > 0:
            time.sleep(args.pause)

        status, body = api.call("POST", "/api/admin/documents/preview-url", payload)
        if status != 200:
            print(f"        PREVIEW FAILED: HTTP {status} {body.get('error', '')}")
            failed.append(url)
            continue
        p = body["data"]
        tables = ", ".join(f"{t['rows']} rows" for t in p.get("tables", [])) or "none"
        print(f"        {p['chunk_count']} chunks · {p.get('sections', '?')} sections · tables: {tables} · \"{p['title'][:60]}\"")
        for w in p.get("warnings", []):
            print(f"        warning: {w}")
        if args.dry_run:
            done += 1
            total_chunks += p["chunk_count"]
            continue

        if present and args.replace:
            for doc_id in present:
                status, body = api.call("DELETE", "/api/admin/documents/" + urllib.parse.quote(doc_id, safe=""))
                if status != 200:
                    print(f"        DELETE FAILED for {doc_id}: HTTP {status} {body.get('error', '')}")
            print(f"        replaced {len(present)} existing document(s)")

        started = time.time()
        status, body = api.call("POST", "/api/admin/documents/ingest-url", payload)
        if status != 200:
            print(f"        INGEST FAILED: HTTP {status} {body.get('error', '')}")
            failed.append(url)
            continue
        n = body["data"]["chunks"]
        print(f"        saved: {n} chunks in {time.time() - started:.1f} s")
        done += 1
        total_chunks += n

    verb = "previewed" if args.dry_run else "saved"
    print(f"\n{done} {verb} ({total_chunks} chunks), {skipped} skipped, {len(failed)} failed.")
    for url in failed:
        print(f"  failed: {url}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
