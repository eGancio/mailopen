# mailopen — instructions for agents

mailopen is a clean-room, open-source, minimal mail client with a unified inbox for many
accounts, in pure Rust with egui. It follows the conventions of the ArtCraft Crafting Apps
(storytold/wordcraft and siblings): Rust only, no Electron/Tauri/JS, MIT OR Apache-2.0,
nothing copied from any proprietary product.

## Never crash
A crash in a mail client loses a half-written message. This outranks features.
- No `unwrap()`, `expect()`, `panic!`, `todo!`, `unreachable!`, indexing (`v[i]`) or `unsafe`
  in non-test code. Workspace lints in `Cargo.toml` deny them; tests may use them (`clippy.toml`).
- Errors are `Result<T, mailopen_core::Error>` and `?`. An unfinished feature returns an error
  or is disabled.
- Everything that comes from the network is hostile: `get()` not `[i]`, saturating arithmetic,
  size caps (`MAX_BODY`), and a "garbage never panics" test for each parser.

## Clean room
- Other mail clients may be *observed* (run them, look at the window) to understand the
  workflow. Never copy its icons, images, strings beyond feature names, or code.
- Protocols come from the RFCs (3501, 5321, 5322, 2045, 6154, 2177). Never copy GPL/LGPL code
  (Thunderbird, Evolution, mutt…).
- Assets are drawn in code (`ui/icons.rs`) or openly licensed with a row in ATTRIBUTION.md
  (today: Manrope, OFL).

## Architecture
| Layer | Crate | Job |
|---|---|---|
| core | `crates/core` | config + keyring, `imap_client`, `smtp_client`, `worker` (one thread per account, `Request` in / `Event` out), `labels` (categories), `documents` + `extract` (attachment index thread per account, text of PDF/Office files) |
| ui | `apps/mailopen` | `app.rs` holds all state and every action; `ui/*.rs` are thin panels that return an action enum and never block |

- Nothing in `crates/core` may depend on egui/eframe. The GUI never does network I/O.
- UI panels borrow `&mut MailApp` but mutate only after the panel closure ends (collect an
  `Action`, apply it after). Keep that pattern.
- Colours that differ between light and dark come from `theme::tokens(ui)`; never hardcode a
  surface or text colour in a panel. `ACCENT`, `ERROR`, `OK` and the avatar palette are shared.
- The compose editor is plain text with a light markup (`core::markup`): never ship an HTML
  editor widget; render to HTML only when sending.
- Strings are Italian for now, in place; when a second language lands they move to one module.

## Quality gates
Before every commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets` (clean),
`cargo test --workspace`. Commit after each arc of work that builds.

## Look at the result
- `MAILOPEN_DEMO=1 cargo run -p mailopen --example ui_shot -- inbox /tmp/inbox.png` renders the
  window offscreen (egui_kittest + wgpu, 2x) with fake data; scenes: `inbox`, `compose`,
  `account`, `light`, `dark`, `settings`, `category`, `unread`, `documents`, `documents-dark`,
  `documents-search` (third argument = query), `categories`. Read the PNG after every UI change and compare with the intended workflow
  (dark account rail, folders with icons, list with avatar + preview, blue selected row,
  reading pane with icon toolbar). Don't screenshot real windows: they move between Spaces.
- Real mail: `ui_shot -- eml out.png msg.eml` (also `eml-dark`) shows a raw message in the
  reading pane; `ui_shot -- viewer out.png msg.eml 0` opens its first attachment in the viewer.
  `SHOT_H=2600` makes the window taller to see a whole message. Get `.eml` files with
  `cargo run -p mailopen-core --example dump_eml -- you@x.it 20 /tmp/eml` (configured account)
  and check parsing with `--example inspect_eml -- /tmp/eml/*.eml` (`DUMP=4000` prints the
  document). Never commit real mail.
- `MAILOPEN_DEMO=1 cargo run -p mailopen` opens the window with the same fake data.

## Running
- `cargo run --release -p mailopen`
- Install: `packaging/macos/build-app.sh` (signs with the Apple Development identity found in the
  keychain, so the Keychain does not prompt on every rebuild; `SIGN_ID=` to pick one).
- `MAILOPEN_CONFIG_DIR=/tmp/x cargo run -p mailopen` for an isolated config (keyring entries
  are still shared: service `mailopen`, user = account id).
- `RUST_LOG=debug` for worker logs.
