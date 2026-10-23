//! Categories ("Progetto", "Richiesta", "Ordine"…) for sorting mail. Stored locally in
//! `labels.toml` next to the config, so they work with every server.
//!
//! A message is in a category when one of its keywords is in the subject (or, for keywords with
//! an `@`, in the sender's address), or when the user put it there by hand. The user can also
//! take a message out of a category its keywords would put it in. Messages are known by their
//! `Message-ID`, which survives moves between folders.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::MessageSummary;

const MAX_TAGGED: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    /// Stable id; never shown.
    pub id: String,
    pub name: String,
    /// Index into the UI palette.
    #[serde(default)]
    pub color: u8,
    /// Lowercase words or word beginnings ("ordin" finds "ordine" and "ordini"); entries with an
    /// `@` match the sender's address ("@fornitore.it").
    #[serde(default)]
    pub keywords: Vec<String>,
}

impl Category {
    /// Whether the keywords alone put `m` in this category.
    pub fn matches(&self, m: &MessageSummary) -> bool {
        if self.keywords.is_empty() {
            return false;
        }
        let subject = m.subject.to_lowercase();
        let from = m.from_addr.to_lowercase();
        self.keywords.iter().any(|k| {
            let k = k.trim().to_lowercase();
            if k.is_empty() {
                false
            } else if k.contains('@') {
                from.contains(&k)
            } else {
                starts_a_word(&subject, &k)
            }
        })
    }

    /// Keywords as the user types them: "ordin, order, ddt".
    pub fn keywords_text(&self) -> String {
        self.keywords.join(", ")
    }

    pub fn set_keywords_text(&mut self, text: &str) {
        self.keywords = text
            .split([',', ';', '\n'])
            .map(|k| k.trim().to_lowercase())
            .filter(|k| !k.is_empty())
            .collect();
    }
}

/// Whether `needle` occurs in `hay` at the start of a word.
fn starts_a_word(hay: &str, needle: &str) -> bool {
    let mut from = 0usize;
    while let Some(i) = hay.get(from..).and_then(|s| s.find(needle)) {
        let at = from.saturating_add(i);
        let before = hay.get(..at).and_then(|s| s.chars().next_back());
        if before.is_none_or(|c| !c.is_alphanumeric()) {
            return true;
        }
        from = at.saturating_add(needle.len().max(1));
        // `find` returns char boundaries; step past to the next one.
        while from < hay.len() && !hay.is_char_boundary(from) {
            from = from.saturating_add(1);
        }
    }
    false
}

/// A message the user classified by hand, with what is needed to list it while its folder is
/// not loaded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tagged {
    pub key: String,
    /// Categories added by hand.
    #[serde(default)]
    pub add: Vec<String>,
    /// Categories the keywords would give, taken away by hand.
    #[serde(default)]
    pub remove: Vec<String>,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub folder: String,
    /// 0 when unknown (after a move the server picks a new one).
    #[serde(default)]
    pub uid: u32,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub from_name: String,
    #[serde(default)]
    pub from_addr: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub timestamp: i64,
}

impl Tagged {
    /// A list row for this message (read, no preview) when its folder is not loaded.
    pub fn summary(&self) -> MessageSummary {
        MessageSummary {
            account_id: self.account_id.clone(),
            folder: self.folder.clone(),
            uid: self.uid,
            from_name: self.from_name.clone(),
            from_addr: self.from_addr.clone(),
            subject: self.subject.clone(),
            timestamp: self.timestamp,
            seen: true,
            answered: false,
            flagged: false,
            size: 0,
            message_id: self.message_id.clone(),
            snippet: String::new(),
            has_attachment: false,
        }
    }
}

/// The key a message is known by: its `Message-ID`, else account/folder/uid.
pub fn message_key(m: &MessageSummary) -> String {
    let id = id_key(&m.message_id);
    if id.is_empty() {
        format!("{}/{}/{}", m.account_id, m.folder, m.uid)
    } else {
        id
    }
}

/// `<AbC@x>` → `abc@x`: a `Message-ID` as a key.
pub fn id_key(message_id: &str) -> String {
    message_id
        .trim()
        .trim_matches(['<', '>'])
        .trim()
        .to_lowercase()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Labels {
    #[serde(default)]
    pub categories: Vec<Category>,
    #[serde(default)]
    pub tagged: Vec<Tagged>,
}

impl Labels {
    pub fn path() -> Result<std::path::PathBuf> {
        Ok(Config::dir()?.join("labels.toml"))
    }

    /// Missing file = the starter categories. A corrupt file is an error (never wiped).
    pub fn load() -> Result<Labels> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Labels::starter());
        }
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
    }

    /// Atomic write: temp file then rename.
    pub fn save(&self) -> Result<()> {
        let dir = Config::dir()?;
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join("labels.toml.tmp");
        let text = toml::to_string(self).map_err(|e| Error::Config(e.to_string()))?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, dir.join("labels.toml"))?;
        Ok(())
    }

    /// The categories a new install starts with; the user edits them in Impostazioni.
    pub fn starter() -> Labels {
        let cat = |id: &str, name: &str, color: u8, words: &str| {
            let mut c = Category {
                id: id.into(),
                name: name.into(),
                color,
                keywords: Vec::new(),
            };
            c.set_keywords_text(words);
            c
        };
        Labels {
            categories: vec![
                cat("progetto", "Progetto", 4, "progett, project"),
                cat(
                    "richiesta",
                    "Richiesta",
                    0,
                    "richiest, request, informazioni, chiarimenti",
                ),
                cat(
                    "ordine",
                    "Ordine",
                    1,
                    "ordin, order, conferma d'ordine, ddt, spedizion, tracking",
                ),
                cat(
                    "preventivo",
                    "Preventivo",
                    6,
                    "preventiv, offert, quotation, quote, rfq, listino",
                ),
                cat(
                    "fattura",
                    "Fattura",
                    2,
                    "fattur, invoice, nota di credito, bonific, pagament, scadenz",
                ),
                cat(
                    "assistenza",
                    "Assistenza",
                    3,
                    "assistenz, guast, ticket, intervent, support, anomali",
                ),
            ],
            tagged: Vec::new(),
        }
    }

    pub fn category(&self, id: &str) -> Option<&Category> {
        self.categories.iter().find(|c| c.id == id)
    }

    fn tagged_for(&self, key: &str) -> Option<&Tagged> {
        self.tagged.iter().find(|t| t.key == key)
    }

    /// Ids of the categories `m` is in, in the order of the category list.
    pub fn of(&self, m: &MessageSummary) -> Vec<&Category> {
        let key = message_key(m);
        let t = self.tagged_for(&key);
        self.categories
            .iter()
            .filter(|c| {
                let added = t.is_some_and(|t| t.add.contains(&c.id));
                let removed = t.is_some_and(|t| t.remove.contains(&c.id));
                added || (!removed && c.matches(m))
            })
            .collect()
    }

    pub fn has(&self, m: &MessageSummary, category: &str) -> bool {
        self.of(m).iter().any(|c| c.id == category)
    }

    /// Put `m` in the category, or take it out. Returns whether it is in it now.
    pub fn toggle(&mut self, m: &MessageSummary, category: &str) -> bool {
        let inside = self.has(m, category);
        let auto = self.category(category).is_some_and(|c| c.matches(m));
        let key = message_key(m);
        let pos = match self.tagged.iter().position(|t| t.key == key) {
            Some(p) => p,
            None => {
                self.tagged.push(Tagged {
                    key: key.clone(),
                    ..Tagged::default()
                });
                self.tagged.len().saturating_sub(1)
            }
        };
        let Some(t) = self.tagged.get_mut(pos) else {
            return inside;
        };
        t.add.retain(|c| c != category);
        t.remove.retain(|c| c != category);
        if inside && auto {
            t.remove.push(category.to_string());
        } else if !inside && !auto {
            t.add.push(category.to_string());
        }
        t.account_id = m.account_id.clone();
        t.folder = m.folder.clone();
        t.uid = m.uid;
        t.message_id = m.message_id.clone();
        t.from_name = m.from_name.clone();
        t.from_addr = m.from_addr.clone();
        t.subject = m.subject.clone();
        t.timestamp = m.timestamp;
        let empty = t.add.is_empty() && t.remove.is_empty();
        if empty {
            self.tagged.remove(pos);
        }
        self.trim();
        !inside
    }

    /// The message moved: remember its new folder (its uid there is not known yet).
    pub fn moved(&mut self, m: &MessageSummary, to: &str) -> bool {
        let key = message_key(m);
        match self.tagged.iter_mut().find(|t| t.key == key) {
            Some(t) => {
                t.folder = to.to_string();
                t.uid = 0;
                true
            }
            None => false,
        }
    }

    /// Messages added to `category` by hand (to list them while their folder is not loaded).
    pub fn added_to<'a>(&'a self, category: &'a str) -> impl Iterator<Item = &'a Tagged> + 'a {
        self.tagged
            .iter()
            .filter(move |t| t.add.iter().any(|c| c == category))
    }

    /// A fresh category with a free id and the next colour.
    pub fn add_category(&mut self, name: &str) -> String {
        let base: String = name
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        let base = if base.trim_matches('-').is_empty() {
            "categoria".to_string()
        } else {
            base
        };
        let mut id = base.clone();
        let mut n = 2u32;
        while self.category(&id).is_some() {
            id = format!("{base}-{n}");
            n = n.saturating_add(1);
        }
        let color = u8::try_from(self.categories.len() % 8).unwrap_or(0);
        self.categories.push(Category {
            id: id.clone(),
            name: name.trim().to_string(),
            color,
            keywords: Vec::new(),
        });
        id
    }

    /// Delete a category and every hand-made assignment to it.
    pub fn remove_category(&mut self, id: &str) {
        self.categories.retain(|c| c.id != id);
        for t in &mut self.tagged {
            t.add.retain(|c| c != id);
            t.remove.retain(|c| c != id);
        }
        self.tagged
            .retain(|t| !(t.add.is_empty() && t.remove.is_empty()));
    }

    /// Keep the file bounded: drop the oldest assignments beyond the cap.
    fn trim(&mut self) {
        if self.tagged.len() > MAX_TAGGED {
            self.tagged.sort_by_key(|t| std::cmp::Reverse(t.timestamp));
            self.tagged.truncate(MAX_TAGGED);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(subject: &str, from: &str, id: &str) -> MessageSummary {
        MessageSummary {
            account_id: "a".into(),
            folder: "INBOX".into(),
            uid: 7,
            from_name: String::new(),
            from_addr: from.into(),
            subject: subject.into(),
            timestamp: 100,
            seen: false,
            answered: false,
            flagged: false,
            size: 0,
            message_id: id.into(),
            snippet: String::new(),
            has_attachment: false,
        }
    }

    #[test]
    fn keywords_match_word_starts_and_senders() {
        let l = Labels::starter();
        let names = |m: &MessageSummary| -> Vec<String> {
            l.of(m).iter().map(|c| c.name.clone()).collect()
        };
        assert_eq!(
            names(&msg("Conferma ORDINI n. 44", "x@y.it", "1")),
            vec!["Ordine"]
        );
        assert_eq!(
            names(&msg("Re: Fattura 12/2026", "x@y.it", "2")),
            vec!["Fattura"]
        );
        // "coordinamento" contains "ordin" but not at the start of a word.
        assert!(names(&msg("Coordinamento cantiere", "x@y.it", "3")).is_empty());
        let mut c = Category {
            id: "p".into(),
            name: "Progetto Alfa".into(),
            color: 0,
            keywords: Vec::new(),
        };
        c.set_keywords_text("alfa; @Partner.example");
        assert!(c.matches(&msg("Aggiornamento", "anna@partner.example", "4")));
        assert!(c.matches(&msg("Report ALFA", "x@y.it", "5")));
        assert!(!c.matches(&msg("Altro", "x@y.it", "6")));
    }

    #[test]
    fn toggle_adds_removes_and_overrides_keywords() {
        let mut l = Labels::starter();
        let m = msg("Ciao", "x@y.it", "<AbC@x>");
        assert!(!l.has(&m, "progetto"));
        assert!(l.toggle(&m, "progetto"));
        assert!(l.has(&m, "progetto"));
        assert_eq!(l.added_to("progetto").count(), 1);
        assert_eq!(l.tagged.first().map(|t| t.key.as_str()), Some("abc@x"));
        assert!(!l.toggle(&m, "progetto"));
        assert!(l.tagged.is_empty());
        // Keyword match taken away by hand, then given back.
        let o = msg("Ordine 5", "x@y.it", "o@x");
        assert!(l.has(&o, "ordine"));
        assert!(!l.toggle(&o, "ordine"));
        assert!(!l.has(&o, "ordine"));
        assert!(l.toggle(&o, "ordine"));
        assert!(l.has(&o, "ordine"));
        assert!(l.tagged.is_empty());
    }

    #[test]
    fn categories_roundtrip_and_remove() {
        let mut l = Labels::starter();
        let id = l.add_category("Progetto acDC");
        assert_eq!(id, "progetto-acdc");
        assert_eq!(l.add_category("Progetto acDC"), "progetto-acdc-2");
        let m = msg("x", "a@b.it", "z@z");
        l.toggle(&m, &id);
        let text = toml::to_string(&l).unwrap();
        let back: Labels = toml::from_str(&text).unwrap();
        assert!(back.has(&m, &id));
        l.remove_category(&id);
        assert!(l.tagged.is_empty());
        assert!(!starts_a_word("àèì ordine", "ordinez"));
        assert!(starts_a_word("àèì ordine", "ordin"));
    }
}
