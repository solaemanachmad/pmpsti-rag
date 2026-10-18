//! Reading web pages for URL ingest: visible text, tables, and table-aware chunks.
//!
//! A page is turned into `PageContent` — its title, its main text and its tables kept
//! separately — so tables can be chunked row by row with their column names instead of
//! being flattened into anonymous `a | b | c` lines.

use scraper::{ElementRef, Html, Node, Selector};
use serde_json::Value;

/// A table found on a page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageTable {
    pub caption: String,
    /// Column names; empty when the table has no header row.
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Headings above the table below the chapter level, e.g. `["1.5 Analisis SWOT"]`
    /// (the chapter `h1` repeats the page title, so it is left out).
    pub section: Vec<String>,
}

/// A part of the main text under one heading.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Section {
    /// Headings above this text, outermost first, e.g.
    /// `["1 Pendahuluan", "1.1 Identitas Prodi", "1.1.4 Program Studi"]`. Empty before the
    /// first heading (and for pages without headings).
    pub path: Vec<String>,
    /// The section's own text (not its subsections'), one line per text node.
    pub text: String,
}

/// What `extract_page` reads from a page.
#[derive(Debug, Clone, Default)]
pub struct PageContent {
    pub title: String,
    /// Main text without tables, one paragraph or line per line.
    pub text: String,
    /// The same text split at headings, in page order (see `sections`).
    pub sections: Vec<Section>,
    pub tables: Vec<PageTable>,
}

/// Elements whose text is never part of the main text: page chrome, code, and tables
/// (tables are read separately by `json_tables` / `html_tables`).
const SKIP_TEXT_IN: &[&str] = &[
    "script", "style", "noscript", "template", "nav", "header", "footer", "aside", "button", "table",
];

fn collapse(s: &str) -> String {
    s.replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Plain text of an HTML fragment: tags removed, entities decoded, whitespace collapsed.
pub fn html_to_text(fragment: &str) -> String {
    let doc = Html::parse_fragment(fragment);
    collapse(&doc.root_element().text().collect::<Vec<_>>().join(" "))
}

/// Visible text of `root`, skipping `SKIP_TEXT_IN` elements. Non-breaking spaces become
/// normal spaces (so "Tabel&nbsp;D.1" matches "Tabel D.1"); blank lines are dropped.
pub fn visible_text(root: ElementRef) -> String {
    let mut parts: Vec<String> = Vec::new();
    for node in root.descendants() {
        let Node::Text(text) = node.value() else { continue };
        let hidden = node.ancestors()
            .filter_map(ElementRef::wrap)
            .any(|el| SKIP_TEXT_IN.contains(&el.value().name()));
        if !hidden {
            parts.push(text.to_string());
        }
    }
    parts.join("\n")
        .replace('\u{a0}', " ")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Headings that start a new section, with their level. `h4`–`h6` are usually short labels
/// ("Visi", "Misi") — splitting there would make tiny chunks, so they stay in the text.
/// Page chrome whose headings are not part of the content outline (e.g. Quarto's
/// "Table of contents" `h2` inside `<nav id="TOC">`).
const NO_HEADINGS_IN: &[&str] = &["nav", "aside", "footer"];

fn section_level(tag: &str) -> Option<usize> {
    match tag {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        _ => None,
    }
}

/// The visible text of `root` (same rules as `visible_text`) split into sections at
/// `h1`–`h3`, each with its heading path. Lets every chunk say where it comes from, so
/// "Visi" of the study programme is not confused with "Visi" of the faculty.
///
/// Headings inside skipped elements are still read: Quarto puts the chapter title `h1`
/// in a `<header>`. Sections without text (a heading directly followed by a
/// subheading) are dropped; their heading still appears in the children's paths.
pub fn sections(root: ElementRef) -> Vec<Section> {
    fn flush(path: &[(usize, String)], lines: &mut Vec<String>, out: &mut Vec<Section>) {
        if !lines.is_empty() {
            out.push(Section {
                path: path.iter().map(|(_, h)| h.clone()).collect(),
                text: std::mem::take(lines).join("\n"),
            });
        }
    }

    fn walk(
        el: ElementRef,
        headings_only: bool,
        path: &mut Vec<(usize, String)>,
        lines: &mut Vec<String>,
        out: &mut Vec<Section>,
    ) {
        for child in el.children() {
            match child.value() {
                Node::Text(text) if !headings_only => {
                    let line = text.replace('\u{a0}', " ").trim().to_string();
                    if !line.is_empty() {
                        lines.push(line);
                    }
                }
                Node::Element(_) => {
                    let Some(child_el) = ElementRef::wrap(child) else { continue };
                    let tag = child_el.value().name();
                    if let Some(level) = section_level(tag) {
                        flush(path, lines, out);
                        while path.last().is_some_and(|(l, _)| *l >= level) {
                            path.pop();
                        }
                        let heading = collapse(&child_el.text().collect::<Vec<_>>().join(" "));
                        if !heading.is_empty() {
                            path.push((level, heading));
                        }
                    } else if !NO_HEADINGS_IN.contains(&tag) {
                        let skip = headings_only || SKIP_TEXT_IN.contains(&tag);
                        walk(child_el, skip, path, lines, out);
                    }
                }
                _ => {}
            }
        }
    }

    let (mut path, mut lines, mut out) = (Vec::new(), Vec::new(), Vec::new());
    walk(root, false, &mut path, &mut lines, &mut out);
    flush(&path, &mut lines, &mut out);
    out
}

/// Headings (`h1`–`h3`, with level) in force at `target`: the last heading of each level
/// before it in page order within `scope`, ignoring `NO_HEADINGS_IN` chrome.
fn heading_path_at(scope: ElementRef, target: ElementRef) -> Vec<(usize, String)> {
    let mut path: Vec<(usize, String)> = Vec::new();
    for node in scope.descendants() {
        if node.id() == target.id() {
            break;
        }
        let Some(el) = ElementRef::wrap(node) else { continue };
        let Some(level) = section_level(el.value().name()) else { continue };
        let in_chrome = el.ancestors().filter_map(ElementRef::wrap)
            .any(|a| NO_HEADINGS_IN.contains(&a.value().name()));
        if in_chrome {
            continue;
        }
        while path.last().is_some_and(|(l, _)| *l >= level) {
            path.pop();
        }
        let heading = collapse(&el.text().collect::<Vec<_>>().join(" "));
        if !heading.is_empty() {
            path.push((level, heading));
        }
    }
    path
}

/// `heading_path_at` without the chapter level, for labelling a table.
fn table_section(scope: ElementRef, table: ElementRef) -> Vec<String> {
    heading_path_at(scope, table).into_iter().filter(|(l, _)| *l > 1).map(|(_, h)| h).collect()
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::String(s) => html_to_text(s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// Tables stored as JSON in `<script type="application/json">`, in either format:
/// - R/Quarto htmlwidgets (DT): `{"x": {"data": [[column], …], "container": "<table>…<th>…",
///   "caption": "…"}}` — data is **column by column**, headers live in the container HTML.
///   This holds every row, including those hidden behind the widget's pagination.
/// - DataTables style: `{"data": [[row], …], "columns": [{"title": …}, …]}` — row by row.
pub fn json_tables(document: &Html) -> Vec<PageTable> {
    let sel = Selector::parse("script[type='application/json']").unwrap();
    document.select(&sel)
        .filter_map(|script| {
            let v = serde_json::from_str::<Value>(&script.text().collect::<String>()).ok()?;
            let mut table = htmlwidget_table(&v).or_else(|| datatables_table(&v))?;
            table.section = table_section(document.root_element(), script);
            Some(table)
        })
        .collect()
}

fn htmlwidget_table(v: &Value) -> Option<PageTable> {
    let x = v.get("x")?;
    let columns: Vec<&Vec<Value>> = x.get("data")?.as_array()?.iter().filter_map(Value::as_array).collect();
    let n_rows = columns.iter().map(|c| c.len()).max().filter(|&n| n > 0)?;
    let rows = (0..n_rows)
        .map(|r| columns.iter().map(|c| c.get(r).map(cell_text).unwrap_or_default()).collect())
        .collect();
    let headers = x.get("container").and_then(Value::as_str)
        .map(|html| {
            let th = Selector::parse("th").unwrap();
            Html::parse_fragment(html).select(&th).map(|e| collapse(&e.text().collect::<String>())).collect()
        })
        .unwrap_or_default();
    let caption = x.get("caption").and_then(Value::as_str).map(html_to_text).unwrap_or_default();
    Some(PageTable { caption, headers, rows, section: vec![] })
}

fn datatables_table(v: &Value) -> Option<PageTable> {
    let rows: Vec<Vec<String>> = v.get("data")?.as_array()?.iter()
        .filter_map(Value::as_array)
        .map(|cells| cells.iter().map(cell_text).collect())
        .collect();
    if rows.is_empty() {
        return None;
    }
    let headers = v.get("columns").and_then(Value::as_array)
        .map(|cols| cols.iter()
            .filter_map(|c| c.get("title").or_else(|| c.get("data")).and_then(Value::as_str).map(html_to_text))
            .collect())
        .unwrap_or_default();
    Some(PageTable { caption: String::new(), headers, rows, section: vec![] })
}

/// True when `el` sits inside a JSON-backed widget: the visible table there is only the
/// current page of rows, so the full data is taken from the JSON instead.
fn inside_widget(el: ElementRef) -> bool {
    el.ancestors().filter_map(ElementRef::wrap).any(|a| {
        let v = a.value();
        v.classes().any(|c| c == "html-widget" || c == "datatables")
            || v.id().is_some_and(|id| id.starts_with("htmlwidget"))
    })
}

/// HTML `<table>`s inside `root` (not inside JSON widgets or other tables).
/// A first row of only `<th>` cells becomes the header.
pub fn html_tables(root: ElementRef) -> Vec<PageTable> {
    let table_sel = Selector::parse("table").unwrap();
    let tr_sel = Selector::parse("tr").unwrap();
    let th_sel = Selector::parse("th").unwrap();
    let td_sel = Selector::parse("td").unwrap();
    let cell_sel = Selector::parse("th, td").unwrap();
    let caption_sel = Selector::parse("caption").unwrap();
    let text = |e: ElementRef| collapse(&e.text().collect::<Vec<_>>().join(" "));

    let mut tables = Vec::new();
    for table in root.select(&table_sel) {
        let nested = table.ancestors().filter_map(ElementRef::wrap).any(|a| a.value().name() == "table");
        if nested || inside_widget(table) {
            continue;
        }
        let mut headers: Vec<String> = Vec::new();
        let mut rows: Vec<Vec<String>> = Vec::new();
        for tr in table.select(&tr_sel) {
            let only_th = tr.select(&td_sel).next().is_none();
            if only_th && headers.is_empty() && rows.is_empty() {
                headers = tr.select(&th_sel).map(text).collect();
                continue;
            }
            let cells: Vec<String> = tr.select(&cell_sel).map(text).collect();
            if cells.iter().any(|c| !c.is_empty()) {
                rows.push(cells);
            }
        }
        if !rows.is_empty() {
            let caption = table.select(&caption_sel).next().map(text).unwrap_or_default();
            tables.push(PageTable { caption, headers, rows, section: table_section(root, table) });
        }
    }
    tables
}

/// One row as text. With matching headers every value keeps its column name
/// (`Nama: Juni Paryadi | NIP: 1967… | Email: …`); a running-number column is dropped.
fn row_line(headers: &[String], row: &[String]) -> String {
    if !headers.is_empty() && headers.len() == row.len() {
        headers.iter().zip(row)
            .filter(|(h, v)| !v.is_empty() && !matches!(h.to_lowercase().as_str(), "no" | "no." | "#"))
            .map(|(h, v)| if h.is_empty() { v.clone() } else { format!("{h}: {v}") })
            .collect::<Vec<_>>()
            .join(" | ")
    } else {
        row.iter().filter(|c| !c.is_empty()).cloned().collect::<Vec<_>>().join(" | ")
    }
}

/// The column that names each row: a header containing "nama"/"name", otherwise the
/// column with the longest non-numeric values.
fn key_column(table: &PageTable) -> Option<usize> {
    if let Some(i) = table.headers.iter().position(|h| {
        let h = h.to_lowercase();
        h.contains("nama") || h.contains("name")
    }) {
        return Some(i);
    }
    let width = table.rows.iter().map(Vec::len).max()?;
    (0..width)
        .map(|c| {
            let vals: Vec<&String> = table.rows.iter().filter_map(|r| r.get(c)).filter(|v| !v.is_empty()).collect();
            let numeric = vals.iter().filter(|v| v.parse::<f64>().is_ok()).count();
            let avg = vals.iter().map(|v| v.len()).sum::<usize>() / vals.len().max(1);
            (c, if numeric * 2 > vals.len() { 0 } else { avg })
        })
        .max_by_key(|&(_, score)| score)
        .filter(|&(_, score)| score > 0)
        .map(|(c, _)| c)
}

/// Longest summary chunk; lists longer than this are cut (the total count stays exact).
const MAX_SUMMARY_CHARS: usize = 4_000;

/// Chunks for one table, ready to embed:
/// 1. a summary chunk with the total number of rows and every row's name — so
///    "berapa jumlah …?" / "daftar lengkap …" can be answered from a single chunk;
/// 2. the rows (`row_line`) grouped up to `max_chars`, each chunk starting with the page
///    title, the table's section (below chapter level), caption and row range, so a chunk
///    is understandable on its own.
///
/// The generated words ("Jumlah data", "baris") are Indonesian on purpose: the corpus and
/// the students' questions are Indonesian, and these words must match their questions.
pub fn table_chunks(page_title: &str, table: &PageTable, max_chars: usize) -> Vec<String> {
    let lines: Vec<String> = table.rows.iter()
        .map(|r| row_line(&table.headers, r))
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return vec![];
    }
    let total = lines.len();
    let mut label = page_title.to_string();
    for part in table.section.iter().chain(std::iter::once(&table.caption)).filter(|p| !p.is_empty()) {
        label.push_str(" — ");
        label.push_str(part);
    }

    let mut chunks = vec![summary_chunk(&label, table, total)];
    let mut start = 0;
    while start < total {
        let mut end = start;
        let mut size = 0;
        while end < total && (end == start || size + lines[end].len() + 1 <= max_chars) {
            size += lines[end].len() + 1;
            end += 1;
        }
        chunks.push(format!("{label} (baris {}–{end} dari {total})\n{}", start + 1, lines[start..end].join("\n")));
        start = end;
    }
    chunks
}

fn summary_chunk(label: &str, table: &PageTable, total: usize) -> String {
    let mut text = format!("{label}\nJumlah data: {total}.");
    let named: Vec<&String> = table.headers.iter().filter(|h| !h.is_empty()).collect();
    if !named.is_empty() {
        text.push_str(&format!("\nKolom: {}.", named.iter().map(|h| h.as_str()).collect::<Vec<_>>().join(", ")));
    }
    if let Some(col) = key_column(table) {
        let title = table.headers.get(col).filter(|h| !h.is_empty()).map(String::as_str).unwrap_or("Daftar");
        // Keep only the name part of long cells, e.g. "Prof. … IPU. (Guru Besar) Bidang …"
        let names: Vec<String> = table.rows.iter()
            .filter_map(|r| r.get(col))
            .map(|v| v.split(" (").next().unwrap_or(v).trim().to_string())
            .filter(|v| !v.is_empty())
            .collect();
        let mut list = String::new();
        for (i, name) in names.iter().enumerate() {
            let piece = if i == 0 { name.clone() } else { format!("; {name}") };
            if text.len() + title.len() + list.len() + piece.len() + 8 > MAX_SUMMARY_CHARS {
                list.push_str("; …");
                break;
            }
            list.push_str(&piece);
        }
        text.push_str(&format!("\n{title}: {list}."));
    }
    text
}

/// Signs of code in a chunk; two different ones make a chunk "code-like".
const CODE_MARKERS: &[&str] = &[
    "function(", "function (", "=>", "});", "document.", "window.", "var ", "$(", "<script", "{\"", "\":{",
];

fn looks_like_code(chunk: &str) -> bool {
    let markers = CODE_MARKERS.iter().filter(|m| chunk.contains(*m)).count();
    let symbols = chunk.chars().filter(|c| "{};=<>".contains(*c)).count();
    markers >= 2 || symbols * 20 > chunk.chars().count().max(1) // more than 5 % code symbols
}

/// DataTables' pager text: only one page of rows was in the HTML.
fn has_pagination_text(text: &str) -> bool {
    let t = text.to_lowercase();
    (t.contains("showing ") && t.contains(" entries")) || (t.contains("menampilkan ") && t.contains(" entri"))
}

/// Quality checks for a URL ingest preview: plain-English warnings for the admin, empty
/// when nothing looks wrong. Each check comes from a problem seen in earlier ingests
/// (JavaScript chunks, half-read paginated tables, sidebar menus, missing headings).
pub fn quality_warnings(page: &PageContent, chunks: &[String], max_chars: usize) -> Vec<String> {
    let mut warnings = Vec::new();
    let plural = |n: usize| if n == 1 { "" } else { "s" };

    if page.text.chars().count() < 300 && page.tables.is_empty() {
        warnings.push(format!(
            "Very little text ({} characters) and no tables — the page may need JavaScript or have moved.",
            page.text.chars().count()
        ));
    }
    let code = chunks.iter().filter(|c| looks_like_code(c)).count();
    if code > 0 {
        warnings.push(format!("{code} chunk{} look like code (JavaScript/CSS), not content.", plural(code)));
    }
    if has_pagination_text(&page.text) {
        warnings.push("Pagination text found (e.g. \"Showing 1 to 10 of N entries\") — a table may be only partly read.".into());
    }
    let mut seen = std::collections::HashSet::new();
    let duplicates = chunks.iter().filter(|c| !seen.insert(c.trim())).count();
    if duplicates > 0 {
        warnings.push(format!("{duplicates} duplicate chunk{}.", plural(duplicates)));
    }
    let short = chunks.iter().filter(|c| c.chars().count() < 80).count();
    if short > 0 {
        warnings.push(format!("{short} very short chunk{} (under 80 characters).", plural(short)));
    }
    // Table summary chunks are long on purpose (every row's name, up to MAX_SUMMARY_CHARS)
    let long = chunks.iter()
        .filter(|c| {
            let limit = if c.contains("\nJumlah data: ") { MAX_SUMMARY_CHARS } else { max_chars + 300 };
            c.chars().count() > limit
        })
        .count();
    if long > 0 {
        warnings.push(format!("{long} chunk{} longer than {} characters.", plural(long), max_chars + 300));
    }
    if page.text.chars().count() > 1_500 && page.sections.iter().all(|s| s.path.is_empty()) {
        warnings.push("No headings found — chunks will have no section path.".into());
    }
    let lines: Vec<&str> = page.text.lines().collect();
    let menu_like = lines.iter().filter(|l| l.split_whitespace().count() <= 3).count();
    if lines.len() >= 30 && menu_like * 10 > lines.len() * 6 {
        warnings.push(format!(
            "{menu_like} of {} text lines are 1–3 words — menus or link lists may have leaked in.",
            lines.len()
        ));
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    const D_TENDIK: &str = include_str!("../tests/fixtures/d_tendik.html");
    const C_EWMP: &str = include_str!("../tests/fixtures/c_ewmp.html");
    const TENAGA_PENDIDIK: &str = include_str!("../tests/fixtures/tenaga_pendidik.html");
    const INTRODUCTION: &str = include_str!("../tests/fixtures/introduction.html");

    #[test]
    fn reads_all_rows_from_column_major_widget_json() {
        let tables = json_tables(&Html::parse_document(D_TENDIK));
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.headers, ["No", "Nama", "NIP", "Email"]);
        assert_eq!(t.rows.len(), 40, "all 40 staff, not just the first page of 10");
        assert_eq!(t.rows[0], ["1", "Juni Paryadi", "196710301987031001", "juni@ugm.ac.id"]);
    }

    #[test]
    fn reads_widget_caption_and_numbers() {
        let t = &json_tables(&Html::parse_document(C_EWMP))[0];
        assert!(t.caption.contains("EWMP"), "caption: {}", t.caption);
        assert_eq!(t.headers.len(), 10);
        assert_eq!(t.rows.len(), 28);
    }

    #[test]
    fn rows_keep_their_column_names() {
        let t = &json_tables(&Html::parse_document(D_TENDIK))[0];
        assert_eq!(row_line(&t.headers, &t.rows[0]), "Nama: Juni Paryadi | NIP: 196710301987031001 | Email: juni@ugm.ac.id");
    }

    #[test]
    fn table_chunks_have_summary_and_context() {
        let t = &json_tables(&Html::parse_document(D_TENDIK))[0];
        let chunks = table_chunks("Lampiran D — Daftar Tenaga Kependidikan", t, 1200);
        let summary = &chunks[0];
        assert!(summary.contains("Jumlah data: 40."));
        assert!(summary.contains("Juni Paryadi") && summary.contains("Murtija"));
        for c in &chunks[1..] {
            assert!(c.starts_with("Lampiran D — Daftar Tenaga Kependidikan (baris "), "missing context: {}", &c[..60]);
            assert!(c.len() <= 1200 + 120, "chunk too long: {}", c.len());
        }
        let all = chunks[1..].join("\n");
        assert_eq!(all.matches("Nama: ").count(), 40, "every row in exactly one chunk");
    }

    #[test]
    fn html_table_without_header_is_read_and_summarised() {
        let doc = Html::parse_document(TENAGA_PENDIDIK);
        let root = doc.select(&Selector::parse("article .post-content").unwrap()).next().unwrap();
        let tables = html_tables(root);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].rows.len(), 60);
        let summary = &table_chunks("Tenaga Pendidik", &tables[0], 1200)[0];
        assert!(summary.contains("Jumlah data: 60."));
        assert!(summary.contains("Prof. Ir. P. Insap Santosa, M.Sc., Ph.D., IPU."), "{}", &summary[..200]);
    }

    #[test]
    fn sections_carry_their_heading_path() {
        let doc = Html::parse_document(INTRODUCTION);
        let root = doc.select(&Selector::parse("main#quarto-document-content").unwrap()).next().unwrap();
        let secs = sections(root);
        // The chapter h1 sits in Quarto's <header>, which is skipped as text but read as a heading.
        assert!(secs.iter().all(|s| s.path.first().map(String::as_str) == Some("1 Pendahuluan")),
            "every section is under the chapter: {:?}", secs.iter().map(|s| &s.path).collect::<Vec<_>>());
        let prodi = secs.iter()
            .find(|s| s.path.last().map(String::as_str) == Some("1.1.4 Program Studi"))
            .expect("section 1.1.4");
        assert_eq!(prodi.path, ["1 Pendahuluan", "1.1 Identitas Prodi", "1.1.4 Program Studi"]);
        // h4 labels stay inside the section text
        assert!(prodi.text.contains("Visi") && prodi.text.contains("Misi"), "{}", prodi.text);
        // A later h2 closes the h3 above it
        let swot = secs.iter().find(|s| s.path.last().map(String::as_str) == Some("1.5 Analisis SWOT")).unwrap();
        assert_eq!(swot.path.len(), 2);
    }

    #[test]
    fn tables_are_labelled_with_their_section_below_chapter_level() {
        let doc = Html::parse_document(INTRODUCTION);
        let root = doc.select(&Selector::parse("main#quarto-document-content").unwrap()).next().unwrap();
        let swot = &html_tables(root)[0];
        assert_eq!(swot.section, ["1.5 Analisis SWOT"]);
        assert!(table_chunks("1 Pendahuluan", swot, 1200)[0].starts_with("1 Pendahuluan — 1.5 Analisis SWOT\n"));
        // A table directly under the chapter title keeps the E5 label (no repeated heading)
        let tendik = &json_tables(&Html::parse_document(D_TENDIK))[0];
        assert!(tendik.section.is_empty(), "{:?}", tendik.section);
    }

    #[test]
    fn quality_warnings_flag_known_problems() {
        let page = PageContent {
            title: "t".into(),
            text: "Showing 1 to 10 of 40 entries".into(),
            ..Default::default()
        };
        let chunks = vec![
            "jQuery(function($){ $(document).ready(function(){ var x = 1; }); });".to_string(),
            "Same text here, long enough to not be flagged as a very short chunk at all.".to_string(),
            "Same text here, long enough to not be flagged as a very short chunk at all.".to_string(),
        ];
        let w = quality_warnings(&page, &chunks, 1200).join("\n");
        assert!(w.contains("look like code"), "{w}");
        assert!(w.contains("Pagination text"), "{w}");
        assert!(w.contains("1 duplicate chunk."), "{w}");
        assert!(w.contains("Very little text"), "{w}");
    }

    #[test]
    fn sections_without_headings_is_one_section() {
        let doc = Html::parse_document("<body><main><p>Hello</p><p>world</p><script>x()</script></main></body>");
        let main = doc.select(&Selector::parse("main").unwrap()).next().unwrap();
        assert_eq!(sections(main), [Section { path: vec![], text: "Hello\nworld".into() }]);
    }

    #[test]
    fn visible_text_skips_scripts_and_tables() {
        let doc = Html::parse_document(
            "<body><main><p>Hello&nbsp;world</p><script>var x = 1;</script>\
             <table><tr><td>cell</td></tr></table><nav>menu</nav></main></body>",
        );
        let main = doc.select(&Selector::parse("main").unwrap()).next().unwrap();
        assert_eq!(visible_text(main), "Hello world");
    }
}

/// Prints what a saved page turns into — run with:
/// `cargo test --release show_page_chunks -- --ignored --nocapture`
#[cfg(test)]
mod show {
    use super::*;

    #[test]
    #[ignore = "prints example output; run explicitly"]
    fn show_page_chunks() {
        for (name, html, content) in [
            ("introduction", include_str!("../tests/fixtures/introduction.html"), "main#quarto-document-content"),
            ("d_tendik", include_str!("../tests/fixtures/d_tendik.html"), "main#quarto-document-content"),
            ("c_ewmp", include_str!("../tests/fixtures/c_ewmp.html"), "main#quarto-document-content"),
            ("tenaga_pendidik", include_str!("../tests/fixtures/tenaga_pendidik.html"), "article .post-content"),
        ] {
            let doc = Html::parse_document(html);
            let root = doc.select(&Selector::parse(content).unwrap()).next().unwrap();
            let mut tables = json_tables(&doc);
            tables.extend(html_tables(root));
            let secs = sections(root);
            println!("\n================ {name}: {} section(s), {} table(s) ================", secs.len(), tables.len());
            for s in secs.iter().take(8) {
                println!("[{}] {} chars: {}", s.path.join(" > "), s.text.len(), s.text.chars().take(80).collect::<String>().replace('\n', " / "));
            }
            for t in &tables {
                let chunks = table_chunks(name, t, 1200);
                println!("rows {} | headers {:?} | {} chunks", t.rows.len(), t.headers, chunks.len());
                for (i, c) in chunks.iter().take(2).enumerate() {
                    println!("--- chunk {i} ({} chars)\n{}", c.len(), c.chars().take(420).collect::<String>());
                }
            }
        }
    }
}
