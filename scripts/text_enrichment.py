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

import re
from typing import Optional

# ──────────────────────────────────────────────────────────────────
#  KAMUS AKRONIM
#  Tambah/edit sesuai dokumen spesifik kamu
# ──────────────────────────────────────────────────────────────────

ACRONYMS: dict[str, str] = {
    # Akademik umum
    "SKS":    "Satuan Kredit Semester",
    "KRS":    "Kartu Rencana Studi",
    "KHS":    "Kartu Hasil Studi",
    "IPK":    "Indeks Prestasi Kumulatif",
    "IP":     "Indeks Prestasi",
    "IPS":    "Indeks Prestasi Semester",
    "TA":     "Tugas Akhir",
    "KKN":    "Kuliah Kerja Nyata",
    "PKL":    "Praktik Kerja Lapangan",
    "DO":     "Drop Out",
    "TK":     "Tidak Kehadiran",

    # Program studi / institusi
    "PMPSTI": "Program Magister Pengelolaan Sistem Teknik Informasi",
    "MTI":    "Magister Teknologi Informasi",
    "DTETI":  "Departemen Teknik Elektro dan Teknologi Informasi",
    "FT":     "Fakultas Teknik",
    "UGM":    "Universitas Gadjah Mada",
    "FMIPA":  "Fakultas Matematika dan Ilmu Pengetahuan Alam",
    "FK":     "Fakultas Kedokteran",

    # Jabatan / SDM
    "EWMP":   "Ekuivalensi Waktu Mendidik Penuh",
    "BKD":    "Beban Kerja Dosen",
    "KBK":    "Kelompok Bidang Keahlian",
    "NIDN":   "Nomor Induk Dosen Nasional",
    "NIP":    "Nomor Induk Pegawai",
    "NIM":    "Nomor Induk Mahasiswa",
    "tendik": "Tenaga Kependidikan",

    # Administrasi
    "SK":     "Surat Keputusan",
    "MOU":    "Memorandum of Understanding",
    "PKS":    "Perjanjian Kerja Sama",
    "RKAT":   "Rencana Kegiatan dan Anggaran Tahunan",
    "SPP":    "Sumbangan Pembinaan Pendidikan",
    "UKT":    "Uang Kuliah Tunggal",
    "BPP":    "Biaya Penyelenggaraan Pendidikan",

    # Dokumen akademik
    "RPS":    "Rencana Pembelajaran Semester",
    "SAP":    "Satuan Acara Perkuliahan",
    "GBPP":   "Garis-garis Besar Program Pengajaran",
    "KKNI":   "Kerangka Kualifikasi Nasional Indonesia",
    "CPL":    "Capaian Pembelajaran Lulusan",
    "CPMK":   "Capaian Pembelajaran Mata Kuliah",
    "OBE":    "Outcome Based Education",

    # Proses tesis/penelitian
    "KTI":    "Karya Tulis Ilmiah",
    "BAB":    "Bagian",
    "DOI":    "Digital Object Identifier",
    "ISSN":   "International Standard Serial Number",
    "ISBN":   "International Standard Book Number",

    # Teknologi informasi
    "TI":     "Teknologi Informasi",
    "SI":     "Sistem Informasi",
    "AI":     "Kecerdasan Buatan",
    "ML":     "Machine Learning",
    "DL":     "Deep Learning",
    "NLP":    "Natural Language Processing",
    "DB":     "Database",
    "API":    "Application Programming Interface",
    "IoT":    "Internet of Things",
}

# Regex: akronim yang berdiri sendiri (dibatasi word boundary)
# Bangun satu kali saat import
_ACRONYM_PATTERNS: list[tuple[re.Pattern, str]] = []

def _build_patterns():
    for acr, expansion in ACRONYMS.items():
        # Match akronim yang BELUM punya penjelasan di sebelahnya
        # Contoh: "3 SKS" → "3 Satuan Kredit Semester (SKS)"
        # Tapi "SKS (Satuan Kredit Semester)" tidak diubah
        pat = re.compile(
            r'\b' + re.escape(acr) + r'\b'
            r'(?!\s*[\(\[])',   # tidak diikuti ( atau [ (sudah ada penjelasan)
            re.IGNORECASE
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
        original = match.group(0)
        # Pertahankan kapitalisasi asli
        if original.isupper():
            return f"{expansion} ({original})"
        elif original[0].isupper():
            return f"{expansion.capitalize()} ({original})"
        else:
            return f"{expansion.lower()} ({original})"

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