//! Images of the reading pane and the attachment viewer, as egui textures.
//! Decoding (and downloading the remote images the user allowed) runs on background threads;
//! results become textures on the next frame. The least recently drawn are dropped past a cap.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use mailopen_core::images::Decoded;
use mailopen_core::worker::Wake;

/// Textures kept at most.
const MAX_TEXTURES: usize = 300;
const THREADS: usize = 3;

pub enum Source {
    Bytes(Arc<[u8]>),
    Url(String),
}

#[derive(Clone, Copy, Debug)]
pub enum ImgState {
    Loading,
    Failed,
    /// Texture and its size in pixels.
    Ready(egui::TextureId, egui::Vec2),
}

enum Slot {
    Loading,
    Failed,
    Ready(egui::TextureHandle, egui::Vec2),
}

struct Job {
    key: String,
    source: Source,
    max_side: u32,
}

pub struct ImageCache {
    slots: HashMap<String, (Slot, u64)>,
    frame: u64,
    jobs: Sender<Job>,
    done: Receiver<(String, Option<Decoded>)>,
}

impl ImageCache {
    pub fn new(wake: Wake) -> Self {
        let (jobs, rx) = channel::<Job>();
        let (done_tx, done) = channel::<(String, Option<Decoded>)>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..THREADS {
            let rx = Arc::clone(&rx);
            let done_tx = done_tx.clone();
            let wake = Arc::clone(&wake);
            let spawned = std::thread::Builder::new()
                .name(format!("mailopen-images-{i}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let Ok(guard) = rx.lock() else { break };
                            guard.recv()
                        };
                        let Ok(job) = job else { break };
                        let decoded = match &job.source {
                            Source::Bytes(b) => mailopen_core::images::decode(b, job.max_side),
                            Source::Url(u) => mailopen_core::images::fetch(u)
                                .and_then(|b| mailopen_core::images::decode(&b, job.max_side)),
                        };
                        if done_tx.send((job.key, decoded)).is_err() {
                            break;
                        }
                        wake();
                    }
                });
            if let Err(e) = spawned {
                log::warn!("thread immagini: {e}");
            }
        }
        ImageCache {
            slots: HashMap::new(),
            frame: 0,
            jobs,
            done,
        }
    }

    /// Turn finished jobs into textures and drop old ones. Call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.frame = self.frame.wrapping_add(1);
        while let Ok((key, decoded)) = self.done.try_recv() {
            let slot = match decoded {
                Some(d) => {
                    let size = [d.width as usize, d.height as usize];
                    let image = egui::ColorImage::from_rgba_unmultiplied(size, &d.rgba);
                    let options = egui::TextureOptions::LINEAR
                        .with_mipmap_mode(Some(egui::TextureFilter::Linear));
                    let handle = ctx.load_texture(format!("img-{key}"), image, options);
                    Slot::Ready(handle, egui::vec2(d.width as f32, d.height as f32))
                }
                None => Slot::Failed,
            };
            let used = self.slots.get(&key).map(|(_, f)| *f).unwrap_or(self.frame);
            self.slots.insert(key, (slot, used));
        }
        if self.slots.len() > MAX_TEXTURES {
            let mut ages: Vec<(u64, String)> = self
                .slots
                .iter()
                .filter(|(_, (s, f))| {
                    matches!(s, Slot::Ready(..) | Slot::Failed) && *f + 2 < self.frame
                })
                .map(|(k, (_, f))| (*f, k.clone()))
                .collect();
            ages.sort();
            let excess = self.slots.len().saturating_sub(MAX_TEXTURES);
            for (_, k) in ages.into_iter().take(excess) {
                self.slots.remove(&k);
            }
        }
    }

    /// The image under `key`, starting the decode the first time (`source` is only called then).
    pub fn get(
        &mut self,
        key: &str,
        max_side: u32,
        source: impl FnOnce() -> Option<Source>,
    ) -> ImgState {
        let frame = self.frame;
        if let Some((slot, used)) = self.slots.get_mut(key) {
            *used = frame;
            return match slot {
                Slot::Loading => ImgState::Loading,
                Slot::Failed => ImgState::Failed,
                Slot::Ready(h, size) => ImgState::Ready(h.id(), *size),
            };
        }
        let Some(source) = source() else {
            self.slots.insert(key.to_string(), (Slot::Failed, frame));
            return ImgState::Failed;
        };
        let job = Job {
            key: key.to_string(),
            source,
            max_side,
        };
        if self.jobs.send(job).is_err() {
            self.slots.insert(key.to_string(), (Slot::Failed, frame));
            return ImgState::Failed;
        }
        self.slots.insert(key.to_string(), (Slot::Loading, frame));
        ImgState::Loading
    }
}
