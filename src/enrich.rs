//! Text enrichment at ingest: acronym expansion.
//!
//! The acronym list lives in `src/acronyms.json`, shared with `scripts/text_enrichment.py`,
//! so URL ingest (Rust) and bulk ingest (Python) expand the same words the same way.

use std::sync::OnceLock;

/// Acronym → expansion, in file order. Keys starting with `_` are comments.
fn acronyms() -> &'static [(String, String)] {
    static LIST: OnceLock<Vec<(String, String)>> = OnceLock::new();
    LIST.get_or_init(|| {
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(include_str!("acronyms.json")).expect("src/acronyms.json is valid JSON");
        map.into_iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .filter_map(|(k, v)| v.as_str().map(|e| (k, e.to_string())))
            .collect()
    })
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Expand acronyms: the first `max_per_acronym` uses of each become "Expansion (ACR)",
/// e.g. "36 SKS" → "36 Satuan Kredit Semester (SKS)".
///
/// Matching is case-sensitive and whole-word ("api" is not "API"). An acronym that is
/// already explained is left alone: directly followed by "(" / "[" ("SKS (Satuan …)") or
/// directly preceded by "(" ("… Teknologi Informasi (PMPSTI)").
pub fn expand_acronyms(text: &str, max_per_acronym: usize) -> String {
    let mut out = text.to_string();
    for (acr, expansion) in acronyms() {
        let mut result = String::with_capacity(out.len());
        let mut expanded = 0;
        let mut rest = out.as_str();
        while let Some(pos) = rest.find(acr.as_str()) {
            let (before, after) = (&rest[..pos], &rest[pos + acr.len()..]);
            let whole_word = !before.chars().next_back().is_some_and(is_word_char)
                && !after.chars().next().is_some_and(is_word_char);
            let explained = before.ends_with('(')
                || after.trim_start().starts_with(['(', '[']);
            result.push_str(before);
            if whole_word && !explained && expanded < max_per_acronym {
                result.push_str(&format!("{expansion} ({acr})"));
                expanded += 1;
            } else {
                result.push_str(acr);
            }
            rest = after;
        }
        result.push_str(rest);
        out = result;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::expand_acronyms;

    // Same cases as scripts/test_text_enrichment.py, so both implementations agree.
    #[test]
    fn pmpsti_uses_official_name() {
        let out = expand_acronyms("PMPSTI adalah program magister.", 2);
        assert!(out.contains("Program Magister Program Studi Teknologi Informasi (PMPSTI)"), "{out}");
    }

    #[test]
    fn regular_acronym_is_expanded() {
        assert!(expand_acronyms("Mahasiswa wajib menempuh 36 SKS.", 2).contains("36 Satuan Kredit Semester (SKS)"));
    }

    #[test]
    fn lowercase_words_are_not_acronyms() {
        let text = "Jangan bermain api di si laboratorium.";
        assert_eq!(expand_acronyms(text, 2), text);
    }

    #[test]
    fn removed_ambiguous_acronyms_stay() {
        let text = "Lihat BAB 3 tentang TK.";
        assert_eq!(expand_acronyms(text, 2), text);
    }

    #[test]
    fn mixed_case_expansion_kept_as_written() {
        assert!(expand_acronyms("Sistem IoT kampus.", 2).contains("Internet of Things (IoT)"));
    }

    #[test]
    fn already_explained_acronym_is_not_expanded_again() {
        let text = "Kurikulum Program Magister Program Studi Teknologi Informasi (PMPSTI) 2022";
        assert_eq!(expand_acronyms(text, 2), text);
        let text = "Minimal 36 SKS (Satuan Kredit Semester).";
        assert_eq!(expand_acronyms(text, 2), text);
    }

    #[test]
    fn only_first_uses_are_expanded_and_words_containing_acronyms_are_not() {
        let out = expand_acronyms("SKS, SKS, SKS. IPK bukan IP.", 2);
        assert_eq!(out.matches("Satuan Kredit Semester (SKS)").count(), 2, "{out}");
        assert!(out.contains("Indeks Prestasi Kumulatif (IPK)") && out.contains("Indeks Prestasi (IP)"), "{out}");
    }
}
