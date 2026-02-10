use std::path::PathBuf;
use tempfile::Builder;

#[cfg(feature = "clipboard")]
use arboard;

#[derive(Debug)]
pub enum PasteImageError {
    ClipboardUnavailable(String),
    #[allow(dead_code)]
    NoImage(String),
    DecodeFailed(String),
    EncodeFailed(String),
    IoError(String),
}

impl std::fmt::Display for PasteImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasteImageError::ClipboardUnavailable(msg) => write!(f, "clipboard unavailable: {msg}"),
            PasteImageError::NoImage(msg) => write!(f, "no image on clipboard: {msg}"),
            PasteImageError::DecodeFailed(msg) => write!(f, "could not decode image: {msg}"),
            PasteImageError::EncodeFailed(msg) => write!(f, "could not encode image: {msg}"),
            PasteImageError::IoError(msg) => write!(f, "io error: {msg}"),
        }
    }
}
impl std::error::Error for PasteImageError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodedImageFormat {
    Png,
}

#[derive(Debug, Clone)]
pub struct PastedImageInfo {
    pub width: u32,
    pub height: u32,
    #[allow(dead_code)]
    pub encoded_format: EncodedImageFormat,
}

/// Capture image from system clipboard, encode to PNG, and return bytes plus metadata.
#[cfg(feature = "clipboard")]
pub fn paste_image_as_png() -> Result<(Vec<u8>, PastedImageInfo), PasteImageError> {
    tracing::debug!("attempting clipboard image read");
    let mut cb = arboard::Clipboard::new()
        .map_err(|e| PasteImageError::ClipboardUnavailable(e.to_string()))?;
    let img = cb
        .get_image()
        .map_err(|e| PasteImageError::NoImage(e.to_string()))?;
    let w = img.width as u32;
    let h = img.height as u32;
    let expected_len = w as usize * h as usize * 4;
    if img.bytes.len() != expected_len {
        return Err(PasteImageError::EncodeFailed(format!("invalid buffer size: expected {expected_len}, got {}", img.bytes.len())));
    }

    let mut png: Vec<u8> = Vec::new();
    let rgba_img = image::RgbaImage::from_raw(w, h, img.bytes.into_owned()).unwrap();
    let dyn_img = image::DynamicImage::ImageRgba8(rgba_img);
    tracing::debug!("clipboard image decoded RGBA {w}x{h}");
    {
        let mut cursor = std::io::Cursor::new(&mut png);
        dyn_img
            .write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| PasteImageError::EncodeFailed(e.to_string()))?;
    }

    tracing::debug!(
        "clipboard image encoded to PNG ({len} bytes)",
        len = png.len()
    );
    Ok((
        png,
        PastedImageInfo { width: w, height: h, encoded_format: EncodedImageFormat::Png },
    ))
}

#[cfg(not(feature = "clipboard"))]
pub fn paste_image_as_png() -> Result<(Vec<u8>, PastedImageInfo), PasteImageError> {
    Err(PasteImageError::ClipboardUnavailable(
        "clipboard feature not enabled for this platform".to_string(),
    ))
}

/// Write clipboard PNG to a temporary file and return the path.
pub fn paste_image_to_temp_png() -> Result<(PathBuf, PastedImageInfo), PasteImageError> {
    let (png, info) = paste_image_as_png()?;
    let tmp = Builder::new()
        .prefix("codex-clipboard-")
        .suffix(".png")
        .tempfile()
        .map_err(|e| PasteImageError::IoError(e.to_string()))?;
    std::fs::write(tmp.path(), &png).map_err(|e| PasteImageError::IoError(e.to_string()))?;
    let (_file, path) = tmp.keep().map_err(|e| PasteImageError::IoError(e.error.to_string()))?;
    Ok((path, info))
}

/// Interpret pasted text as an image (data URL or raw base64) and write a PNG temporary file.
pub fn try_decode_base64_image_to_temp_png(pasted: &str) -> Result<(PathBuf, PastedImageInfo), PasteImageError> {
    let s = pasted.trim();
    if s.is_empty() {
        return Err(PasteImageError::DecodeFailed("empty".into()));
    }

    let (_maybe_mime, b64) = if let Some(rest) = s.strip_prefix("data:") {
        if let Some(idx) = rest.find(',') {
            let (head, tail) = rest.split_at(idx);
            if !head.contains(";base64") {
                return Err(PasteImageError::DecodeFailed("data URL without base64".into()));
            }
            (Some(head.split(';').next().unwrap_or("").to_string()), &tail[1..])
        } else {
            return Err(PasteImageError::DecodeFailed("malformed data URL".into()));
        }
    } else {
        let looks_imagey = s.starts_with("iVBORw0K")
            || s.starts_with("/9j/")
            || s.starts_with("R0lGODlh")
            || s.starts_with("R0lGODdh");
        if !looks_imagey {
            return Err(PasteImageError::DecodeFailed("not image-like base64".into()));
        }
        (None, s)
    };

    let compact: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(compact)
        .map_err(|e| PasteImageError::DecodeFailed(e.to_string()))?;

    let dyn_img = image::load_from_memory(&bytes)
        .map_err(|e| PasteImageError::DecodeFailed(e.to_string()))?;
    let (w, h) = (dyn_img.width(), dyn_img.height());

    let mut png: Vec<u8> = Vec::new();
    {
        let mut cursor = std::io::Cursor::new(&mut png);
        dyn_img
            .write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| PasteImageError::EncodeFailed(e.to_string()))?;
    }

    let tmp = Builder::new()
        .prefix("codex-clipboard-")
        .suffix(".png")
        .tempfile()
        .map_err(|e| PasteImageError::IoError(e.to_string()))?;
    std::fs::write(tmp.path(), &png).map_err(|e| PasteImageError::IoError(e.to_string()))?;
    let (_file, path) = tmp.keep().map_err(|e| PasteImageError::IoError(e.error.to_string()))?;

    tracing::debug!("decoded pasted base64 image to {w}x{h} PNG at {}", path.to_string_lossy());
    Ok((path, PastedImageInfo { width: w, height: h, encoded_format: EncodedImageFormat::Png }))
}

/// Normalize pasted text that may represent a filesystem path.
pub fn normalize_pasted_path(pasted: &str) -> Option<PathBuf> {
    let pasted = pasted.trim();

    if let Ok(url) = url::Url::parse(pasted)
        && url.scheme() == "file" {
            return url.to_file_path().ok();
        }

    let looks_like_windows_path = {
        let drive = pasted
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
            && pasted.get(1..2) == Some(":")
            && pasted
                .get(2..3)
                .map(|s| s == "\\" || s == "/")
                .unwrap_or(false);
        let unc = pasted.starts_with("\\\\");
        drive || unc
    };
    if looks_like_windows_path {
        return Some(PathBuf::from(pasted));
    }

    let parts: Vec<String> = shlex::Shlex::new(pasted).collect();
    if parts.len() == 1 {
        return parts.into_iter().next().map(PathBuf::from);
    }

    None
}

// Image format inference removed alongside clipboard image helpers.
