import os
import sys
from pathlib import Path

# Tests import `worker`, `gemini_embed` and `text_enrichment` the same way `python -m worker` does (cwd = scripts/)
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
os.environ.setdefault("GEMINI_API_KEY", "test-key")
