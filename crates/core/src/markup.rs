//! The compose editor's light markup → HTML. Deliberately small: headings (`# `, `## `),
//! bullet and numbered lists, **bold**, *italic*, `[text](url)` links and bare URLs. Blank lines
//! separate paragraphs; single newlines become line breaks. The plain-text alternative of a
//! message is the markup itself, which reads fine as text.

use crate::smtp_client::escape_html;

/// Does the text use any markup worth rendering?
pub fn has_markup(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("# ")
            || t.starts_with("## ")
            || t.starts_with("- ")
            || t.starts_with("* ")
            || starts_with_number_dot(t)
    }) || text.contains("**")
        || text.contains("](")
        || text.contains("http://")
        || text.contains("https://")
        || has_italic(text)
}

fn starts_with_number_dot(t: &str) -> bool {
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    !digits.is_empty() && t.get(digits.len()..).is_some_and(|r| r.starts_with(". "))
}

fn has_italic(text: &str) -> bool {
    // a single * with a non-space after it and a closing one later on the same line
    text.lines().any(|l| {
        let l = l.replace("**", "");
        let mut it = l.match_indices('*');
        matches!((it.next(), it.next()), (Some((a, _)), Some((b, _))) if b > a + 1)
    })
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Block {
    None,
    Para,
    Ul,
    Ol,
}

/// Render the markup as a self-contained HTML fragment.
pub fn to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len().saturating_mul(2));
    let mut block = Block::None;
    let close = |out: &mut String, block: Block| match block {
        Block::Para => out.push_str("</p>\n"),
        Block::Ul => out.push_str("</ul>\n"),
        Block::Ol => out.push_str("</ol>\n"),
        Block::None => {}
    };
    for raw in text.lines() {
        let line = raw.trim_end();
        let t = line.trim_start();
        if t.is_empty() {
            close(&mut out, block);
            block = Block::None;
            continue;
        }
        if let Some(h) = t.strip_prefix("## ") {
            close(&mut out, block);
            block = Block::None;
            out.push_str(&format!("<h2>{}</h2>\n", inline(h)));
            continue;
        }
        if let Some(h) = t.strip_prefix("# ") {
            close(&mut out, block);
            block = Block::None;
            out.push_str(&format!("<h1>{}</h1>\n", inline(h)));
            continue;
        }
        if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            if block != Block::Ul {
                close(&mut out, block);
                out.push_str("<ul>\n");
                block = Block::Ul;
            }
            out.push_str(&format!("<li>{}</li>\n", inline(item)));
            continue;
        }
        if starts_with_number_dot(t) {
            let item = t.split_once(". ").map(|(_, r)| r).unwrap_or("");
            if block != Block::Ol {
                close(&mut out, block);
                out.push_str("<ol>\n");
                block = Block::Ol;
            }
            out.push_str(&format!("<li>{}</li>\n", inline(item)));
            continue;
        }
        match block {
            Block::Para => {
                out.push_str("<br>\n");
                out.push_str(&inline(t));
            }
            _ => {
                close(&mut out, block);
                out.push_str("<p>");
                out.push_str(&inline(t));
                block = Block::Para;
            }
        }
    }
    close(&mut out, block);
    format!("<div style=\"font-family:sans-serif;font-size:14px;line-height:1.45\">\n{out}</div>")
}

/// Inline markup on one line: links, bold, italic. Everything else is escaped.
fn inline(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut bold = false;
    let mut italic = false;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        // [text](url)
        if c == '['
            && let Some(close) = find(&chars, i, ']')
            && chars.get(close + 1) == Some(&'(')
            && let Some(end) = find(&chars, close + 1, ')')
        {
            let text: String = chars.get(i + 1..close).unwrap_or(&[]).iter().collect();
            let url: String = chars.get(close + 2..end).unwrap_or(&[]).iter().collect();
            if is_url(&url) {
                out.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    escape_html(&url),
                    escape_html(&text)
                ));
                i = end + 1;
                continue;
            }
        }
        // **bold**
        if c == '*' && chars.get(i + 1) == Some(&'*') {
            out.push_str(if bold { "</b>" } else { "<b>" });
            bold = !bold;
            i += 2;
            continue;
        }
        // *italic*
        if c == '*' {
            let next_is_space = chars.get(i + 1).is_none_or(|c| c.is_whitespace());
            if italic || !next_is_space {
                out.push_str(if italic { "</i>" } else { "<i>" });
                italic = !italic;
                i += 1;
                continue;
            }
        }
        // bare URL
        if (c == 'h') && (rest_starts(&chars, i, "http://") || rest_starts(&chars, i, "https://")) {
            let mut j = i;
            while j < chars.len() && !chars.get(j).is_none_or(|c| c.is_whitespace()) {
                j += 1;
            }
            let mut url: String = chars.get(i..j).unwrap_or(&[]).iter().collect();
            while url.ends_with(['.', ',', ')', ';']) {
                url.pop();
                j -= 1;
            }
            out.push_str(&format!("<a href=\"{0}\">{0}</a>", escape_html(&url)));
            i = j;
            continue;
        }
        out.push_str(&escape_html(&c.to_string()));
        i += 1;
    }
    if italic {
        out.push_str("</i>");
    }
    if bold {
        out.push_str("</b>");
    }
    out
}

fn find(chars: &[char], from: usize, what: char) -> Option<usize> {
    chars
        .get(from + 1..)?
        .iter()
        .position(|c| *c == what)
        .map(|p| from + 1 + p)
}

fn rest_starts(chars: &[char], at: usize, pat: &str) -> bool {
    let rest: String = chars
        .get(at..)
        .unwrap_or(&[])
        .iter()
        .take(pat.chars().count())
        .collect();
    rest == pat
}

fn is_url(u: &str) -> bool {
    u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_blocks_and_inline() {
        let md = "# Titolo\n\nCiao **Anna**, vedi *qui*.\nSeconda riga\n\n- uno\n- due\n\n1. a\n2. b\n\nSito: https://example.it/x, grazie [link](https://a.b/c)";
        let h = to_html(md);
        assert!(h.contains("<h1>Titolo</h1>"));
        assert!(
            h.contains("<p>Ciao <b>Anna</b>, vedi <i>qui</i>.<br>\nSeconda riga</p>"),
            "{h}"
        );
        assert!(h.contains("<ul>\n<li>uno</li>\n<li>due</li>\n</ul>"));
        assert!(h.contains("<ol>\n<li>a</li>\n<li>b</li>\n</ol>"));
        assert!(h.contains("<a href=\"https://example.it/x\">https://example.it/x</a>,"));
        assert!(h.contains("<a href=\"https://a.b/c\">link</a>"));
    }

    #[test]
    fn escapes_and_detects() {
        let h = to_html("a < b & c");
        assert!(h.contains("a &lt; b &amp; c"));
        assert!(!has_markup("solo testo\nnormale 2 * 3"));
        assert!(has_markup("**x**"));
        assert!(has_markup("- lista"));
        assert!(has_markup("3. terzo"));
        assert!(has_markup("vedi *questo* qui"));
        assert!(has_markup("https://x.it"));
    }

    #[test]
    fn garbage_never_panics() {
        for s in [
            "",
            "*",
            "**",
            "[",
            "[a](",
            "[a](b",
            "# ",
            "1. ",
            "***",
            "\u{ff}\n\n\n*a",
        ] {
            let _ = to_html(s);
            let _ = has_markup(s);
        }
    }
}
