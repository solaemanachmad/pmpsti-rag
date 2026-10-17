# PMPSTI RAG

Sistem Retrieval-Augmented Generation (RAG) untuk dokumen akademik PMPSTI, dibangun dengan Rust (Actix-web) + SvelteKit + PostgreSQL + pgvector.

---

## Arsitektur

```
Frontend (SvelteKit :5173)
    ↕ /api/* proxy
Backend Rust (Actix-web :8080)
    ↕ SQLx
PostgreSQL + pgvector (:5432)   ←   Python ingest pipeline
```

---

## Prasyarat

- [Podman](https://podman.io/) — untuk menjalankan PostgreSQL
- [Rust + Cargo](https://rustup.rs/) — untuk backend
- [Node.js 18+](https://nodejs.org/) — untuk frontend
- [Python 3.10+](https://python.org/) — untuk ingest dokumen (opsional)

---

## 1. Jalankan Database (PostgreSQL + pgvector)

### Pertama kali (buat container baru)

```powershell
podman run -d `
  --name pmpsti-db `
  -e POSTGRES_USER=admin `
  -e POSTGRES_PASSWORD=admin `
  -e POSTGRES_DB=pmpsti_rag `
  -p 5432:5432 `
  docker.io/pgvector/pgvector:pg16
```

### Selanjutnya (start container yang sudah ada)

```powershell
podman start pmpsti-db
```

### Cek status

```powershell
podman ps
```

### Aktifkan extension pgvector (sekali saja)

```powershell
podman exec -it pmpsti-db psql -U admin -d pmpsti_rag -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

---

## 2. Konfigurasi Environment

Salin `.env.example` menjadi `.env` di root project, lalu sesuaikan:

```env
# Database
DATABASE_URL=postgres://admin:admin@localhost:5432/pmpsti_rag

# JWT
JWT_SECRET=ganti-dengan-secret-yang-kuat

# LLM Backend — pilih salah satu:

# Ollama (lokal)
LLM_BACKEND=ollama
LLM_API_URL=http://localhost:11434/api/chat
LLM_MODEL=csalab/sahabatai1:llama3_base_Q4_K_M
LLM_MAX_TOKENS=2048

# Groq (cloud, cepat)
# LLM_BACKEND=openai
# LLM_API_URL=https://api.groq.com/openai/v1/chat/completions
# LLM_API_KEY=gsk_...
# LLM_MODEL=llama-3.3-70b-versatile
# LLM_MAX_TOKENS=2048
```

---

## 3. Jalankan Backend Rust

```powershell
cd C:\Users\achsoe\Developments\pmpsti_rag
cargo run
```

Tunggu hingga muncul:

```
Server listening on 0.0.0.0:8080
AI model loaded successfully.
```

> Backend membutuhkan `GEMINI_API_KEY` di `.env` untuk meng-embed pertanyaan (Gemini `gemini-embedding-001`).

---

## 4. Jalankan Frontend SvelteKit

Buka **terminal baru** (jangan tutup terminal backend):

```powershell
cd C:\Users\achsoe\Developments\pmpsti_rag\frontend
npm install        # hanya pertama kali
npm run dev
```

Buka browser: **http://localhost:5173**

---

## 5. Setup Akun Admin

### Daftar akun baru

Buka `http://localhost:5173/register`, daftar dengan email dan password yang diingat.

### Set role admin via psql

```powershell
podman exec -it pmpsti-db psql -U admin -d pmpsti_rag
```

```sql
UPDATE users SET role = 'admin' WHERE email = 'email-kamu@domain.com';
SELECT id, email, role FROM users;
\q
```

Login ulang agar JWT token di-refresh dengan role admin yang baru.

---

## 6. Ingest Dokumen (Python)

```powershell
cd C:\Users\achsoe\Developments\pmpsti_rag\scripts

# Install dependensi (sekali saja)
pip install -r requirements.txt

# Jalankan ingest — letakkan dokumen PDF/DOCX di folder docs/
python ingest.py
```

Dokumen yang sudah diingest akan muncul di halaman Admin → Documents.

> `ingest.py` butuh `GEMINI_API_KEY` (dari `.env` di root). Target DB bisa diatur dengan env `DATABASE_URL` atau `--db`.

### Re-embed semua dokumen (setelah ganti model embedding)

Embedding dokumen **harus** dibuat dengan model yang sama dengan backend (Gemini `gemini-embedding-001`).

```bash
cd scripts
python regen_embeddings.py --all --dry-run   # cek jumlah baris
python regen_embeddings.py --all             # timpa semua embedding
python regen_embeddings.py                   # hanya isi yang masih NULL
```

### Salin embedding antar database (tanpa memanggil Gemini)

Jika embedding sudah dibuat di branch Neon lain (mis. `test-embedding`), salin ke production tanpa regen:

```bash
# .env di root: DATABASE_URL = sumber, PROD_DATABASE_URL = tujuan
cd scripts
python copy_embeddings.py --dry-run   # laporan saja, tidak menulis
python copy_embeddings.py             # salin (minta konfirmasi 'ya')
```

Baris dicocokkan berdasarkan `id` **dan** isi (title + content); baris yang berbeda dilewati dan dilaporkan. Buat branch backup production di Neon dulu, dan hapus `PROD_DATABASE_URL` dari `.env` setelah selesai.

---

## Perintah Berguna

### Database

```powershell
# Masuk psql
podman exec -it pmpsti-db psql -U admin -d pmpsti_rag

# Lihat semua tabel
\dt

# Lihat users
SELECT id, email, role, is_active FROM users;

# Lihat dokumen yang sudah diingest
SELECT document_id, title, category FROM documents LIMIT 20;

# Lihat sesi chat
SELECT id, title, updated_at FROM chat_sessions ORDER BY updated_at DESC LIMIT 10;

# Hitung chunks
SELECT COUNT(*) FROM chunks;
```

### Reset password user (via register ulang)

Karena password di-hash dengan Argon2, tidak bisa diubah langsung dari psql.
Daftar akun baru lewat UI, lalu set role admin seperti langkah di atas.

### Stop semua service

```powershell
# Stop database
podman stop pmpsti-db

# Backend & frontend: Ctrl+C di masing-masing terminal
```

---

## Alur Kerja Git & Deploy

`main` = **Production** (`pmpsti-rag.vercel.app`). Perubahan dikerjakan di branch terpisah dan diuji di **Preview** Vercel sebelum masuk Production.

```
buat branch → push → buka PR → tes Preview → merge ke main → Vercel deploy ke Production otomatis → hapus branch
```

1. **Buat branch** dari `main` yang terbaru:
   ```bash
   git switch main && git pull
   git switch -c feat/nama-fitur      # atau fix/nama-perbaikan
   ```
2. **Commit & push** branch:
   ```bash
   git add <file> && git commit -m "feat: deskripsi singkat"
   git push -u origin feat/nama-fitur
   ```
3. **Buka Pull Request** ke `main` di GitHub (tombol *Compare & pull request*).
4. **Tes Preview** — Vercel otomatis membuat URL Preview untuk branch tersebut (link muncul di PR). Cek di HP dan laptop.
5. **Merge ke `main`** — klik *Merge pull request* → *Confirm merge*. Vercel otomatis deploy ke Production.
6. **Hapus branch** — klik *Delete branch* di PR, lalu bersihkan di lokal:
   ```bash
   git switch main && git pull
   git branch -d feat/nama-fitur
   ```

> Environment variable untuk Preview diatur terpisah dari Production di Vercel → Settings → Environment Variables. Tanpa `PUBLIC_API_URL`, frontend memakai proxy `/api` bawaan.

---

## Struktur Project

```
pmpsti_rag/
├── src/                  # Backend Rust
│   ├── main.rs           # Entry point, routing
│   ├── db.rs             # Database layer (SQLx)
│   ├── handlers.rs       # HTTP handlers
│   ├── search.rs         # Search engine (hybrid vector + BM25)
│   ├── models.rs         # Request/response structs
│   ├── auth.rs           # JWT middleware
│   ├── ai.rs             # LLM integration
│   └── rag.rs            # RAG pipeline
├── frontend/             # SvelteKit frontend
│   └── src/
│       ├── routes/       # Halaman aplikasi
│       └── lib/          # Store, API client
├── scripts/
│   ├── ingest.py         # Pipeline ingest dokumen
│   └── requirements.txt
├── docs/                 # Folder dokumen untuk diingest
├── .env                  # Konfigurasi (tidak di-commit)
└── Cargo.toml
```

---

## Tech Stack

| Komponen | Teknologi |
|----------|-----------|
| Backend | Rust, Actix-web 4, SQLx 0.7 |
| Database | PostgreSQL 16, pgvector (HNSW index) |
| Embedding | Gemini `gemini-embedding-001` (768-dim, taskType RETRIEVAL_DOCUMENT / RETRIEVAL_QUERY) — dipakai oleh ingest Python **dan** backend |
| Search | Hybrid: 70% vector + 30% BM25 |
| Auth | JWT HS256, Argon2id |
| Frontend | SvelteKit, Tailwind CSS v3 |
| LLM | Ollama / Groq / OpenAI-compatible |
| Ingest | Python, PyMuPDF, SemanticChunker (model lokal e5 hanya untuk menentukan batas chunk) |
