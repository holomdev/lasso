//! The application icon.
//!
//! The `.ico` is embedded in the executable rather than loaded from disk, so a
//! copied binary still shows the right icon in the notification area.
//!
//! `CreateIconFromResourceEx` wants the raw bits of a single icon image, not the
//! `.ico` container, so the container is parsed here to pick the entry matching
//! the size Windows asks for. The parsing is pure and covered by tests against
//! the real bundled file, which also catches a corrupted asset.

use anyhow::{Context, Result};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, HICON, IMAGE_FLAGS};

/// The icon shipped inside the executable.
const ICON_BYTES: &[u8] = include_bytes!("../assets/lasso.ico");

/// Header sizes from the `.ico` container format.
const ICONDIR_SIZE: usize = 6;
const ICONDIRENTRY_SIZE: usize = 16;

/// `dwVer` passed to `CreateIconFromResourceEx`: the icon format version.
const ICON_RESOURCE_VERSION: u32 = 0x0003_0000;

/// One image inside the `.ico` container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub width: u32,
    pub height: u32,
    /// Size of the image in bytes.
    pub bytes: u32,
    /// Where the image starts inside the file.
    pub offset: u32,
}

fn read_u8(bytes: &[u8], at: usize) -> Option<u8> {
    bytes.get(at).copied()
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Every image the container holds, in file order.
///
/// A stored dimension of zero means 256 pixels: that is how the format fits 256
/// into a single byte.
pub fn entries(ico: &[u8]) -> Vec<Entry> {
    let Some(count) = read_u16(ico, 4) else {
        return Vec::new();
    };

    let dimension = |value: u8| if value == 0 { 256 } else { u32::from(value) };
    let mut result = Vec::new();
    for index in 0..count as usize {
        let base = ICONDIR_SIZE + index * ICONDIRENTRY_SIZE;
        let (Some(width), Some(height), Some(bytes), Some(offset)) = (
            read_u8(ico, base),
            read_u8(ico, base + 1),
            read_u32(ico, base + 8),
            read_u32(ico, base + 12),
        ) else {
            break;
        };
        result.push(Entry {
            width: dimension(width),
            height: dimension(height),
            bytes,
            offset,
        });
    }
    result
}

/// The image closest to `wanted` pixels, never one smaller than asked for.
///
/// Picking the smallest image that still covers the request avoids handing
/// Windows something it would have to enlarge, which is what makes a tray icon
/// look blurry. When nothing is large enough, the largest available is used.
pub fn best_entry(ico: &[u8], wanted: u32) -> Option<Entry> {
    let all = entries(ico);
    all.iter()
        .filter(|entry| entry.width >= wanted)
        .min_by_key(|entry| entry.width)
        .or_else(|| all.iter().max_by_key(|entry| entry.width))
        .copied()
}

/// Load the embedded icon at `size` pixels.
pub fn icon(size: u32) -> Result<HICON> {
    let entry = best_entry(ICON_BYTES, size)
        .with_context(|| format!("the bundled icon has no image for {size}px"))?;
    let start = entry.offset as usize;
    let image = ICON_BYTES
        .get(start..start + entry.bytes as usize)
        .context("the bundled icon is truncated")?;

    unsafe {
        CreateIconFromResourceEx(
            image,
            true.into(),
            ICON_RESOURCE_VERSION,
            size as i32,
            size as i32,
            IMAGE_FLAGS(0),
        )
        .context("CreateIconFromResourceEx failed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_icon_has_the_expected_sizes() {
        let sizes: Vec<u32> = entries(ICON_BYTES).iter().map(|entry| entry.width).collect();
        assert_eq!(sizes, vec![16, 24, 32, 48, 64, 128, 256]);
    }

    #[test]
    fn every_entry_points_inside_the_file() {
        for entry in entries(ICON_BYTES) {
            let end = entry.offset as usize + entry.bytes as usize;
            assert!(entry.bytes > 0, "{entry:?} is empty");
            assert!(
                end <= ICON_BYTES.len(),
                "{entry:?} runs past the end of the file"
            );
        }
    }

    #[test]
    fn the_closest_size_is_chosen_without_shrinking() {
        assert_eq!(best_entry(ICON_BYTES, 16).unwrap().width, 16);
        assert_eq!(best_entry(ICON_BYTES, 32).unwrap().width, 32);
        assert_eq!(
            best_entry(ICON_BYTES, 20).unwrap().width,
            24,
            "a 16px image must not be enlarged to cover 20px"
        );
        assert_eq!(best_entry(ICON_BYTES, 100).unwrap().width, 128);
        assert_eq!(
            best_entry(ICON_BYTES, 4096).unwrap().width,
            256,
            "the largest image when nothing is big enough"
        );
    }

    #[test]
    fn garbage_is_rejected_rather_than_guessed() {
        assert!(entries(&[]).is_empty());
        assert!(entries(&[0, 0, 1, 0]).is_empty());
        assert!(best_entry(&[], 16).is_none());
    }
}
