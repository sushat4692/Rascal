//! Always-visible scrollbars for the app's scroll areas.

use gpui::{AnyElement, ElementId, IntoElement, ParentElement, Styled, div};
use gpui_component::scroll::{Scrollbar, ScrollbarHandle, ScrollbarShow};

/// Space reserved on the trailing edge so the thumb does not cover content.
pub const CONTENT_GUTTER: f32 = 12.0;

pub fn vertical<H>(id: impl Into<ElementId>, handle: &H) -> AnyElement
where
    H: ScrollbarHandle + Clone,
{
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .child(
            Scrollbar::vertical(handle)
                .id(id)
                .scrollbar_show(ScrollbarShow::Always),
        )
        .into_any_element()
}
