#!/usr/bin/env python3
"""
regen_embeddings.py — Buat ulang embedding dokumen dengan Gemini gemini-embedding-001.

Model ini SAMA dengan yang dipakai backend Rust untuk meng-embed pertanyaan,
jadi skor vektor di pencarian hybrid menjadi bermakna.

Pemakaian:
    # Cek dulu berapa baris yang akan diproses (tanpa menulis apa pun)
    python regen_embeddings.py --all --dry-run

    # Re-embed SEMUA baris (dipakai sekali untuk migrasi dari e5 → Gemini)
    python regen_embeddings.py --all

    # Hanya isi baris yang embedding-nya masih NULL (mis. gagal karena kuota)
    python regen_embeddings.py

DATABASE_URL dan GEMINI_API_KEY dibaca dari env atau file .env di root project.
Contoh ke branch Neon:
    DATABASE_URL="postgresql://...neon.tech/neondb?sslmode=require" python regen_embeddings.py --all
"""
import argparse
import os
import sys

import psycopg2

from gemini_embed import GeminiEmbeddings  # juga memuat .env

CHUNK = 50  # baris per commit


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--all", action="store_true",
                    help="Re-embed semua baris (bukan hanya yang NULL)")
    ap.add_argument("--dry-run", action="store_true",
                    help="Hanya tampilkan jumlah baris, tidak memanggil API / menulis DB")
    ap.add_argument("--db", default=os.environ.get(
        "DATABASE_URL", "postgresql://admin:admin@localhost:5432/pmpsti_rag"))
    args = ap.parse_args()

    host = args.db.split("@")[-1].split("/")[0]
    print(f"Database : {host}")
    print(f"Mode     : {'SEMUA baris' if args.all else 'hanya embedding NULL'}")

    conn = psycopg2.connect(args.db)
    cur = conn.cursor()

    where = "" if args.all else "WHERE embedding IS NULL"
    cur.execute(f"SELECT id, COALESCE(title, ''), COALESCE(content, '') "
                f"FROM documents {where} ORDER BY id")
    rows = cur.fetchall()
    total = len(rows)
    print(f"Baris    : {total}\n")

    if total == 0 or args.dry_run:
        print("Selesai (tidak ada yang ditulis).")
        return

    if args.all:
        ans = input(f"Timpa embedding {total} baris di {host}? ketik 'ya': ")
        if ans.strip().lower() != "ya":
            print("Dibatalkan.")
            sys.exit(1)

    model = GeminiEmbeddings(task_type="RETRIEVAL_DOCUMENT")

    for i in range(0, total, CHUNK):
        batch = rows[i:i + CHUNK]
        ids = [r[0] for r in batch]
        titles = [r[1] for r in batch]
        # Judul ikut dimasukkan ke teks supaya konteks dokumen ikut ter-embed
        texts = [f"{r[1]}\n\n{r[2]}" if r[1] else r[2] for r in batch]

        vecs = model.embed(texts, "RETRIEVAL_DOCUMENT", titles=titles)

        for doc_id, emb in zip(ids, vecs):
            vec_literal = "[" + ",".join(f"{x:.8f}" for x in emb) + "]"
            cur.execute("UPDATE documents SET embedding = %s::vector WHERE id = %s",
                        (vec_literal, doc_id))
        conn.commit()

        done = min(i + CHUNK, total)
        print(f"  [{done:4d}/{total}] {done / total * 100:5.1f}%  (id {ids[0]}…{ids[-1]})")

    print("\nSelesai! Semua embedding sudah memakai Gemini gemini-embedding-001.")
    cur.close()
    conn.close()


if __name__ == "__main__":
    main()
