//! Passwords live in the OS keyring (macOS Keychain, Windows Credential Manager,
//! Secret Service on Linux), never in the config file.

use keyring::Entry;

use crate::error::Result;

const SERVICE: &str = "mailopen";

pub fn store(account_id: &str, password: &str) -> Result<()> {
    Entry::new(SERVICE, account_id)?.set_password(password)?;
    Ok(())
}

pub fn load(account_id: &str) -> Result<String> {
    Ok(Entry::new(SERVICE, account_id)?.get_password()?)
}

pub fn delete(account_id: &str) -> Result<()> {
    match Entry::new(SERVICE, account_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
