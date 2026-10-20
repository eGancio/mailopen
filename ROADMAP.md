# mailopen roadmap

Stage: **v0.1, first usable build** (20/10/2026).

## Done
- Workspace, lints, never-crash rules
- Accounts: presets, test login, keyring, TOML config, edit/remove
- IMAP over rustls: folders with special-use detection, newest-N headers, body on demand,
  seen/unseen, delete to Trash (MOVE with COPY fallback), reconnect once on error
- SMTP over rustls: TLS and STARTTLS, reply threading headers, copy to Sent
- UI (20/10 afternoon): dark account rail with badges, folder column with icons
  and "Le tue cartelle", list with avatar, preview and attachment mark, blue selected row,
  reader with icon toolbar; light theme, system Arial, icons drawn in code
- Demo mode (`MAILOPEN_DEMO=1`) and offscreen screenshots (`examples/ui_shot.rs`)
- Manrope font bundled (OFL), light/dark/system theme and Impostazioni window (20/10, 12:10)
- Per-account signature, larger inputs (20/10, 12:25)
- Compose in its own OS window (subject on top, A + Cc/Bcc, Da chip, body),
  Bcc; address book `contacts.toml` filled from every sender/recipient seen, suggestions while
  typing (Enter/Tab picks the first); macOS .app packaging with signed bundle (20/10, 12:45)
- Colour styles (8) chosen by the user, general and per account (applied when that account is
  selected); signature as plain text or pasted HTML, sent as text+HTML alternative (20/10, 13:05)
- Compose editor toolbar (H1/H2, bold, italic, lists, link, clear) with a light markup rendered
  to HTML on send (`core::markup`); sender logos (Gravatar, then domain favicon, disk cache in
  `avatars/`, off switch in Impostazioni); bigger Impostazioni window (20/10, 13:20)

## Next
1. Local cache (SQLite): instant start, offline reading, "load more"
2. Attachments: save to disk; attach files when sending
3. HTML messages: keep the original and offer "show as HTML" (safe subset, no remote images)
4. OAuth 2.0 for Gmail and Microsoft 365
5. IDLE push instead of polling; unread badge in the Dock / taskbar
6. Drafts folder; archive; move to folder; multi-select
7. English/Italian UI switch
8. Packaging: .app bundle and notarisation, Windows installer, Linux AppImage
