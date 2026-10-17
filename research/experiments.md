# Experiment log — PMPSTI RAG

Running record of every evaluation, kept so results can be reproduced and cited in a paper.
Add one entry per experiment: date, code version, setup, numbers, finding.

Raw results (per-question ranks, answers, settings) are in `scripts/eval/results/*.json`.
Corpus statistics for the 2026-10-17 state are in `data/corpus_2026-10-17.json`.

---

## System under test

| Component | Setting |
|---|---|
| Backend | Rust (Actix-web 4, SQLx 0.7) |
| Database | PostgreSQL 18.6 on Neon, pgvector 0.8.6 (HNSW, cosine), GIN full-text index (`indonesian` config) |
| Embedding | Gemini `gemini-embedding-001`, 768 dims; `RETRIEVAL_DOCUMENT` for chunks, `RETRIEVAL_QUERY` for questions |
| Generator (LLM) | Gemini `gemini-3.5-flash-lite` |
| Search modes | semantic (cosine), keyword (`ts_rank`), hybrid (0.7 · cosine + 0.3 · ts_rank), RRF (k = 60) |
| Corpus (2026-10-17) | 343 chunks, 25 documents (21 guidebook pages, 3 web pages, 1 text); median chunk 800 chars; fingerprint `cac776475ebf…` |

## Test sets

| Set | File | Size | Measures |
|---|---|---|---|
| Document set | `scripts/eval/questions.json` | 44 questions (42 scored; 2 sources not ingested) | Is the right document in the top-k? Half from real query logs, half drafted to cover every guidebook section |
| Answer set | `eval/pertanyaan_uji.csv` | 30 questions with reference answers and keywords | Retrieval: are the keywords in the top 5 chunks? (21 scored). End-to-end: are they in the generated answer? (23 scored, incl. 2 out-of-scope) |

Metrics: document hit@1, hit@5, MRR over the top 10; **ans@5** = all keyword groups appear in the top 5 chunks;
**end-to-end (e2e)** = all keyword groups appear in the answer from `/api/ask_public`
(for out-of-scope questions: the answer must say the information was not found).

---

## E0 — Retrieval baseline · 2026-10-17 · code `bb2e651`

`scripts/eval/results/20261017-2144_baseline.json`

| Mode | hit@1 | hit@5 | MRR | ans@5 |
|---|---|---|---|---|
| semantic | 0.881 | 0.976 | 0.921 | 0.905 |
| keyword (`plainto_tsquery`, AND) | 0.048 | 0.071 | 0.060 | 0.000 |
| **hybrid** (signed-in chat) | **0.905** | **0.976** | **0.933** | **0.952** |
| rrf (guest chat, backend default) | 0.738 | 0.929 | 0.833 | 0.857 |

Finding: dense retrieval is strong; keyword search almost never matches because `plainto_tsquery`
requires every word (including question words such as *bagaimana*, *berapa*), and the Indonesian
stemmer over-stems (*kerjasama* → `rjasama`, *berapa* → `apa`). Guests and signed-in users used
different modes.

## E1 — Keyword and fusion variants · 2026-10-17

`20261017-2144_baseline.json` (v2 rows), `20261017-2148_variants.json`

| Mode | hit@1 | hit@5 | MRR | ans@5 |
|---|---|---|---|---|
| keyword_v2 (OR, question words removed, `indonesian`) | 0.595 | 0.833 | 0.690 | 0.619 |
| keyword_v2_simple (same, `simple` config) | 0.571 | 0.833 | 0.671 | 0.619 |
| rrf_v2 (semantic + keyword_v2, equal weight) | 0.833 | 0.881 | 0.862 | 0.714 |
| rrf_w03 (keyword weight 0.3) | 0.833 | 0.929 | 0.868 | 0.857 |
| hybrid_v2 (hybrid with keyword_v2) | 0.881 | 0.976 | 0.921 | 0.905 |
| hybrid (unchanged) | 0.905 | 0.976 | 0.933 | 0.952 |

Finding: fixing keyword search on its own helps a lot (hit@5 0.071 → 0.833), but fusing it with
equal weight **hurts** (rrf_v2 ans@5 0.714 vs 0.857) because the keyword list is much weaker than the
dense list on this corpus. The original hybrid (keyword as a small tie-breaker) stays best.
Decision: hybrid as the default everywhere; keyword_v2 only for `search_mode: "keyword"` and as the
fallback when embedding fails.

## E2 — RRF instability from score ties · 2026-10-17

Same RRF SQL scored hit@1 **0.738** (E0) and **0.881** (`20261017-2152_p1.json`) with no code change.
Cause: most chunks have `ts_rank = 0`, so the keyword list is ordered by arbitrary tie order, which
changed with the selected columns (query plan). Three repeated runs at the second setting were stable
(0.881). Finding: the RRF results served to guests depended on non-deterministic tie order — a further
reason to drop RRF as the default.

## E3 — End-to-end before the fix · code `bb2e651` (= production until 2026-10-17)

`20261017-2217_e2e-main.json` — backend built from `main`, same database.

**Result: 4 / 23 correct (17 %)**, of which 2 were the out-of-scope questions (the system answered
"not available" to nearly everything).

Root cause (introduced in `f4b7d94`, 2026-10-07): context was expanded **before** filtering.
Every chunk of each matched document was re-fetched with score 1.0 in document order, de-duplication
kept those copies instead of the ranked hits, and the 5/6-entry cap then passed **chunks 0–4 of the
first document** to the LLM. Retrieval metrics (E0) did not reveal this: the right chunk was ranked
first, but never reached the generator.

An earlier run of this experiment was discarded: 7/23 requests failed with Gemini HTTP 429
(per-minute quota). The e2e mode now pauses 8 s between questions and retries on 429.

## E4 — End-to-end after the fix · code `63bc4fb` (merged as `1eab56e`)

`20261017-2223_e2e-p1.json` (run 1), `20261017-2321_my-test.json` (run 2, by the author)

Change: select-then-expand (`RagEngine::build_context`): filtered hits are kept, then ±1 neighbour
chunks (or more of the document for list/procedural questions) are added within a character budget,
merged into one context per document (= one source card = one citation number); hybrid default;
single score threshold 0.35.

| Run | Correct | Failed |
|---|---|---|
| E3 before (`bb2e651`) | 4 / 23 (17 %) | 19 |
| E4 run 1 | **20 / 23 (87 %)** | Q01, Q20, Q21 |
| E4 run 2 | **21 / 23 (91 %)** | Q20, Q21 |

Paired comparison with E3 (exact McNemar test): run 1 — 17 fail→pass, 1 pass→fail, p = 0.00015;
run 2 — 17 fail→pass, 0 pass→fail, p = 0.000015.

Remaining failures:
- **Q01** (expansion of "PMPSTI"): the stored documents contain a wrong expansion injected at ingest
  by `scripts/text_enrichment.py`; whether the LLM repeats it varies between runs (LLM non-determinism).
- **Q20, Q21**: answers are correct on manual reading; the expected keywords ("Kaprodi",
  "kedua pembimbing") do not match the document wording ("Ketua Program Studi") → scoring false negatives.

---

## Threats to validity (to address before publication)

- **Small test sets** (23 e2e / 42 document questions), written by the system's author; no
  inter-annotator agreement yet.
- **Keyword-based answer scoring** produces false negatives (Q20, Q21) and could produce false
  positives; add human judgement (correct / partial / wrong / hallucinated) or an LLM-as-judge with
  human spot checks.
- **LLM non-determinism**: report the mean and spread over ≥ 3 runs per configuration (Q01 flipped
  between runs).
- **Single corpus and language** (Indonesian academic guidebook); 16 % of chunks were JavaScript junk
  from early web ingestion (fixed in later steps) — report data cleaning separately from retrieval changes.
- **Same database as production**: `test-embedding` was a copy of production; freeze it as a Neon
  snapshot branch so the numbers can be reproduced.

## How to reproduce

```bash
# Retrieval metrics (read-only, uses the DB in DATABASE_URL)
cd scripts && python eval_retrieval.py --label <name> --verbose

# End-to-end: start the backend at the commit under test, then
cargo run --release            # in the repo root, separate terminal
cd scripts && python eval_retrieval.py --label <name> --backend http://localhost:8080
```

Each results JSON records the Git commit, models, settings, corpus fingerprint and test-set hashes
(from 2026-10-17 onward; earlier files: see the commit noted in each entry above).

## Data and privacy

Stored here: aggregate statistics and evaluation results only. Not stored: chunk text, raw query logs,
user accounts. The staff table (names, NIP, e-mail) is public on the source website; it remains in the
database by decision of the project owner but is not copied into this folder.
