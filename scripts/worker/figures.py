"""Describe figures (timelines, diagrams, charts, scanned tables) with Gemini vision.

One call per candidate page. The description becomes its own chunk (kind 'figure') so
questions about a figure (e.g. the evaluation timeline) can be answered and cited by page.
"""
from __future__ import annotations

import base64
import os
import time

import requests

NONE_MARKER = "TIDAK_ADA"
PROMPT = f"""Halaman ini berasal dari dokumen akademik resmi (Bahasa Indonesia).
Tugas: jelaskan HANYA gambar, diagram, bagan, timeline, atau tabel berbentuk gambar pada halaman ini.
Abaikan paragraf teks biasa.

Aturan:
- Tulis dalam Bahasa Indonesia, faktual, tanpa interpretasi atau saran.
- Mulai dengan judul/caption gambar bila ada (misalnya "Gambar 3.1 ...").
- Sebutkan SEMUA label, angka, tanggal, semester, dan langkah persis seperti tertulis, sesuai urutan.
- Untuk timeline/alur: tulis sebagai daftar bernomor dari awal sampai akhir.
- Untuk bagan organisasi: tulis hubungan atasan → bawahan.
- Jangan menebak teks yang tidak terbaca; tulis "(tidak terbaca)".
- Jika tidak ada gambar/diagram yang berisi informasi, jawab persis: {NONE_MARKER}"""


class FigureDescriber:
    def __init__(self, api_key: str | None = None, model: str | None = None, delay_s: float = 4.0):
        self.api_key = api_key or os.environ.get("GEMINI_API_KEY", "")
        if not self.api_key:
            raise RuntimeError("GEMINI_API_KEY belum di-set")
        self.model = model or os.environ.get("GEMINI_MODEL", "gemini-3.5-flash-lite")
        # used in order when a model's daily quota is gone (same variable as the backend)
        fallbacks = os.environ.get("LLM_FALLBACK_MODELS", "gemini-3.1-flash-lite")
        self.models = list(dict.fromkeys([self.model] + [m.strip() for m in fallbacks.split(",") if m.strip()]))
        self.delay_s = delay_s
        self.session = requests.Session()
        self.calls = 0

    def describe(self, png: bytes) -> str | None:
        """Description text, or None when the page has no informative figure."""
        body = {
            "contents": [{"role": "user", "parts": [
                {"text": PROMPT},
                {"inline_data": {"mime_type": "image/png", "data": base64.b64encode(png).decode()}},
            ]}],
            "generationConfig": {"temperature": 0.0, "maxOutputTokens": 2048},
        }
        attempt = 0
        while attempt < 6:
            url = f"https://generativelanguage.googleapis.com/v1beta/models/{self.models[0]}:generateContent"
            if self.calls:
                time.sleep(self.delay_s)          # stay under the per-minute free-tier limit
            self.calls += 1
            resp = self.session.post(url, headers={"x-goog-api-key": self.api_key}, json=body, timeout=180)
            if resp.status_code in (429, 500, 503):
                if "PerDay" in resp.text or "per day" in resp.text.lower():
                    if len(self.models) > 1:      # this model is done for today: use the next one
                        print(f"  Kuota harian {self.models.pop(0)} habis, lanjut dengan {self.models[0]}")
                        continue
                    raise RuntimeError("Kuota harian Gemini habis — coba lagi besok")
                time.sleep(min(60, 5 * 2 ** attempt))
                attempt += 1
                continue
            if resp.status_code == 404 and len(self.models) > 1:   # model name not available
                self.models.pop(0)
                continue
            if not resp.ok:
                raise RuntimeError(f"Gemini {resp.status_code}: {resp.text[:300]}")
            parts = resp.json().get("candidates", [{}])[0].get("content", {}).get("parts", [])
            text = "".join(p.get("text", "") for p in parts).strip()
            if not text or text.startswith(NONE_MARKER):
                return None
            return text
        raise RuntimeError("Gemini gagal setelah beberapa percobaan")
