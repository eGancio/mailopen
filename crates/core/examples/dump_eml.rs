//! Save the newest messages of an account's inbox as `.eml` files, to test the parser and the
//! HTML renderer on real mail. Uses the configured account and its keyring password.
//!
//! ```sh
//! cargo run -p mailopen-core --example dump_eml -- you@example.it 20 /tmp/eml
//! ```

use mailopen_core::imap_client::ImapClient;
use mailopen_core::{Config, secrets};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let email = args
        .next()
        .ok_or("uso: dump_eml <email> <quanti> <cartella>")?;
    let count: u32 = args.next().and_then(|n| n.parse().ok()).unwrap_or(20);
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| "eml".into()));
    let config = Config::load().map_err(|e| e.to_string())?;
    let account = config
        .accounts
        .iter()
        .find(|a| a.email.eq_ignore_ascii_case(&email))
        .ok_or_else(|| format!("account {email} non configurato"))?;
    let password = secrets::load(&account.id).map_err(|e| e.to_string())?;
    let mut client = ImapClient::connect(account, &password).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let list = client
        .summaries("INBOX", count)
        .map_err(|e| e.to_string())?;
    for m in &list {
        let raw = client.raw("INBOX", m.uid).map_err(|e| e.to_string())?;
        let path = out.join(format!("{}.eml", m.uid));
        std::fs::write(&path, &raw).map_err(|e| e.to_string())?;
        println!("{}\t{} KB\t{}", m.uid, raw.len() / 1024, m.subject);
    }
    client.logout();
    Ok(())
}
