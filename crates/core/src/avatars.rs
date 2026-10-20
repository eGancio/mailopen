//! Sender logos: Gravatar for the address, then the domain's favicon (not for free-mail
//! domains, where it would show the provider's logo for everyone). Results are cached on
//! disk as 64×64 PNGs; misses are cached too and retried after a week.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Config;

pub const SIZE: u32 = 64;
const MAX_BYTES: u64 = 512 * 1024;
const MISS_TTL: u64 = 7 * 24 * 3600;

/// Decoded RGBA pixels, `SIZE`×`SIZE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Logo {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

const FREE_MAIL: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "outlook.it",
    "hotmail.com",
    "hotmail.it",
    "live.com",
    "live.it",
    "msn.com",
    "yahoo.com",
    "yahoo.it",
    "ymail.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "libero.it",
    "virgilio.it",
    "iol.it",
    "tiscali.it",
    "alice.it",
    "tin.it",
    "pec.it",
    "aruba.it",
    "legalmail.it",
    "fastmail.com",
    "protonmail.com",
    "proton.me",
    "email.it",
    "inwind.it",
];

fn cache_dir() -> Option<PathBuf> {
    Config::dir().ok().map(|d| d.join("avatars"))
}

fn key(email: &str) -> String {
    format!(
        "{:x}",
        md5::compute(email.trim().to_ascii_lowercase().as_bytes())
    )
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The logo for an address, from the cache or the network. `None` = no logo known.
pub fn fetch(email: &str) -> Option<Logo> {
    let email = email.trim().to_ascii_lowercase();
    let domain = email.rsplit('@').next().unwrap_or("").to_string();
    if !email.contains('@') || domain.is_empty() {
        return None;
    }
    let dir = cache_dir()?;
    let k = key(&email);
    let hit = dir.join(format!("{k}.png"));
    if let Ok(bytes) = std::fs::read(&hit) {
        return decode(&bytes, false);
    }
    let miss = dir.join(format!("{k}.none"));
    if let Ok(meta) = std::fs::metadata(&miss)
        && let Ok(modified) = meta.modified()
        && SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default()
            < Duration::from_secs(MISS_TTL)
    {
        return None;
    }
    let found = download(&email, &domain);
    let _ = std::fs::create_dir_all(&dir);
    match &found {
        Some(logo) => {
            let img = image::RgbaImage::from_raw(logo.width, logo.height, logo.rgba.clone());
            if let Some(img) = img {
                let _ = img.save_with_format(&hit, image::ImageFormat::Png);
            }
        }
        None => {
            let _ = std::fs::write(&miss, now().to_string());
        }
    }
    found
}

fn download(email: &str, domain: &str) -> Option<Logo> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(8)))
            .http_status_as_error(false)
            .build(),
    );
    let mut urls = vec![format!(
        "https://www.gravatar.com/avatar/{}?d=404&s={SIZE}",
        key(email)
    )];
    if !FREE_MAIL.contains(&domain) {
        // The site's own icon first (what the HTML declares, then the classic path), then the
        // DuckDuckGo icon service as a fallback.
        urls.extend(html_icon_links(&agent, domain));
        urls.push(format!("https://{domain}/favicon.ico"));
        urls.push(format!("https://icons.duckduckgo.com/ip3/{domain}.ico"));
    }
    for url in &urls {
        let Some(bytes) = get(&agent, url) else {
            continue;
        };
        // Placeholder icons are tiny; a real logo is not.
        if bytes.len() < 200 {
            continue;
        }
        if let Some(logo) = decode(&bytes, true) {
            return Some(logo);
        }
    }
    None
}

/// `<link rel="icon" href=…>` and apple-touch-icon URLs from the site's home page.
fn html_icon_links(agent: &ureq::Agent, domain: &str) -> Vec<String> {
    let Some(bytes) = get(agent, &format!("https://{domain}/")) else {
        return Vec::new();
    };
    let html = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
    let mut out = Vec::new();
    let mut rest = html.as_str();
    while let Some(i) = rest.find("<link") {
        let Some(tag_rest) = rest.get(i..) else { break };
        let end = tag_rest.find('>').unwrap_or(tag_rest.len());
        let tag = tag_rest.get(..end).unwrap_or("");
        if tag.contains("icon")
            && let Some(href) = attr(tag, "href")
        {
            let url = if href.starts_with("http://") || href.starts_with("https://") {
                href.to_string()
            } else if href.starts_with("//") {
                format!("https:{href}")
            } else if href.starts_with('/') {
                format!("https://{domain}{href}")
            } else {
                format!("https://{domain}/{href}")
            };
            if !out.contains(&url) {
                out.push(url);
            }
        }
        rest = tag_rest.get(end..).unwrap_or("");
        if out.len() >= 4 {
            break;
        }
    }
    out
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let i = tag.find(&format!("{name}="))?;
    let after = tag.get(i + name.len() + 1..)?;
    let quote = after.chars().next()?;
    if quote == '"' || quote == '\'' {
        let inner = after.get(1..)?;
        let end = inner.find(quote)?;
        inner.get(..end)
    } else {
        let end = after.find([' ', '>', '/']).unwrap_or(after.len());
        after.get(..end)
    }
}

fn get(agent: &ureq::Agent, url: &str) -> Option<Vec<u8>> {
    let mut resp = agent.get(url).call().ok()?;
    if resp.status() != 200 {
        return None;
    }
    resp.body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .ok()
}

/// Decode any supported format, crop to a square and scale to `SIZE`.
fn decode(bytes: &[u8], resize: bool) -> Option<Logo> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if resize {
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 || w > 4096 || h > 4096 {
            return None;
        }
        let side = w.min(h);
        let cropped = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
        cropped.resize_exact(SIZE, SIZE, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(Logo {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_decoding() {
        assert_eq!(key("A@B.it"), key(" a@b.it "));
        assert_eq!(key("a@b.it").len(), 32);
        assert!(decode(b"not an image", true).is_none());
        // 2x1 PNG, resized to SIZE
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255]));
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let logo = decode(&png, true).unwrap();
        assert_eq!((logo.width, logo.height), (SIZE, SIZE));
        assert_eq!(logo.rgba.len(), (SIZE * SIZE * 4) as usize);
        assert!(fetch("nessuna-chiocciola").is_none());
        assert_eq!(
            attr("<link rel=\"icon\" href=\"/logo.jpg\">", "href"),
            Some("/logo.jpg")
        );
        assert_eq!(
            attr("<link rel=icon href=favicon.png>", "href"),
            Some("favicon.png")
        );
        assert_eq!(attr("<link rel=\"icon\">", "href"), None);
    }

    /// Needs the network and a real domain:
    /// `MAILOPEN_LOGO_EMAIL=info@bambutech.ai cargo test -p mailopen-core -- --ignored logo_live`
    #[test]
    #[ignore]
    fn logo_live() {
        let email =
            std::env::var("MAILOPEN_LOGO_EMAIL").unwrap_or_else(|_| "info@bambutech.ai".into());
        let domain = email.rsplit('@').next().unwrap_or("").to_string();
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        eprintln!("link icone: {:?}", html_icon_links(&agent, &domain));
        let logo = download(&email, &domain);
        assert!(logo.is_some(), "nessun logo per {email}");
        assert_eq!(logo.map(|l| (l.width, l.height)), Some((SIZE, SIZE)));
    }
}
