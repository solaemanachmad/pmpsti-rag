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
the same cache as eval_retrieval.py (eval/.cache/query_embeddings.json), so a re-run is free.
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

ROOT = SCRIPTS.parent
CACHE_FILE = ROOT / "eval" / ".cache" / "query_embeddings.json"
RESULTS = SCRIPTS / "eval" / "results"
THRESHOLDS = [0.80, 0.82, 0.84, 0.86, 0.88, 0.90, 0.92, 0.94, 0.96]
VOWELS = "aiueo"


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


def typo_variants(q: str, rng: random.Random) -> list[str]:
    """Two misspellings the way people type fast: lower case, no '?', dropped vowels or letters,
    swapped neighbours. Words of 1–3 letters and acronyms in the middle stay readable."""
    base = re.sub(r"[?!.]+$", "", q.strip()).lower()
    words = base.split()
    long_idx = [i for i, w in enumerate(words) if len(w) >= 5 and w.isalpha()]
    if not long_idx:
        return [base]

    def drop_vowel(w: str) -> str:
        pos = [i for i, c in enumerate(w) if c in VOWELS and 0 < i < len(w) - 1]
        if not pos:
            return w
        i = rng.choice(pos)
        return w[:i] + w[i + 1:]

    def swap(w: str) -> str:
        i = rng.randrange(1, len(w) - 2)
        return w[:i] + w[i + 1] + w[i] + w[i + 2:]

    v1 = list(words)
    for i in rng.sample(long_idx, min(2, len(long_idx))):
        v1[i] = drop_vowel(v1[i])
    v2 = list(words)
    picks = rng.sample(long_idx, min(2, len(long_idx)))
    v2[picks[0]] = swap(v2[picks[0]])
    if len(picks) > 1:
        w = v2[picks[1]]
        v2[picks[1]] = w[:-1] if len(w) > 5 else w
    if len(v2) > 2 and v2[0] in ("apa", "apakah", "berapa", "bagaimana", "siapa"):
        v2[0] = {"apa": "ap", "apakah": "apakh", "berapa": "brp", "bagaimana": "gmn", "siapa": "siapa"}[v2[0]]
    return [" ".join(v1), " ".join(v2)]


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
        for t, v in zip(missing, model.embed(missing, "RETRIEVAL_QUERY")):
            cache[key(t)] = v
        CACHE_FILE.parent.mkdir(parents=True, exist_ok=True)
        CACHE_FILE.write_text(json.dumps(cache))
    return {t: cache[key(t)] for t in texts}


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

    rng = random.Random(20261104)
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
    good = [r for r in rows if r["typos_grouped"] >= 0.95]
    suggestion = max(good, key=lambda r: r["threshold"])["threshold"] if good else None
    print(f"\nSuggested SIMILAR_QUESTION_THRESHOLD: {suggestion if suggestion else 'none groups 95% of typos'}")

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
            "thresholds": rows, "suggested": suggestion,
            "lowest_typo_pairs": [{"q": q, "variant": v, "sim": round(s, 4)} for q, v, s in sorted(same, key=lambda x: x[2])[:20]],
            "highest_different_pairs": [{"a": a, "b": b, "sim": round(s, 4)} for a, b, s in sorted(diff, key=lambda x: -x[2])[:20]],
        }, ensure_ascii=False, indent=1))
        print(f"\nSaved {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
