use std::fmt;

/// Every failure in the core crate. The GUI shows `Display` to the user.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("TLS: {0}")]
    Tls(String),
    #[error("IMAP: {0}")]
    Imap(String),
    #[error("SMTP: {0}")]
    Smtp(String),
    #[error("Indirizzo non valido: {0}")]
    Address(String),
    #[error("Configurazione: {0}")]
    Config(String),
    #[error("Portachiavi: {0}")]
    Keyring(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<imap::Error> for Error {
    fn from(e: imap::Error) -> Self {
        Error::Imap(e.to_string())
    }
}

impl From<lettre::transport::smtp::Error> for Error {
    fn from(e: lettre::transport::smtp::Error) -> Self {
        Error::Smtp(e.to_string())
    }
}

impl From<lettre::error::Error> for Error {
    fn from(e: lettre::error::Error) -> Self {
        Error::Smtp(e.to_string())
    }
}

impl From<lettre::address::AddressError> for Error {
    fn from(e: lettre::address::AddressError) -> Self {
        Error::Address(e.to_string())
    }
}

impl From<keyring::Error> for Error {
    fn from(e: keyring::Error) -> Self {
        Error::Keyring(e.to_string())
    }
}

impl Error {
    pub fn other(msg: impl fmt::Display) -> Self {
        Error::Other(msg.to_string())
    }
}
