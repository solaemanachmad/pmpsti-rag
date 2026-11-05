"""looks_cut_off() in eval_retrieval.py: e2e answers that stop mid-text are request errors.

    cd scripts && python -m pytest -q eval/test_cut_off.py
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from eval_retrieval import looks_cut_off  # noqa: E402


def test_answers_cut_off_in_the_e6_eval_are_flagged():
    # Real endings from the E6 e2e runs (2026-11-05), all HTTP 200
    for answer in ["**P", "Halo! Saya As", "Halo! Tentu, berikut adalah daftar",
                   "Pendaftaran ujian Pra dan Pendadaran online dibuka",
                   "Berdasarkan Buku Panduan Akademik PMP",
                   "masa studi diatur sebagai berikut:",
                   "| 3 | Suyanto | 1967071419990"]:
        assert looks_cut_off(answer), answer


def test_finished_answers_are_not_flagged():
    for answer in ["Tesis 6 SKS [1].", "Ada yang bisa saya bantu?", "Terima kasih!",
                   "| No | Nama |\n| 1 | Suyanto |", "Email: **juni@ugm.ac.id**",
                   "Lihat `pasca.jteti.ugm.ac.id`", "(lihat sumber [2])  \n", ""]:
        assert not looks_cut_off(answer), answer
