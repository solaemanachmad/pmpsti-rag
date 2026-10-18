from worker.chunker import MAX_CHARS, chunk_pages, clean_page_markdown, heading_level


def test_headings_carry_over_pages_and_tables_keep_header():
    rows = "\n".join(f"|{i}|Nama Dosen {i}|Lektor|Teknologi Informasi|" for i in range(1, 60))
    pages = [
        (11, "## **II. Struktur Organisasi**\n\n## **B. Staf Pengajar**\n\nDosen tetap adalah:\n\n"
             "|No|Nama|Jabatan|Bidang|\n|---|---|---|---|\n" + rows + "\n\nTabel 2.1. Daftar Dosen\n"),
        (12, "Lanjutan paragraf di halaman berikutnya yang cukup panjang untuk menjadi chunk sendiri."),
    ]
    chunks = chunk_pages(pages)
    tables = [c for c in chunks if c.kind == "table"]
    assert len(tables) > 1                                     # long table split into row groups
    assert all(t.content.splitlines()[1].startswith("|No|") for t in tables)   # caption, then header
    assert all(t.content.startswith("Tabel 2.1") for t in tables)
    assert all(len(t.content) <= MAX_CHARS + 120 for t in tables)
    assert not any(c.kind == "text" and c.content.startswith("Tabel 2.1") for c in chunks)
    last = chunks[-1]
    assert last.page == 12 and last.heading_path == ["II. Struktur Organisasi", "B. Staf Pengajar"]
    assert last.title("Panduan") == "Panduan — II. Struktur Organisasi > B. Staf Pengajar"


def test_numbered_bold_list_item_is_not_a_heading():
    md = ("**Ketentuan Cuti Kuliah:**\n\n**1. Minimal diajukan pada awal semester 3**\n\n"
          "**2. Tidak diperkenankan cuti pada periode perpanjangan masa studi.**\n\n"
          "**1. Pengertian Tesis**\n\nTesis adalah karya tulis dari hasil studi sistematis atas masalah.")
    chunks = chunk_pages([(20, md)])
    paths = {tuple(c.heading_path) for c in chunks}
    assert ("Ketentuan Cuti Kuliah",) in paths or any("Ketentuan Cuti Kuliah" in p for p in paths)
    assert not any("Minimal diajukan" in " ".join(p) for p in paths)
    assert any(p[-1] == "1. Pengertian Tesis" for p in paths if p)


def test_heading_levels():
    assert heading_level("VII. Kurikulum", 2) == 1
    assert heading_level("A. Visi Program Studi", 2) == 2
    assert heading_level("1. Pengertian Tesis", 0) == 3
    assert heading_level("Cuti Kuliah", 0) == 4


def test_clean_page_markdown():
    md = "Masa studi 1<sup>1</sup> /2 kali.\n\n20\n"
    assert clean_page_markdown(md, 20).strip() == "Masa studi 1 1/2 kali."
