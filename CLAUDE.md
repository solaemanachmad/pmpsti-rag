# PMPSTI RAG — rules for Claude Code

RAG assistant for PMPSTI (Program Magister Program Studi Teknologi Informasi, DTETI FT UGM).
Students ask in Indonesian; answers must come only from the ingested documents and cite them.
This repository is **public**.

## Layout
- `src/` Rust backend (Actix-web 4, SQLx 0.7): `db.rs` schema + SQL, `search.rs` embeddings + search modes,
  `rag.rs` context building + LLM, `reader.rs` HTML/table reader, `enrich.rs` + `acronyms.json`, `handlers.rs` HTTP.
- `frontend/` SvelteKit 2 + **Svelte 4** + Tailwind 3 (do not use Svelte 5 runes). Calls the backend only through `/api` proxy.
- `src/sources.rs` + `src/sources_api.rs`: link sources (Drive / website PDF), jobs, draft review (approve/reject).
- `scripts/worker/`: PDF ingest worker (`python -m worker`), run by `.github/workflows/ingest-worker.yml`.
  Writes chunks only as `status = 'draft'`; search uses only `status = 'active'`.
- `scripts/` Python: `ingest.py` (PDF/DOCX, legacy manual path), `gemini_embed.py`, `eval_retrieval.py`, `smoke_check.py`.
- `eval/pertanyaan_uji.csv`, `scripts/eval/questions.json` test sets; `scripts/eval/results/` raw results (committed).
- `research/experiments.md` experiment log for a future paper. `docs/design/` design docs.

## Commands
- Rust: `cargo check`, `cargo test`, `cargo run` (needs root `.env`, `PORT=8080` locally).
- Frontend: `cd frontend && npm run check && npm run dev`.
- Python: `cd scripts && source .venv/bin/activate`; tests `python -m pytest -q worker/tests test_text_enrichment.py`.
- Eval (read-only DB): `python scripts/eval_retrieval.py --label <name>`; end-to-end adds `--backend http://localhost:8080`.

## Workflow
- Never commit to `main`. Branch `feat/…`, `fix/…`, `docs/…` → PR → Vercel Preview → merge.
- One PR = one concern. Follow the PR plan in `docs/design/ingest-from-links.md` §8.
- Before opening a PR: `cargo check` + `cargo test`, `npm run check` if frontend changed, and the eval if
  retrieval, chunking, prompts or data can change.

## Database safety
- **Never write to the production Neon branch** from scripts or tests. Use a Neon branch (`test-embedding` or a new one).
  Check the host printed by scripts before confirming writes.
- `eval-snapshot-2026-10-17` is frozen: read-only, never modify.
- Schema changes go in `Database::setup` as idempotent statements (`CREATE … IF NOT EXISTS`, `ADD COLUMN IF NOT EXISTS`).
  Existing rows must keep working (e.g. new `status` defaults to `'active'`).
- Every search SQL in `db.rs` is mirrored by an `m_*` function in `scripts/eval_retrieval.py`. Change both together.

## Quotas (Gemini free tier)
- ~500 generation calls/day; an end-to-end eval run of 26 questions costs ~80. Embeddings are cached in `eval/.cache/`.
- Prefer retrieval-only eval while iterating; run end-to-end once per finished change.

## Research log
- After every measured change add an entry to `research/experiments.md` (date, commit, change, numbers, finding)
  and commit the results JSON with it.
- Never commit raw query logs, chunk text, user data, `.env` or API keys.

## Domain notes
- Two guidebook versions disagree: PDF 2022 (40 SKS, tesis 8 SKS) vs website Kurikulum 2022 Revisi-2 (Nov 2025: 36 SKS, tesis 6 SKS).
  The website is newer.
- Do not rewrite stored document text with acronym expansions in a way that changes URLs, e-mails or names.
