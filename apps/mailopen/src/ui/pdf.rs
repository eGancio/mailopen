//! PDF pages through hayro (pure Rust). The parsed document is not `Send`, so it lives on a
//! thread of its own; a panic inside a malformed file ends that thread, not the app.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use mailopen_core::worker::Wake;

/// Pages shown at most.
const MAX_PAGES: usize = 500;
/// Rendered pages kept as textures at most.
const MAX_TEXTURES: usize = 12;
/// Rendered pixels per side at most (hayro's canvas is 16-bit).
const MAX_SIDE: f32 = 12_000.0;

struct PageImage {
    index: usize,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

enum Event {
    Opened(Vec<(f32, f32)>),
    Page(PageImage),
    Failed(String),
}

/// An open PDF in the viewer: page sizes, then pages rendered on request.
pub struct PdfDoc {
    tx: Sender<usize>,
    rx: Receiver<Event>,
    /// Page sizes in points (1/72 inch), once known.
    pub sizes: Vec<(f32, f32)>,
    pub error: Option<String>,
    pub opened: bool,
    textures: HashMap<usize, egui::TextureHandle>,
    requested: HashSet<usize>,
    last_used: HashMap<usize, u64>,
    frame: u64,
}

fn render_page<'a>(
    page: &'a hayro::hayro_syntax::page::Page<'a>,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    target_width: f32,
) -> (u32, u32, Vec<u8>) {
    let (w, h) = page.render_dimensions();
    let mut scale = target_width / w.max(1.0);
    if h * scale > MAX_SIDE {
        scale = MAX_SIDE / h.max(1.0);
    }
    let pixmap = hayro::render(
        page,
        cache,
        settings,
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
        },
    );
    (
        u32::from(pixmap.width()),
        u32::from(pixmap.height()),
        pixmap.data_as_u8_slice().to_vec(),
    )
}

fn load_error(e: &hayro::hayro_syntax::LoadPdfError) -> String {
    let s = format!("{e:?}");
    if s.contains("Decrypt") || s.contains("ncrypt") {
        "PDF protetto da password: aprilo con l'app predefinita.".into()
    } else {
        "PDF non leggibile: prova ad aprirlo con l'app predefinita.".into()
    }
}

impl PdfDoc {
    /// Start reading `data`; pages render `target_width` pixels wide.
    pub fn open(data: Arc<[u8]>, target_width: u32, wake: Wake) -> PdfDoc {
        let (tx, jobs) = channel::<usize>();
        let (events, rx) = channel::<Event>();
        let spawned = std::thread::Builder::new()
            .name("mailopen-pdf".into())
            .spawn(move || {
                let pdf = match Pdf::new(data.to_vec()) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = events.send(Event::Failed(load_error(&e)));
                        wake();
                        return;
                    }
                };
                let pages = pdf.pages();
                let sizes: Vec<(f32, f32)> = pages
                    .iter()
                    .take(MAX_PAGES)
                    .map(|p| p.render_dimensions())
                    .collect();
                if sizes.is_empty() {
                    let _ = events.send(Event::Failed("Il PDF non ha pagine.".into()));
                    wake();
                    return;
                }
                if events.send(Event::Opened(sizes)).is_err() {
                    return;
                }
                wake();
                let cache = RenderCache::new();
                let settings = InterpreterSettings::default();
                while let Ok(index) = jobs.recv() {
                    let Some(page) = pages.get(index) else {
                        continue;
                    };
                    let (width, height, rgba) =
                        render_page(page, &cache, &settings, target_width as f32);
                    let img = PageImage {
                        index,
                        width,
                        height,
                        rgba,
                    };
                    if events.send(Event::Page(img)).is_err() {
                        break;
                    }
                    wake();
                }
            });
        let error = spawned
            .err()
            .map(|e| format!("Impossibile leggere il PDF: {e}"));
        PdfDoc {
            tx,
            rx,
            sizes: Vec::new(),
            error,
            opened: false,
            textures: HashMap::new(),
            requested: HashSet::new(),
            last_used: HashMap::new(),
            frame: 0,
        }
    }

    /// Take finished pages. Call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.frame = self.frame.wrapping_add(1);
        loop {
            match self.rx.try_recv() {
                Ok(Event::Opened(sizes)) => {
                    self.sizes = sizes;
                    self.opened = true;
                }
                Ok(Event::Page(p)) => {
                    let image = egui::ColorImage::from_rgba_premultiplied(
                        [p.width as usize, p.height as usize],
                        &p.rgba,
                    );
                    let options = egui::TextureOptions::LINEAR
                        .with_mipmap_mode(Some(egui::TextureFilter::Linear));
                    let tex = ctx.load_texture(format!("pdf-page-{}", p.index), image, options);
                    self.textures.insert(p.index, tex);
                    self.last_used.insert(p.index, self.frame);
                }
                Ok(Event::Failed(e)) => self.error = Some(e),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if !self.opened && self.error.is_none() {
                        self.error = Some(
                            "PDF non leggibile: prova ad aprirlo con l'app predefinita.".into(),
                        );
                    }
                    break;
                }
            }
        }
        if self.textures.len() > MAX_TEXTURES {
            let mut ages: Vec<(u64, usize)> = self
                .textures
                .keys()
                .map(|i| (self.last_used.get(i).copied().unwrap_or(0), *i))
                .filter(|(f, _)| *f + 2 < self.frame)
                .collect();
            ages.sort();
            let excess = self.textures.len().saturating_sub(MAX_TEXTURES);
            for (_, i) in ages.into_iter().take(excess) {
                self.textures.remove(&i);
                self.requested.remove(&i);
            }
        }
    }

    /// The texture of page `index`, asking for it when missing.
    pub fn page(&mut self, index: usize) -> Option<egui::TextureId> {
        self.last_used.insert(index, self.frame);
        if let Some(t) = self.textures.get(&index) {
            return Some(t.id());
        }
        if self.requested.insert(index) && self.tx.send(index).is_err() {
            self.requested.remove(&index);
        }
        None
    }
}

/// Render every page to PNG files in `dir` (for printing). Runs on the caller's thread.
pub fn pages_to_png(data: &[u8], dpi: f32, dir: &Path) -> Result<Vec<(PathBuf, f32, f32)>, String> {
    let pdf = Pdf::new(data.to_vec()).map_err(|e| load_error(&e))?;
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    let mut out = Vec::new();
    for (i, page) in pdf.pages().iter().take(MAX_PAGES).enumerate() {
        let (w_pt, h_pt) = page.render_dimensions();
        let target = (w_pt * dpi / 72.0).clamp(200.0, 4000.0);
        let (w, h, rgba) = render_page(page, &cache, &settings, target);
        let img = image::RgbaImage::from_raw(w, h, rgba)
            .ok_or_else(|| "pagina non convertita".to_string())?;
        let path = dir.join(format!("pagina-{:03}.png", i + 1));
        img.save_with_format(&path, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        out.push((path, w_pt, h_pt));
    }
    if out.is_empty() {
        return Err("Il PDF non ha pagine.".into());
    }
    Ok(out)
}
