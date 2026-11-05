//! Application assets: gpui-component's icon bundle plus a small refresh icon.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == "icons/refresh-cw.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/refresh-cw.svg"
            ))));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_component_assets::Assets.list(path)?;
        if "icons/refresh-cw.svg".starts_with(path) {
            paths.push("icons/refresh-cw.svg".into());
        }
        Ok(paths)
    }
}
