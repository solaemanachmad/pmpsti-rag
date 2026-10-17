"""
copy_embeddings.py — Salin embedding dokumen dari satu database ke database lain
================================================================================
Untuk memindahkan embedding Gemini yang sudah dibuat di branch Neon lain
(mis. `test-embedding`) ke `production`, TANPA memanggil Gemini lagi.

Baris dicocokkan berdasarkan `id` DAN isi (title + content). Baris yang isinya
berbeda atau hanya ada di salah satu database tidak disentuh, lalu dilaporkan.

    # 1) Cek dulu — tidak menulis apa pun
    python copy_embeddings.py --dry-run

    # 2) Salin
    python copy_embeddings.py

    # Opsional: kosongkan embedding production yang tidak punya pasangan,
    # lalu isi dengan:  DATABASE_URL=$PROD_DATABASE_URL python regen_embeddings.py
    python copy_embeddings.py --null-unmatched

Sumber  : --src, default env DATABASE_URL       (branch yang sudah di-regen)
Tujuan  : --dst, default env PROD_DATABASE_URL  (production)
Keduanya dibaca dari env atau file .env di root project.
"""

import argparse
import os
import sys

import psycopg2

import gemini_embed  # noqa: F401 — memuat .env dari root project

BATCH = 200


def host_of(url: str) -> str:
    return url.split("@")[-1].split("/")[0]


def load(cur):
    cur.execute(
        "SELECT id, md5(COALESCE(title, '') || E'\\x1f' || COALESCE(content, '')), embedding::text "
        "FROM documents"
    )
    return {r[0]: (r[1], r[2]) for r in cur.fetchall()}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", default=os.environ.get("DATABASE_URL"))
    ap.add_argument("--dst", default=os.environ.get("PROD_DATABASE_URL"))
    ap.add_argument("--dry-run", action="store_true", help="Hanya laporan, tidak menulis DB")
    ap.add_argument("--null-unmatched", action="store_true",
                    help="Set embedding NULL di tujuan untuk baris tanpa pasangan di sumber")
    ap.add_argument("--yes", action="store_true", help="Lewati konfirmasi")
    args = ap.parse_args()

    if not args.src or not args.dst:
        sys.exit("DATABASE_URL (sumber) dan PROD_DATABASE_URL (tujuan) wajib diisi di .env atau via --src/--dst.")
    if args.src == args.dst:
        sys.exit("Sumber dan tujuan sama — tidak ada yang perlu disalin.")

    print(f"Sumber : {host_of(args.src)}")
    print(f"Tujuan : {host_of(args.dst)}\n")

    src_conn, dst_conn = psycopg2.connect(args.src), psycopg2.connect(args.dst)
    src, dst = load(src_conn.cursor()), load(dst_conn.cursor())

    to_copy, same, changed, no_src_emb = [], 0, [], []
    for doc_id, (dst_hash, dst_emb) in dst.items():
        if doc_id not in src:
            continue
        src_hash, src_emb = src[doc_id]
        if src_hash != dst_hash:
            changed.append(doc_id)
        elif src_emb is None:
            no_src_emb.append(doc_id)
        elif src_emb == dst_emb:
            same += 1
        else:
            to_copy.append((src_emb, doc_id))

    only_dst = sorted(set(dst) - set(src))
    only_src = sorted(set(src) - set(dst))
    unmatched = sorted(set(only_dst) | set(changed) | set(no_src_emb))

    print(f"Baris di sumber          : {len(src)}")
    print(f"Baris di tujuan          : {len(dst)}")
    print(f"Akan disalin             : {len(to_copy)}")
    print(f"Sudah sama (dilewati)    : {same}")
    print(f"Isi berbeda (dilewati)   : {len(changed)}")
    print(f"Sumber belum ter-embed   : {len(no_src_emb)}")
    print(f"Hanya ada di tujuan      : {len(only_dst)}")
    print(f"Hanya ada di sumber      : {len(only_src)}  (tidak disalin — dokumen baru di sumber)")
    if unmatched:
        preview = ", ".join(map(str, unmatched[:10])) + (" …" if len(unmatched) > 10 else "")
        print(f"\nTanpa pasangan di tujuan : {len(unmatched)} baris (id {preview})")
        print("  → tetap memakai embedding lama kecuali pakai --null-unmatched lalu regen_embeddings.py")

    if args.dry_run or (not to_copy and not (args.null_unmatched and unmatched)):
        print("\nSelesai (tidak ada yang ditulis).")
        return

    if not args.yes:
        ans = input(f"\nTulis {len(to_copy)} embedding ke {host_of(args.dst)}? ketik 'ya': ")
        if ans.strip().lower() != "ya":
            sys.exit("Dibatalkan.")

    cur = dst_conn.cursor()
    for i in range(0, len(to_copy), BATCH):
        cur.executemany("UPDATE documents SET embedding = %s::vector WHERE id = %s", to_copy[i:i + BATCH])
        dst_conn.commit()
        print(f"  [{min(i + BATCH, len(to_copy)):5d}/{len(to_copy)}]")

    if args.null_unmatched and unmatched:
        cur.execute("UPDATE documents SET embedding = NULL WHERE id = ANY(%s)", (unmatched,))
        dst_conn.commit()
        print(f"  {len(unmatched)} embedding dikosongkan — isi dengan regen_embeddings.py (tanpa --all).")

    print("\nSelesai!")
    src_conn.close()
    dst_conn.close()


if __name__ == "__main__":
    main()
