"""
expand.py — abbreviation expansion of a question before the search, the SAME rule as
expand_query() in src/abbrev.rs (Admin → Abbreviations, Settings → "Expand abbreviations").
Keep both in step; scripts/eval/test_expand_query.py repeats the Rust test cases.

  "kapan sempro?" → "kapan sempro (seminar proposal)?"
"""

import json
from pathlib import Path

ACRONYMS = Path(__file__).resolve().parents[2] / "src" / "acronyms.json"


def builtin() -> list[tuple[str, str]]:
    data = json.loads(ACRONYMS.read_text(encoding="utf-8"))
    return [(k, v) for k, v in data.items() if not k.startswith("_") and isinstance(v, str)]


def dictionary(custom: list[tuple[str, str]]) -> list[tuple[str, str, bool]]:
    out = [(s, e, True) for s, e in custom]
    taken = {s.lower() for s, _ in custom}
    out += [(s, e, False) for s, e in builtin() if s.lower() not in taken]
    return out


def expand_query(text: str, custom: list[tuple[str, str]]) -> str:
    entries = dictionary(custom)
    lower_text = text.lower()
    done: set[str] = set()
    out: list[str] = []
    word: list[str] = []

    def flush() -> None:
        if not word:
            return
        w = "".join(word)
        out.append(w)
        key = w.lower()
        if key not in done:
            for short, expansion, is_custom in entries:
                any_case = is_custom or any(c.islower() for c in short) or len(short) >= 3
                if (short.lower() == key) if any_case else (short == w):
                    if expansion.lower() not in lower_text:
                        out.append(f" ({expansion})")
                    done.add(key)
                    break
        word.clear()

    for c in text:
        if c.isalnum():
            word.append(c)
        else:
            flush()
            out.append(c)
    flush()
    return "".join(out)


def load_custom(cur) -> list[tuple[str, str]]:
    """Active admin entries from a database that has the table (else none)."""
    cur.execute("SELECT to_regclass('public.abbreviations')")
    if cur.fetchone()[0] is None:
        return []
    cur.execute("SELECT short, expansion FROM abbreviations WHERE status = 'active' ORDER BY short")
    return [(s, e) for s, e in cur.fetchall()]
