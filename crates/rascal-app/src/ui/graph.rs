use gpui::Hsla;
use gpui_component::theme::Theme;

pub fn lane_color(theme: &Theme, color: u8) -> Hsla {
    match color % 8 {
        0 => theme.primary,
        1 => theme.info,
        2 => theme.success,
        3 => theme.warning,
        4 => theme.danger,
        5 => theme.chart_1,
        6 => theme.chart_2,
        _ => theme.chart_3,
    }
}

pub fn gutter_width_for(lanes: usize) -> f32 {
    lanes.clamp(1, rascal_core::MAX_GRAPH_LANES) as f32 * 14.0 + 8.0
}
