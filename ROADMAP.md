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

- HTML mail drawn natively (20/10 evening): `core::html` turns HTML into a small document
  (inline CSS, simple `<style>` selectors, legacy attributes, hidden preheaders and tracking
  pixels dropped, depth/size caps) and `ui/html_view.rs` draws it on white paper: selectable
  text, links with the URL on hover (`mailto:` opens a new message), `cid:`/`data:` images,
  boxes, buttons, lists, quotes, tables with content-based column widths (stacked when too
  narrow). Remote images blocked until "Mostra immagini" / "Sempre da questo mittente"
  (`trusted_image_senders`). Plain text gets clickable URLs and `>` quotes.
- Attachments: real names, inline images kept out of the list, cards with type badge; viewer
  window with image, PDF (hayro, pure Rust, own thread), text, `.eml`, `.docx`/`.xlsx`
  previews; save (native dialog, "Salva tutti"), open with the default app, print (PDF and
  images through a browser print page, no dependence on the PDF app's print verb), forward.
  Compose attaches files (button or drag and drop) and forwards keep the original's files.
  Print the message itself from the reader toolbar.
- List previews: base64/quoted-printable parts cut by the partial fetch are decoded (open
  multiparts are closed first); preheaders used; invisible fillers, bare URLs and `[x.png]`
  placeholders dropped; empty previews filled in when the message is opened.

- App icon drawn in code (`src/icon_art.rs`, same design as the macOS one) on every window, and
  on Windows embedded in `mailopen.exe` with version info by `build.rs` (21/10)

- Organising mail (21/10): mark read/unread and flag from the toolbar, the right-click menu
  and Ctrl+U; quick filters (Tutti, Non letti, Contrassegnati, Con allegati); arrow keys move
  through the list; move to folder. Categories (`core::labels`, `labels.toml`): starter set
  Progetto, Richiesta, Ordine, Preventivo, Fattura, Assistenza, each with keywords matched at
  word starts in the subject (or `@domain` on the sender) plus by-hand assignments keyed by
  Message-ID; sidebar section, chips in list and reader, editor in Impostazioni.
- Search on the server (Enter in the search box): UID SEARCH TEXT in every folder but Trash,
  Spam and Drafts. SEARCH replies are read by hand (Dovecot progress lines, Zoho's
  `* SEARCH ` with a trailing space); a session out of step is dropped, and a panic of the
  IMAP library inside the worker costs only the connection.
- Documenti view (`core::documents`, `core::extract`): an indexer thread per account with
  its own connection looks at the newest N messages of each folder (BODY.PEEK, never marks
  read), lists every attachment and keeps the text of PDF (hayro, OCR layer included),
  .docx, .xlsx, text and .eml files (20k characters each), JSON per account in
  `documents/`. Filters by type, search by name/sender/subject/content with an excerpt;
  click shows the message, double click the preview, right click save/open/print/forward.
  `examples/index_probe.rs` and `search_probe.rs` try it read-only on a real account.

## Next
1. Local cache (SQLite): instant start, offline reading, "load more"
2. OAuth 2.0 for Gmail and Microsoft 365
3. IDLE push instead of polling; unread badge in the Dock / taskbar
4. Drafts folder; archive; multi-select; drag to a folder or a category
5. English/Italian UI switch
6. Packaging: .app bundle and notarisation, Windows installer, Linux AppImage
7. HTML: `<style>` descendant selectors, inline-block columns side by side, dark-mode paper
