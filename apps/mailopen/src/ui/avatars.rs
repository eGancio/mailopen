//! Sender logos as egui textures. One background thread fetches (see core `avatars`);
//! results arrive on a channel and become textures on the next frame.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};

use mailopen_core::Logo;
use mailopen_core::worker::Wake;

pub enum Slot {
    Loading,
    Missing,
    Ready(egui::TextureHandle),
}

pub struct AvatarCache {
    slots: HashMap<String, Slot>,
    asked: HashSet<String>,
    tx: Sender<String>,
    rx: Receiver<(String, Option<Logo>)>,
    pub enabled: bool,
}

impl AvatarCache {
    pub fn new(wake: Wake, enabled: bool) -> Self {
        let (tx, jobs) = channel::<String>();
        let (done_tx, rx) = channel::<(String, Option<Logo>)>();
        let spawned = std::thread::Builder::new()
            .name("mailopen-logos".into())
            .spawn(move || {
                while let Ok(email) = jobs.recv() {
                    let logo = mailopen_core::avatars::fetch(&email);
                    if done_tx.send((email, logo)).is_err() {
                        break;
                    }
                    wake();
                }
            });
        if let Err(e) = spawned {
            log::warn!("thread loghi: {e}");
        }
        AvatarCache {
            slots: HashMap::new(),
            asked: HashSet::new(),
            tx,
            rx,
            enabled,
        }
    }

    /// Turn finished downloads into textures. Call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((email, logo)) = self.rx.try_recv() {
            let slot = match logo {
                Some(l) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [l.width as usize, l.height as usize],
                        &l.rgba,
                    );
                    Slot::Ready(ctx.load_texture(
                        format!("logo-{email}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                None => Slot::Missing,
            };
            self.slots.insert(email, slot);
        }
    }

    /// The logo texture for an address, asking for it the first time.
    pub fn get(&mut self, email: &str) -> Option<egui::load::SizedTexture> {
        if !self.enabled || !email.contains('@') {
            return None;
        }
        let key = email.trim().to_ascii_lowercase();
        match self.slots.get(&key) {
            Some(Slot::Ready(h)) => Some(egui::load::SizedTexture::new(
                h.id(),
                egui::vec2(64.0, 64.0),
            )),
            Some(_) => None,
            None => {
                if self.asked.insert(key.clone()) {
                    self.slots.insert(key.clone(), Slot::Loading);
                    let _ = self.tx.send(key);
                }
                None
            }
        }
    }
}
