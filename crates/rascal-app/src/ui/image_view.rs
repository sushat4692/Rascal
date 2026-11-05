//! Image preview and image-to-image comparison.

use std::{io::Cursor, path::Path, sync::Arc};

use gpui::prelude::*;
use gpui::*;
use gpui_component::theme::ActiveTheme;
use gpui_component::v_flex;
use image::{DynamicImage, ImageFormat as RasterFormat, ImageReader, Rgba, RgbaImage};
use rascal_core::{ImageDiff, ImageFile};

use crate::i18n::LanguageExt;

const MAX_IMAGE_DIMENSION: u32 = 8192;
const MAX_DIFF_PIXELS: u64 = 16_000_000;
const DIFF_THRESHOLD: u8 = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageDiffMode {
    #[default]
    SideBySide,
    Slider,
    Difference,
}

impl Global for ImageDiffMode {}

impl ImageDiffMode {
    pub fn key(self) -> &'static str {
        match self {
            ImageDiffMode::SideBySide => "side-by-side",
            ImageDiffMode::Slider => "slider",
            ImageDiffMode::Difference => "difference",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "side-by-side" => Some(ImageDiffMode::SideBySide),
            "slider" => Some(ImageDiffMode::Slider),
            "difference" => Some(ImageDiffMode::Difference),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ImageDiffMode::SideBySide => "並べて比較",
            ImageDiffMode::Slider => "スライダー比較",
            ImageDiffMode::Difference => "差分ハイライト",
        }
    }
}

pub fn current_mode(cx: &App) -> ImageDiffMode {
    cx.try_global::<ImageDiffMode>()
        .copied()
        .unwrap_or_default()
}

pub fn set_mode(mode: ImageDiffMode, cx: &mut App) {
    if current_mode(cx) == mode {
        return;
    }
    cx.set_global(mode);
    cx.refresh_windows();
}

pub fn is_image_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg"
            )
        })
        .unwrap_or(false)
}

#[derive(Clone)]
pub struct LoadedImage {
    path: String,
    size: u64,
    width: Option<u32>,
    height: Option<u32>,
    format: &'static str,
    bytes: Arc<Vec<u8>>,
    image: Arc<Image>,
}

impl LoadedImage {
    fn from_core(file: ImageFile) -> Result<Self, String> {
        if file
            .bytes
            .starts_with(b"version https://git-lfs.github.com/spec/")
        {
            return Err("Git LFSのpointerのため表示できません".to_string());
        }

        let ext = Path::new(&file.path)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_svg = ext == "svg"
            || (file.bytes.starts_with(b"<?xml")
                && file.bytes.windows(4).any(|window| window == b"<svg"));

        let raster = if is_svg {
            None
        } else {
            Some(
                image::guess_format(&file.bytes)
                    .ok()
                    .or_else(|| RasterFormat::from_extension(&ext))
                    .ok_or_else(|| "対応していない画像形式です".to_string())?,
            )
        };

        let (gpui_format, format_label, width, height) = if is_svg {
            (gpui::ImageFormat::Svg, "SVG", None, None)
        } else {
            let format = raster.expect("raster format");
            let gpui_format = match format {
                RasterFormat::Png => gpui::ImageFormat::Png,
                RasterFormat::Jpeg => gpui::ImageFormat::Jpeg,
                RasterFormat::Gif => gpui::ImageFormat::Gif,
                RasterFormat::WebP => gpui::ImageFormat::Webp,
                RasterFormat::Bmp => gpui::ImageFormat::Bmp,
                RasterFormat::Tiff => gpui::ImageFormat::Tiff,
                _ => return Err("対応していない画像形式です".to_string()),
            };
            let dimensions = ImageReader::with_format(Cursor::new(&file.bytes), format)
                .into_dimensions()
                .map_err(|_| "画像を読み込めません".to_string())?;
            if dimensions.0 > MAX_IMAGE_DIMENSION || dimensions.1 > MAX_IMAGE_DIMENSION {
                return Err("画像が大きすぎます".to_string());
            }
            let label = match format {
                RasterFormat::Png => "PNG",
                RasterFormat::Jpeg => "JPEG",
                RasterFormat::Gif => "GIF",
                RasterFormat::WebP => "WEBP",
                RasterFormat::Bmp => "BMP",
                RasterFormat::Tiff => "TIFF",
                _ => "IMAGE",
            };
            (gpui_format, label, Some(dimensions.0), Some(dimensions.1))
        };

        let bytes = Arc::new(file.bytes);
        let image = Arc::new(Image::from_bytes(gpui_format, (*bytes).clone()));
        Ok(Self {
            path: file.path,
            size: file.size,
            width,
            height,
            format: format_label,
            bytes,
            image,
        })
    }

    fn metadata(&self) -> String {
        let dimensions = match (self.width, self.height) {
            (Some(width), Some(height)) => format!("{width}×{height}"),
            _ => "—".to_string(),
        };
        format!(
            "{} · {} · {}",
            self.format,
            dimensions,
            format_size(self.size)
        )
    }

    fn name(&self) -> &str {
        Path::new(&self.path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&self.path)
    }
}

struct DiffReady {
    image: Arc<Image>,
    changed: u64,
    total: u64,
    width: u32,
    height: u32,
    mismatch: bool,
}

enum DiffState {
    Idle,
    Loading,
    Ready(DiffReady),
    Error(String),
}

pub struct ImageView {
    single: Option<LoadedImage>,
    old: Option<LoadedImage>,
    new: Option<LoadedImage>,
    error: Option<String>,
    stage_width: f32,
    stage_height: f32,
    split_ratio: f32,
    dragging: bool,
    drag_origin_x: f32,
    drag_origin_ratio: f32,
    diff_state: DiffState,
}

impl ImageView {
    pub fn single(file: ImageFile, cx: &mut App) -> Entity<Self> {
        cx.new(|_cx| {
            let (single, error) = match LoadedImage::from_core(file) {
                Ok(image) => (Some(image), None),
                Err(message) => (None, Some(message)),
            };
            Self {
                single,
                old: None,
                new: None,
                error,
                stage_width: 600.0,
                stage_height: 400.0,
                split_ratio: 0.5,
                dragging: false,
                drag_origin_x: 0.0,
                drag_origin_ratio: 0.5,
                diff_state: DiffState::Idle,
            }
        })
    }

    pub fn diff(diff: ImageDiff, cx: &mut App) -> Entity<Self> {
        cx.new(|_cx| {
            let mut error = None;
            let old = match diff.old {
                Some(file) => match LoadedImage::from_core(file) {
                    Ok(image) => Some(image),
                    Err(message) => {
                        error = Some(message);
                        None
                    }
                },
                None => None,
            };
            let new = match diff.new {
                Some(file) => match LoadedImage::from_core(file) {
                    Ok(image) => Some(image),
                    Err(message) => {
                        error = Some(message);
                        None
                    }
                },
                None => None,
            };
            Self {
                single: None,
                old,
                new,
                error,
                stage_width: 600.0,
                stage_height: 400.0,
                split_ratio: 0.5,
                dragging: false,
                drag_origin_x: 0.0,
                drag_origin_ratio: 0.5,
                diff_state: DiffState::Idle,
            }
        })
    }

    fn has_both(&self) -> bool {
        self.old.is_some() && self.new.is_some()
    }

    fn ensure_difference(&mut self, cx: &mut Context<Self>) {
        if !self.has_both() || !matches!(self.diff_state, DiffState::Idle) {
            return;
        }
        let (Some(old), Some(new)) = (&self.old, &self.new) else {
            return;
        };
        let old_bytes = old.bytes.clone();
        let new_bytes = new.bytes.clone();
        self.diff_state = DiffState::Loading;
        let task = cx
            .background_executor()
            .spawn(async move { compute_difference(&old_bytes, &new_bytes) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.diff_state = match result {
                    Ok(result) => DiffState::Ready(DiffReady {
                        image: Arc::new(Image::from_bytes(gpui::ImageFormat::Png, result.png)),
                        changed: result.changed,
                        total: result.total,
                        width: result.width,
                        height: result.height,
                        mismatch: result.mismatch,
                    }),
                    Err(message) => DiffState::Error(message),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn render_single(&self, image: &LoadedImage, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(image.metadata()),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .bg(theme.muted.opacity(0.12))
                    .child(
                        img(image.image.clone())
                            .object_fit(ObjectFit::Contain)
                            .w_full()
                            .h_full(),
                    ),
            )
            .into_any_element()
    }

    fn render_side_by_side(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let old = self.old.as_ref().map(|image| {
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .border_r_1()
                .border_color(theme.border)
                .child(metadata_line(image, cx))
                .child(image_stage(image, theme))
                .into_any_element()
        });
        let new = self
            .new
            .as_ref()
            .map(|image| {
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(metadata_line(image, cx))
                    .child(image_stage(image, theme))
                    .into_any_element()
            })
            .unwrap_or_else(|| missing_pane(cx));

        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .when_some(old, |this, old| this.child(old))
            .child(new)
            .into_any_element()
    }

    fn render_slider(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(old) = self.old.as_ref() else {
            return missing_pane(cx);
        };
        let Some(new) = self.new.as_ref() else {
            return missing_pane(cx);
        };

        let width = self.stage_width.max(1.0);
        let stage_height = self.stage_height.max(1.0);
        let split = (self.split_ratio * width).clamp(0.0, width);
        let old_w = old.width.map(|width| width as f32).unwrap_or(width);
        let old_h = old
            .height
            .map(|height| height as f32)
            .unwrap_or(stage_height);
        let new_w = new.width.map(|width| width as f32).unwrap_or(width);
        let new_h = new
            .height
            .map(|height| height as f32)
            .unwrap_or(stage_height);
        let max_w = old_w.max(new_w).max(1.0);
        let max_h = old_h.max(new_h).max(1.0);
        let scale = (width / max_w).min(stage_height / max_h).min(1.0);
        let old_display_w = old_w * scale;
        let old_display_h = old_h * scale;
        let new_display_w = new_w * scale;
        let new_display_h = new_h * scale;
        let old_left = (width - old_display_w) * 0.5;
        let old_top = (stage_height - old_display_h) * 0.5;
        let new_left = (width - new_display_w) * 0.5;
        let new_top = (stage_height - new_display_h) * 0.5;
        let entity = cx.entity();
        let measure = canvas(
            move |bounds, window, cx| {
                let width: f32 = bounds.size.width.into();
                let height: f32 = bounds.size.height.into();
                window.defer(cx, move |_, cx| {
                    entity.update(cx, |this, cx| {
                        if (this.stage_width - width).abs() > 0.5
                            || (this.stage_height - height).abs() > 0.5
                        {
                            this.stage_width = width;
                            this.stage_height = height;
                            cx.notify();
                        }
                    });
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let new_layer = div()
            .absolute()
            .top(px(new_top))
            .left(px(new_left))
            .w(px(new_display_w))
            .h(px(new_display_h))
            .child(img(new.image.clone()).w_full().h_full());
        let old_layer = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(split))
            .h_full()
            .overflow_hidden()
            .child(
                img(old.image.clone())
                    .absolute()
                    .top(px(old_top))
                    .left(px(old_left))
                    .w(px(old_display_w))
                    .h(px(old_display_h)),
            );
        let divider = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(split - 0.5))
            .w(px(1.0))
            .bg(theme.foreground.opacity(0.8));
        let handle = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(split - 10.0))
            .w(px(20.0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(18.0))
                    .rounded_full()
                    .bg(theme.foreground)
                    .border_1()
                    .border_color(theme.border),
            );

        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(old.metadata()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(new.metadata()),
                    ),
            )
            .child(
                div()
                    .id("image-slider-stage")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            if this.has_both() && this.stage_width > 0.0 {
                                this.dragging = true;
                                this.drag_origin_x = f32::from(event.position.x);
                                this.drag_origin_ratio = this.split_ratio;
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if this.dragging && this.stage_width > 0.0 {
                            let delta = f32::from(event.position.x) - this.drag_origin_x;
                            this.split_ratio =
                                (this.drag_origin_ratio + delta / this.stage_width).clamp(0.0, 1.0);
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if this.dragging {
                                this.dragging = false;
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if this.dragging {
                                this.dragging = false;
                                cx.notify();
                            }
                        }),
                    )
                    .child(measure)
                    .child(new_layer)
                    .child(old_layer)
                    .child(divider)
                    .child(handle),
            )
            .into_any_element()
    }

    fn render_difference(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        match &self.diff_state {
            DiffState::Loading => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("差分を計算中…"))
                .into_any_element(),
            DiffState::Error(message) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.danger)
                .child(localized_error(cx, message))
                .into_any_element(),
            DiffState::Ready(diff) => {
                let stats = if diff.mismatch {
                    cx.tr("画像サイズが異なります")
                } else if diff.total > 0 {
                    format!(
                        "{}: {} / {} ({:.2}%)",
                        cx.tr("差分ピクセル"),
                        diff.changed,
                        diff.total,
                        diff.changed as f64 / diff.total as f64 * 100.0
                    )
                    .into()
                } else {
                    cx.tr("差分なし")
                };
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(stats),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{}×{}", diff.width, diff.height)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .p_4()
                            .bg(theme.muted.opacity(0.2))
                            .child(
                                img(diff.image.clone())
                                    .object_fit(ObjectFit::Contain)
                                    .w_full()
                                    .h_full(),
                            ),
                    )
                    .into_any_element()
            }
            DiffState::Idle => div().flex_1().into_any_element(),
        }
    }
}

fn metadata_line(image: &LoadedImage, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(theme.foreground)
                .child(image.name().to_string()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(image.metadata()),
        )
        .into_any_element()
}

fn image_stage(image: &LoadedImage, theme: &gpui_component::theme::Theme) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .w_full()
        .flex()
        .items_center()
        .justify_center()
        .p_3()
        .bg(theme.muted.opacity(0.12))
        .child(
            img(image.image.clone())
                .object_fit(ObjectFit::Contain)
                .w_full()
                .h_full(),
        )
        .into_any_element()
}

fn missing_pane(cx: &mut App) -> AnyElement {
    div()
        .flex_1()
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(cx.tr("なし"))
        .into_any_element()
}

fn mode_toggle(id: &str, current: ImageDiffMode, cx: &mut App) -> AnyElement {
    let theme = cx.theme();
    let mut group = div()
        .flex()
        .items_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(theme.border)
        .overflow_hidden();
    for mode in [
        ImageDiffMode::SideBySide,
        ImageDiffMode::Slider,
        ImageDiffMode::Difference,
    ] {
        let selected = mode == current;
        group = group.child(
            div()
                .id(ElementId::Name(
                    format!("image-mode-{id}-{}", mode.key()).into(),
                ))
                .px_2()
                .py_0p5()
                .text_xs()
                .cursor_pointer()
                .when(selected, |this| {
                    this.bg(theme.primary.opacity(0.22))
                        .text_color(theme.foreground)
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(!selected, |this| {
                    this.text_color(theme.muted_foreground)
                        .hover(|this| this.bg(theme.list_hover))
                })
                .on_click(move |_, _, cx| set_mode(mode, cx))
                .child(cx.tr(mode.label())),
        );
    }
    group.into_any_element()
}

impl Render for ImageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if current_mode(cx) == ImageDiffMode::Difference {
            self.ensure_difference(cx);
        }
        let theme = cx.theme();
        window.set_rem_size(theme.font_size);

        if let Some(error) = &self.error {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.danger)
                .child(localized_error(cx, error))
                .into_any_element();
        }

        if let Some(single) = &self.single {
            return self.render_single(single, cx);
        }

        if !self.has_both() {
            if let Some(single) = self.new.as_ref().or(self.old.as_ref()) {
                return self.render_single(single, cx);
            }
            return missing_pane(cx);
        }
        let old = self.old.as_ref().expect("old image");
        let new = self.new.as_ref().expect("new image");

        let mode = current_mode(cx);

        let toolbar = div()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{} → {}", old.path, new.path)),
            )
            .child(mode_toggle("image-diff", mode, cx));

        let body = match mode {
            ImageDiffMode::SideBySide => self.render_side_by_side(cx),
            ImageDiffMode::Slider => self.render_slider(cx),
            ImageDiffMode::Difference => self.render_difference(cx),
        };

        v_flex()
            .size_full()
            .min_h_0()
            .child(toolbar)
            .child(body)
            .into_any_element()
    }
}

struct DiffComputation {
    png: Vec<u8>,
    changed: u64,
    total: u64,
    width: u32,
    height: u32,
    mismatch: bool,
}

fn compute_difference(old: &[u8], new: &[u8]) -> Result<DiffComputation, String> {
    let old = image::load_from_memory(old)
        .map_err(|_| "画像を読み込めません".to_string())?
        .to_rgba8();
    let new = image::load_from_memory(new)
        .map_err(|_| "画像を読み込めません".to_string())?
        .to_rgba8();

    let width = old.width().max(new.width());
    let height = old.height().max(new.height());
    let total = width as u64 * height as u64;
    if total > MAX_DIFF_PIXELS {
        return Err("画像が大きすぎます".to_string());
    }

    let mut out = RgbaImage::new(width, height);
    let mut changed = 0u64;
    for y in 0..height {
        for x in 0..width {
            let a = old.get_pixel_checked(x, y);
            let b = new.get_pixel_checked(x, y);
            let differs = match (a, b) {
                (None, None) => false,
                (Some(a), Some(b)) => {
                    a.0[0].abs_diff(b.0[0]) > DIFF_THRESHOLD
                        || a.0[1].abs_diff(b.0[1]) > DIFF_THRESHOLD
                        || a.0[2].abs_diff(b.0[2]) > DIFF_THRESHOLD
                        || a.0[3].abs_diff(b.0[3]) > DIFF_THRESHOLD
                }
                _ => true,
            };
            if differs {
                changed += 1;
                out.put_pixel(x, y, Rgba([255, 80, 112, 255]));
            }
        }
    }

    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(out)
        .write_to(&mut png, RasterFormat::Png)
        .map_err(|_| "差分画像を作成できません".to_string())?;

    Ok(DiffComputation {
        png: png.into_inner(),
        changed,
        total,
        width,
        height,
        mismatch: old.width() != new.width() || old.height() != new.height(),
    })
}

fn localized_error(cx: &App, message: &str) -> String {
    let key = match message {
        "Git LFSのpointerのため表示できません" => "Git LFSのpointerのため表示できません",
        "対応していない画像形式です" => "対応していない画像形式です",
        "画像が大きすぎます" => "画像が大きすぎます",
        "画像を読み込めません" => "画像を読み込めません",
        "画像ファイルではありません" => "画像ファイルではありません",
        _ => return message.to_string(),
    };
    cx.tr(key).to_string()
}

fn format_size(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let size = size as f64;
    if size >= MB {
        format!("{:.1} MB", size / MB)
    } else if size >= KB {
        format!("{:.1} KB", size / KB)
    } else {
        format!("{} B", size as u64)
    }
}

#[cfg(test)]
mod tests;
