# PMPSTI RAG

A Retrieval-Augmented Generation (RAG) assistant for PMPSTI academic documents at Universitas Gadjah Mada, built with Rust (Actix-web), SvelteKit, PostgreSQL and pgvector.

- **Production:** https://pmpsti-rag.vercel.app (frontend on Vercel, backend on Render/Fly, database on Neon)

---

## Architecture

```
Frontend (SvelteKit :5173)
    ↕ /api/* proxy
Rust backend (Actix-web :8080)
    ↕ SQLx
PostgreSQL + pgvector (:5432 locally, Neon in the cloud)   ←   Python ingest pipeline
```

---

## Prerequisites

- [Podman](https://podman.io/) (or Docker), to run PostgreSQL locally (optional if you use Neon)
- [Rust + Cargo](https://rustup.rs/), for the backend
- [Node.js 18–22](https://nodejs.org/), for the frontend (the Vercel adapter rejects newer versions for local builds)
- [Python 3.10+](https://python.org/), for document ingestion (optional)
- A [Gemini API key](https://aistudio.google.com/apikey), used for embeddings and the LLM

---

## 1. Start the database (PostgreSQL + pgvector)

Skip this section if your `DATABASE_URL` points to Neon.

### First time (create the container)

```bash
podman run -d \
  --name pmpsti-db \
  -e POSTGRES_USER=admin \
  -e POSTGRES_PASSWORD=admin \
  -e POSTGRES_DB=pmpsti_rag \
  -p 5432:5432 \
  docker.io/pgvector/pgvector:pg16
```

### Afterwards (start the existing container)

```bash
podman start pmpsti-db
```

### Check status

```bash
podman ps
```

### Enable the pgvector extension (once)

```bash
podman exec -it pmpsti-db psql -U admin -d pmpsti_rag -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

---

## 2. Configure the environment

Create a `.env` file in the project root. It is git-ignored and must never be committed.

```env
# Database: local Podman, or a Neon branch (see "Code and database workflow")
DATABASE_URL=postgres://admin:admin@localhost:5432/pmpsti_rag

# Server (the frontend dev proxy expects 8080; the default is 7860)
HOST=0.0.0.0
PORT=8080

# Auth
JWT_SECRET=replace-with-a-long-random-secret

# Optional: create or update an admin account on startup
# ADMIN_EMAIL=you@mail.ugm.ac.id
# ADMIN_PASSWORD=...

# Gemini: required for embeddings (gemini-embedding-001), also used as the LLM
GEMINI_API_KEY=...
LLM_BACKEND=gemini
GEMINI_MODEL=gemini-3.5-flash-lite
# Used in order when the model above hits its quota (429); default gemini-3.1-flash-lite
# LLM_FALLBACK_MODELS=gemini-3.1-flash-lite

# Alternative LLMs:
# Ollama (local)
# LLM_BACKEND=ollama
# LLM_API_URL=http://localhost:11434/api/chat
# LLM_MODEL=llama3.2
# Groq or any OpenAI-compatible API
# LLM_BACKEND=openai
# LLM_API_URL=https://api.groq.com/openai/v1/chat/completions
# LLM_API_KEY=gsk_...
# LLM_MODEL=llama-3.3-70b-versatile

# Email (verification and password reset)
# RESEND_API_KEY=...
# APP_BASE_URL=http://localhost:5173
```

---

## 3. Run the Rust backend

```bash
cargo run
```

Wait for the startup banner (`PMPSTI API`, database, LLM, listen address) followed by `Server siap di http://0.0.0.0:8080`.

> The backend needs `GEMINI_API_KEY` to embed questions with Gemini `gemini-embedding-001`.

---

## 4. Run the SvelteKit frontend

In a **new terminal** (keep the backend running):

```bash
cd frontend
npm install        # first time only
npm run dev
```

Open **http://localhost:5173**.

The frontend always calls the backend through its own `/api` route (the Vite dev proxy locally, `src/routes/api/[...path]/+server.ts` on Vercel), so no API URL needs to be configured.

---

## 5. Set up an admin account

**Option A (automatic):** set `ADMIN_EMAIL` and `ADMIN_PASSWORD` in `.env` and restart the backend.

**Option B (manual):**

1. Register at `http://localhost:5173/register`.
2. Promote the account in psql:

   ```bash
   podman exec -it pmpsti-db psql -U admin -d pmpsti_rag
   ```

   ```sql
   UPDATE users SET role = 'admin' WHERE email = 'you@mail.ugm.ac.id';
   SELECT id, email, role FROM users;
   \q
   ```

3. Sign in again so the session picks up the new role.

---

## 6. Ingest documents (Python)

```bash
cd scripts
python -m venv .venv && source .venv/bin/activate   # first time only
pip install -r requirements.txt                      # first time only

# Put PDF/DOCX files in docs/, then:
python ingest.py
```

Ingested documents appear under **Admin → Documents**. You can also add web pages there with **Add from URL**.

> `ingest.py` needs `GEMINI_API_KEY` (read from the root `.env`). Choose the target database with the `DATABASE_URL` env var or `--db`.

### Re-embed all documents (after changing the embedding model)

Document embeddings **must** use the same model as the backend (Gemini `gemini-embedding-001`).

```bash
cd scripts
python regen_embeddings.py --all --dry-run   # count rows only
python regen_embeddings.py --all             # overwrite every embedding
python regen_embeddings.py                   # fill only rows whose embedding is NULL
```

### Copy embeddings between databases (no Gemini calls)

If embeddings were already generated on another Neon branch (e.g. `test-embedding`), copy them to production instead of regenerating:

```bash
# Root .env: DATABASE_URL = source, PROD_DATABASE_URL = destination
cd scripts
python copy_embeddings.py --dry-run   # report only, writes nothing
python copy_embeddings.py             # copy (asks you to type 'ya' to confirm)
```

Rows are matched by `id` **and** content (title + content). Rows that differ, or exist on only one side, are skipped and reported. Create a production backup branch in Neon first, and remove `PROD_DATABASE_URL` from `.env` when you are done.

---

## 7. Sources: web pages, website PDFs and Google Drive

All content is added in **Admin → Sources** as a **link**. New and changed content is saved as
**drafts**; drafts are not used in answers until an admin approves them in the same page.
**Admin → Documents** is the library of what is indexed, grouped per source.
Design: [`docs/design/ingest-from-links.md`](docs/design/ingest-from-links.md).

| Source type | Example | Read by |
|---|---|---|
| Website (several pages) | `https://mtiugm.github.io/panduan_pmpsti/` + chosen pages | backend (`src/web_jobs.rs`, same reader as before) |
| Web page (standalone) | `https://jteti.ugm.ac.id/tenaga-pendidik/` | backend |
| Website PDF | `https://jteti.ugm.ac.id/…/panduan.pdf` | Python worker |
| Google Drive file / folder | `https://drive.google.com/drive/folders/<id>` | Python worker |

**Websites.** Paste one or more page links. If they lie under a site that is already a source, they are
added to it; otherwise the form suggests the site (from the page's home link and title) and you choose
*part of this website* or *standalone page*. A site source keeps **one document per page** (each page is
its own citation) and has two modes: **only the pages you add** (add more later under *Pages*), or
**also find all pages under the root** (sitemap.xml, else links between pages). Pages ingested earlier
with Add from URL or `ingest_urls.py` are **replaced (archived)** when their managed version is approved.

Drive files must be shared as **Anyone with the link can view**. If the same PDF is on Drive and on
the website, add both: the file is stored once and answers cite the website link.

### Lifecycle

```
Add source → job queued → backend (web pages) or worker (PDFs): read → chunks → embeddings → DRAFT
          → Admin → Sources → review → Approve / Approve all (active; older version archived) / Reject
Re-check (daily, weekly, monthly or every N days per source; or off): changed page/file → new draft → review again. Unchanged content costs nothing.
```

### Test questions (before approving)

In the review of a draft, **Generate & check** asks Gemini for 3 questions a student could ask whose answers
are in that draft (with short answers), then checks where search ranks the draft for each question among the
approved documents plus this draft. "found #1" is good; "not in top 5" means students asking that may not get
this page (missing key words, bad chunks); "Search finds X first" may point at a duplicate or an older version.
A good question can be added to the suggested questions (FAQ) with one click. Costs 1 generation call and
3 embeddings per check; runs only when the admin presses the button.

### Versions (when sources disagree)

Fill in **Version** and **Valid from** on a source (Sources → pencil) or on a single document
(Documents → pencil; a document's own value overrides its source's). Every context chunk sent to the
model carries a `Versi:` line, and the prompt tells it to answer from the newest version and mention the
older value. Example: PDF 2022 (40 SKS, tesis 8 SKS) vs website Kurikulum 2022 Revisi-2, valid from
2025-11 (36 SKS, tesis 6 SKS). Documents without a version are treated as "unknown": the answer gives both
values and suggests confirming with the study programme.

### Run the worker

Web pages are processed by the backend right away. PDFs are processed by the worker, which runs in
**GitHub Actions** (`.github/workflows/ingest-worker.yml`): daily, and on demand
from the Actions tab ("Ingest worker" → Run workflow). Repository secrets: `DATABASE_URL`,
`GEMINI_API_KEY`, `GOOGLE_API_KEY` (a Google Cloud API key with the **Google Drive API** enabled).

To start it right after an admin adds a source, set on the backend (Render):
`GITHUB_REPO=solaemanachmad/pmpsti-rag` and `GITHUB_WORKER_TOKEN` (fine-grained token, this repo only,
permission *Actions: Read and write*). Without them, new sources wait for the daily run.

Locally (from `scripts/`, with `DATABASE_URL` pointing to a Neon **test** branch):

```bash
pip install -r worker/requirements.txt
python -m worker --file "../docs/panduan akademik/Buku Panduan Akademik MTI 2022.pdf" --no-figures  # preview only, no database
python -m worker                    # process queued jobs
python -m worker --source-id 3      # queue + process one source
python -m pytest -q worker/tests    # tests (integration test needs TEST_DATABASE_URL to a throwaway DB)
```

Locally the worker needs `PROD_DATABASE_URL` in the root `.env` (so it recognises production) and refuses
to write to that database unless `--allow-production` is given. The GitHub workflow passes the flag;
drafts are not searchable until an admin approves them.

---

### Unanswered questions

Admin → **Unanswered** lists questions the documents could not answer (the model said the information is
not in the documents), grouped by wording and counted, most asked first. Add a source that covers a question,
test it in Chat, then **Mark handled**. Each answer is classified when it is logged (`query_logs.outcome`:
`answered`, `not_found`, `no_sources`); guest questions are logged too. Questions logged before this feature
have no outcome and do not appear.

### Settings (Admin → Settings)

Model, fallback models, temperature, answer length and search limits (candidates, max sources, min score)
can be changed without a redeploy. Defaults come from the environment (`GEMINI_MODEL` / `LLM_MODEL`,
`LLM_FALLBACK_MODELS`); saved values are in the `app_settings` table, every change is kept in
`app_settings_history` (who, when, what), and **Reset to defaults** goes back to the environment.
**Test** sends one tiny request to a model (one call of quota). The provider and the **API keys stay in
the environment** (Render): they are never stored in the database; the page only shows whether a key is set.
After changing the search limits, run the eval.

## Code and database workflow

There are **two separate flows**: one for **code** (Git, GitHub, Vercel) and one for **data** (Neon). Normal deployments only use the code flow.

### 1. Code flow (every change)

`main` is **Production** (`pmpsti-rag.vercel.app`). Work happens on a separate branch and is tested on a Vercel **Preview** before it reaches Production.

```
create branch → commit → push → open PR → test Preview → merge into main → Production deploys automatically → delete branch
```

1. **Create a branch** from the latest `main`:
   ```bash
   git switch main && git pull
   git switch -c feat/feature-name      # or fix/bug-name, docs/topic
   ```
2. **Commit and push**:
   ```bash
   git add <files> && git commit -m "feat: short description"
   git push -u origin feat/feature-name
   ```
3. **Open a pull request** into `main` on GitHub (the *Compare & pull request* button).
4. **Test the Preview.** Vercel builds a Preview URL for the branch and links it in the PR. Check it on both phone and laptop.
5. **Merge into `main`**: *Merge pull request* → *Confirm merge*. Vercel deploys to Production automatically. Once the deploy is live, run the [smoke check](#smoke-check-the-live-site).
6. **Delete the branch**: click *Delete branch* on the PR, then clean up locally:
   ```bash
   git switch main && git pull
   git branch -d feat/feature-name
   ```

The code flow never changes database contents and never uses `PROD_DATABASE_URL`.

> Vercel environment variables are set separately for Preview and Production (Vercel → Settings → Environment Variables). The frontend needs none: it reaches the backend through its built-in `/api` proxy, which keeps the login cookie first-party (Safari/iOS block third-party cookies).

### 2. Which database each environment uses

| Where it runs | `DATABASE_URL` points to | Where it is set |
|---|---|---|
| **Your machine** (local backend, scripts) | Neon **test** branch (e.g. `test-embedding`) or local Podman | Root `.env` |
| **Production backend** (Render/Fly) | Neon **production** branch | Secrets in the Render/Fly dashboard |

Both are called `DATABASE_URL`, but each environment reads its own value. Your local `.env` never affects the live site, so you can experiment locally without risking real users' data.

### 3. Data flow (only occasionally)

Use it only when **document data** in production must change, for example when adding or updating documents or switching the embedding model.

**A. Write directly to production.** Fine for adding a few new documents:

```bash
DATABASE_URL="<production connection string>" python ingest.py
```

You can also use **Admin → Documents → Add from URL** on the live site, which writes to production.

**B. Prepare on a test branch, then copy.** For large or risky changes, such as switching embedding models:

1. Do the work on the test branch (your local `.env`).
2. Test it locally.
3. Create a **backup branch** of production in Neon.
4. Temporarily add `PROD_DATABASE_URL=<production connection string>` to the root `.env`.
5. Run `python copy_embeddings.py --dry-run`, review the report, then run `python copy_embeddings.py`.
6. **Delete `PROD_DATABASE_URL`** from `.env`.

`copy_embeddings.py` copies only the **embedding column** of documents that exist in both databases. Brand-new documents still need to be ingested into production with option A.

### 4. When code and data change together

For example, a backend change that needs new embeddings. Order the steps so production is never broken for long:

1. Prepare and test the data on the test branch.
2. Merge the code PR; Vercel and the backend deploy.
3. Copy the data to production right away, or just before merging.
4. Test the live site.

---

## Useful commands

### Database

```bash
# Open psql
podman exec -it pmpsti-db psql -U admin -d pmpsti_rag
```

```sql
-- List tables
\dt

-- Users
SELECT id, email, role, is_active FROM users;

-- Ingested documents
SELECT document_id, title, category FROM documents LIMIT 20;

-- Recent chat sessions
SELECT id, title, updated_at FROM chat_sessions ORDER BY updated_at DESC LIMIT 10;

-- Count chunks
SELECT COUNT(*) FROM chunks;
```

### Smoke check the live site

`scripts/smoke_check.py` logs in through the frontend's `/api` proxy, calls `/api/auth/me` with the returned `auth_token` cookie, and checks that a document id containing `%2F` still reaches its backend route. It is read-only and needs only the Python standard library. Run it after every Production deploy, or against a Preview URL before merging:

```bash
cd scripts
python smoke_check.py                                       # https://pmpsti-rag.vercel.app
python smoke_check.py --base-url https://<preview>.vercel.app
```

Put a real account's credentials in `SMOKE_EMAIL` / `SMOKE_PASSWORD` (environment or root `.env`; never commit them). Five failed logins lock the login for 15 minutes, so fix wrong credentials before retrying. A failing check means:

| Check | Failure | Likely cause |
|---|---|---|
| `login` | no `auth_token` cookie | the proxy dropped `Set-Cookie`, or the frontend calls the backend directly instead of `/api` |
| `auth/me` | 401 "Token tidak ditemukan" | the cookie is not sent back (third-party cookie, e.g. `PUBLIC_API_URL` pointing at Render) |
| `encoded id route` | empty 404 | the proxy decoded `%2F` into `/` (deleting documents like `appendix/f_org` fails) |

### Reset a user's password

Passwords are hashed with Argon2, so they cannot be changed directly in psql. Use **Forgot password** on the sign-in page (requires email to be configured), or set `ADMIN_EMAIL` / `ADMIN_PASSWORD` for the admin account and restart the backend.

### Stop everything

```bash
# Stop the database
podman stop pmpsti-db

# Backend and frontend: Ctrl+C in each terminal
```

---

## Project structure

```
pmpsti-rag/
├── src/                     # Rust backend
│   ├── main.rs              # Entry point, config, routing
│   ├── db.rs                # Database layer (SQLx)
│   ├── handlers.rs          # HTTP handlers
│   ├── search.rs            # Hybrid search (vector + BM25)
│   ├── models.rs            # Request/response structs
│   ├── auth.rs              # JWT middleware
│   ├── ai.rs                # LLM integration
│   └── rag.rs               # RAG pipeline
├── frontend/                # SvelteKit frontend
│   ├── src/routes/          # Pages (public, chat, keys, profile, admin/*)
│   ├── src/lib/components/  # Sidebar, chat, UI components
│   ├── src/lib/nav.ts       # Single source for the sidebar and admin menu
│   ├── src/lib/api/         # API client
│   └── static/              # Logos and favicons
├── scripts/
│   ├── ingest.py            # Document ingestion pipeline
│   ├── gemini_embed.py      # Gemini embedding helper (shared by scripts)
│   ├── regen_embeddings.py  # Re-embed documents
│   ├── copy_embeddings.py   # Copy embeddings between databases
│   ├── smoke_check.py       # Live login + /api proxy check
│   └── requirements.txt
├── docs/                    # Documents to ingest
├── .env                     # Configuration (not committed)
└── Cargo.toml
```

---

## Tech stack

| Component | Technology |
|----------|-----------|
| Backend | Rust, Actix-web 4, SQLx 0.7 |
| Database | PostgreSQL 16, pgvector (HNSW index), Neon in the cloud |
| Embedding | Gemini `gemini-embedding-001` (768-dim, task types RETRIEVAL_DOCUMENT / RETRIEVAL_QUERY), used by both the Python ingest and the backend |
| Search | Hybrid: 70% vector + 30% BM25 |
| Auth | JWT HS256, Argon2id |
| Frontend | SvelteKit, Tailwind CSS v3, lucide icons |
| LLM | Gemini / Ollama / any OpenAI-compatible API |
| Ingest | Python, PyMuPDF, SemanticChunker (a local e5 model only decides chunk boundaries) |
| Hosting | Vercel (frontend), Render/Fly (backend), Neon (database) |
