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
        self.delay_s = delay_s
        self.session = requests.Session()
        self.calls = 0

    def describe(self, png: bytes) -> str | None:
        """Description text, or None when the page has no informative figure."""
        url = f"https://generativelanguage.googleapis.com/v1beta/models/{self.model}:generateContent"
        body = {
            "contents": [{"role": "user", "parts": [
                {"text": PROMPT},
                {"inline_data": {"mime_type": "image/png", "data": base64.b64encode(png).decode()}},
            ]}],
            "generationConfig": {"temperature": 0.0, "maxOutputTokens": 2048},
        }
        for attempt in range(6):
            if self.calls:
                time.sleep(self.delay_s)          # stay under the per-minute free-tier limit
            self.calls += 1
            resp = self.session.post(url, headers={"x-goog-api-key": self.api_key}, json=body, timeout=180)
            if resp.status_code in (429, 500, 503):
                if "PerDay" in resp.text or "per day" in resp.text.lower():
                    raise RuntimeError("Kuota harian Gemini habis — coba lagi besok")
                time.sleep(min(60, 5 * 2 ** attempt))
                continue
            if not resp.ok:
                raise RuntimeError(f"Gemini {resp.status_code}: {resp.text[:300]}")
            parts = resp.json().get("candidates", [{}])[0].get("content", {}).get("parts", [])
            text = "".join(p.get("text", "") for p in parts).strip()
            if not text or text.startswith(NONE_MARKER):
                return None
            return text
        raise RuntimeError("Gemini gagal setelah beberapa percobaan")
