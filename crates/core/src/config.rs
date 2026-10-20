//! Account list, stored as TOML in the platform config directory.
//! Passwords never go here: see [`crate::secrets`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SmtpSecurity {
    /// Implicit TLS, usually port 465.
    #[default]
    Tls,
    /// STARTTLS on a plain connection, usually port 587.
    StartTls,
}

/// How the signature is written: in the plain editor, or pasted as HTML.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SignatureKind {
    #[default]
    Text,
    Html,
}

/// Light or dark window, or follow the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Stable id, also the keyring entry name. Never shown to the user.
    pub id: String,
    /// Display name ("Lavoro", "Personale").
    pub name: String,
    /// Address used in `From:`.
    pub email: String,
    /// Login user name (defaults to the address).
    pub username: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    #[serde(default)]
    pub smtp_security: SmtpSecurity,
    /// Index into the UI palette; keeps the same colour between runs.
    #[serde(default)]
    pub color: u8,
    /// Appended to new messages and replies (plain text, may be several lines).
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub signature_kind: SignatureKind,
    /// Pasted HTML signature, sent as the HTML alternative of the message.
    #[serde(default)]
    pub signature_html: String,
    /// Colour style used while this account is selected; `None` = the general one.
    #[serde(default)]
    pub style: Option<String>,
}

impl Account {
    /// A fresh id from the address plus the clock; unique enough for a config file.
    pub fn new_id(email: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let slug: String = email
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        format!("{slug}-{nanos:x}")
    }

    /// Short label for avatars: the first letter of the name, or of the address.
    pub fn initial(&self) -> String {
        self.name
            .trim()
            .chars()
            .next()
            .or_else(|| self.email.chars().next())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// Seconds between automatic refreshes of the open folder.
    #[serde(default = "default_refresh")]
    pub refresh_seconds: u64,
    /// How many recent messages to load per folder.
    #[serde(default = "default_page")]
    pub messages_per_folder: u32,
    #[serde(default)]
    pub theme: ThemePref,
    /// Colour style name (see the app's palette list). Unknown names fall back to the first.
    #[serde(default)]
    pub style: String,
    /// Fetch sender logos (Gravatar for the address, then the domain's favicon).
    #[serde(default = "default_true")]
    pub sender_logos: bool,
}

fn default_true() -> bool {
    true
}

fn default_refresh() -> u64 {
    120
}
fn default_page() -> u32 {
    60
}

impl Config {
    /// `$MAILOPEN_CONFIG_DIR` or the platform config directory
    /// (macOS: `~/Library/Application Support/mailopen`).
    pub fn dir() -> Result<PathBuf> {
        if let Some(dir) = std::env::var_os("MAILOPEN_CONFIG_DIR") {
            return Ok(PathBuf::from(dir));
        }
        directories::ProjectDirs::from("", "", "mailopen")
            .map(|d| d.config_dir().to_path_buf())
            .ok_or_else(|| Error::Config("cartella di configurazione non trovata".into()))
    }

    pub fn path() -> Result<PathBuf> {
        Ok(Self::dir()?.join("config.toml"))
    }

    /// Missing file = empty config. A corrupt file is an error (never silently wiped).
    pub fn load() -> Result<Config> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Config {
                accounts: Vec::new(),
                refresh_seconds: default_refresh(),
                messages_per_folder: default_page(),
                theme: ThemePref::System,
                style: String::new(),
                sender_logos: true,
            });
        }
        let text = std::fs::read_to_string(&path)?;
        let mut cfg: Config =
            toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
        cfg.refresh_seconds = cfg.refresh_seconds.clamp(20, 3600);
        cfg.messages_per_folder = cfg.messages_per_folder.clamp(10, 500);
        Ok(cfg)
    }

    /// Atomic write: temp file then rename.
    pub fn save(&self) -> Result<()> {
        let dir = Self::dir()?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("config.toml");
        let tmp = dir.join("config.toml.tmp");
        let text = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn remove(&mut self, id: &str) {
        self.accounts.retain(|a| a.id != id);
    }

    /// First palette index not used yet (wraps after the palette size).
    pub fn next_color(&self) -> u8 {
        let used: Vec<u8> = self.accounts.iter().map(|a| a.color).collect();
        (0..8u8)
            .find(|c| !used.contains(c))
            .unwrap_or((self.accounts.len() % 8) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let a = Account {
            id: Account::new_id("a@b.it"),
            name: "Test".into(),
            email: "a@b.it".into(),
            username: "a@b.it".into(),
            imap_host: "imap.b.it".into(),
            imap_port: 993,
            smtp_host: "smtp.b.it".into(),
            smtp_port: 465,
            smtp_security: SmtpSecurity::Tls,
            color: 3,
            signature: String::new(),
            signature_kind: SignatureKind::Text,
            signature_html: String::new(),
            style: None,
        };
        let cfg = Config {
            accounts: vec![a.clone()],
            refresh_seconds: 120,
            messages_per_folder: 60,
            theme: ThemePref::Dark,
            style: "Verde".into(),
            sender_logos: true,
        };
        let text = toml::to_string(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.accounts, vec![a]);
    }

    #[test]
    fn initial_falls_back_to_email() {
        let mut a = Account {
            id: "x".into(),
            name: "".into(),
            email: "zoe@x.it".into(),
            username: "".into(),
            imap_host: "".into(),
            imap_port: 0,
            smtp_host: "".into(),
            smtp_port: 0,
            smtp_security: SmtpSecurity::Tls,
            color: 0,
            signature: String::new(),
            signature_kind: SignatureKind::Text,
            signature_html: String::new(),
            style: None,
        };
        assert_eq!(a.initial(), "Z");
        a.name = "lavoro".into();
        assert_eq!(a.initial(), "L");
    }
}
