# Research records

Material for a future journal paper about PMPSTI RAG: what was measured, how, and with which
code, models and data. Keep this folder up to date with every experiment.

| Path | Content |
|---|---|
| `experiments.md` | Experiment log: setup, results, findings and threats to validity per experiment |
| `data/corpus_<date>.json` | Aggregate corpus statistics and fingerprint for a database state |
| `../scripts/eval/results/` | Raw results of every evaluation run (per-question detail) |
| `../scripts/eval/questions.json`, `../eval/pertanyaan_uji.csv` | Test sets |

## Checklist for each new experiment

1. Freeze the database state before changing data: create a Neon branch, e.g. `eval-snapshot-<date>`.
2. Run the evaluation on the code under test (retrieval and `--backend` end-to-end; repeat
   end-to-end ≥ 3 times for paper-grade numbers).
3. If the corpus changed, export new statistics to `data/corpus_<date>.json`.
4. Add an entry to `experiments.md`: date, commit, change, numbers, finding.
5. Commit the results JSON together with the entry.

Never commit raw query logs, chunk text or user data — this repository is public.
