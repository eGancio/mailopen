//! Address book built from the mail that goes through the app: every sender and recipient is
//! remembered and offered as a suggestion while typing. Stored as TOML next to the config.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{Error, Result};

const MAX_CONTACTS: usize = 5000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub name: String,
    pub email: String,
    /// How many times it was seen or used.
    pub uses: u32,
    /// Unix seconds of the last time.
    pub last: i64,
}

impl Contact {
    /// "Name <email>" or the bare address.
    pub fn display(&self) -> String {
        if self.name.trim().is_empty() {
            self.email.clone()
        } else {
            format!("{} <{}>", self.name.trim(), self.email)
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Contacts {
    #[serde(default)]
    pub contacts: Vec<Contact>,
}

impl Contacts {
    pub fn path() -> Result<std::path::PathBuf> {
        Ok(Config::dir()?.join("contacts.toml"))
    }

    pub fn load() -> Result<Contacts> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Contacts::default());
        }
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
    }

    pub fn save(&self) -> Result<()> {
        let dir = Config::dir()?;
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join("contacts.toml.tmp");
        let text = toml::to_string(self).map_err(|e| Error::Config(e.to_string()))?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, dir.join("contacts.toml"))?;
        Ok(())
    }

    /// "Name <a@b.it>", "<a@b.it>" or "a@b.it" → (name, email). None when there is no address.
    pub fn parse_entry(s: &str) -> Option<(String, String)> {
        let s = s.trim();
        let (name, email) = match (s.rfind('<'), s.ends_with('>')) {
            (Some(i), true) => {
                let name = s
                    .get(..i)
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string();
                let email = s
                    .get(i + 1..s.len().saturating_sub(1))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                (name, email)
            }
            _ => (String::new(), s.to_string()),
        };
        if !email.contains('@') || email.contains(char::is_whitespace) {
            return None;
        }
        Some((name, email.to_ascii_lowercase()))
    }

    /// Remember an address. Returns true when something changed. Automatic senders are skipped.
    pub fn remember(&mut self, name: &str, email: &str, now: i64) -> bool {
        let email = email.trim().to_ascii_lowercase();
        if !email.contains('@') {
            return false;
        }
        let local = email.split('@').next().unwrap_or("");
        if [
            "noreply",
            "no-reply",
            "no_reply",
            "donotreply",
            "mailer-daemon",
            "bounce",
        ]
        .iter()
        .any(|p| local.starts_with(p))
        {
            return false;
        }
        let name = name.trim();
        if let Some(c) = self.contacts.iter_mut().find(|c| c.email == email) {
            c.uses = c.uses.saturating_add(1);
            c.last = c.last.max(now);
            if c.name.is_empty() && !name.is_empty() {
                c.name = name.to_string();
            }
            return true;
        }
        self.contacts.push(Contact {
            name: name.to_string(),
            email,
            uses: 1,
            last: now,
        });
        if self.contacts.len() > MAX_CONTACTS {
            self.contacts.sort_by_key(|c| std::cmp::Reverse(c.last));
            self.contacts.truncate(MAX_CONTACTS);
        }
        true
    }

    /// Remember every address of a "a@b.it, Name <c@d.it>" list.
    pub fn remember_list(&mut self, entries: &[String], now: i64) -> bool {
        let mut changed = false;
        for e in entries {
            for part in crate::smtp_client::split_addresses(e) {
                if let Some((n, m)) = Self::parse_entry(&part) {
                    changed |= self.remember(&n, &m, now);
                }
            }
        }
        changed
    }

    /// Best matches for what was typed so far: name or address containing the text,
    /// most used and most recent first.
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<Contact> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<&Contact> = self
            .contacts
            .iter()
            .filter(|c| c.email.contains(&q) || c.name.to_lowercase().contains(&q))
            .collect();
        hits.sort_by(|a, b| {
            let pa = a.email.starts_with(&q) || a.name.to_lowercase().starts_with(&q);
            let pb = b.email.starts_with(&q) || b.name.to_lowercase().starts_with(&q);
            pb.cmp(&pa)
                .then(b.uses.cmp(&a.uses))
                .then(b.last.cmp(&a.last))
        });
        hits.into_iter().take(limit).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_remember() {
        assert_eq!(
            Contacts::parse_entry("Anna Rossi <Anna@Example.it>"),
            Some(("Anna Rossi".into(), "anna@example.it".into()))
        );
        assert_eq!(
            Contacts::parse_entry("bo@example.it"),
            Some((String::new(), "bo@example.it".into()))
        );
        assert_eq!(Contacts::parse_entry("non valido"), None);
        let mut c = Contacts::default();
        assert!(c.remember("Anna", "anna@example.it", 10));
        assert!(c.remember("", "ANNA@example.it", 20));
        assert!(!c.remember("x", "noreply@example.it", 1));
        assert_eq!(c.contacts.len(), 1);
        assert_eq!(c.contacts.first().map(|x| x.uses), Some(2));
        assert_eq!(c.contacts.first().map(|x| x.last), Some(20));
        assert!(c.remember_list(&["Bo <bo@example.it>, carla@example.it".into()], 30));
        assert_eq!(c.contacts.len(), 3);
    }

    #[test]
    fn suggest_prefers_prefix_and_usage() {
        let mut c = Contacts::default();
        c.remember("Marco Verdi", "marco@verdi.it", 1);
        c.remember("Anna Marcolini", "anna@example.it", 5);
        c.remember("Anna Marcolini", "anna@example.it", 6);
        let s = c.suggest("mar", 5);
        assert_eq!(s.first().map(|x| x.email.as_str()), Some("marco@verdi.it"));
        assert_eq!(s.len(), 2);
        assert!(c.suggest("", 5).is_empty());
        assert!(c.suggest("zzz", 5).is_empty());
        let text = c.contacts.first().map(|x| x.display()).unwrap_or_default();
        assert!(text.contains('<'));
    }
}
