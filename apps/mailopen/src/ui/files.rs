//! Attachments leaving the app: save with a native dialog, open with the default program,
//! print, and files read from disk for sending. Printing runs on a thread; its outcome comes
//! back on a channel.
//!
//! Printing does not depend on the default PDF/image program supporting the "print" verb
//! (Chrome and Photos do not): PDFs are rendered to pages, images and text are laid out, and a
//! print page opens in the browser with the system print dialog.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use mailopen_core::{Attachment, FileKind, MessageBody};

/// Files larger than this are not attached.
pub const MAX_ATTACH_FILE: u64 = 25 * 1024 * 1024;

pub fn downloads_dir() -> Option<PathBuf> {
    directories::UserDirs::new()
        .and_then(|u| u.download_dir().map(Path::to_path_buf))
        .or_else(|| directories::UserDirs::new().map(|u| u.home_dir().to_path_buf()))
}

/// "name.pdf", "name (2).pdf"… the first that does not exist in `dir`.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 2..1000 {
        let p = dir.join(format!("{stem} ({n}){ext}"));
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("{stem}-{}{ext}", std::process::id()))
}

/// Ask where to save `a` and write it. `Ok(None)` when the user cancels.
pub fn save_dialog(a: &Attachment) -> Result<Option<PathBuf>, String> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Salva allegato")
        .set_file_name(&a.name);
    if let Some(d) = downloads_dir() {
        dialog = dialog.set_directory(d);
    }
    let ext = a.extension();
    if !ext.is_empty() {
        dialog = dialog.add_filter(ext.to_uppercase(), &[ext.as_str()]);
    }
    let Some(path) = dialog.save_file() else {
        return Ok(None);
    };
    std::fs::write(&path, &a.data).map_err(|e| format!("Non salvato: {e}"))?;
    Ok(Some(path))
}

/// Ask for a folder and save every attachment there. Returns the folder and how many.
pub fn save_all_dialog(list: &[Attachment]) -> Result<Option<(PathBuf, usize)>, String> {
    let mut dialog = rfd::FileDialog::new().set_title("Salva tutti gli allegati");
    if let Some(d) = downloads_dir() {
        dialog = dialog.set_directory(d);
    }
    let Some(dir) = dialog.pick_folder() else {
        return Ok(None);
    };
    for a in list {
        let path = unique_path(&dir, &a.name);
        std::fs::write(&path, &a.data).map_err(|e| format!("{}: {e}", a.name))?;
    }
    Ok(Some((dir, list.len())))
}

/// A private temp folder for this attachment (`%TEMP%/mailopen/<hash>/`).
fn temp_dir_for(a: &Attachment) -> Result<PathBuf, String> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    a.name.hash(&mut h);
    a.data.len().hash(&mut h);
    a.data.get(..4096).unwrap_or(&a.data).hash(&mut h);
    let dir = std::env::temp_dir()
        .join("mailopen")
        .join(format!("{:016x}", h.finish()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Write `a` to its temp folder (for opening or printing).
pub fn temp_copy(a: &Attachment) -> Result<PathBuf, String> {
    let path = temp_dir_for(a)?.join(&a.name);
    std::fs::write(&path, &a.data).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn open_with_default(a: &Attachment) -> Result<(), String> {
    let path = temp_copy(a)?;
    open::that_detached(&path).map_err(|e| format!("Nessuna app per aprirlo: {e}"))
}

/// Show `path` in the system file manager.
pub fn reveal(path: &Path) {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    if let Err(e) = open::that_detached(target) {
        log::warn!("cartella non aperta: {e}");
    }
}

/// Print on a background thread. The receiver gets a status line (or an error).
pub fn print(a: Attachment) -> Receiver<Result<String, String>> {
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new()
        .name("mailopen-print".into())
        .spawn(move || {
            let _ = tx.send(print_now(&a));
        });
    if let Err(e) = spawned {
        log::warn!("thread di stampa: {e}");
    }
    rx
}

fn print_now(a: &Attachment) -> Result<String, String> {
    let dir = temp_dir_for(a)?.join("stampa");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let title = escape(&a.name);
    let html = match a.kind() {
        FileKind::Pdf => {
            let pages = crate::ui::pdf::pages_to_png(&a.data, 200.0, &dir)?;
            let mut body = String::new();
            for (path, w, h) in &pages {
                let file = path
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let landscape = w > h;
                body.push_str(&format!(
                    "<div class=\"page{}\"><img src=\"{}\"></div>\n",
                    if landscape { " land" } else { "" },
                    escape(&file)
                ));
            }
            print_page(&title, &body, "img{width:100%;height:auto;display:block}")
        }
        FileKind::Image => {
            let name = format!(
                "immagine.{}",
                if a.extension().is_empty() {
                    "png".into()
                } else {
                    a.extension()
                }
            );
            std::fs::write(dir.join(&name), &a.data).map_err(|e| e.to_string())?;
            let body = format!("<div class=\"page\"><img src=\"{}\"></div>", escape(&name));
            print_page(
                &title,
                &body,
                "img{max-width:100%;max-height:96vh;display:block;margin:0 auto}",
            )
        }
        FileKind::Text => {
            let text = decode_text(&a.data);
            let body = format!("<h1>{title}</h1><pre>{}</pre>", escape(&text));
            print_page(
                &title,
                &body,
                "pre{white-space:pre-wrap;font:10pt Consolas,Menlo,monospace}h1{font:bold 12pt sans-serif}",
            )
        }
        _ => return print_with_system(a),
    };
    let page = dir.join("stampa.html");
    std::fs::write(&page, html).map_err(|e| e.to_string())?;
    open::that_detached(&page).map_err(|e| format!("Browser non aperto: {e}"))?;
    Ok("Anteprima di stampa aperta nel browser.".into())
}

/// A message (HTML or text) as a print page: header lines, then the body.
pub fn print_message(
    subject: &str,
    header: &[(String, String)],
    body: &MessageBody,
    allow_remote: bool,
) -> Receiver<Result<String, String>> {
    let subject = subject.to_string();
    let header = header.to_vec();
    let body = body.clone();
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new()
        .name("mailopen-print".into())
        .spawn(move || {
            let _ = tx.send(print_message_now(&subject, &header, &body, allow_remote));
        });
    if let Err(e) = spawned {
        log::warn!("thread di stampa: {e}");
    }
    rx
}

fn print_message_now(
    subject: &str,
    header: &[(String, String)],
    body: &MessageBody,
    allow_remote: bool,
) -> Result<String, String> {
    let fake = Attachment {
        name: format!("{subject}.html"),
        mime: "text/html".into(),
        content_id: None,
        data: Arc::from(body.text.as_bytes()),
    };
    let dir = temp_dir_for(&fake)?.join("stampa");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut head = format!("<h1>{}</h1><table class=\"hdr\">", escape(subject));
    for (k, v) in header {
        head.push_str(&format!(
            "<tr><th>{}</th><td>{}</td></tr>",
            escape(k),
            escape(v)
        ));
    }
    head.push_str("</table><hr>");
    let content = match &body.html {
        Some(html) => {
            // Inline parts as files next to the page; remote images only if allowed.
            let mut html = html.clone();
            for (i, part) in body.inline.iter().enumerate() {
                if let Some(cid) = &part.content_id {
                    let file = format!(
                        "inline-{i}.{}",
                        if part.extension().is_empty() {
                            "png".into()
                        } else {
                            part.extension()
                        }
                    );
                    if std::fs::write(dir.join(&file), &part.data).is_ok() {
                        html = replace_ci(&html, &format!("cid:{cid}"), &file);
                    }
                }
            }
            if !allow_remote {
                html = html
                    .replace("src=\"http", "data-src=\"http")
                    .replace("src='http", "data-src='http");
            }
            strip_scripts(&html)
        }
        None => format!("<pre class=\"plain\">{}</pre>", escape(&body.text)),
    };
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{t}</title><style>\
         body{{font-family:Segoe UI,Helvetica,Arial,sans-serif;margin:24px}}\
         h1{{font-size:16pt;margin:0 0 8px}}table.hdr th{{text-align:left;color:#555;padding-right:12px;font-weight:600}}\
         table.hdr{{font-size:10pt;margin-bottom:8px}}pre.plain{{white-space:pre-wrap;font:10.5pt Segoe UI,Helvetica,sans-serif}}\
         @media print{{body{{margin:0}}}}</style></head>\
         <body onload=\"setTimeout(function(){{window.print()}},400)\">{head}{content}</body></html>",
        t = escape(subject)
    );
    let page = dir.join("messaggio.html");
    std::fs::write(&page, html).map_err(|e| e.to_string())?;
    open::that_detached(&page).map_err(|e| format!("Browser non aperto: {e}"))?;
    Ok("Anteprima di stampa aperta nel browser.".into())
}

fn strip_scripts(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let mut i = 0usize;
    while let Some(start) = lower.get(i..).and_then(|s| s.find("<script")) {
        let start = i.saturating_add(start);
        out.push_str(html.get(i..start).unwrap_or(""));
        let end = lower
            .get(start..)
            .and_then(|s| s.find("</script>"))
            .map(|e| start.saturating_add(e).saturating_add("</script>".len()))
            .unwrap_or(html.len());
        i = end;
    }
    out.push_str(html.get(i..).unwrap_or(""));
    out
}

fn replace_ci(hay: &str, needle: &str, with: &str) -> String {
    let lower = hay.to_ascii_lowercase();
    let n = needle.to_ascii_lowercase();
    if n.is_empty() {
        return hay.to_string();
    }
    let mut out = String::with_capacity(hay.len());
    let mut i = 0usize;
    while let Some(pos) = lower.get(i..).and_then(|s| s.find(&n)) {
        let start = i.saturating_add(pos);
        out.push_str(hay.get(i..start).unwrap_or(""));
        out.push_str(with);
        i = start.saturating_add(n.len());
    }
    out.push_str(hay.get(i..).unwrap_or(""));
    out
}

fn print_page(title: &str, body: &str, extra_css: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>\
         @page{{margin:10mm}}body{{margin:0}}.page{{page-break-after:always;break-after:page}}\
         .page:last-child{{page-break-after:auto;break-after:auto}}{extra_css}</style></head>\
         <body onload=\"setTimeout(function(){{window.print()}},400)\">{body}</body></html>"
    )
}

/// Office files and the rest: the "print" verb of the program that opens them, else open it.
fn print_with_system(a: &Attachment) -> Result<String, String> {
    let path = temp_copy(a)?;
    if print_verb(&path).is_ok() {
        return Ok("Inviato alla stampa con l'app predefinita.".into());
    }
    open::that_detached(&path).map_err(|e| format!("Nessuna app per stamparlo: {e}"))?;
    Ok("Aperto con l'app predefinita: stampa da lì (Ctrl+P).".into())
}

#[cfg(windows)]
fn print_verb(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let p = path.to_string_lossy().replace('\'', "''");
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("Start-Process -FilePath '{p}' -Verb Print -ErrorAction Stop"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

#[cfg(not(windows))]
fn print_verb(path: &Path) -> Result<(), String> {
    let out = Command::new("lp")
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// UTF-8, else Windows-1252/Latin-1 (old CSV exports).
pub fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|b| char::from(*b)).collect(),
    }
}

fn escape(s: &str) -> String {
    mailopen_core::smtp_client::escape_html(s)
}

/// Read a file from disk as an attachment for sending.
pub fn read_attachment(path: &Path) -> Result<Attachment, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "allegato".into());
    if meta.len() > MAX_ATTACH_FILE {
        return Err(format!(
            "{name}: troppo grande ({} MB, massimo {} MB)",
            meta.len() / (1024 * 1024),
            MAX_ATTACH_FILE / (1024 * 1024)
        ));
    }
    let data = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    Ok(Attachment {
        mime: mime_for(&name).to_string(),
        name: mailopen_core::imap_client::sanitize_name(&name),
        content_id: None,
        data: Arc::from(data),
    })
}

pub fn pick_files() -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Allega file")
        .pick_files()
        .unwrap_or_default()
}

fn mime_for(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "ics" => "text/calendar",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "eml" => "message/rfc822",
        "json" => "application/json",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(decode_text(b"caf\xe8"), "cafè");
        assert_eq!(decode_text("già".as_bytes()), "già");
        assert_eq!(mime_for("a.PDF"), "application/pdf");
        assert_eq!(
            strip_scripts("a<script>x</script>b<SCRIPT src=y></SCRIPT>c"),
            "abc"
        );
        assert_eq!(
            replace_ci("src=\"CID:AbC\"", "cid:abc", "f.png"),
            "src=\"f.png\""
        );
        let dir = std::env::temp_dir().join("mailopen-test-unique");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("x.pdf"), b"1");
        assert_eq!(unique_path(&dir, "x.pdf"), dir.join("x (2).pdf"));
    }
}
