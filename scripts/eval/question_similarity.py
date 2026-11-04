"""
question_similarity.py — choose SIMILAR_QUESTION_THRESHOLD (grouping of similar questions)
==========================================================================================
The backend groups questions in query_logs when their embeddings are at least
SIMILAR_QUESTION_THRESHOLD similar (cosine; default 0.90): typos and other wordings of one
question should share a group (FAQ, Unanswered, Query Logs), different questions should not.

This script measures both sides on the test sets:
  - same question:      each test question vs typo variants of it (made here, deterministic)
  - different question: every pair of different test questions

and prints, per threshold, how many typo variants would be grouped correctly and how many
different questions would wrongly be merged, plus a suggested threshold.

Uses the same embeddings as the backend (gemini-embedding-001, RETRIEVAL_QUERY, 768 dims) and
the same cache as eval_retrieval.py (scripts/eval/.cache/query_embeddings.json), so a re-run is free.
First run: about 230 embeddings (5 batch calls). Needs GEMINI_API_KEY (.env in the repo root).
No database access.

  cd scripts && source .venv/bin/activate
  python eval/question_similarity.py                 # report
  python eval/question_similarity.py --save          # also write eval/results/<date>_similarity.json
"""

import argparse
import csv
import datetime as dt
import hashlib
import json
import math
import random
import re
import sys
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPTS))
from gemini_embed import GeminiEmbeddings  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parent))
from typos import SEED, typo_variants  # noqa: E402

ROOT = SCRIPTS.parent
CACHE_FILE = SCRIPTS / "eval" / ".cache" / "query_embeddings.json"  # shared with eval_retrieval.py
RESULTS = SCRIPTS / "eval" / "results"
THRESHOLDS = [0.80, 0.82, 0.84, 0.86, 0.88, 0.90, 0.92, 0.94, 0.96]


def test_questions() -> list[str]:
    qs = [q["q"] for q in json.loads((SCRIPTS / "eval" / "questions.json").read_text())["questions"]]
    with open(ROOT / "eval" / "pertanyaan_uji.csv", encoding="utf-8-sig") as f:
        qs += [row["pertanyaan"] for row in csv.DictReader(f) if row.get("pertanyaan")]
    seen, out = set(), []
    for q in qs:
        k = q.strip().lower()
        if k and k not in seen:
            seen.add(k)
            out.append(q.strip())
    return out


def embed_all(texts: list[str]) -> dict[str, list[float]]:
    try:
        cache = json.loads(CACHE_FILE.read_text())
    except FileNotFoundError:
        cache = {}
    key = lambda q: hashlib.sha1(q.encode()).hexdigest()  # same key as eval_retrieval.py
    missing = sorted({t for t in texts if key(t) not in cache})
    if missing:
        print(f"Embedding {len(missing)} new texts (Gemini RETRIEVAL_QUERY)...")
        model = GeminiEmbeddings(task_type="RETRIEVAL_QUERY")
        CACHE_FILE.parent.mkdir(parents=True, exist_ok=True)
        step = 25  # saved after every step, so an interrupted run keeps what it already paid for
        for i in range(0, len(missing), step):
            part = missing[i:i + step]
            print(f"  {i + len(part)}/{len(missing)}", flush=True)
            for t, v in zip(part, model.embed(part, "RETRIEVAL_QUERY")):
                cache[key(t)] = v
            CACHE_FILE.write_text(json.dumps(cache))
    return {t: cache[key(t)] for t in texts}


def trigrams(t: str) -> set[str]:
    """Letter trigrams per word, padded like pg_trgm — same as trigram_similarity in src/db.rs."""
    out = set()
    for w in re.findall(r"[0-9a-z]+", t.lower()):
        w = f"  {w} "
        out |= {w[i:i + 3] for i in range(len(w) - 2)}
    return out


def overlap(a: str, b: str) -> float:
    x, y = trigrams(a), trigrams(b)
    return len(x & y) / len(x | y) if x | y else 0.0


def cosine(a: list[float], b: list[float]) -> float:
    dot = sum(x * y for x, y in zip(a, b))
    return dot / (math.sqrt(sum(x * x for x in a)) * math.sqrt(sum(y * y for y in b)))


def pct(values: list[float], p: float) -> float:
    s = sorted(values)
    return s[min(len(s) - 1, max(0, round(p / 100 * (len(s) - 1))))]


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--save", action="store_true", help="write the numbers to eval/results/")
    args = ap.parse_args()

    rng = random.Random(SEED)
    questions = test_questions()
    variants = {q: typo_variants(q, rng) for q in questions}
    vec = embed_all(questions + [v for vs in variants.values() for v in vs])

    same = [(q, v, cosine(vec[q], vec[v])) for q in questions for v in variants[q] if v != q]
    diff = [(a, b, cosine(vec[a], vec[b]))
            for i, a in enumerate(questions) for b in questions[i + 1:]]

    print(f"\n{len(questions)} questions, {len(same)} typo variants, {len(diff)} pairs of different questions\n")
    print("same question (typo):     min {:.3f}  p5 {:.3f}  median {:.3f}".format(
        min(s for *_, s in same), pct([s for *_, s in same], 5), pct([s for *_, s in same], 50)))
    print("different questions:      p95 {:.3f}  p99 {:.3f}  max {:.3f}".format(
        pct([s for *_, s in diff], 95), pct([s for *_, s in diff], 99), max(s for *_, s in diff)))

    print("\nthreshold   typos grouped   different questions merged")
    rows = []
    for t in THRESHOLDS:
        ok = sum(s >= t for *_, s in same) / len(same)
        bad = sum(s >= t for *_, s in diff)
        rows.append({"threshold": t, "typos_grouped": round(ok, 4), "different_merged": bad})
        print(f"  {t:.2f}        {ok:6.1%}          {bad:4d}  ({bad / len(diff):.2%})")

    # Highest threshold that still groups 95% of typos; then check it merges almost nothing else
    # The backend groups when BOTH hold: embedding ≥ SIMILAR_QUESTION_THRESHOLD and trigram
    # overlap ≥ SIMILAR_QUESTION_MIN_OVERLAP. Grid of both:
    same2 = [(q, v, s, overlap(q, v)) for q, v, s in same]
    diff2 = [(a, b, s, overlap(a, b)) for a, b, s in diff]
    print("\nembedding  + overlap   typos grouped   different questions merged")
    grid = []
    for t in [0.75, 0.78, 0.80, 0.82, 0.85, 0.88]:
        for o in [0.35, 0.40, 0.45, 0.50]:
            ok = sum(s >= t and x >= o for *_, s, x in same2) / len(same2)
            bad = sum(s >= t and x >= o for *_, s, x in diff2)
            grid.append({"threshold": t, "min_overlap": o, "typos_grouped": round(ok, 4), "different_merged": bad})
            print(f"  {t:.2f}     {o:.2f}       {ok:6.1%}          {bad:4d}  ({bad / len(diff2):.2%})")
    # most typos grouped while merging at most 0.2% of different pairs
    fine = [g for g in grid if g["different_merged"] <= 0.002 * len(diff2)]
    best = max(fine, key=lambda g: (g["typos_grouped"], g["threshold"])) if fine else None
    suggestion = best and {"SIMILAR_QUESTION_THRESHOLD": best["threshold"], "SIMILAR_QUESTION_MIN_OVERLAP": best["min_overlap"]}
    print(f"\nSuggested: {suggestion or 'no setting merges ≤ 0.2% of different questions'}"
          + (f" — groups {best['typos_grouped']:.0%} of typos, merges {best['different_merged']} different pairs" if best else ""))
    if best:
        print("Different questions merged at that setting (check by hand; true duplicates are fine):")
        for a, b, s, x in sorted(diff2, key=lambda d: -d[2]):
            if s >= best["threshold"] and x >= best["min_overlap"]:
                print(f"  emb {s:.3f}  overlap {x:.2f}  {a}  |  {b}")

    print("\nMost similar DIFFERENT questions (check by hand: true duplicates in the test sets are fine):")
    for a, b, s in sorted(diff, key=lambda x: -x[2])[:10]:
        print(f"  {s:.3f}  {a}  |  {b}")
    print("\nLeast similar typo variants:")
    for q, v, s in sorted(same, key=lambda x: x[2])[:8]:
        print(f"  {s:.3f}  {q}  →  {v}")

    if args.save:
        RESULTS.mkdir(parents=True, exist_ok=True)
        out = RESULTS / f"{dt.datetime.now():%Y%m%d-%H%M}_similarity.json"
        out.write_text(json.dumps({
            "model": "gemini-embedding-001 / RETRIEVAL_QUERY / 768",
            "questions": len(questions), "typo_pairs": len(same), "different_pairs": len(diff),
            "thresholds": rows, "grid": grid, "suggested": suggestion,
            "lowest_typo_pairs": [{"q": q, "variant": v, "sim": round(s, 4)} for q, v, s in sorted(same, key=lambda x: x[2])[:20]],
            "highest_different_pairs": [{"a": a, "b": b, "sim": round(s, 4)} for a, b, s in sorted(diff, key=lambda x: -x[2])[:20]],
        }, ensure_ascii=False, indent=1))
        print(f"\nSaved {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
