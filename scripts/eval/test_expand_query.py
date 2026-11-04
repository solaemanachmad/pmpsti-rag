"""Same cases as the tests of expand_query() in src/abbrev.rs — the two must agree.

    cd scripts && python -m pytest -q eval/test_expand_query.py
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from expand import expand_query  # noqa: E402

CUSTOM = [("sempro", "seminar proposal"), ("ta", "tugas akhir")]


def test_expands_known_short_forms_once():
    assert expand_query("kapan sempro? sempro lagi", CUSTOM) == "kapan sempro (seminar proposal)? sempro lagi"
    assert expand_query("Berapa sks tesis?", []) == "Berapa sks (Satuan Kredit Semester) tesis?"
    assert expand_query("jumlah tendik DTETI", []) == \
        "jumlah tendik (Tenaga Kependidikan) DTETI (Departemen Teknik Elektro dan Teknologi Informasi)"


def test_two_letter_capitals_only_as_written_unless_the_admin_added_them():
    assert expand_query("how do I apply", []) == "how do I apply"
    assert expand_query("syarat DO", []) == "syarat DO (Drop Out)"
    assert expand_query("syarat ta", CUSTOM) == "syarat ta (tugas akhir)"


def test_leaves_explained_and_unknown_words_alone():
    assert expand_query("seminar proposal (sempro) kapan", CUSTOM) == "seminar proposal (sempro) kapan"
    assert expand_query("halo", CUSTOM) == "halo"
    assert expand_query("", CUSTOM) == ""
