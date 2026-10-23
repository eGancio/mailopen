//! Render the app offscreen to a PNG, for looking at UI work without a window.
//!
//! ```sh
//! MAILOPEN_DEMO=1 cargo run -p mailopen --example ui_shot -- inbox /tmp/inbox.png
//! MAILOPEN_DEMO=1 cargo run -p mailopen --example ui_shot -- compose /tmp/compose.png
//! MAILOPEN_DEMO=1 cargo run -p mailopen --example ui_shot -- account /tmp/account.png
//! MAILOPEN_DEMO=1 cargo run -p mailopen --example ui_shot -- dark /tmp/dark.png
//! ```

use mailopen::app::MailApp;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let scene = args.next().unwrap_or_else(|| "inbox".to_string());
    let out = args.next().unwrap_or_else(|| "ui_shot.png".to_string());
    let extra = args.next();
    let index: usize = args.next().and_then(|i| i.parse().ok()).unwrap_or(0);
    let mut app = MailApp::new_with_ctx(&egui::Context::default());
    let mut slow = false;
    match scene.as_str() {
        // A real message in the reading pane: `eml out.png message.eml`
        "eml" | "eml-dark" | "viewer" => {
            let path = extra.ok_or("serve il percorso del file .eml")?;
            let raw = std::fs::read(&path).map_err(|e| e.to_string())?;
            app.load_eml(&raw);
            if scene == "eml-dark" {
                app.config.theme = mailopen_core::ThemePref::Dark;
            }
            if scene == "viewer"
                && let Some((key, a)) = app.selected.clone().and_then(|k| {
                    let a = app.bodies.get(&k)?.attachments.get(index)?.clone();
                    Some((k, a))
                })
            {
                app.open_attachment(Some(key), a);
            }
            slow = true;
        }
        "compose" => {
            app.compose_new();
            if let Some(c) = app.compose.as_mut() {
                c.subject = "Offerta colonnine".into();
                c.to = "Anna Rossi <anna.rossi@example.it>, ".into();
                c.body = "# Riepilogo\n\nCiao **Anna**, come *anticipato* ti mando i punti:\n\n- consegna entro **marzo**\n- installazione inclusa\n\n1. accettazione\n2. acconto\n\nDettagli: https://example.it/offerta e [il listino](https://example.it/listino)".into();
            }
        }
        "dark" => app.config.theme = mailopen_core::ThemePref::Dark,
        "light" => app.config.theme = mailopen_core::ThemePref::Light,
        "style" => {
            app.config.theme = mailopen_core::ThemePref::Light;
            app.config.style = "Viola".into();
            if let Some(a) = app.config.accounts.get_mut(1) {
                a.style = Some("Arancio".into());
            }
            app.select_rail(mailopen::app::Rail::Account("demo-b".into()));
            app.settings_open = true;
        }
        "settings" => {
            app.config.theme = mailopen_core::ThemePref::Dark;
            app.settings_open = true;
        }
        // Organising mail: categories, documents, quick filters.
        "category" => app.set_view(mailopen::app::View::Category("preventivo".into())),
        "unread" => app.filter = mailopen::app::ListFilter::Unread,
        "documents" | "documents-dark" | "documents-search" => {
            if scene == "documents-dark" {
                app.config.theme = mailopen_core::ThemePref::Dark;
            }
            app.set_view(mailopen::app::View::Documents);
            if scene == "documents-search" {
                app.doc_query = extra.clone().unwrap_or_else(|| "wallbox".into());
            }
            if let Some(e) = app.filtered_documents().first().cloned() {
                app.open_document(&e, None);
            }
        }
        "categories" => {
            app.settings_open = true;
            app.settings_categories = true;
        }
        "account" => {
            app.account_dialog = Some(mailopen::ui::account_dialog::AccountDialog::new(
                app.config.next_color(),
            ));
        }
        _ => {}
    }
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(
            1240.0,
            std::env::var("SHOT_H")
                .ok()
                .and_then(|h| h.parse().ok())
                .unwrap_or(780.0),
        ))
        .with_pixels_per_point(2.0)
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .build_ui(move |ui| app.root(ui));
    harness.run_steps(60);
    if slow {
        // Images and PDF pages decode on background threads.
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(60));
            harness.run_steps(3);
        }
    }
    let image = harness.render()?;
    image.save(&out).map_err(|e| e.to_string())?;
    println!("{out}");
    Ok(())
}
