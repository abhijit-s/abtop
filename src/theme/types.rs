//! Theme data types. No logic — see the parent module for constructors and lookup.

use ratatui::style::Color;

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub start: (u8, u8, u8),
    pub mid: (u8, u8, u8),
    pub end: (u8, u8, u8),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,

    // base
    pub main_bg: Color,
    pub main_fg: Color,
    pub title: Color,
    pub hi_fg: Color,
    pub selected_bg: Color,
    pub selected_fg: Color,
    pub inactive_fg: Color,
    pub graph_text: Color,
    pub meter_bg: Color,
    pub proc_misc: Color,
    pub div_line: Color,
    pub session_id: Color,

    // semantic
    pub status_fg: Color,
    pub warning_fg: Color,

    // box borders
    pub cpu_box: Color,
    pub mem_box: Color,
    pub net_box: Color,
    pub proc_box: Color,

    // gradients
    pub cpu_grad: Gradient,
    pub proc_grad: Gradient,
    pub used_grad: Gradient,
    pub free_grad: Gradient,
    pub cached_grad: Gradient,
}
