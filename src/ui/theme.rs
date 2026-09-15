//! The look: one serif at a real type scale on a paper ground, cyan for
//! interaction and magenta for "needs you", hierarchy from size and
//! whitespace instead of frames and rules. Colors are named tokens in a
//! [`Palette`] (one per theme); the egui style is derived from them once,
//! and every view draws with the helpers here so the same thing looks the
//! same everywhere.
#![allow(clippy::unreadable_literal)]

use std::sync::Arc;

use egui::{
    Button, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Response,
    RichText, Stroke, TextStyle, Ui, Vec2, Visuals,
};

use crate::core::CardState;

/// The color tokens of one theme.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Window ground.
    pub bg: Color32,
    /// Rail, cards, response blocks, inputs.
    pub surface: Color32,
    pub text: Color32,
    /// Cyan: primary button fill, the *working* dot.
    pub accent: Color32,
    /// Cyan that reads as text at small sizes (links, ghost buttons).
    pub accent_text: Color32,
    /// Cyan one step lighter, for the *starting* dot.
    pub accent_soft: Color32,
    /// User-turn block fill and its text.
    pub accent_fill: Color32,
    pub accent_on_fill: Color32,
    /// Magenta: the *waiting on you* dot.
    pub accent_2: Color32,
    /// Magenta that reads as text.
    pub accent_2_text: Color32,
    /// Neutral ramp, lightest to darkest in light mode; flipped in dark.
    pub n300: Color32,
    pub n400: Color32,
    pub n500: Color32,
    pub n600: Color32,
    pub n700: Color32,
    pub n800: Color32,
    /// Code blocks stay dark on both themes.
    pub code_fill: Color32,
    pub code_text: Color32,
}

// Hex colors read as designers write them (`0xf3f2f2`); separators
// would only obscure the triplet.
const fn rgb(hex: u32) -> Color32 {
    let [_, r, g, b] = hex.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

pub const LIGHT: Palette = Palette {
    dark: false,
    bg: rgb(0xf3f2f2),
    surface: rgb(0xeae9e9),
    text: rgb(0x201e1d),
    accent: rgb(0x0088b0),
    accent_text: rgb(0x006786),
    accent_soft: rgb(0x62c5ee),
    accent_fill: rgb(0xe9f8ff),
    accent_on_fill: rgb(0x0a303e),
    accent_2: rgb(0xd6006c),
    accent_2_text: rgb(0xaa0b56),
    n300: rgb(0xd7d3d3),
    n400: rgb(0xbab6b6),
    n500: rgb(0x9b9797),
    n600: rgb(0x7d7979),
    n700: rgb(0x605d5d),
    n800: rgb(0x444141),
    code_fill: rgb(0x2d2b2b),
    code_text: rgb(0xf8f4f4),
};

pub const DARK: Palette = Palette {
    dark: true,
    bg: rgb(0x201e1d),
    surface: rgb(0x2d2b2b),
    text: rgb(0xf3f2f2),
    accent: rgb(0x0088b0),
    accent_text: rgb(0x62c5ee),
    accent_soft: rgb(0x62c5ee),
    accent_fill: rgb(0x0a303e),
    accent_on_fill: rgb(0xe9f8ff),
    accent_2: rgb(0xd6006c),
    accent_2_text: rgb(0xff90b1),
    n300: rgb(0x444141),
    n400: rgb(0x605d5d),
    n500: rgb(0x7d7979),
    n600: rgb(0x9b9797),
    n700: rgb(0xbab6b6),
    n800: rgb(0xd7d3d3),
    code_fill: rgb(0x2d2b2b),
    code_text: rgb(0xf8f4f4),
};

/// The palette of the theme `ui` is drawing in.
#[must_use]
pub fn palette(ui: &Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

impl Palette {
    /// The text color at `alpha` (0..=1): hairlines, hover tints.
    #[must_use]
    pub fn text_alpha(&self, alpha: f32) -> Color32 {
        self.text.gamma_multiply(alpha)
    }

    /// The 1 px divider: text at 16 %.
    #[must_use]
    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0, self.text_alpha(0.16))
    }

    /// The color a status dot takes for `state`; `None` means hollow.
    #[must_use]
    pub fn dot_fill(&self, state: &CardState) -> Option<Color32> {
        match state {
            CardState::WaitingOnYou => Some(self.accent_2),
            CardState::Working => Some(self.accent),
            CardState::Starting => Some(self.accent_soft),
            CardState::Idle => Some(self.n400),
            CardState::Exited(Some(code)) if *code != 0 => Some(self.accent_2),
            CardState::Exited(_) | CardState::NotRunning | CardState::NotResumable => None,
        }
    }

    /// The color text about `state` takes: the kicker on a card, the
    /// state line in a header.
    #[must_use]
    pub fn state_text(&self, state: &CardState) -> Color32 {
        match state {
            CardState::WaitingOnYou => self.accent_2_text,
            CardState::Exited(Some(code)) if *code != 0 => self.accent_2_text,
            CardState::Working | CardState::Starting => self.accent_text,
            CardState::Idle
            | CardState::Exited(_)
            | CardState::NotRunning
            | CardState::NotResumable => self.n600,
        }
    }
}

// Font families. `Proportional` is the serif; the two named families are
// its semibold and true-italic faces, picked per text style below.
const SERIF: &str = "serif";
const SERIF_BOLD: &str = "serif-bold";
const SERIF_ITALIC: &str = "serif-italic";

#[must_use]
pub fn bold() -> FontFamily {
    FontFamily::Name(SERIF_BOLD.into())
}

#[must_use]
pub fn italic() -> FontFamily {
    FontFamily::Name(SERIF_ITALIC.into())
}

/// Source Serif 4 in front of egui's own fonts, which stay as fallbacks
/// for the glyphs the serif lacks (dots, arrows, emoji).
#[must_use]
pub fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            SERIF,
            &include_bytes!("../../assets/fonts/SourceSerif4-Regular.ttf")[..],
        ),
        (
            SERIF_BOLD,
            &include_bytes!("../../assets/fonts/SourceSerif4-Semibold.ttf")[..],
        ),
        (
            SERIF_ITALIC,
            &include_bytes!("../../assets/fonts/SourceSerif4-It.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let fallback: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (family, face) in [
        (FontFamily::Proportional, SERIF),
        (bold(), SERIF_BOLD),
        (italic(), SERIF_ITALIC),
    ] {
        let mut faces = vec![face.to_owned()];
        faces.extend(fallback.iter().cloned());
        fonts.families.insert(family, faces);
    }
    fonts
}

// Text styles beyond egui's five. `TextStyle::Name` takes an `Arc<str>`,
// which cannot be built in a `const`, so these are functions.

/// Page title: 36 semibold.
#[must_use]
pub fn h1() -> TextStyle {
    TextStyle::Name("h1".into())
}
/// Card title: 17 semibold.
#[must_use]
pub fn card_title() -> TextStyle {
    TextStyle::Name("card-title".into())
}
/// Section label: 10.5, uppercase and letter-spaced at the call site.
#[must_use]
pub fn kicker_style() -> TextStyle {
    TextStyle::Name("kicker".into())
}
/// Meta lines, state lines, tree rows, actions: 13.
#[must_use]
pub fn meta() -> TextStyle {
    TextStyle::Name("meta".into())
}
/// Card excerpts: 13 true italic.
#[must_use]
pub fn excerpt() -> TextStyle {
    TextStyle::Name("excerpt".into())
}
/// Rail brand and dialog titles: 18 semibold.
#[must_use]
pub fn brand() -> TextStyle {
    TextStyle::Name("brand".into())
}
/// Semibold at body size: names in rows, the file in the preview header.
#[must_use]
pub fn strong() -> TextStyle {
    TextStyle::Name("strong".into())
}

fn text_styles() -> std::collections::BTreeMap<TextStyle, FontId> {
    use FontFamily::{Monospace, Proportional};
    [
        (TextStyle::Small, FontId::new(12.0, Proportional)),
        (TextStyle::Body, FontId::new(14.0, Proportional)),
        (TextStyle::Button, FontId::new(13.0, bold())),
        // `ui.heading` is the group title (h4).
        (TextStyle::Heading, FontId::new(19.0, bold())),
        (TextStyle::Monospace, FontId::new(12.0, Monospace)),
        (h1(), FontId::new(36.0, bold())),
        (card_title(), FontId::new(17.0, bold())),
        (kicker_style(), FontId::new(10.5, Proportional)),
        (meta(), FontId::new(13.0, Proportional)),
        (excerpt(), FontId::new(13.0, italic())),
        (brand(), FontId::new(18.0, bold())),
        (strong(), FontId::new(14.0, bold())),
    ]
    .into()
}

fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let radius = CornerRadius::same(2);
    v.panel_fill = p.bg;
    // Dialogs and popups float on paper with a hairline: the same
    // surface as the rail and the cards would blend into them.
    v.window_fill = p.bg;
    v.faint_bg_color = p.surface;
    v.extreme_bg_color = p.surface;
    v.text_edit_bg_color = Some(p.surface);
    v.code_bg_color = p.n300;
    v.hyperlink_color = p.accent_text;
    v.weak_text_color = Some(p.n600);
    v.error_fg_color = p.accent_2_text;
    v.warn_fg_color = p.accent_2_text;
    v.selection.bg_fill = p.accent.gamma_multiply(0.3);
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.window_corner_radius = CornerRadius::same(4);
    v.window_stroke = Stroke::new(1.0, p.n400);
    v.window_shadow = egui::Shadow {
        offset: [0, 12],
        blur: 40,
        spread: 0,
        color: rgb(0x2d2b2b).gamma_multiply(0.35),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 3],
        blur: 10,
        spread: 0,
        color: rgb(0x2d2b2b).gamma_multiply(0.16),
    };
    v.menu_corner_radius = radius;
    v.striped = false;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;
    v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.5 };

    let w = &mut v.widgets;
    // Text and the hairline every region may draw.
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.bg_stroke = p.hairline();
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.corner_radius = radius;
    // Buttons: no fill until hovered, then a 7 % text tint; pressed 14 %.
    w.inactive.bg_fill = Color32::TRANSPARENT;
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;
    w.hovered.bg_fill = p.text_alpha(0.07);
    w.hovered.weak_bg_fill = p.text_alpha(0.07);
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.text_alpha(0.14);
    w.active.weak_bg_fill = p.text_alpha(0.14);
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;
    w.open = w.hovered;
    v
}

/// Push the fonts and both themes' styles into `ctx`. Once per app.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    for (theme, p) in [(egui::Theme::Light, &LIGHT), (egui::Theme::Dark, &DARK)] {
        ctx.style_mut_of(theme, |style| {
            style.text_styles = text_styles();
            style.visuals = visuals(p);
            let s = &mut style.spacing;
            s.item_spacing = Vec2::new(10.0, 6.0);
            s.button_padding = Vec2::new(10.0, 5.0);
            s.indent = 13.0;
            s.interact_size = Vec2::new(40.0, 26.0);
            s.icon_width = 14.0;
            s.icon_width_inner = 8.0;
            s.menu_margin = egui::Margin::same(8);
            s.window_margin = egui::Margin::same(20);
            style.interaction.selectable_labels = false;
            style.url_in_tooltip = true;
        });
    }
}

/// A status dot: 8 px filled circle, or a hollow 1 px ring for a session
/// that is not running. `size` is its diameter.
pub fn status_dot(ui: &mut Ui, state: &CardState, size: f32) -> Response {
    let p = palette(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    match p.dot_fill(state) {
        Some(fill) => {
            ui.painter().circle_filled(rect.center(), size / 2.0, fill);
        }
        None => {
            ui.painter()
                .circle_stroke(rect.center(), size / 2.0 - 0.5, Stroke::new(1.0, p.n500));
        }
    }
    response
}

/// Uppercase, letter-spaced section label in `color`.
pub fn kicker(ui: &mut Ui, text: &str, color: Color32) -> Response {
    ui.label(
        RichText::new(text.to_uppercase())
            .text_style(kicker_style())
            .extra_letter_spacing(1.0)
            .color(color),
    )
}

/// The section label between a board's groups: neutral, with the 36 px
/// above and 12 px below the design asks for.
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(24.0);
    kicker(ui, text, palette(ui).n600);
    ui.add_space(6.0);
}

/// Filled cyan button: the one primary action of a view.
pub fn primary(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(
        Button::new(RichText::new(text).color(p.bg))
            .fill(p.accent)
            .corner_radius(2),
    )
}

/// Outlined button: the secondary action next to a primary one.
pub fn secondary(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(Button::new(text).stroke(p.hairline()).corner_radius(2))
}

/// Text-only button in cyan; a 7 % tint on hover.
pub fn ghost(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(Button::new(RichText::new(text).color(p.accent_text)).frame_when_inactive(false))
}

/// Text-only button in neutral: Kill, Remove, Stop, and other actions
/// that should not invite a click.
pub fn ghost_muted(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(Button::new(RichText::new(text).color(p.n700)).frame_when_inactive(false))
}

/// Text in the semibold face at body size.
#[must_use]
pub fn strong_text(text: impl Into<String>) -> RichText {
    RichText::new(text).text_style(strong())
}

/// Text in the 13 px meta size, neutral-600 unless recolored.
#[must_use]
pub fn meta_text(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text)
        .text_style(meta())
        .color(palette(ui).n600)
}

/// A monospace path or command in neutral-600.
#[must_use]
pub fn mono_text(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().color(palette(ui).n600)
}

/// The `Frame` of a surface-filled block: cards, response blocks,
/// the preview pane. No border; the fill is the edge.
pub fn surface(ui: &Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(palette(ui).surface)
        .corner_radius(2)
}

/// Draw a 1 px dashed rectangle outline, for the "+ New session" cell.
pub fn dashed_rect(ui: &Ui, rect: egui::Rect, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    for i in 0..4 {
        ui.painter().add(egui::Shape::dashed_line(
            &[corners[i], corners[(i + 1) % 4]],
            stroke,
            4.0,
            3.0,
        ));
    }
}
