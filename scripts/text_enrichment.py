"""
text_enrichment.py  —  Pre-chunking text enrichment untuk RAG
=============================================================
Teknik yang diimplementasi:
  1. Acronym Expansion    — perluas singkatan akademik Indonesia
  2. Context Breadcrumb   — tambah konteks hierarki ke tiap chunk
  3. Sentence Repair      — perbaiki kalimat terpotong (PDF artifact)
  4. Numeric Normalization — "3 SKS" → "3 Satuan Kredit Semester (SKS)"

Cara pakai di ingest.py:
  from text_enrichment import enrich_text, enrich_chunk
"""

import json
import re
from pathlib import Path
from typing import Optional

# ──────────────────────────────────────────────────────────────────
#  KAMUS AKRONIM
#  Tambah/edit sesuai dokumen spesifik kamu
# ──────────────────────────────────────────────────────────────────

# Shared with the Rust URL ingest (src/enrich.rs), so both expand the same words.
# Edit src/acronyms.json, not this file. Keys starting with "_" are comments.
_ACRONYMS_FILE = Path(__file__).resolve().parent.parent / "src" / "acronyms.json"
ACRONYMS: dict[str, str] = {
    k: v for k, v in json.loads(_ACRONYMS_FILE.read_text(encoding="utf-8")).items()
    if not k.startswith("_")
}

# Regex per acronym, standing alone (word boundaries). Built once at import.
_ACRONYM_PATTERNS: list[tuple[re.Pattern, str, str]] = []

def _build_patterns():
    for acr, expansion in ACRONYMS.items():
        # Match only acronyms that are NOT already explained next to them:
        # "3 SKS" → "3 Satuan Kredit Semester (SKS)", but "SKS (Satuan …)" and
        # "… Teknologi Informasi (PMPSTI)" stay. Same rules as src/enrich.rs.
        # Case-SENSITIVE on purpose: acronyms are written as in ACRONYMS ("SKS", "IoT").
        # Ignoring case turned ordinary words into acronyms ("api" = fire → "API").
        pat = re.compile(
            r'(?<!\()'          # not directly after ( (already explained)
            r'\b' + re.escape(acr) + r'\b'
            r'(?!\s*[\(\[])'    # not followed by ( or [ (already explained)
        )
        _ACRONYM_PATTERNS.append((pat, acr, expansion))

_build_patterns()


# ──────────────────────────────────────────────────────────────────
#  1. ACRONYM EXPANSION
# ──────────────────────────────────────────────────────────────────

def expand_acronyms(text: str, max_expansions_per_acr: int = 2) -> str:
    """
    Perluas akronim di teks.
    Setiap akronim hanya di-expand max N kali (tidak semua kemunculan)
    agar teks tidak terlalu verbose.

    "Mahasiswa wajib mengambil minimal 36 SKS."
    → "Mahasiswa wajib mengambil minimal 36 Satuan Kredit Semester (SKS)."
    """
    counts: dict[str, int] = {}

    def replace_fn(match: re.Match, acr: str, expansion: str) -> str:
        n = counts.get(acr, 0)
        if n >= max_expansions_per_acr:
            return match.group(0)  # Biarkan apa adanya
        counts[acr] = n + 1
        # Matching is case-sensitive, so the match is the acronym exactly as listed;
        # use the expansion as written (str.capitalize() would turn "Internet of Things"
        # into "Internet of things").
        return f"{expansion} ({match.group(0)})"

    for pat, acr, expansion in _ACRONYM_PATTERNS:
        text = pat.sub(
            lambda m, a=acr, e=expansion: replace_fn(m, a, e),
            text
        )
        counts.pop(acr, None)  # Reset per-akronim count tiap paragraf

    return text


# ──────────────────────────────────────────────────────────────────
#  2. CONTEXT BREADCRUMB INJECTION
# ──────────────────────────────────────────────────────────────────

def inject_breadcrumb(
    chunk_text: str,
    metadata: dict,
    doc_title: str = "",
) -> str:
    """
    Tambah breadcrumb hierarki di awal chunk sebelum embedding.

    Input chunk:  "SKS: 3, Jenis: Wajib."
    Output:       "[Kurikulum S1 > Semester 1 > Daftar Mata Kuliah]
                  SKS: 3, Jenis: Wajib."

    Ini membuat embedding-nya jauh lebih kaya konteks.
    """
    parts = []
    if doc_title:
        parts.append(doc_title)

    for key in ["Chapter", "Subchapter", "Section"]:
        val = metadata.get(key, "")
        if val and str(val).strip():
            parts.append(str(val).strip())

    if not parts:
        return chunk_text

    breadcrumb = " > ".join(parts)
    return f"[{breadcrumb}]\n\n{chunk_text}"


# ──────────────────────────────────────────────────────────────────
#  3. SENTENCE REPAIR  (PDF artifact)
# ──────────────────────────────────────────────────────────────────

def repair_sentences(text: str) -> str:
    """
    Perbaiki artifact umum dari ekstraksi PDF:

    - "kata- \nkata" → "kata-kata"   (hyphen line break)
    - "kali-\nmat"   → "kalimat"     (hyphen tanpa spasi)
    - Baris pendek (<40 char) yang bukan heading disambung ke baris berikut
    """
    # Hyphen di akhir baris → gabung kata
    text = re.sub(r'(\w)-\s*\n\s*(\w)', r'\1\2', text)

    # Baris yang diakhiri huruf kecil + newline + baris berikut huruf kecil
    # (kemungkinan baris terpotong di tengah kalimat)
    text = re.sub(r'([a-zàáâãäå])\n([a-zàáâãäå])', r'\1 \2', text)

    return text


# ──────────────────────────────────────────────────────────────────
#  4. NUMERIC NORMALIZATION
# ──────────────────────────────────────────────────────────────────

def normalize_numerics(text: str) -> str:
    """
    Normalisasi pola numerik yang sering muncul di dokumen akademik.

    "3 SKS"   → "3 Satuan Kredit Semester (SKS)"
    "IPK 3.5" → "Indeks Prestasi Kumulatif (IPK) 3.5"
    "Sem. 1"  → "Semester 1"
    """
    # "Sem." → "Semester"
    text = re.sub(r'\bSem\.\s*(\d)', r'Semester \1', text)

    # "Thn." → "Tahun"
    text = re.sub(r'\bThn\.\s*(\d)', r'Tahun \1', text)

    # "Mhs." → "Mahasiswa"
    text = re.sub(r'\bMhs\.', 'Mahasiswa', text)

    # "Drs." / "Dr." → tetap (nama gelar, jangan diubah)
    # "S.Kom" / "M.Kom" → tetap

    return text


# ──────────────────────────────────────────────────────────────────
#  PIPELINE UTAMA
# ──────────────────────────────────────────────────────────────────

def enrich_text(text: str) -> str:
    """
    Jalankan semua enrichment SEBELUM chunking.
    Urutan penting: repair dulu → normalize → expand acronym.
    """
    text = repair_sentences(text)
    text = normalize_numerics(text)
    text = expand_acronyms(text)
    return text


def enrich_chunk(
    chunk_text: str,
    metadata: dict,
    doc_title: str = "",
    for_embedding: bool = True,
) -> str:
    """
    Enrichment SETELAH chunking, tepat sebelum embedding.

    for_embedding=True  → tambah breadcrumb (konteks judul/bab) untuk embedding
    for_embedding=False → kembalikan teks bersih untuk disimpan ke DB

    Catatan: prefix "passage: " (khusus model E5) sudah tidak dipakai karena
    embedding sekarang memakai Gemini dengan taskType RETRIEVAL_DOCUMENT.
    """
    if for_embedding:
        return inject_breadcrumb(chunk_text, metadata, doc_title)
    return chunk_text


# ──────────────────────────────────────────────────────────────────
#  SELF-TEST
# ──────────────────────────────────────────────────────────────────

if __name__ == "__main__":
    test_cases = [
        "Mahasiswa wajib menempuh minimal 36 SKS dengan IPK minimal 3.0.",
        "Beban EWMP dosen tetap PMPSTI adalah 12 SKS per semester.",
        "Pengajuan KRS dilakukan melalui sistem akademik UGM.",
        "Tesis mahasiswa MTI harus memenuhi standar KKNI level 8.",
        "Total SKS semester ini adalah 18 SKS dengan IPS 3.75.",
    ]

    print("=" * 60)
    print("  TEXT ENRICHMENT — Self Test")
    print("=" * 60)

    for t in test_cases:
        enriched = enrich_text(t)
        print(f"\nASLI:   {t}")
        print(f"HASIL:  {enriched}")

    print("\n" + "=" * 60)
    print("  BREADCRUMB TEST")
    print("=" * 60)
    chunk = "Kode: MKU101, Mata Kuliah: Kalkulus I, SKS: 3, Jenis: Wajib."
    meta  = {"Chapter": "Kurikulum S1", "Subchapter": "Semester 1"}
    print(f"\nChunk:  {chunk}")
    print(f"Embed:  {enrich_chunk(chunk, meta, 'panduan_pmpsti')}")