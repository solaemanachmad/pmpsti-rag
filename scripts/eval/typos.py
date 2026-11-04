"""
typos.py — deterministic misspellings of test questions, the way people type fast on a phone:
lower case, no final "?", a dropped vowel or letter, two neighbouring letters swapped, short
question words ("brp", "gmn"). Used by question_similarity.py and eval_retrieval.py --typos.
"""

import random
import re

VOWELS = "aiueo"
SEED = 20261104


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
