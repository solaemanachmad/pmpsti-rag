"""Same cases as the tests of expand_query() in src/abbrev.rs — the two must agree.

    cd scripts && python -m pytest -q eval/test_expand_query.py
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from expand import expand_query  # noqa: E402

CUSTOM = [("sempro", "seminar proposal"), ("ta", "tugas akhir")]


def test_expands_the_admins_short_forms_once_in_any_case():
    assert expand_query("kapan sempro? sempro lagi", CUSTOM) == "kapan sempro (seminar proposal)? sempro lagi"
    assert expand_query("syarat TA", CUSTOM) == "syarat TA (tugas akhir)"
    assert expand_query("data", CUSTOM) == "data"


def test_built_in_acronyms_are_not_expanded():
    assert expand_query("Berapa SKS tesis PMPSTI?", []) == "Berapa SKS tesis PMPSTI?"
    assert expand_query("jumlah tendik", CUSTOM) == "jumlah tendik"


def test_leaves_explained_and_unknown_words_alone():
    assert expand_query("seminar proposal (sempro) kapan", CUSTOM) == "seminar proposal (sempro) kapan"
    assert expand_query("halo", CUSTOM) == "halo"
    assert expand_query("", CUSTOM) == ""
