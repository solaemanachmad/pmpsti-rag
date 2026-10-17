"""
eval_retrieval.py — measure retrieval and answer quality against two test sets
===============================================================================
1. scripts/eval/questions.json — does the right DOCUMENT appear in the top results?
2. eval/pertanyaan_uji.csv     — do the top chunks CONTAIN THE ANSWER (kata_kunci column)?

Every mode uses the SAME SQL as the backend (src/db.rs): semantic, keyword (OR tsquery
without question words), hybrid (0.7 vector + 0.3 ts_rank, the chat default), rrf (k=60).
Change the m_* functions here together with the backend so results stay comparable.

    python eval_retrieval.py                     # all modes, save results
    python eval_retrieval.py --label baseline    # name the run
    python eval_retrieval.py --modes semantic,hybrid
    python eval_retrieval.py --compare eval/results/<file>.json
    python eval_retrieval.py --verbose           # per-question details
    python eval_retrieval.py --backend http://localhost:8080   # + end-to-end answer check

Metrics:
  doc hit@1 / hit@5 / MRR  : right document at rank 1 / in the top 5 (out of 10 results)
  ans@5                    : every kata_kunci group appears in the text of the top 5 chunks
  e2e (--backend)          : kata_kunci appears in the backend's ANSWER (/api/ask_public),
                             including out-of-scope questions that must be answered "not found".
                             Uses LLM quota (about 3 Gemini calls per question).
Questions whose source is not ingested yet, or is out of scope, are counted separately.

The database comes from DATABASE_URL (root .env) and is opened READ-ONLY.
Question embeddings are cached in eval/.cache/ so later runs use no embedding quota.
"""

import argparse
import csv
import datetime
import hashlib
import json
import os
import platform
import re
import subprocess
import sys
import time
from pathlib import Path

import psycopg2
import requests

from gemini_embed import DIM as EMBED_DIM, MODEL as EMBED_MODEL, GeminiEmbeddings  # also loads .env

HERE = Path(__file__).resolve().parent
EVAL_DIR = HERE / "eval"
DOC_SET = EVAL_DIR / "questions.json"
ANSWER_SET = HERE.parent / "eval" / "pertanyaan_uji.csv"
CACHE_FILE = EVAL_DIR / ".cache" / "query_embeddings.json"
RESULTS_DIR = EVAL_DIR / "results"
TOP_K = 10
ANSWER_K = 5
E2E_PAUSE_S = 8

COLS = "document_id, COALESCE(source_url, '') AS source_url, COALESCE(content, '') AS content, chunk_index"


# ── Document → stable path ─────────────────────────────────────────
def normalise(path: str) -> str:
    path = re.sub(r"^https?://[^/]+/", "", path.strip())
    path = re.sub(r"^panduan_pmpsti/", "", path)
    path = re.sub(r"\.html?$", "", path)
    path = re.sub(r"/index$", "", path)
    return path.strip("/")


def doc_keys(document_id: str, source_url: str) -> set[str]:
    keys = {normalise(document_id)}
    if source_url:
        keys.add(normalise(source_url))
    return keys


# ── Query embedding (cache) ────────────────────────────────────────
def embed_questions(questions: list[str]) -> dict[str, str]:
    try:
        cache = json.loads(CACHE_FILE.read_text())
    except FileNotFoundError:
        cache = {}
    key = lambda q: hashlib.sha1(q.encode()).hexdigest()
    missing = sorted({q for q in questions if key(q) not in cache})
    if missing:
        print(f"Embedding {len(missing)} new questions (Gemini RETRIEVAL_QUERY)...")
        model = GeminiEmbeddings(task_type="RETRIEVAL_QUERY")
        for q, v in zip(missing, model.embed(missing, "RETRIEVAL_QUERY")):
            cache[key(q)] = v
        CACHE_FILE.parent.mkdir(parents=True, exist_ok=True)
        CACHE_FILE.write_text(json.dumps(cache))
    as_literal = lambda v: "[" + ",".join(f"{x:.8f}" for x in v) + "]"
    return {q: as_literal(cache[key(q)]) for q in questions}


# ── Keywords — same as keyword_tsquery() in src/db.rs ───────────────
# Question and function words that make keyword search fail when every word must match.
STOPWORDS = set("""
apa apakah siapa bagaimana berapa kapan mana dimana di ke dari dan atau yang untuk
dengan pada dalam ini itu adalah ada saja sudah belum bisa dapat harus akan juga
saya kamu kami mohon tolong sebutkan jelaskan semua beserta tentang serta jika
the a an of to in on for and or is are do does did how what who which when where
can i my me with about
""".split())


def keyword_tsquery(text: str) -> str | None:
    words = re.split(r"[^0-9a-z]+", text.lower())
    terms = [w for w in dict.fromkeys(words) if len(w) > 1 and w not in STOPWORDS]
    return " | ".join(terms) or None


# ── Search modes (mirror src/db.rs) ─────────────────────────────────
def m_semantic(cur, text, vec, limit):
    cur.execute(f"""SELECT {COLS} FROM documents WHERE embedding IS NOT NULL
                    ORDER BY embedding <=> %s::vector LIMIT %s""", (vec, limit))
    return cur.fetchall()


def m_keyword(cur, text, vec, limit):  # search_fulltext: also the fallback when embedding fails
    q = keyword_tsquery(text)
    if not q:
        return []
    cur.execute(f"""SELECT {COLS} FROM documents
                    WHERE to_tsvector('indonesian', content) @@ to_tsquery('indonesian', %s)
                    ORDER BY ts_rank(to_tsvector('indonesian', content),
                                     to_tsquery('indonesian', %s), 1) DESC
                    LIMIT %s""", (q, q, limit))
    return cur.fetchall()


def m_hybrid(cur, text, vec, limit):  # search_hybrid — default mode
    cur.execute(f"""SELECT {COLS} FROM documents
                    ORDER BY (0.7 * COALESCE(1.0 - (embedding <=> %s::vector), 0.0)
                            + 0.3 * ts_rank(to_tsvector('indonesian', content),
                                            plainto_tsquery('indonesian', %s), 1)) DESC
                    LIMIT %s""", (vec, text, limit))
    return cur.fetchall()


def m_rrf(cur, text, vec, limit, k=60):  # search_rrf — only when requested explicitly
    cand = max(limit * 3, 30)
    cur.execute(f"""SELECT {COLS} FROM documents
                    ORDER BY ts_rank(to_tsvector('indonesian', content),
                                     plainto_tsquery('indonesian', %s), 1) DESC
                    LIMIT %s""", (text, cand))
    scores, rows = {}, {}
    for results in (m_semantic(cur, text, vec, cand), cur.fetchall()):
        for rank, row in enumerate(results, start=1):
            key = (row[0], row[3])  # (document_id, chunk_index)
            scores[key] = scores.get(key, 0.0) + 1.0 / (k + rank)
            rows[key] = row
    return [rows[key] for key in sorted(scores, key=scores.get, reverse=True)[:limit]]


MODES = {"semantic": m_semantic, "keyword": m_keyword, "hybrid": m_hybrid, "rrf": m_rrf}


# ── Run metadata (for reproducibility / papers) ───────────────────
def file_sha256(path: Path) -> str | None:
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else None


def git_state() -> dict:
    def run(*cmd):
        try:
            return subprocess.run(cmd, cwd=HERE, capture_output=True, text=True, timeout=10).stdout.strip()
        except Exception:
            return ""
    return {"commit": run("git", "rev-parse", "HEAD") or None,
            "branch": run("git", "rev-parse", "--abbrev-ref", "HEAD") or None,
            "dirty": bool(run("git", "status", "--porcelain", "--untracked-files=no"))}


def run_metadata(cur, args) -> dict:
    cur.execute("SELECT id, document_id, COALESCE(content, '') FROM documents ORDER BY id")
    rows = cur.fetchall()
    fingerprint = hashlib.sha256("\n".join(
        f"{i}\t{d}\t{hashlib.sha256(c.encode()).hexdigest()}" for i, d, c in rows).encode()).hexdigest()
    llm_backend = os.environ.get("LLM_BACKEND", "")
    return {
        "git": git_state(),
        "python": platform.python_version(),
        "embedding": {"model": EMBED_MODEL, "dimensions": EMBED_DIM,
                      "query_task_type": "RETRIEVAL_QUERY", "document_task_type": "RETRIEVAL_DOCUMENT"},
        "llm_for_e2e": {"backend": llm_backend,
                        "model": os.environ.get("GEMINI_MODEL" if llm_backend == "gemini" else "LLM_MODEL", "")}
                       if args.backend else None,
        "settings": {"top_k": TOP_K, "answer_k": ANSWER_K, "e2e_pause_s": E2E_PAUSE_S,
                     "hybrid_weights": [0.7, 0.3], "rrf_k": 60, "ts_config": "indonesian"},
        "corpus": {"chunks": len(rows), "documents": len({d for _, d, _ in rows}),
                   "fingerprint_sha256": fingerprint},
        "test_sets": {"questions.json": file_sha256(DOC_SET), "pertanyaan_uji.csv": file_sha256(ANSWER_SET)},
        "note": "e2e answers come from an LLM and vary between runs; repeat runs to report mean and spread.",
    }


# ── Scoring ────────────────────────────────────────────────────────
def first_hit_rank(results, expect: set[str]) -> int | None:
    for rank, row in enumerate(results, start=1):
        if doc_keys(row[0], row[1]) & expect:
            return rank
    return None


def answer_found(results, keywords: str) -> bool:
    text = " ".join(" ".join(row[2].split()) for row in results[:ANSWER_K]).lower()
    groups = [g for g in keywords.split("|") if g.strip()]
    return all(any(alt.strip().lower() in text for alt in g.split("/")) for g in groups)


def summarise(ranks: list[int | None]) -> dict:
    n = len(ranks) or 1
    return {"n": len(ranks),
            "hit@1": sum(1 for r in ranks if r == 1) / n,
            "hit@5": sum(1 for r in ranks if r and r <= 5) / n,
            "mrr": sum(1 / r for r in ranks if r) / n}


def load_answer_set() -> tuple[list[dict], list[dict]]:
    if not ANSWER_SET.exists():
        return [], []
    rows = list(csv.DictReader(ANSWER_SET.open(encoding="utf-8-sig")))
    scored = [r for r in rows if r["status_sumber"] == "ada" and r["jenis"] != "multi_turn"]
    return scored, [r for r in rows if r not in scored]


def run_e2e(backend: str, rows: list[dict], verbose: bool) -> dict:
    """Ask the real backend and check kata_kunci in its answers."""
    rows = [r for r in rows if r["jenis"] != "multi_turn" and r["status_sumber"] in ("ada", "-")]
    stamp = datetime.datetime.now().strftime("%H%M%S")
    results = []
    print(f"\nEnd-to-end: {len(rows)} questions to {backend} ...")
    for n, r in enumerate(rows):
        if n:
            time.sleep(E2E_PAUSE_S)  # ~3 Gemini calls per question; stay under the per-minute limit
        for attempt in range(4):
            try:
                resp = requests.post(f"{backend.rstrip('/')}/api/ask_public", timeout=180,
                                     headers={"X-Guest-Token": f"guest_eval_{stamp}_{r['id']}_{attempt}"},
                                     json={"query": r["pertanyaan"]})
                data = resp.json()
                data = data.get("data", data)
                answer = data.get("answer", "") if isinstance(data, dict) else ""
                if not answer:
                    answer = f"[HTTP {resp.status_code}] {str(data)[:200]}"
            except Exception as e:  # network / timeout
                answer = f"[ERROR] {e}"
            if "429" not in answer[:120]:
                break
            print(f"    (Gemini 429 — waiting 30 s, retry {attempt + 1}/3)")
            time.sleep(30)
        flat = " ".join(answer.split())
        ok = answer_found([(None, None, flat, None)], r["kata_kunci"])
        results.append({"id": r["id"], "type": r["jenis"], "q": r["pertanyaan"], "ok": ok, "answer": flat[:500]})
        print(f"  {'✓' if ok else '·'} {r['id']:<4} {r['pertanyaan'][:60]}")
        if verbose and not ok:
            print(f"        → {flat[:220]}")
    score = sum(p["ok"] for p in results) / (len(results) or 1)
    print(f"End-to-end correct: {sum(p['ok'] for p in results)}/{len(results)} ({score:.0%})")
    return {"score": score, "questions": results}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default=os.environ.get("DATABASE_URL"))
    ap.add_argument("--label", default="run")
    ap.add_argument("--modes", default=",".join(MODES))
    ap.add_argument("--compare", help="Earlier results file to compare against")
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--backend", help="Backend URL for the end-to-end check, e.g. http://localhost:8080")
    args = ap.parse_args()
    if not args.db:
        sys.exit("DATABASE_URL is not set.")
    modes = [m.strip() for m in args.modes.split(",")]

    doc_qs = json.loads(DOC_SET.read_text())["questions"]
    ans_qs, ans_skipped = load_answer_set()
    vectors = embed_questions([q["q"] for q in doc_qs] + [r["pertanyaan"] for r in ans_qs])

    conn = psycopg2.connect(args.db)
    conn.set_session(readonly=True)
    cur = conn.cursor()
    cur.execute("SELECT DISTINCT document_id, COALESCE(source_url, '') FROM documents")
    present = set().union(*(doc_keys(d, u) for d, u in cur.fetchall()))
    host = args.db.split("@")[-1].split("/")[0]
    meta = run_metadata(cur, args)

    # Set 1 — documents
    doc_results = []
    for q in doc_qs:
        expect = {normalise(e) for e in q["expect"]}
        ranks = {m: first_hit_rank(MODES[m](cur, q["q"], vectors[q["q"]], TOP_K), expect) for m in modes}
        doc_results.append({"id": q["id"], "q": q["q"], "type": q["type"],
                            "ingested": bool(expect & present), "ranks": ranks})
    scored = [p for p in doc_results if p["ingested"]]
    doc_summary = {m: summarise([p["ranks"][m] for p in scored]) for m in modes}

    # Set 2 — answers
    ans_results = []
    for r in ans_qs:
        q = r["pertanyaan"]
        found = {m: answer_found(MODES[m](cur, q, vectors[q], ANSWER_K), r["kata_kunci"]) for m in modes}
        ans_results.append({"id": r["id"], "q": q, "type": r["jenis"], "found": found})
    ans_summary = {m: sum(p["found"][m] for p in ans_results) / (len(ans_results) or 1) for m in modes}

    # ── Report ──
    print(f"\nDatabase : {host}   corpus {meta['corpus']['chunks']} chunks, fingerprint {meta['corpus']['fingerprint_sha256'][:12]}")
    print(f"Code     : {(meta['git']['commit'] or '?')[:7]}{' (uncommitted changes)' if meta['git']['dirty'] else ''}")
    print(f"Document set : {len(scored)} scored ({len(doc_results) - len(scored)} sources not ingested yet)")
    print(f"Answer set   : {len(ans_results)} scored ({len(ans_skipped)} skipped: not ingested / unverified / out of scope / multi-turn)\n")
    print(f"{'mode':<20}{'doc hit@1':>10}{'doc hit@5':>10}{'MRR':>7}{'ans@5':>8}")
    for m in modes:
        s = doc_summary[m]
        print(f"{m:<20}{s['hit@1']:>10.0%}{s['hit@5']:>10.0%}{s['mrr']:>7.2f}{ans_summary[m]:>8.0%}")

    if args.verbose:
        fmt = lambda r: "—" if r is None else str(r)
        print(f"\nRank of the right document (— = not in the top {TOP_K}):")
        print(f"{'id':<20}" + "".join(f"{m[:11]:>12}" for m in modes))
        for p in doc_results:
            print(f"{p['id']:<20}" + "".join(f"{fmt(p['ranks'][m]):>12}" for m in modes)
                  + ("" if p["ingested"] else "  (not ingested)"))
        print(f"\nAnswer in the top {ANSWER_K} chunks (✓/·):")
        print(f"{'id':<6}" + "".join(f"{m[:11]:>12}" for m in modes) + "  question")
        for p in ans_results:
            print(f"{p['id']:<6}" + "".join(f"{'✓' if p['found'][m] else '·':>12}" for m in modes) + f"  {p['q'][:60]}")

    e2e = None
    if args.backend:
        all_rows = list(csv.DictReader(ANSWER_SET.open(encoding="utf-8-sig")))
        e2e = run_e2e(args.backend, all_rows, args.verbose)

    # ── Save & compare ──
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M")
    out = RESULTS_DIR / f"{stamp}_{args.label}.json"
    out.write_text(json.dumps({"label": args.label, "time": stamp, "db_host": host, "meta": meta,
                               "doc_summary": doc_summary, "answer_summary": ans_summary,
                               "doc_questions": doc_results, "answer_questions": ans_results,
                               "e2e": e2e, "backend": args.backend}, indent=2))
    print(f"\nResults saved: {out.relative_to(HERE)}")

    if args.compare:
        prev = json.loads(Path(args.compare).read_text())
        print(f"\nCompared with '{prev['label']}' ({prev['time']}):")
        for m in modes:
            if m in prev.get("doc_summary", {}):
                a, b = prev["doc_summary"][m], doc_summary[m]
                pa, pb = prev["answer_summary"].get(m, 0), ans_summary[m]
                print(f"  {m:<20} doc hit@5 {a['hit@5']:.0%} → {b['hit@5']:.0%}   ans@5 {pa:.0%} → {pb:.0%}")
        if e2e and prev.get("e2e"):
            print(f"  {'end-to-end':<20} {prev['e2e']['score']:.0%} → {e2e['score']:.0%}")


if __name__ == "__main__":
    main()
