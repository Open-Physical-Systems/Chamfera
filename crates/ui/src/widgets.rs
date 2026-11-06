//! The interface's building blocks, drawn from the theme tokens: the egui style, ribbon
//! tools and groups, icon buttons, tree rows, dialogs and buttons. The measurements are
//! the spacing and type scales of `docs/design/ui.md`.

use crate::color;
use crate::icons::{Icon, Icons};
use chamfera_render::Theme;
use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Margin, Rect, Response, Sense, Shadow, Stroke,
    StrokeKind, Ui, Vec2, vec2,
};

// Spacing scale (points).
pub const S1: f32 = 2.0;
pub const S2: f32 = 4.0;
pub const S3: f32 = 8.0;
pub const S4: f32 = 12.0;
pub const S5: f32 = 16.0;

// Corner radii.
pub const R_SMALL: u8 = 4;
pub const R_MEDIUM: u8 = 6;
pub const R_LARGE: u8 = 8;

// Type scale (points).
pub const T_CAPTION: f32 = 10.5;
pub const T_SMALL: f32 = 11.5;
pub const T_BODY: f32 = 13.0;
pub const T_TITLE: f32 = 14.5;

// Fixed heights of the window areas.
pub const APP_BAR_HEIGHT: f32 = 38.0;
pub const TAB_HEIGHT: f32 = 26.0;
pub const TOOL_HEIGHT: f32 = 56.0;
pub const GROUP_LABEL_HEIGHT: f32 = 18.0;
/// The whole ribbon: tabs, tools, group captions and padding.
pub const RIBBON_HEIGHT: f32 = TAB_HEIGHT + TOOL_HEIGHT + GROUP_LABEL_HEIGHT + 16.0;
pub const TIMELINE_HEIGHT: f32 = 42.0;
pub const STATUS_HEIGHT: f32 = 26.0;
pub const ROW_HEIGHT: f32 = 26.0;

/// What widgets draw with: the theme and the icons.
#[derive(Clone, Copy)]
pub struct Look<'a> {
    pub theme: &'a Theme,
    pub icons: &'a Icons,
}

impl Look<'_> {
    pub fn c(&self, f: impl Fn(&Theme) -> chamfera_render::Rgba) -> Color32 {
        color(f(self.theme))
    }

    /// Icon colours for an enabled or disabled control.
    pub fn icon_colours(&self, enabled: bool) -> (Color32, Color32) {
        if enabled {
            (color(self.theme.icon), color(self.theme.icon_accent))
        } else {
            let muted = color(self.theme.text_muted).gamma_multiply(0.55);
            (muted, muted)
        }
    }

    pub fn icon(&self, ui: &Ui, icon: Icon, rect: Rect, enabled: bool) {
        let (b, a) = self.icon_colours(enabled);
        self.icons.paint(ui.painter(), icon, rect, b, a);
    }
}

/// The egui style for a theme: fonts sizes, spacing, rounded controls and every colour
/// from the tokens.
pub fn style_for(theme: &Theme, base: &egui::Style) -> egui::Style {
    use egui::{FontFamily, TextStyle};
    let t = theme;
    let mut s = base.clone();
    s.text_styles = [
        (
            TextStyle::Small,
            FontId::new(T_SMALL, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(T_BODY, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(T_BODY, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(T_TITLE, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();
    let sp = &mut s.spacing;
    sp.item_spacing = vec2(S3, 6.0);
    sp.button_padding = vec2(10.0, 4.0);
    sp.interact_size = vec2(24.0, 24.0);
    sp.window_margin = Margin::same(S4 as i8);
    sp.menu_margin = Margin::same(S2 as i8 + 2);
    sp.indent = 14.0;
    sp.combo_height = 240.0;
    sp.icon_width = 14.0;
    sp.icon_width_inner = 8.0;
    sp.icon_spacing = 6.0;
    sp.tooltip_width = 320.0;

    let dark = t.mode == chamfera_render::ThemeMode::Dark;
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = color(t.panel);
    v.window_fill = color(t.panel);
    v.window_stroke = Stroke::new(1.0, color(t.border));
    v.window_corner_radius = CornerRadius::same(R_LARGE);
    v.window_shadow = shadow(t, 18);
    v.popup_shadow = shadow(t, 10);
    v.menu_corner_radius = CornerRadius::same(R_MEDIUM);
    v.faint_bg_color = color(t.panel_alt);
    v.extreme_bg_color = color(t.chrome);
    v.text_edit_bg_color = Some(if dark {
        color(t.chrome)
    } else {
        Color32::WHITE
    });
    v.code_bg_color = color(t.panel_alt);
    v.override_text_color = Some(color(t.text));
    v.weak_text_color = Some(color(t.text_muted));
    v.hyperlink_color = color(t.accent);
    v.error_fg_color = color(t.error);
    v.warn_fg_color = color(t.warning);
    v.selection.bg_fill = color(t.selected);
    v.selection.stroke = Stroke::new(1.0, color(t.accent));
    v.text_cursor.stroke = Stroke::new(1.5, color(t.accent));
    v.striped = false;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;
    v.disabled_alpha = 0.45;
    v.handle_shape = egui::style::HandleShape::Circle;

    let w = &mut v.widgets;
    let radius = CornerRadius::same(R_SMALL + 1);
    w.noninteractive.bg_fill = color(t.panel);
    w.noninteractive.weak_bg_fill = color(t.panel);
    w.noninteractive.bg_stroke = Stroke::new(1.0, color(t.border));
    w.noninteractive.fg_stroke = Stroke::new(1.0, color(t.text));
    w.noninteractive.corner_radius = radius;
    w.inactive.bg_fill = color(t.panel_alt);
    w.inactive.weak_bg_fill = color(t.panel_alt);
    w.inactive.bg_stroke = Stroke::new(1.0, color(t.border));
    w.inactive.fg_stroke = Stroke::new(1.0, color(t.text));
    w.inactive.corner_radius = radius;
    w.inactive.expansion = 0.0;
    w.hovered.bg_fill = color(t.hover);
    w.hovered.weak_bg_fill = color(t.hover);
    w.hovered.bg_stroke = Stroke::new(1.0, color(t.cube_edge));
    w.hovered.fg_stroke = Stroke::new(1.0, color(t.text));
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;
    // Pressed, and keyboard focus: the accent ring shows where the keys go.
    w.active.bg_fill = color(t.selected);
    w.active.weak_bg_fill = color(t.selected);
    w.active.bg_stroke = Stroke::new(1.5, color(t.accent));
    w.active.fg_stroke = Stroke::new(1.0, color(t.text));
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;
    w.open.bg_fill = color(t.selected);
    w.open.weak_bg_fill = color(t.selected);
    w.open.bg_stroke = Stroke::new(1.0, color(t.border));
    w.open.fg_stroke = Stroke::new(1.0, color(t.text));
    w.open.corner_radius = radius;
    s.visuals = v;
    s
}

fn shadow(t: &Theme, blur: u8) -> Shadow {
    Shadow {
        offset: [0, (blur / 4) as i8],
        blur,
        spread: 0,
        color: color(t.shadow),
    }
}

/// A focus ring around a control the keyboard is on.
pub fn focus_ring(ui: &Ui, look: Look, response: &Response, radius: u8) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(1.0),
            CornerRadius::same(radius + 1),
            Stroke::new(1.5, look.c(|t| t.accent)),
            StrokeKind::Outside,
        );
    }
}

/// The background of a flat control: nothing at rest, `hover` under the pointer,
/// `selected` when active or pressed.
fn flat_background(ui: &Ui, look: Look, response: &Response, active: bool, radius: u8) {
    let fill = if active || response.is_pointer_button_down_on() {
        Some(look.c(|t| t.selected))
    } else if response.hovered() && response.sense.interactive() {
        Some(look.c(|t| t.hover))
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter()
            .rect_filled(response.rect, CornerRadius::same(radius), fill);
    }
    if active {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(radius),
            Stroke::new(1.0, look.c(|t| t.accent).gamma_multiply(0.6)),
            StrokeKind::Inside,
        );
    }
    focus_ring(ui, look, response, radius);
}

fn sense(enabled: bool) -> Sense {
    if enabled {
        Sense::click()
    } else {
        Sense::hover()
    }
}

/// A big ribbon tool: an icon over its label. `menu` adds a small chevron for tools that
/// open a list.
pub fn tool_button(
    ui: &mut Ui,
    look: Look,
    icon: Icon,
    text: &str,
    enabled: bool,
    active: bool,
    menu: bool,
) -> Response {
    let font = FontId::proportional(T_SMALL);
    let text_colour = if enabled {
        look.c(|t| t.text)
    } else {
        look.c(|t| t.text_muted).gamma_multiply(0.75)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, text_colour);
    let extra = if menu { 10.0 } else { 0.0 };
    let width = (galley.size().x + extra + 14.0).max(54.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, TOOL_HEIGHT), sense(enabled));
    if ui.is_rect_visible(rect) {
        flat_background(ui, look, &response, active, R_MEDIUM);
        let icon_rect = Rect::from_center_size(
            egui::pos2(rect.center().x, rect.min.y + 6.0 + 13.0),
            Vec2::splat(26.0),
        );
        look.icon(ui, icon, icon_rect, enabled);
        let text_pos = egui::pos2(
            rect.center().x - (galley.size().x + extra) * 0.5,
            rect.max.y - 7.0 - galley.size().y,
        );
        let text_width = galley.size().x;
        let text_mid = text_pos.y + galley.size().y * 0.5;
        ui.painter().galley(text_pos, galley, text_colour);
        if menu {
            let chevron = Rect::from_center_size(
                egui::pos2(text_pos.x + text_width + 6.0, text_mid + 0.5),
                Vec2::splat(11.0),
            );
            let (b, _) = look.icon_colours(enabled);
            look.icons
                .paint(ui.painter(), Icon::ChevronDown, chevron, b, b);
        }
    }
    response
}

/// A square icon-only button (`side` points), with an icon of `icon_size` points.
pub fn icon_button(
    ui: &mut Ui,
    look: Look,
    icon: Icon,
    side: f32,
    icon_size: f32,
    enabled: bool,
    active: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), sense(enabled));
    if ui.is_rect_visible(rect) {
        flat_background(ui, look, &response, active, R_SMALL + 1);
        look.icon(
            ui,
            icon,
            Rect::from_center_size(rect.center(), Vec2::splat(icon_size)),
            enabled,
        );
    }
    response
}

/// A ribbon group: its tools in a row, the caption under them. With `menu` the caption
/// has a chevron and returns a clickable response that opens the group's full list.
pub fn ribbon_group<R>(
    ui: &mut Ui,
    look: Look,
    caption: &str,
    menu: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> (R, Response) {
    let inner = ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(S1, 0.0);
        let r = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(S1, S1);
                add(ui)
            })
            .inner;
        let width = ui.min_rect().width().max(40.0);
        let (rect, response) = ui.allocate_exact_size(
            vec2(width, GROUP_LABEL_HEIGHT),
            if menu { Sense::click() } else { Sense::hover() },
        );
        if menu && (response.hovered() || response.is_pointer_button_down_on()) {
            ui.painter().rect_filled(
                rect.shrink2(vec2(0.0, 1.0)),
                CornerRadius::same(R_SMALL),
                look.c(|t| t.hover),
            );
        }
        focus_ring(ui, look, &response, R_SMALL);
        let muted = look.c(|t| t.text_muted);
        let galley =
            ui.painter()
                .layout_no_wrap(caption.to_owned(), FontId::proportional(T_CAPTION), muted);
        let extra = if menu { 12.0 } else { 0.0 };
        let pos = egui::pos2(
            rect.center().x - (galley.size().x + extra) * 0.5,
            rect.center().y - galley.size().y * 0.5,
        );
        let end = pos.x + galley.size().x;
        ui.painter().galley(pos, galley, muted);
        if menu {
            let chevron =
                Rect::from_center_size(egui::pos2(end + 7.0, rect.center().y), Vec2::splat(10.0));
            look.icons
                .paint(ui.painter(), Icon::ChevronDown, chevron, muted, muted);
        }
        (r, response)
    });
    inner.inner
}

/// A thin vertical line between ribbon groups.
pub fn ribbon_divider(ui: &mut Ui, look: Look) {
    let (rect, _) = ui.allocate_exact_size(
        vec2(S3 + 1.0, TOOL_HEIGHT + GROUP_LABEL_HEIGHT),
        Sense::hover(),
    );
    let x = rect.center().x.round() + 0.5;
    ui.painter().line_segment(
        [
            egui::pos2(x, rect.min.y + 6.0),
            egui::pos2(x, rect.max.y - 4.0),
        ],
        Stroke::new(1.0, look.c(|t| t.border)),
    );
}

/// A short vertical line between controls in a bar.
pub fn bar_divider(ui: &mut Ui, look: Look, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(S3 + 1.0, height), Sense::hover());
    let x = rect.center().x.round() + 0.5;
    ui.painter().line_segment(
        [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
        Stroke::new(1.0, look.c(|t| t.border)),
    );
}

/// A one-line caption in the muted colour, at the caption size.
pub fn caption(ui: &mut Ui, look: Look, text: &str) -> Response {
    ui.label(
        egui::RichText::new(text)
            .size(T_CAPTION)
            .color(look.c(|t| t.text_muted)),
    )
}

/// The primary action of a dialog: filled with the accent.
pub fn primary_button(ui: &mut Ui, look: Look, text: &str, enabled: bool) -> Response {
    let text = egui::RichText::new(text).color(look.c(|t| t.on_accent));
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(text)
            .fill(look.c(|t| t.accent))
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(R_SMALL + 1))
            .min_size(vec2(76.0, 28.0)),
    );
    focus_ring(ui, look, &response, R_SMALL + 1);
    response
}

/// A secondary action of a dialog: outlined.
pub fn secondary_button(ui: &mut Ui, look: Look, text: &str) -> Response {
    let response = ui.add(
        egui::Button::new(text)
            .fill(look.c(|t| t.panel))
            .stroke(Stroke::new(1.0, look.c(|t| t.border)))
            .corner_radius(CornerRadius::same(R_SMALL + 1))
            .min_size(vec2(76.0, 28.0)),
    );
    focus_ring(ui, look, &response, R_SMALL + 1);
    response
}

/// The frame of dialogs: the panel colour, a hairline, rounded, a soft shadow, no margin
/// (the title row and body add their own).
pub fn dialog_frame(look: Look) -> Frame {
    Frame::new()
        .fill(look.c(|t| t.panel))
        .stroke(Stroke::new(1.0, look.c(|t| t.border)))
        .corner_radius(CornerRadius::same(R_LARGE))
        .shadow(shadow(look.theme, 20))
}

/// A dialog's title row: icon, title and a close button. Returns true when closed.
pub fn dialog_title(ui: &mut Ui, look: Look, icon: Icon, title: &str) -> bool {
    let mut close = false;
    Frame::new()
        .inner_margin(Margin {
            left: S4 as i8,
            right: S3 as i8,
            top: S3 as i8,
            bottom: S3 as i8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = S3;
                let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                look.icon(ui, icon, r, true);
                ui.label(
                    egui::RichText::new(title)
                        .size(T_TITLE)
                        .color(look.c(|t| t.text)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, look, Icon::Close, 24.0, 16.0, true, false)
                        .on_hover_text("Close")
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        });
    let r = ui.min_rect();
    ui.painter().hline(
        r.x_range(),
        r.max.y.round() + 0.5,
        Stroke::new(1.0, look.c(|t| t.border)),
    );
    close
}

/// The padded body of a dialog.
pub fn dialog_body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .inner_margin(Margin {
            left: S4 as i8,
            right: S4 as i8,
            top: S4 as i8,
            bottom: S4 as i8,
        })
        .show(ui, add)
        .inner
}

/// A dialog's button row, right-aligned: secondary buttons first, the primary last.
pub fn dialog_footer<R>(ui: &mut Ui, look: Look, add: impl FnOnce(&mut Ui) -> R) -> R {
    let top = ui.min_rect().max.y;
    let r = Frame::new()
        .fill(look.c(|t| t.chrome))
        .corner_radius(CornerRadius {
            nw: 0,
            ne: 0,
            sw: R_LARGE,
            se: R_LARGE,
        })
        .inner_margin(Margin::symmetric(S4 as i8, S3 as i8 + 2))
        .show(ui, |ui| {
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                vec2(width, 28.0),
                egui::Layout::right_to_left(egui::Align::Center),
                add,
            )
            .inner
        });
    ui.painter().hline(
        r.response.rect.x_range(),
        top.round() + 0.5,
        Stroke::new(1.0, look.c(|t| t.border)),
    );
    r.inner
}

/// A label in a dialog's field column: muted, a fixed width so fields line up.
pub fn field_label(ui: &mut Ui, look: Look, text: &str) {
    fixed_text(
        ui,
        84.0,
        egui::RichText::new(text).color(look.c(|t| t.text_muted)),
        egui::Align::Min,
    );
}

/// Text in a fixed-width cell, aligned left (`Min`), centred or right (`Max`).
pub fn fixed_text(ui: &mut Ui, width: f32, text: egui::RichText, align: egui::Align) {
    let layout = match align {
        egui::Align::Min => egui::Layout::left_to_right(egui::Align::Center),
        egui::Align::Center => egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        egui::Align::Max => egui::Layout::right_to_left(egui::Align::Center),
    };
    ui.allocate_ui_with_layout(vec2(width, 26.0), layout, |ui| {
        ui.set_min_size(vec2(width, 26.0));
        ui.add(egui::Label::new(text).truncate());
    });
}

/// A small filled dot, for status (sketch constraint state, unsaved changes).
pub fn dot(ui: &mut Ui, colour: Color32, diameter: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter + 4.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), diameter * 0.5, colour);
    response
}

/// A menu row with an icon before its text.
pub fn menu_item(ui: &mut Ui, look: Look, icon: Icon, text: &str, enabled: bool) -> Response {
    let height = 26.0;
    let width = ui.available_width().max(180.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), sense(enabled));
    if ui.is_rect_visible(rect) {
        if enabled && (response.hovered() || response.has_focus()) {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(R_SMALL), look.c(|t| t.hover));
        }
        let icon_rect = Rect::from_center_size(
            egui::pos2(rect.min.x + 6.0 + 9.0, rect.center().y),
            Vec2::splat(18.0),
        );
        look.icon(ui, icon, icon_rect, enabled);
        let colour = if enabled {
            look.c(|t| t.text)
        } else {
            look.c(|t| t.text_muted)
        };
        ui.painter().text(
            egui::pos2(rect.min.x + 32.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(T_BODY),
            colour,
        );
    }
    response
}
