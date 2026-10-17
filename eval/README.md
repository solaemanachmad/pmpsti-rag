# Set Pertanyaan Uji (Evaluasi RAG)

`pertanyaan_uji.csv` berisi 30 pertanyaan beserta jawaban acuan untuk mengukur kualitas
jawaban PMPSTI RAG. Jalankan ulang setiap kali pipeline ingest, chunking, embedding,
pencarian, prompt, atau model diubah, supaya terlihat apakah hasilnya membaik atau memburuk.

## Kolom

| Kolom | Isi |
|---|---|
| `id` | Q01–Q30 |
| `jenis` | fakta, daftar, hitung, prosedur, perbandingan, konflik_versi, di_luar_cakupan, multi_turn |
| `pertanyaan` | Pertanyaan seperti yang akan diketik mahasiswa |
| `jawaban_acuan` | Jawaban yang benar menurut sumber |
| `kata_kunci` | Untuk cek otomatis. `\|` = semua kelompok harus muncul; `/` = cukup salah satu. Pencocokan tidak membedakan huruf besar/kecil |
| `sumber` | Dokumen dan bagian/halaman asal jawaban |
| `status_sumber` | `ada` = sumber sudah ada di DB (situs panduan); `cek` = hanya ada di PDF 2022, pastikan PDF sudah di-ingest; `belum` = sumber belum di-ingest; `-` = memang tidak ada di sumber |
| `catatan` | Konflik antar-versi, data tidak konsisten, atau bug yang diuji |

## Cara menilai (sementara, manual)

Untuk tiap pertanyaan, catat:
1. **Retrieval**: apakah chunk yang memuat jawaban masuk 5 besar sumber yang ditampilkan?
2. **Jawaban**: Benar / Sebagian / Salah / Mengarang (halusinasi).

## Prinsip sumber

- Situs panduan (Kurikulum 2022 **Revisi-2**, Nov 2025) lebih baru daripada
  Buku Panduan Akademik MTI 2022 (PDF). Jika berbeda, jawaban acuan memakai situs.
- Pertanyaan `di_luar_cakupan` harus dijawab "tidak ditemukan", bukan dikarang.

Disusun 17 Oktober 2026. Data dosen diambil dari jteti.ugm.ac.id/tenaga-pendidik pada tanggal yang sama.
