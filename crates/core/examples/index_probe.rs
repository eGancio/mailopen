//! Try the document index and the server search on a configured account, read only: nothing is
//! saved and no flag changes. Prints what the index would hold for the newest messages.
//!
//! ```sh
//! cargo run -p mailopen-core --example index_probe -- you@example.it 60 fattura
//! ```

use std::time::Instant;

use mailopen_core::documents::entries_of;
use mailopen_core::imap_client::{self, ImapClient};
use mailopen_core::{Config, secrets};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let email = args
        .next()
        .ok_or("uso: index_probe <email> <quanti> [parola da cercare]")?;
    let count: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(60);
    let word = args.next();
    let config = Config::load().map_err(|e| e.to_string())?;
    let account = config
        .accounts
        .iter()
        .find(|a| a.email.eq_ignore_ascii_case(&email))
        .ok_or_else(|| format!("account {email} non configurato"))?;
    let password = secrets::load(&account.id).map_err(|e| e.to_string())?;
    let mut client = ImapClient::connect(account, &password).map_err(|e| e.to_string())?;
    let folders = client.folders().map_err(|e| e.to_string())?;
    let inbox = folders
        .iter()
        .find(|f| f.kind == mailopen_core::FolderKind::Inbox)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| "INBOX".into());

    let t = Instant::now();
    let (validity, uids) = client.all_uids(&inbox).map_err(|e| e.to_string())?;
    println!(
        "{inbox}: {} messaggi, UIDVALIDITY {validity} ({} ms)",
        uids.len(),
        t.elapsed().as_millis()
    );
    let newest: Vec<u32> = uids.iter().rev().take(count).copied().collect();
    let t = Instant::now();
    let headers = client.headers(&inbox, &newest).map_err(|e| e.to_string())?;
    let wanted: Vec<u32> = headers
        .iter()
        .filter(|(_, size, h)| *size <= 30 * 1024 * 1024 && imap_client::may_have_files(h))
        .map(|(u, _, _)| *u)
        .collect();
    println!(
        "intestazioni: {} in {} ms, con possibili allegati: {}",
        headers.len(),
        t.elapsed().as_millis(),
        wanted.len()
    );
    let t = Instant::now();
    let mut files = 0usize;
    let mut with_text = 0usize;
    for chunk in wanted.chunks(25) {
        let raws = client.raws(&inbox, chunk).map_err(|e| e.to_string())?;
        for (uid, raw) in raws {
            let header = headers
                .iter()
                .find(|(u, _, _)| *u == uid)
                .map(|(_, _, h)| h.as_slice())
                .unwrap_or(b"");
            for e in entries_of(&account.id, &inbox, uid, header, &raw) {
                files += 1;
                if !e.text.is_empty() {
                    with_text += 1;
                }
                let preview: String = e.text.chars().take(60).collect();
                println!(
                    "  {:?}\t{} KB\t{} car.\t{}\t{}",
                    e.kind(),
                    e.size / 1024,
                    e.text.chars().count(),
                    e.name,
                    preview
                );
            }
        }
    }
    println!(
        "allegati: {files}, con testo: {with_text}, in {} ms",
        t.elapsed().as_millis()
    );
    if let Some(word) = word {
        let t = Instant::now();
        let found = client
            .search(&inbox, &word, 10)
            .map_err(|e| e.to_string())?;
        println!(
            "ricerca sul server «{word}»: {} risultati in {} ms",
            found.len(),
            t.elapsed().as_millis()
        );
        for m in &found {
            println!("  {}\t{}\t{}", m.uid, m.message_id.len(), m.subject);
        }
        if client.broken {
            // Like the worker: a session out of step is replaced, never reused.
            println!("sessione da rifare: riconnetto");
            client = ImapClient::connect(account, &password).map_err(|e| e.to_string())?;
        }
        if let Some(m) = found.iter().find(|m| !m.message_id.is_empty()) {
            let again = client
                .find_by_message_id(&inbox, &m.message_id)
                .map_err(|e| e.to_string())?;
            println!(
                "ritrovato per Message-ID {:?}: {:?} (atteso {})",
                m.message_id,
                again.map(|x| x.uid),
                m.uid
            );
        }
    }
    client.logout();
    Ok(())
}
