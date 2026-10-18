# Design: Ingest documents from links (Google Drive folder + official website)

Status: **implemented** in branch `feat/sources-pipeline` (PRs A–E combined) · 2026-10-18 · scope: PDF and Google Docs

## 1. Goal

Stop uploading PDFs into the repo or the server. Every document lives at a **link** that
students can open:

1. a **shared / institutional Google Drive folder** (files set to "Anyone with the link can view"), and
2. the **official department website** (direct PDF links, e.g. on jteti.ugm.ac.id).

The system fetches documents from those links, keeps them up to date, and **every answer cites
the document link and page number**.

Non-goals (later): knowledge graph, structured-data SQL answers, private (non-public) Drive files.

## 2. How it fits the current system

| Already in place (main / `feat/p2-*`) | This design adds |
|---|---|
| Rust URL ingest for HTML pages with `reader.rs`, table-aware chunking, preview with quality warnings (`/admin/documents/preview-url`) | PDF sources from Drive and website |
| `documents` table = one row per chunk (`document_id`, `source_url`, `page_number`, `file_hash`, …) | `sources` and `ingest_jobs` tables, chunk `status`, version fields |
| Python `scripts/ingest.py` (PyMuPDF / pymupdf4llm, OCR, table-to-prose) run by hand | Same code run as a **worker** (GitHub Actions or laptop) |
| Eval: `scripts/eval_retrieval.py`, `eval/pertanyaan_uji.csv`, `research/experiments.md` | Questions for figures and citation correctness |

### Who does what

| Part | Language / where | Why |
|---|---|---|
| Sources list, jobs, review, activate/archive, search, chat, citations | **Rust** backend (Render) | Already serves the admin UI; light and always on |
| Fetch PDF → text per page → tables/figures → chunks → embeddings → write **draft** chunks | **Python worker** (GitHub Actions, manual + scheduled; or laptop) | Best PDF libraries; deterministic text (important for reproducible research numbers); too heavy for Render free (512 MB) |
| HTML pages | **Rust** (unchanged) | Existing reader works and is measured (E5: 24/26) |

Alternative considered: Rust sends the PDF to Gemini and asks for a transcription. One pipeline
and an instant preview, but an LLM can silently change numbers or skip text in official rules,
results are not reproducible, and it spends generation quota on every page. Rejected for text;
Gemini is used **only to describe figures and complex tables** (§6).

## 3. Source types

| `kind` | Example | Fetch | Change detection |
|---|---|---|---|
| `drive_folder` | shared folder of guidebooks | Drive API v3 `files.list` with an API key (`q='<FOLDER_ID>' in parents`), then each file | `md5Checksum`, `modifiedTime` per file |
| `drive_file` | one shared PDF | `files/<ID>?alt=media&key=…` | `md5Checksum` |
| `web_pdf` | `https://jteti.ugm.ac.id/…/panduan.pdf` | HTTP GET (reject non-2xx, non-PDF content type) | `ETag` / `Last-Modified`, then SHA-256 of bytes |
| `web_page` | existing HTML ingest | Rust reader | content hash (existing) |

Drive notes:
- Public files can be listed and downloaded with an **API key** (no OAuth) from a Google Cloud project with the Drive API enabled. Store it as `GOOGLE_API_KEY` (Render + GitHub Actions secret).
- Google Docs/Sheets need `files/<ID>/export?mimeType=application/pdf` (phase 2).
- The **Drive viewer ignores `#page=N`**. A direct web PDF link usually honours it. Citations therefore always print "p. N" in text.

**Same document in both places** (same SHA-256): store one document; cite the **official website URL**
(`citation_url`), keep the Drive file as a mirror (`mirror_url`).

## 4. Data model

```sql
CREATE TABLE sources (
  id            BIGSERIAL PRIMARY KEY,
  kind          TEXT NOT NULL,          -- drive_folder | drive_file | web_pdf | web_page
  url           TEXT NOT NULL UNIQUE,   -- what the admin entered
  title         TEXT,
  category      TEXT DEFAULT '',
  version_label TEXT DEFAULT '',        -- e.g. "Kurikulum 2022 Revisi-2"
  valid_from    DATE,                   -- when this version applies
  supersedes_id BIGINT REFERENCES sources(id),  -- older version it replaces
  auto_check    BOOLEAN DEFAULT TRUE,   -- include in scheduled re-checks
  last_checked  TIMESTAMPTZ,
  created_by    BIGINT,                 -- users.id
  created_at    TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE source_files (             -- one row per actual file (a folder has many)
  id            BIGSERIAL PRIMARY KEY,
  source_id     BIGINT REFERENCES sources(id) ON DELETE CASCADE,
  external_id   TEXT,                   -- Drive file id, or URL
  name          TEXT,
  citation_url  TEXT NOT NULL,          -- link shown to students
  mirror_url    TEXT,
  sha256        TEXT,
  remote_rev    TEXT,                   -- md5Checksum / ETag / modifiedTime
  document_id   TEXT,                   -- links to documents.document_id
  status        TEXT DEFAULT 'pending'  -- pending | processing | draft | active | archived | error
);

CREATE TABLE ingest_jobs (
  id            BIGSERIAL PRIMARY KEY,
  source_file_id BIGINT REFERENCES source_files(id) ON DELETE CASCADE,
  state         TEXT DEFAULT 'queued',  -- queued | running | done | failed
  log           TEXT DEFAULT '',        -- warnings shown in the preview
  stats         JSONB,                  -- pages, chunks, figures, coverage numbers
  created_at    TIMESTAMPTZ DEFAULT NOW(),
  finished_at   TIMESTAMPTZ
);

ALTER TABLE documents ADD COLUMN IF NOT EXISTS source_file_id BIGINT;
ALTER TABLE documents ADD COLUMN IF NOT EXISTS status TEXT DEFAULT 'active';   -- draft | active | archived
ALTER TABLE documents ADD COLUMN IF NOT EXISTS chunk_kind TEXT DEFAULT 'text'; -- text | table | figure | summary
CREATE INDEX IF NOT EXISTS documents_status_idx ON documents (status);
```

Migration: existing chunks get `status = 'active'`; one `sources` row is created per distinct
`source_url` so the admin list shows everything in one place. **All search SQL adds
`WHERE status = 'active'`** (semantic, keyword, hybrid, RRF, and `eval_retrieval.py`, which mirrors them).

## 5. Flow

```
Admin → Sources → "Add source" (Drive folder / file / PDF URL, title, category, version, valid_from, supersedes?)
   │  Rust: insert sources (+ source_files for Drive folders via files.list), create ingest_jobs (queued)
   ▼
Python worker (GitHub Actions: manual "Run workflow" + daily schedule)
   1. take queued jobs (SELECT … FOR UPDATE SKIP LOCKED)
   2. download → SHA-256 → skip if unchanged
   3. text per page (pymupdf4llm), OCR for scanned pages, tables → markdown rows with headers
   4. figures/complex tables → Gemini description (§6)
   5. chunk (heading-aware; tables per row group + summary chunk, as in E5), embed (RETRIEVAL_DOCUMENT)
   6. write chunks with status = 'draft'; job log = warnings + coverage stats
   ▼
Admin → Review (draft): pages, chunk samples, figure descriptions, warnings
   • Approve → draft chunks become 'active'; chunks of the superseded version become 'archived'
   • Reject  → draft chunks deleted, job marked failed with reason
   ▼
Scheduled re-check: changed remote_rev / sha256 → new job → new draft → review again
```

Quality warnings in the job log (shown before approval):
- pages with almost no text (likely scanned → OCR used / failed)
- **number check**: numbers in the PDF text layer that are missing from the chunks
- figure descriptions created / failed
- duplicate of an existing document (same SHA-256)
- contradicting an active document of another version (later; manual for now)

## 6. Describing tables and figures (borrowed from RAG-Anything)

- Detect images and drawings per page with PyMuPDF; render the region (or the whole page when a
  figure dominates it) to PNG.
- Ask Gemini (`gemini-3.5-flash-lite`, multimodal) for a **factual description in Indonesian**:
  what the figure shows, every label, number and step in order; no interpretation.
- Store as its own chunk: `chunk_kind = 'figure'`, `page_number`, content prefixed
  `"Gambar 3.1 (hal. 15) — Timeline evaluasi awal studi: …"`.
- Budget: one call per figure; the 2022 guidebook has ~10 figures → ~10 calls per ingest.
- Complex tables that pymupdf4llm cannot linearise get the same treatment.

## 7. Citations

- `build_context` already merges chunks into one context per document = one source card.
  Each card gets: **title · version · pages used · `citation_url`**.
- For `web_pdf` links, the card link is `citation_url#page=N` (first page used).
- Prompt rule: cite as `[n]` and mention the page (“Buku Panduan 2022, hal. 24”).
- When two active documents disagree, the newer `valid_from` wins and the answer mentions the older rule (follow-up to `feat/versioned-search`).

## 8. Delivery (PRs)

| PR | Branch | Content | Check |
|---|---|---|---|
| A | `feat/sources-schema` | tables + migration, `status` filter in all search SQL and `eval_retrieval.py`, citation fields on source cards | eval unchanged (all existing chunks active) |
| B | `feat/pdf-worker` | Python worker: Drive folder/file + web PDF fetch, per-page text, chunk, embed, write drafts; GitHub Actions workflow | ingest the 2022 guidebook from Drive into a Neon branch; compare chunks with current PDF rows |
| C | `feat/admin-sources` | Admin UI: Sources list, add source, job status, review → approve/reject, supersede/archive | manual test on Vercel preview |
| D | `feat/figure-descriptions` | Gemini figure/table descriptions as chunks | new eval questions about figures (e.g. evaluation timeline) |
| E | `feat/recheck` | scheduled re-check and change detection | change a Drive file → new draft appears |

Each PR: run `eval_retrieval.py` (+ `--backend` e2e when retrieval/answers can change) and add an entry to `research/experiments.md`.

## 9. Implementation notes (2026-10-18)

- Rust: `src/sources.rs` (schema, link parsing, approve/reject), `src/sources_api.rs` (endpoints under
  `/api/admin/sources` and `/api/admin/drafts`), `status = 'active'` filter in every search query and in
  `fetch_all_chunks_for_documents`; source cards carry `pages` and link to `#page=N` for direct PDFs.
- Jobs point at a source (`ingest_jobs.source_id`); the worker expands a Drive folder into `source_files`.
- Draft `document_id` = `<slug(title)>-<sha256[:8]>`, so a new version never collides with the active one.
- Worker: `scripts/worker/` (`fetch`, `pdf_reader`, `chunker`, `figures`, `store`, `__main__`); tests in `scripts/worker/tests/`.
- Optional immediate run: backend env `GITHUB_REPO` + `GITHUB_WORKER_TOKEN` dispatches the workflow.
- Defaults chosen for the open decisions below: admins only; daily re-check; private Drive files not supported.

## 10. Open decisions

1. **Google Cloud project for the Drive API key** — whose account (ideally institutional)?
2. **Who may add sources** — admins only, or also an "editor" role?
3. **Re-check frequency** — daily is plenty for guidebooks; weekly saves Actions minutes.
4. **Existing PDF chunks** — replace them with the worker's output after PR B, as a new version (old ones archived)?
5. **Private Drive files** — out of scope now; would need a service account shared on the folder.
