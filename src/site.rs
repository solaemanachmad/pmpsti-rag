//! Web sites as sources: which pages belong to a site, and which site a page belongs to.
//! Pure functions (no network) so they are unit tested on saved pages in tests/fixtures/.
//! Design: docs/design/ingest-from-links.md §3 (web_site / web_page).

use reqwest::Url;

/// Normalise a page URL for storage and comparison: no fragment, no trailing "index.html".
pub fn normalize_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw.trim()).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    url.set_fragment(None);
    let mut s = url.to_string();
    for suffix in ["index.html", "index.htm", "index.php"] {
        if s.ends_with(&format!("/{suffix}")) {
            s.truncate(s.len() - suffix.len());
        }
    }
    Some(s)
}

/// Root of a site as a directory URL ending in "/" (what pages are matched against).
pub fn normalize_root(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw.trim()).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    // Keep the directory: "/a/page.html" → "/a/", "/panduan_pmpsti" → "/panduan_pmpsti/"
    let path = url.path().to_string();
    let last = path.rsplit('/').next().unwrap_or("");
    let dir = if path.ends_with('/') {
        path
    } else if last.contains('.') {
        path[..path.len() - last.len()].to_string()
    } else {
        format!("{path}/")
    };
    url.set_path(&dir);
    Some(url.to_string())
}

/// True if `page` lies under `root` (same scheme/host, path prefix).
pub fn is_under(page: &str, root: &str) -> bool {
    match (normalize_url(page), normalize_root(root)) {
        (Some(p), Some(r)) => p.starts_with(&r),
        _ => false,
    }
}

/// The site (out of `roots`, as (id, root)) a page belongs to: the longest matching root.
pub fn parent_for<'a, T: Copy>(page: &str, roots: &'a [(T, String)]) -> Option<(T, &'a str)> {
    roots.iter()
        .filter(|(_, root)| is_under(page, root))
        .max_by_key(|(_, root)| root.len())
        .map(|(id, root)| (*id, root.as_str()))
}

const SKIP_EXT: &[&str] = &[
    ".pdf", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".zip", ".docx", ".xlsx", ".pptx",
    ".css", ".js", ".json", ".xml", ".ico", ".mp4", ".mp3",
];

/// Links on a page that point to other pages under `root` (normalised, de-duplicated).
pub fn page_links(html: &str, page_url: &str, root: &str) -> Vec<String> {
    let Ok(base) = Url::parse(page_url) else { return vec![] };
    let doc = scraper::Html::parse_document(html);
    let Ok(sel) = scraper::Selector::parse("a[href]") else { return vec![] };
    let mut out: Vec<String> = Vec::new();
    for a in doc.select(&sel) {
        let href = a.value().attr("href").unwrap_or("").trim();
        if href.is_empty() || href.starts_with('#') || href.starts_with("mailto:") || href.starts_with("javascript:") {
            continue;
        }
        let Ok(mut u) = base.join(href) else { continue };
        u.set_query(None);
        let path = u.path().to_lowercase();
        if SKIP_EXT.iter().any(|ext| path.ends_with(ext)) {
            continue;
        }
        if let Some(n) = normalize_url(u.as_str()) {
            if is_under(&n, root) && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

/// `<loc>` entries of a sitemap.xml that lie under `root`.
pub fn sitemap_pages(xml: &str, root: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in xml.split("<loc>").skip(1) {
        let Some(end) = part.find("</loc>") else { continue };
        let loc = part[..end].trim().replace("&amp;", "&");
        if let Some(n) = normalize_url(&loc) {
            let lower = n.to_lowercase();
            if is_under(&n, root) && !SKIP_EXT.iter().any(|e| lower.ends_with(e)) && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SiteGuess {
    pub root:      String,
    pub site_name: String,
}

/// Guess the site a page belongs to: the "home" link of the page (navbar brand, rel=home,
/// or the deepest link to an ancestor directory such as "../index.html"), and the site
/// name from og:site_name or the part of <title> after " – ".
pub fn guess_site(html: &str, page_url: &str) -> Option<SiteGuess> {
    let page = normalize_url(page_url)?;
    let base = Url::parse(&page).ok()?;
    let doc = scraper::Html::parse_document(html);

    let resolve = |href: &str| base.join(href).ok().and_then(|u| normalize_root(u.as_str()));
    let ancestor = |root: &str| page.starts_with(root) && page.len() > root.len();

    let mut root: Option<String> = None;
    for css in ["a.navbar-brand[href]", "a[rel~=home][href]", "link[rel~=home][href]"] {
        let Ok(sel) = scraper::Selector::parse(css) else { continue };
        if let Some(r) = doc.select(&sel)
            .filter_map(|e| e.value().attr("href").and_then(resolve))
            .find(|r| ancestor(r))
        {
            root = Some(r);
            break;
        }
    }
    if root.is_none() {
        // deepest link that points at an ancestor directory index ("../index.html", "./", "/")
        let sel = scraper::Selector::parse("a[href]").ok()?;
        root = doc.select(&sel)
            .filter_map(|e| e.value().attr("href"))
            .filter(|h| {
                let h = h.split(['?', '#']).next().unwrap_or("");
                h.ends_with('/') || h.ends_with("index.html") || h.ends_with("index.php") || h == ".." || h == "."
            })
            .filter_map(resolve)
            .filter(|r| ancestor(r))
            .max_by_key(|r| r.len());
    }
    let root = root.or_else(|| {
        // fallback: first path segment ("https://host/panduan_pmpsti/") or the host
        let first = base.path_segments().and_then(|mut s| s.next()).unwrap_or("");
        let host_root = format!("{}://{}/", base.scheme(), base.host_str()?);
        if !first.is_empty() && !first.contains('.') {
            Some(format!("{host_root}{first}/"))
        } else {
            Some(host_root)
        }
    })?;

    let meta_name = scraper::Selector::parse("meta[property='og:site_name']").ok()
        .and_then(|sel| doc.select(&sel).next())
        .and_then(|e| e.value().attr("content").map(str::to_string));
    let title_name = scraper::Selector::parse("title").ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|e| e.text().collect::<String>().replace('\u{a0}', " "))
        .and_then(|t| t.rsplit_once(" – ").or_else(|| t.rsplit_once(" | ")).map(|(_, s)| s.trim().to_string()));
    let site_name = meta_name.or(title_name).unwrap_or_default();

    Some(SiteGuess { root, site_name })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUIDE: &str = "https://mtiugm.github.io/panduan_pmpsti/";

    #[test]
    fn normalises_urls_and_roots() {
        assert_eq!(normalize_url("https://x.id/a/index.html#top").unwrap(), "https://x.id/a/");
        assert_eq!(normalize_root("https://mtiugm.github.io/panduan_pmpsti").unwrap(), GUIDE);
        assert_eq!(normalize_root("https://mtiugm.github.io/panduan_pmpsti/index.html").unwrap(), GUIDE);
        assert_eq!(normalize_root("https://x.id/a/page.html").unwrap(), "https://x.id/a/");
        assert_eq!(normalize_root("https://x.id/a/page.php?id=3").unwrap(), "https://x.id/a/");
        assert_eq!(normalize_root("https://x.id/?lang=id").unwrap(), "https://x.id/");
        assert!(normalize_url("ftp://x").is_none());
    }

    #[test]
    fn longest_parent_wins() {
        let roots = vec![(1, "https://mtiugm.github.io/".to_string()), (2, GUIDE.to_string())];
        let page = "https://mtiugm.github.io/panduan_pmpsti/chapter1/introduction.html";
        assert_eq!(parent_for(page, &roots).map(|p| p.0), Some(2));
        assert_eq!(parent_for("https://jteti.ugm.ac.id/tenaga-pendidik/", &roots), None);
        assert!(!is_under("https://mtiugm.github.io/panduan_pmpsti_old/x.html", GUIDE));
    }

    #[test]
    fn guesses_quarto_guidebook_root_and_name() {
        let html = std::fs::read_to_string("tests/fixtures/introduction.html").unwrap();
        let g = guess_site(&html, "https://mtiugm.github.io/panduan_pmpsti/chapter1/introduction.html").unwrap();
        assert_eq!(g.root, GUIDE);
        assert_eq!(g.site_name, "Buku Panduan Akademik PMPSTI");
    }

    #[test]
    fn guesses_from_the_address_alone() {
        let g = guess_site("", "https://mtiugm.github.io/panduan_pmpsti/chapter4/thesis.html").unwrap();
        assert_eq!(g.root, GUIDE);
        // a top-level page: the guess is the page itself, so /resolve offers "standalone page"
        let g = guess_site("", "https://jteti.ugm.ac.id/tenaga-pendidik/").unwrap();
        assert_eq!(g.root, "https://jteti.ugm.ac.id/tenaga-pendidik/");
    }

    #[test]
    fn finds_sidebar_pages_under_the_root() {
        let html = std::fs::read_to_string("tests/fixtures/introduction.html").unwrap();
        let links = page_links(&html, "https://mtiugm.github.io/panduan_pmpsti/chapter1/introduction.html", GUIDE);
        assert!(links.contains(&"https://mtiugm.github.io/panduan_pmpsti/front/b_preface.html".to_string()));
        assert!(links.contains(&GUIDE.to_string())); // ../index.html
        assert!(links.iter().all(|l| l.starts_with(GUIDE)));
    }

    #[test]
    fn reads_sitemaps() {
        let xml = "<urlset><url><loc>https://mtiugm.github.io/panduan_pmpsti/index.html</loc></url>\
                   <url><loc>https://mtiugm.github.io/panduan_pmpsti/chapter4/thesis.html</loc></url>\
                   <url><loc>https://mtiugm.github.io/other/x.html</loc></url>\
                   <url><loc>https://mtiugm.github.io/panduan_pmpsti/files/a.pdf</loc></url></urlset>";
        assert_eq!(sitemap_pages(xml, GUIDE), vec![
            GUIDE.to_string(),
            "https://mtiugm.github.io/panduan_pmpsti/chapter4/thesis.html".to_string(),
        ]);
    }
}
