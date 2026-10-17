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

`frontend/.env` may set `PUBLIC_API_URL`. Leave it empty locally so requests go through the `/api` dev proxy.

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
5. **Merge into `main`**: *Merge pull request* → *Confirm merge*. Vercel deploys to Production automatically.
6. **Delete the branch**: click *Delete branch* on the PR, then clean up locally:
   ```bash
   git switch main && git pull
   git branch -d feat/feature-name
   ```

The code flow never changes database contents and never uses `PROD_DATABASE_URL`.

> Vercel environment variables are set separately for Preview and Production (Vercel → Settings → Environment Variables). Without `PUBLIC_API_URL`, the frontend falls back to its built-in `/api` proxy.

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
