# Test questions (RAG evaluation)

`pertanyaan_uji.csv` holds 30 questions with reference answers for measuring the answer quality
of PMPSTI RAG. Re-run it whenever the ingest pipeline, chunking, embedding, search, prompt or
model changes, to see whether results got better or worse.

The questions and answers stay in Indonesian on purpose — that is what students type.

## Columns

| Column | Content |
|---|---|
| `id` | Q01–Q30 |
| `jenis` | Question type: fakta (fact), daftar (list), hitung (count), prosedur (procedure), perbandingan (comparison), konflik_versi (version conflict), di_luar_cakupan (out of scope), multi_turn |
| `pertanyaan` | The question as a student would type it |
| `jawaban_acuan` | The correct answer according to the source |
| `kata_kunci` | For automatic checks. `\|` = every group must appear; `/` = any one alternative is enough. Case-insensitive |
| `sumber` | Source document and section/page of the answer |
| `status_sumber` | `ada` = source is in the database (guide site); `cek` = only in the 2022 PDF, make sure it is ingested; `belum` = source not ingested yet; `-` = not in any source |
| `catatan` | Notes: conflicts between versions, inconsistent data, or the bug being tested |

## How to score

Automatically (from `scripts/`):

```bash
python eval_retrieval.py                                   # retrieval: are the answers in the top 5 chunks?
python eval_retrieval.py --backend http://localhost:8080   # end-to-end: is the answer itself correct?
```

`eval_retrieval.py` scores the `ada` questions (plus `-` for the end-to-end check, which must be
answered "not found") and skips `cek`, `belum` and `multi_turn`.

Manually, per question:
1. **Retrieval**: is the chunk containing the answer among the top 5 sources shown?
2. **Answer**: correct / partial / wrong / made up (hallucination).

## Source priority

- The guide site (Curriculum 2022 **Revision 2**, Nov 2025) is newer than the
  MTI Academic Guidebook 2022 (PDF). When they differ, the reference answer follows the site.
- `di_luar_cakupan` questions must be answered "not found", never invented.

Compiled on 17 October 2026. Lecturer data taken from jteti.ugm.ac.id/tenaga-pendidik on the same day.
