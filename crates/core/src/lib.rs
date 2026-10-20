//! mailopen-core: accounts, IMAP/SMTP access and the background worker.
//!
//! Nothing in this crate knows about the GUI. The app talks to it through
//! [`worker::AccountWorker`] (requests in, [`worker::Event`]s out).

pub mod avatars;
pub mod config;
pub mod contacts;
pub mod error;
pub mod imap_client;
pub mod markup;
pub mod model;
pub mod presets;
pub mod secrets;
pub mod smtp_client;
pub mod worker;

pub use avatars::Logo;
pub use config::{Account, Config, SignatureKind, SmtpSecurity, ThemePref};
pub use contacts::{Contact, Contacts};
pub use error::{Error, Result};
pub use model::{Folder, FolderKind, MessageBody, MessageSummary, OutgoingMessage};
pub use worker::{AccountWorker, Event, Request};
