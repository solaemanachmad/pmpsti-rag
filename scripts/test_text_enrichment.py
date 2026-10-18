"""
Checks for text_enrichment.expand_acronyms. Run from scripts/:

    python test_text_enrichment.py

(Also works with pytest.) Each case guards a bug found in the 2026-10-18 ingest review.
"""

from text_enrichment import expand_acronyms


def test_pmpsti_uses_official_name():
    # Was "Program Magister Pengelolaan Sistem Teknik Informasi" (wrong) — eval question Q01
    out = expand_acronyms("PMPSTI adalah program magister.")
    assert "Program Magister Program Studi Teknologi Informasi (PMPSTI)" in out, out


def test_regular_acronym_is_expanded():
    assert "36 Satuan Kredit Semester (SKS)" in expand_acronyms("Mahasiswa wajib menempuh 36 SKS.")


def test_lowercase_words_are_not_acronyms():
    # Case-insensitive matching used to turn "api" (fire) into "API", "si" into "SI"
    text = "Jangan bermain api di si laboratorium."
    assert expand_acronyms(text) == text


def test_removed_ambiguous_acronyms_stay():
    # BAB means chapter (not "Bagian"); TK usually means Taman Kanak-kanak
    text = "Lihat BAB 3 tentang TK."
    assert expand_acronyms(text) == text


def test_mixed_case_expansion_kept_as_written():
    # str.capitalize() used to produce "Internet of things"
    assert "Internet of Things (IoT)" in expand_acronyms("Sistem IoT kampus.")



def test_already_explained_acronym_is_not_expanded_again():
    # "(PMPSTI)" right after the full name became "(Program Magister … (PMPSTI))"
    text = "Kurikulum Program Magister Program Studi Teknologi Informasi (PMPSTI) 2022"
    assert expand_acronyms(text) == text
    text = "Minimal 36 SKS (Satuan Kredit Semester)."
    assert expand_acronyms(text) == text


if __name__ == "__main__":
    tests = [f for name, f in sorted(globals().items()) if name.startswith("test_")]
    for t in tests:
        t()
        print(f"ok  {t.__name__}")
    print(f"{len(tests)} passed")
