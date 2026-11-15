//! Remote images (thumbnails, team logos) decoded with Windows' own codecs into small RGBA
//! buffers. Worker threads only: decoding is synchronous WinRT.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapBounds, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

use crate::services::http;

/// Decoded straight-alpha RGBA, ready for `slint::SharedPixelBuffer`.
#[derive(Debug, PartialEq)]
pub struct Pixels {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub type Shared = Arc<Pixels>;

/// Largest source image we'll decode (thumbnails and logos are far smaller): ~40 megapixels.
const MAX_SOURCE_PIXELS: u64 = 40_000_000;

/// Whether `url` is HTTPS on one of `hosts` (or a subdomain of one). Feeds name their own image
/// URLs; only fetching from the services' known image hosts keeps a feed from making the app
/// contact arbitrary servers.
pub fn host_allowed(url: &str, hosts: &[&str]) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false; // no userinfo tricks ("https://i.ytimg.com@evil.example/")
    }
    let host = authority.split(':').next().unwrap_or("").to_ascii_lowercase();
    hosts.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// Downloads and decodes `url`, scaled and centre-cropped to exactly `width`×`height`.
pub fn fetch(url: &str, width: u32, height: u32) -> Option<Pixels> {
    let bytes = http::get_bytes(url).ok()?;
    decode(&bytes, width, height).ok()
}

pub fn decode(bytes: &[u8], width: u32, height: u32) -> windows::core::Result<Pixels> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(bytes)?;
    writer.StoreAsync()?.join()?;
    writer.DetachStream()?;
    stream.Seek(0)?;

    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let (pw, ph) = (decoder.PixelWidth()?.max(1), decoder.PixelHeight()?.max(1));
    // A hostile or broken image could claim enormous dimensions; don't let it near the decoder.
    if u64::from(pw) * u64::from(ph) > MAX_SOURCE_PIXELS {
        return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_INVALIDARG));
    }
    // Cover: scale so both sides fill the target, then crop the overflow evenly.
    let scale = (width as f64 / pw as f64).max(height as f64 / ph as f64);
    let (sw, sh) = (((pw as f64 * scale).round() as u32).max(width), ((ph as f64 * scale).round() as u32).max(height));
    let transform = BitmapTransform::new()?;
    transform.SetScaledWidth(sw)?;
    transform.SetScaledHeight(sh)?;
    transform.SetBounds(BitmapBounds { X: (sw - width) / 2, Y: (sh - height) / 2, Width: width, Height: height })?;
    transform.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    let rgba = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Rgba8,
            BitmapAlphaMode::Straight,
            &transform,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .join()?
        .DetachPixelData()?
        .to_vec();
    if rgba.len() != (width * height * 4) as usize {
        return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_UNEXPECTED));
    }
    Ok(Pixels { rgba, width, height })
}

/// How long a failed image is left alone before trying again. Long enough that a broken URL isn't
/// hammered every 30-second live refresh; short enough that a brief outage doesn't blank images
/// for good.
const RETRY_FAILED: Duration = Duration::from_secs(10 * 60);

/// Image downloads in flight at once (per widget). Browsers use ~6 connections per host.
const PARALLEL: usize = 6;

enum Entry {
    Ready(Shared),
    Failed(Instant),
}

/// Keeps the images a widget is currently showing, so a refresh only downloads new ones.
pub struct Cache {
    width: u32,
    height: u32,
    map: HashMap<String, Entry>,
    fetch: fn(&str, u32, u32) -> Option<Pixels>,
    hosts: &'static [&'static str],
}

impl Cache {
    /// A cache that only fetches from `hosts` (see [`host_allowed`]).
    pub fn new(width: u32, height: u32, hosts: &'static [&'static str]) -> Self {
        Self { width, height, map: HashMap::new(), fetch, hosts }
    }

    pub fn get(&mut self, url: &str) -> Option<Shared> {
        if !self.wants(url) {
            return match self.map.get(url) {
                Some(Entry::Ready(px)) => Some(px.clone()),
                _ => None,
            };
        }
        let px = (self.fetch)(url, self.width, self.height);
        self.store(url, px)
    }

    /// Downloads every image in `urls` that `get` would, several at once, so a cold start with
    /// dozens of logos or thumbnails costs a few round trips rather than one per image.
    pub fn prefetch<'a>(&mut self, urls: impl IntoIterator<Item = &'a str>) {
        let mut seen = std::collections::HashSet::new();
        let todo: Vec<&str> = urls.into_iter().filter(|u| self.wants(u) && seen.insert(*u)).collect();
        let (width, height, fetch) = (self.width, self.height, self.fetch);
        let got = crate::services::poller::fan_out(&todo, PARALLEL, |u| fetch(u, width, height));
        for (url, px) in todo.into_iter().zip(got) {
            self.store(url, px);
        }
    }

    /// Whether `url` is allowed and neither cached nor recently failed.
    fn wants(&self, url: &str) -> bool {
        !url.is_empty()
            && host_allowed(url, self.hosts)
            && match self.map.get(url) {
                Some(Entry::Ready(_)) => false,
                Some(Entry::Failed(at)) => at.elapsed() >= RETRY_FAILED,
                None => true,
            }
    }

    fn store(&mut self, url: &str, px: Option<Pixels>) -> Option<Shared> {
        let px = px.map(Arc::new);
        let entry = match &px {
            Some(px) => Entry::Ready(px.clone()),
            None => Entry::Failed(Instant::now()),
        };
        self.map.insert(url.to_string(), entry);
        px
    }

    /// Forget everything not in `keep`.
    pub fn retain<'a>(&mut self, keep: impl IntoIterator<Item = &'a str>) {
        let keep: std::collections::HashSet<&str> = keep.into_iter().collect();
        self.map.retain(|k, _| keep.contains(k.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn failing(_: &str, _: u32, _: u32) -> Option<super::Pixels> {
        CALLS.fetch_add(1, Ordering::SeqCst);
        None
    }

    #[test]
    fn only_known_hosts_over_https() {
        let hosts = &["ytimg.com", "redd.it"];
        assert!(super::host_allowed("https://i.ytimg.com/vi/x/mqdefault.jpg", hosts));
        assert!(super::host_allowed("https://I.YTIMG.COM:443/x", hosts));
        assert!(super::host_allowed("https://preview.redd.it/a.jpg?w=1", hosts));
        assert!(!super::host_allowed("http://i.ytimg.com/x", hosts), "plain http");
        assert!(!super::host_allowed("https://evilytimg.com/x", hosts), "suffix without a dot");
        assert!(!super::host_allowed("https://i.ytimg.com.evil.example/x", hosts));
        assert!(!super::host_allowed("https://i.ytimg.com@evil.example/x", hosts), "userinfo");
        assert!(!super::host_allowed("file:///C:/x.png", hosts));
        let mut cache = super::Cache { fetch: failing, ..super::Cache::new(4, 4, hosts) };
        let before = CALLS.load(Ordering::SeqCst);
        assert!(cache.get("https://tracker.example/pixel.png").is_none());
        assert_eq!(CALLS.load(Ordering::SeqCst), before, "never fetched");
    }

    #[test]
    fn failures_back_off_then_retry() {
        let mut cache = super::Cache { fetch: failing, ..super::Cache::new(4, 4, &["x"]) };
        assert!(cache.get("https://x/a.png").is_none());
        assert!(cache.get("https://x/a.png").is_none());
        assert_eq!(CALLS.load(Ordering::SeqCst), 1, "a fresh failure isn't retried at once");
        // Pretend the failure is old.
        if let Some(super::Entry::Failed(at)) = cache.map.get_mut("https://x/a.png") {
            *at -= super::RETRY_FAILED;
        }
        assert!(cache.get("https://x/a.png").is_none());
        assert_eq!(CALLS.load(Ordering::SeqCst), 2, "an old failure is retried");
        assert!(cache.get("").is_none());
        assert_eq!(CALLS.load(Ordering::SeqCst), 2, "empty URLs never fetch");
    }

    static PREFETCHED: AtomicUsize = AtomicUsize::new(0);

    fn tiny(url: &str, w: u32, h: u32) -> Option<super::Pixels> {
        PREFETCHED.fetch_add(1, Ordering::SeqCst);
        (!url.ends_with("bad")).then(|| super::Pixels { rgba: vec![0; (w * h * 4) as usize], width: w, height: h })
    }

    #[test]
    fn prefetch_fetches_each_wanted_image_once() {
        let mut cache = super::Cache { fetch: tiny, ..super::Cache::new(2, 2, &["x"]) };
        let urls: Vec<String> = (0..20).map(|i| format!("https://x/{i}.png")).collect();
        let mut list: Vec<&str> = urls.iter().map(|u| u.as_str()).collect();
        list.extend(["https://x/0.png", "https://evil.example/1.png", "", "https://x/bad"]);
        cache.prefetch(list.iter().copied());
        assert_eq!(PREFETCHED.load(Ordering::SeqCst), 21, "deduped; disallowed hosts and empties skipped");
        assert!(urls.iter().all(|u| cache.get(u).is_some()), "now served from the cache");
        assert!(cache.get("https://x/bad").is_none());
        cache.prefetch(list.iter().copied());
        assert_eq!(PREFETCHED.load(Ordering::SeqCst), 21, "nothing refetched, failures backed off");
    }

    #[test]
    fn decodes_and_crops_to_size() {
        crate::platform::init_worker_thread();
        // A 2×1 BMP: one red pixel, one blue pixel.
        #[rustfmt::skip]
        let bmp: Vec<u8> = [
            &b"BM"[..], &62u32.to_le_bytes(), &[0, 0, 0, 0], &54u32.to_le_bytes(),
            &40u32.to_le_bytes(), &2i32.to_le_bytes(), &1i32.to_le_bytes(), &1u16.to_le_bytes(), &24u16.to_le_bytes(),
            &[0; 24],
            &[0, 0, 255, 255, 0, 0, 0, 0],
        ].concat();
        let px = super::decode(&bmp, 4, 4).unwrap();
        assert_eq!((px.width, px.height, px.rgba.len()), (4, 4, 64));
    }
}
