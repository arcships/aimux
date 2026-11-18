//! Image size helpers that several image packages share.

use aimux_core::shared::Size;

/// The reduced `"W:H"` aspect ratio of a pixel size, or `None` when a side is
/// zero.
pub(crate) fn size_to_aspect_ratio(size: &Size) -> Option<String> {
    let (w, h) = (size.width(), size.height());
    if w == 0 || h == 0 {
        return None;
    }
    let g = gcd(w, h);
    Some(format!("{}:{}", w / g, h / g))
}

/// Greatest common divisor of two `u32` values.
fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}
