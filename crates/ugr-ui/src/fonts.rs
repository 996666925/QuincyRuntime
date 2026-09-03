//! Typeface initialization shared by the raster renderer.

use std::sync::OnceLock;

use skia_safe::{FontMgr, FontStyle, Typeface};

pub(crate) fn default_typeface() -> Result<Typeface, String> {
    static TYPEFACE: OnceLock<Option<Typeface>> = OnceLock::new();
    TYPEFACE
        .get_or_init(|| FontMgr::new().legacy_make_typeface(None, FontStyle::normal()))
        .clone()
        .ok_or_else(|| "failed to load the system default typeface".to_owned())
}
