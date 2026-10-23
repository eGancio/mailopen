//! Try IMAP SEARCH criteria on a configured account's inbox, read only.
//!
//! ```sh
//! cargo run -p mailopen-core --example search_probe -- you@example.it fattura
//! ```

use mailopen_core::imap_client::{ImapClient, search_criteria};
use mailopen_core::{Config, secrets};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let email = args.next().ok_or("uso: search_probe <email> <parola>")?;
    let word = args.next().unwrap_or_else(|| "fattura".into());
    let config = Config::load().map_err(|e| e.to_string())?;
    let account = config
        .accounts
        .iter()
        .find(|a| a.email.eq_ignore_ascii_case(&email))
        .ok_or_else(|| format!("account {email} non configurato"))?;
    let password = secrets::load(&account.id).map_err(|e| e.to_string())?;
    let mut client = ImapClient::connect(account, &password).map_err(|e| e.to_string())?;
    client.set_debug(std::env::var_os("IMAP_DEBUG").is_some());
    let mut tries = vec![
        format!("SUBJECT \"{word}\""),
        format!("BODY \"{word}\""),
        format!("TEXT \"{word}\""),
        format!("OR SUBJECT \"{word}\" BODY \"{word}\""),
    ];
    tries.extend(search_criteria(&word));
    for criteria in tries {
        match client.raw_search("INBOX", &criteria) {
            Ok(raw) => println!("{criteria}\n  {}", raw.trim().replace("\r\n", " | ")),
            Err(e) => println!("{criteria}\n  ERRORE {e}"),
        }
    }
    client.logout();
    Ok(())
}
