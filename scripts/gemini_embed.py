"""
gemini_embed.py — Embedding via Gemini gemini-embedding-001 (768-dim)
=====================================================================
Model yang SAMA dengan backend Rust (src/search.rs & src/handlers.rs),
supaya vektor dokumen dan vektor pertanyaan berada di ruang yang sama.

  - Dokumen yang disimpan  → taskType RETRIEVAL_DOCUMENT
  - Pertanyaan (di Rust)   → taskType RETRIEVAL_QUERY

Butuh env GEMINI_API_KEY (dibaca juga dari file .env di root project).
Kompatibel dengan interface LangChain Embeddings (embed_documents/embed_query).
"""

import os
import time
from pathlib import Path

import requests

MODEL = "gemini-embedding-001"
DIM = 768
BATCH_URL = (
    f"https://generativelanguage.googleapis.com/v1beta/models/{MODEL}:batchEmbedContents"
)

# Free tier dibatasi per menit; atur via env bila perlu.
BATCH_SIZE = int(os.environ.get("EMBED_BATCH_SIZE", "50"))
BATCH_DELAY_S = float(os.environ.get("EMBED_BATCH_DELAY_S", "2"))
MAX_RETRIES = 6


def _load_dotenv():
    """Baca .env di root project (tanpa dependensi tambahan)."""
    env_path = Path(__file__).resolve().parent.parent / ".env"
    if not env_path.exists():
        return
    for line in env_path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, val = line.split("=", 1)
        os.environ.setdefault(key.strip(), val.strip().strip('"').strip("'"))


_load_dotenv()


def document_text(title: str, content: str) -> str:
    """The text embedded for a stored chunk: title, a blank line, the content.

    The SAME recipe as document_embed_text in src/handlers.rs (Rust URL ingest), used by
    ingest.py and regen_embeddings.py, so every path produces the same vector for the same
    chunk. Pass the title as Gemini's `title` too (embed(..., titles=[...])).
    """
    return f"{title}\n\n{content}" if title else content


class GeminiEmbeddings:
    def __init__(self, api_key: str | None = None, task_type: str = "RETRIEVAL_DOCUMENT"):
        self.api_key = api_key or os.environ.get("GEMINI_API_KEY", "")
        if not self.api_key:
            raise RuntimeError("GEMINI_API_KEY belum di-set (env atau .env)")
        self.task_type = task_type
        self.session = requests.Session()

    def _batch(self, texts: list[str], task_type: str, titles: list[str] | None) -> list[list[float]]:
        reqs = []
        for i, t in enumerate(texts):
            r = {
                "model": f"models/{MODEL}",
                "content": {"parts": [{"text": t or " "}]},
                "taskType": task_type,
                "outputDimensionality": DIM,
            }
            # 'title' hanya berlaku untuk RETRIEVAL_DOCUMENT
            if titles and task_type == "RETRIEVAL_DOCUMENT" and titles[i]:
                r["title"] = titles[i]
            reqs.append(r)

        for attempt in range(MAX_RETRIES):
            resp = self.session.post(
                BATCH_URL,
                headers={"x-goog-api-key": self.api_key},
                json={"requests": reqs},
                timeout=120,
            )
            if resp.status_code in (429, 500, 503):
                wait = min(60, 5 * 2 ** attempt)
                print(f"    [gemini] HTTP {resp.status_code}, tunggu {wait}s "
                      f"(percobaan {attempt + 1}/{MAX_RETRIES})")
                time.sleep(wait)
                continue
            if not resp.ok:
                raise RuntimeError(f"Gemini embed {resp.status_code}: {resp.text[:500]}")
            data = resp.json()
            vecs = [e["values"] for e in data["embeddings"]]
            if len(vecs) != len(texts) or any(len(v) != DIM for v in vecs):
                raise RuntimeError("Respons embedding tidak sesuai (jumlah/dimensi)")
            return vecs
        raise RuntimeError("Gemini embed gagal setelah beberapa percobaan (rate limit?)")

    def embed(self, texts: list[str], task_type: str | None = None,
              titles: list[str] | None = None) -> list[list[float]]:
        task_type = task_type or self.task_type
        out: list[list[float]] = []
        for i in range(0, len(texts), BATCH_SIZE):
            if i > 0:
                time.sleep(BATCH_DELAY_S)
            out.extend(self._batch(
                texts[i:i + BATCH_SIZE], task_type,
                titles[i:i + BATCH_SIZE] if titles else None,
            ))
        return out

    # ── Interface LangChain ───────────────────────────────────────
    def embed_documents(self, texts: list[str]) -> list[list[float]]:
        return self.embed(texts, self.task_type)

    def embed_query(self, text: str) -> list[float]:
        return self.embed([text], "RETRIEVAL_QUERY")[0]
