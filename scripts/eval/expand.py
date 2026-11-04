"""
expand.py — abbreviation expansion of a question before the search, the SAME rule as
expand_query() in src/abbrev.rs (Admin → Abbreviations, Settings → "Expand abbreviations").
Keep both in step; scripts/eval/test_expand_query.py repeats the Rust test cases.

  "kapan sempro?" → "kapan sempro (seminar proposal)?"
"""


def expand_query(text: str, entries: list[tuple[str, str]]) -> str:
    """The admin's short forms only, any case, whole word, first use; built-in acronyms are not
    expanded (documents already carry them, and expanding them lowered retrieval, E11)."""
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
            for short, meaning in entries:
                if short.lower() == key:
                    if meaning.lower() not in lower_text:
                        out.append(f" ({meaning})")
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
