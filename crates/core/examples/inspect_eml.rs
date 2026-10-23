//! Parse `.eml` files the way the app does and print what it would show: attachments, inline
//! images, the list preview (from the first bytes only, like the real list) and the text.
//!
//! ```sh
//! cargo run -p mailopen-core --example inspect_eml -- /tmp/eml/*.eml
//! ```

use mailopen_core::imap_client::{parse_body, snippet_from};

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(raw) = std::fs::read(&path) else {
            println!("{path}: non leggibile");
            continue;
        };
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(raw.len());
        let (header, body) = raw.split_at(split);
        let partial = body.get(..6144).unwrap_or(body);
        let b = parse_body(&raw);
        println!("== {path}");
        println!(
            "   html: {}  blocchi: {}  immagini remote: {}",
            b.is_html,
            b.doc.blocks.len(),
            b.doc.remote_images
        );
        for a in &b.attachments {
            println!(
                "   allegato: {} [{}] {} KB {:?}",
                a.name,
                a.mime,
                a.size() / 1024,
                a.kind()
            );
        }
        if !b.inline.is_empty() {
            println!("   inline: {}", b.inline.len());
        }
        println!("   anteprima: {}", snippet_from(header, partial));
        let text: String = b.text.chars().take(160).collect();
        println!("   testo: {}", text.replace('\n', " / "));
        if let Ok(n) = std::env::var("DUMP") {
            let dump = format!("{:#?}", b.doc.blocks);
            let n: usize = n.parse().unwrap_or(4000);
            println!("{}", dump.chars().take(n).collect::<String>());
        }
    }
}
