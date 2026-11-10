//! The application shell in SolidCraft's visual language (spec D10, 9.1).

use crate::analysis_ui::Which;
use crate::app::{App, Cmd, Tool};
use crate::catalog;
use crate::icons::{self, Icon};
use crate::theme::{self, c};
use ecs_schematic::{Command, Item, Prim};
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Margin, Order, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, Ui, pos2, vec2,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A menu row with a right-aligned monospace shortcut.
pub fn menu_item(ui: &mut Ui, label: &str, shortcut: &str) -> bool {
    let width = ui.available_width().max(200.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, c::HOVER);
    }
    ui.painter().text(
        rect.left_center() + vec2(8.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        theme::regular(13.0),
        c::TEXT,
    );
    if !shortcut.is_empty() {
        ui.painter().text(
            rect.right_center() - vec2(8.0, 0.0),
            Align2::RIGHT_CENTER,
            shortcut,
            theme::mono(11.0),
            c::TEXT_DIM,
        );
    }
    resp.clicked()
}

/// A square icon button for title bars and toolbars.
fn icon_button(ui: &mut Ui, icon: Icon, size: f32, tip: &str, enabled: bool, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(
        vec2(size, size),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if active {
        ui.painter().rect_filled(rect, 5.0, c::ACTIVE_TAB_BG);
    } else if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, 5.0, c::HOVER);
    }
    let col = if enabled { c::TEXT } else { c::TEXT_FAINT };
    icons::paint(ui.painter(), rect.shrink(size * 0.2), icon, col, !enabled);
    let resp = if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    };
    enabled && resp.clicked()
}

/// Paint a schematic symbol scaled into `rect`.
pub fn paint_symbol(painter: &egui::Painter, rect: Rect, symbol: &str, color: Color32) {
    let sym = ecs_schematic::symbols::get(symbol);
    let [lo, hi] = sym.bbox;
    let w = (hi[0] - lo[0]) as f32;
    let h = (hi[1] - lo[1]) as f32;
    let s = (rect.width() / w).min(rect.height() / h);
    let ctr = rect.center();
    let mid = [(lo[0] + hi[0]) as f32 / 2.0, (lo[1] + hi[1]) as f32 / 2.0];
    let p = |q: [f64; 2]| {
        pos2(
            ctr.x + (q[0] as f32 - mid[0]) * s,
            ctr.y + (q[1] as f32 - mid[1]) * s,
        )
    };
    let stroke = Stroke::new(1.3, color);
    for prim in sym.prims.iter().chain(&sym.prims_off) {
        match prim {
            Prim::Line { a, b } => {
                painter.line_segment([p(*a), p(*b)], stroke);
            }
            Prim::Polyline {
                points,
                closed,
                fill,
            } => {
                let pts: Vec<Pos2> = points.iter().map(|q| p(*q)).collect();
                if *fill {
                    painter.add(egui::Shape::convex_polygon(
                        pts.clone(),
                        color,
                        Stroke::NONE,
                    ));
                }
                if *closed {
                    painter.add(egui::Shape::closed_line(pts, stroke));
                } else {
                    painter.add(egui::Shape::line(pts, stroke));
                }
            }
            Prim::Circle { c: centre, r, fill } => {
                if *fill {
                    painter.circle_filled(p(*centre), *r as f32 * s, color);
                } else {
                    painter.circle_stroke(p(*centre), *r as f32 * s, stroke);
                }
            }
            Prim::Arc {
                c: centre,
                r,
                a0,
                a1,
            } => {
                let pts: Vec<Pos2> = (0..=10)
                    .map(|k| {
                        let a = a0 + (a1 - a0) * k as f64 / 10.0;
                        p([centre[0] + r * a.cos(), centre[1] + r * a.sin()])
                    })
                    .collect();
                painter.add(egui::Shape::line(pts, stroke));
            }
            Prim::Text { at, text, size } => {
                painter.text(
                    p(*at),
                    Align2::CENTER_CENTER,
                    text,
                    theme::bold((*size as f32 * s).max(6.0)),
                    color,
                );
            }
        }
    }
}

impl App {
    /// Title bar: logo, home, File menu, quick actions, document tab, window controls.
    pub fn titlebar(&mut self, ui: &mut Ui) -> Option<Cmd> {
        let mut cmd = None;
        let ctx = ui.ctx().clone();
        egui::Panel::top("titlebar")
            .exact_size(34.0)
            .frame(
                egui::Frame::new()
                    .fill(c::TITLEBAR)
                    .inner_margin(Margin::symmetric(8, 0)),
            )
            .show(ui, |ui| {
                let bar = ui.max_rect();
                // Dragging empty title bar moves the window; double click maximizes.
                let drag = ui.interact(bar, ui.scope_id().with("drag"), Sense::click_and_drag());
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    icons::show(ui, Icon::Logo, 20.0, c::TEXT);
                    ui.label(
                        RichText::new("Electrocraft")
                            .font(theme::bold(14.0))
                            .color(c::TEXT),
                    );
                    ui.add_space(4.0);
                    if icon_button(ui, Icon::Home, 26.0, "Start (Esc)", true, self.home) {
                        cmd = Some(Cmd::Home);
                    }
                    self.file_menu(ui, &mut cmd);
                    if icon_button(ui, Icon::Save, 26.0, "Save (Ctrl+S)", self.has_doc, false) {
                        cmd = Some(Cmd::Save);
                    }
                    if icon_button(
                        ui,
                        Icon::Undo,
                        26.0,
                        "Undo (Ctrl+Z)",
                        self.history.can_undo(),
                        false,
                    ) {
                        cmd = Some(Cmd::Undo);
                    }
                    if icon_button(
                        ui,
                        Icon::Redo,
                        26.0,
                        "Redo (Ctrl+Y)",
                        self.history.can_redo(),
                        false,
                    ) {
                        cmd = Some(Cmd::Redo);
                    }
                    let sep = ui.allocate_exact_size(vec2(9.0, 20.0), Sense::hover()).0;
                    ui.painter().line_segment(
                        [sep.center_top(), sep.center_bottom()],
                        Stroke::new(1.0, c::PANEL_BORDER),
                    );
                    if self.has_doc {
                        // One tab per open document; long lists shrink the captions.
                        let n = self.tabs.len().max(1);
                        let room = (ui.available_width() - 220.0).max(120.0);
                        let max_label = (room / n as f32 - 44.0).clamp(40.0, 220.0);
                        for k in 0..self.tabs.len() {
                            let (name, modified) = self.tab_info(k);
                            let active = k == self.active && !self.home;
                            let title = format!("{name}{}", if modified { " •" } else { "" });
                            let font = if k == self.active {
                                theme::bold(13.0)
                            } else {
                                theme::regular(13.0)
                            };
                            let galley = ui.painter().layout(
                                title.clone(),
                                font,
                                if active { c::TEXT } else { c::TEXT_DIM },
                                f32::INFINITY,
                            );
                            let w = galley.size().x.min(max_label);
                            let (rect, resp) =
                                ui.allocate_exact_size(vec2(w + 40.0, 26.0), Sense::click());
                            let hovered = resp.hovered();
                            ui.painter().rect_filled(
                                rect,
                                5.0,
                                if active {
                                    c::TAB
                                } else if hovered {
                                    c::HOVER
                                } else {
                                    c::BAR
                                },
                            );
                            if active {
                                ui.painter().line_segment(
                                    [
                                        rect.left_bottom() + vec2(6.0, -1.0),
                                        rect.right_bottom() + vec2(-6.0, -1.0),
                                    ],
                                    Stroke::new(2.0, c::ACCENT),
                                );
                            }
                            let text_rect = Rect::from_min_size(
                                rect.min + vec2(10.0, 0.0),
                                vec2(w, rect.height()),
                            );
                            ui.painter().with_clip_rect(text_rect).galley(
                                rect.left_center() + vec2(10.0, -galley.size().y / 2.0),
                                galley,
                                c::TEXT,
                            );
                            let x = Rect::from_center_size(
                                rect.right_center() - vec2(14.0, 0.0),
                                vec2(14.0, 14.0),
                            );
                            let over_x = ui
                                .ctx()
                                .input(|i| i.pointer.hover_pos())
                                .is_some_and(|p| x.expand(4.0).contains(p));
                            if over_x {
                                ui.painter().rect_filled(x.expand(3.0), 3.0, c::HOVER);
                            }
                            icons::paint(
                                ui.painter(),
                                x,
                                Icon::Close,
                                if over_x { c::TEXT } else { c::TEXT_DIM },
                                true,
                            );
                            let resp = resp.on_hover_text(match self.tab_path(k) {
                                Some(p) => p.display().to_string(),
                                None => format!("{name} (not saved yet)"),
                            });
                            if resp.clicked() {
                                cmd = Some(if over_x {
                                    Cmd::CloseTab(Some(k))
                                } else {
                                    Cmd::SwitchTab(k)
                                });
                            }
                            if resp.middle_clicked() {
                                cmd = Some(Cmd::CloseTab(Some(k)));
                            }
                        }
                    }
                    self.plus_menu(ui, &mut cmd);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if icon_button(ui, Icon::Close, 30.0, "", true, false) {
                            cmd = Some(Cmd::Quit);
                        }
                        if icon_button(ui, Icon::Maximize, 30.0, "", true, false) {
                            let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                        }
                        if icon_button(ui, Icon::Minimize, 30.0, "", true, false) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        ui.add_space(10.0);
                        let galley = ui.painter().layout_no_wrap(
                            format!("v{VERSION}"),
                            theme::mono(11.0),
                            c::TEXT_DIM,
                        );
                        let (r, _) =
                            ui.allocate_exact_size(galley.size() + vec2(14.0, 8.0), Sense::hover());
                        ui.painter().rect_stroke(
                            r,
                            4.0,
                            Stroke::new(1.0, c::PANEL_BORDER),
                            StrokeKind::Inside,
                        );
                        ui.painter()
                            .galley(r.min + vec2(7.0, 4.0), galley, c::TEXT_DIM);
                        ui.add_space(6.0);
                        if icon_button(ui, Icon::Help, 26.0, "Keyboard shortcuts", true, false) {
                            cmd = Some(Cmd::Shortcuts);
                        }
                        if icon_button(ui, Icon::Settings, 26.0, "Settings", true, false) {
                            cmd = Some(Cmd::Settings);
                        }
                    });
                });
            });
        cmd
    }

    /// "+" after the tabs: a new schematic, a file or a sample, each in a new tab.
    fn plus_menu(&mut self, ui: &mut Ui, cmd: &mut Option<Cmd>) {
        let recent = self.recent.clone();
        let btn = egui::Button::new(RichText::new("+").font(theme::bold(17.0)).color(c::TEXT))
            .fill(Color32::TRANSPARENT)
            .min_size(vec2(26.0, 24.0));
        let (resp, _) = egui::containers::menu::MenuButton::from_button(btn).ui(ui, |ui| {
            ui.set_min_width(250.0);
            ui.label(
                RichText::new("Open in a new tab")
                    .color(c::TEXT_FAINT)
                    .small(),
            );
            if menu_item(ui, "New Schematic", "Ctrl+N") {
                *cmd = Some(Cmd::New);
                ui.close();
            }
            if menu_item(ui, "Open File...", "Ctrl+O") {
                *cmd = Some(Cmd::Open);
                ui.close();
            }
            if !recent.is_empty() {
                ui.separator();
                ui.label(RichText::new("Recent").color(c::TEXT_FAINT).small());
                for p in recent.iter().take(6) {
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if menu_item(ui, &name, "") {
                        *cmd = Some(Cmd::OpenPath(p.clone()));
                        ui.close();
                    }
                }
            }
            ui.separator();
            ui.label(RichText::new("Samples").color(c::TEXT_FAINT).small());
            for smp in crate::samples::all() {
                if menu_item(ui, smp.title, "") {
                    *cmd = Some(Cmd::OpenSample(smp.id));
                    ui.close();
                }
            }
        });
        resp.on_hover_text("New tab: a new schematic, a file or a sample");
    }

    fn file_menu(&mut self, ui: &mut Ui, cmd: &mut Option<Cmd>) {
        let has_doc = self.has_doc;
        let recent = self.recent.clone();
        let btn =
            egui::Button::new(RichText::new("File").color(c::TEXT)).fill(Color32::TRANSPARENT);
        egui::containers::menu::MenuButton::from_button(btn).ui(ui, |ui| {
            ui.set_min_width(250.0);
            let items: [(&str, &str, Cmd, bool); 6] = [
                ("New Schematic", "Ctrl+N", Cmd::New, true),
                ("Open...", "Ctrl+O", Cmd::Open, true),
                ("Save", "Ctrl+S", Cmd::Save, has_doc),
                ("Save As...", "Ctrl+Shift+S", Cmd::SaveAs, has_doc),
                ("Save All", "", Cmd::SaveAll, has_doc && self.tabs.len() > 1),
                ("Close Tab", "Ctrl+W", Cmd::CloseTab(None), has_doc),
            ];
            for (label, key, c2, en) in items {
                ui.add_enabled_ui(en, |ui| {
                    if menu_item(ui, label, key) {
                        *cmd = Some(c2);
                        ui.close();
                    }
                });
            }
            if !recent.is_empty() {
                ui.separator();
                ui.label(RichText::new("Open Recent").color(c::TEXT_FAINT).small());
                for p in recent.iter().take(5) {
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if menu_item(ui, &name, "") {
                        *cmd = Some(Cmd::OpenPath(p.clone()));
                        ui.close();
                    }
                }
            }
            ui.separator();
            for (label, c2) in [
                ("Import SPICE Models...", Cmd::ImportSpice),
                ("Export SPICE Netlist...", Cmd::ExportSpice),
                ("Export Bill of Materials...", Cmd::ExportBom),
                ("Parts Catalogs...", Cmd::PartsPanel),
                ("User Components Folder...", Cmd::OpenComponentsFolder),
            ] {
                if menu_item(ui, label, "") {
                    *cmd = Some(c2);
                    ui.close();
                }
            }
            ui.separator();
            for (label, key, c2) in [
                ("Command Palette", "Ctrl+K", Cmd::Palette),
                ("Keyboard Shortcuts", "", Cmd::Shortcuts),
                ("About Electrocraft", "", Cmd::About),
                ("Quit", "Ctrl+Q", Cmd::Quit),
            ] {
                if menu_item(ui, label, key) {
                    *cmd = Some(c2);
                    ui.close();
                }
            }
        });
    }

    /// The Start screen: blue sidebar and New / Recent / Samples cards.
    pub fn home_screen(&mut self, ui: &mut Ui) -> Option<Cmd> {
        let mut cmd = None;
        egui::Panel::left("home-side")
            .exact_size(248.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(c::HOME_SIDE)
                    .inner_margin(Margin::symmetric(10, 14)),
            )
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                if self.has_doc {
                    let (r, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    if resp.hovered() {
                        ui.painter()
                            .rect_filled(r, 6.0, c::HOME_SIDE_SEL.gamma_multiply(0.6));
                    }
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(r.left_center() + vec2(18.0, 0.0), vec2(14.0, 14.0)),
                        Icon::Chevron,
                        c::TEXT,
                        true,
                    );
                    ui.painter().text(
                        r.left_center() + vec2(36.0, 0.0),
                        Align2::LEFT_CENTER,
                        format!("Back to {}", self.title()),
                        theme::bold(13.0),
                        c::TEXT,
                    );
                    let chip = Rect::from_center_size(
                        r.right_center() - vec2(24.0, 0.0),
                        vec2(30.0, 18.0),
                    );
                    ui.painter().rect_stroke(
                        chip,
                        4.0,
                        Stroke::new(1.0, c::TEXT.gamma_multiply(0.7)),
                        StrokeKind::Inside,
                    );
                    ui.painter().text(
                        chip.center(),
                        Align2::CENTER_CENTER,
                        "Esc",
                        theme::mono(10.5),
                        c::TEXT,
                    );
                    if resp.clicked() {
                        self.home = false;
                    }
                    ui.add_space(8.0);
                }
                let side =
                    |ui: &mut Ui, label: &str, key: &str, selected: bool, enabled: bool| -> bool {
                        let (r, resp) = ui.allocate_exact_size(
                            vec2(ui.available_width(), 36.0),
                            if enabled {
                                Sense::click()
                            } else {
                                Sense::hover()
                            },
                        );
                        if selected {
                            ui.painter().rect_filled(r, 6.0, c::HOME_SIDE_SEL);
                        } else if resp.hovered() && enabled {
                            ui.painter()
                                .rect_filled(r, 6.0, c::HOME_SIDE_SEL.gamma_multiply(0.6));
                        }
                        let col = if enabled {
                            c::TEXT
                        } else {
                            c::TEXT.gamma_multiply(0.45)
                        };
                        let font = if selected {
                            theme::bold(13.5)
                        } else {
                            theme::regular(13.5)
                        };
                        ui.painter().text(
                            r.left_center() + vec2(12.0, 0.0),
                            Align2::LEFT_CENTER,
                            label,
                            font,
                            col,
                        );
                        if !key.is_empty() {
                            ui.painter().text(
                                r.right_center() - vec2(12.0, 0.0),
                                Align2::RIGHT_CENTER,
                                key,
                                theme::mono(11.0),
                                col,
                            );
                        }
                        enabled && resp.clicked()
                    };
                if side(ui, "Start", "", self.home_page == 0, true) {
                    self.home_page = 0;
                }
                if side(ui, "New", "Ctrl+N", false, true) {
                    cmd = Some(Cmd::New);
                }
                if side(ui, "Open", "Ctrl+O", false, true) {
                    cmd = Some(Cmd::Open);
                }
                let sep = |ui: &mut Ui| {
                    ui.add_space(6.0);
                    let r = ui
                        .allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover())
                        .0;
                    ui.painter().line_segment(
                        [
                            r.left_center() + vec2(12.0, 0.0),
                            r.right_center() - vec2(12.0, 0.0),
                        ],
                        Stroke::new(1.0, c::TEXT.gamma_multiply(0.25)),
                    );
                    ui.add_space(6.0);
                };
                sep(ui);
                if side(ui, "Save", "Ctrl+S", false, self.has_doc) {
                    cmd = Some(Cmd::Save);
                }
                if side(ui, "Save As", "Ctrl+Shift+S", false, self.has_doc) {
                    cmd = Some(Cmd::SaveAs);
                }
                side(ui, "Export netlist", "", false, false);
                sep(ui);
                if side(ui, "Keyboard shortcuts", "", false, true) {
                    cmd = Some(Cmd::Shortcuts);
                }
                if side(ui, "About Electrocraft", "", false, true) {
                    cmd = Some(Cmd::About);
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(c::HOME_MAIN).inner_margin(Margin {
                left: 40,
                right: 40,
                top: 30,
                bottom: 20,
            }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.label(
                        RichText::new("Start")
                            .font(theme::bold(28.0))
                            .color(c::TEXT),
                    );
                    ui.label(
                        RichText::new(
                            "Create a schematic, open a recent file, or explore a sample circuit.",
                        )
                        .color(c::TEXT_DIM)
                        .size(13.5),
                    );
                    ui.add_space(22.0);
                    ui.label(RichText::new("New").font(theme::bold(16.0)));
                    ui.add_space(6.0);
                    let w = ((ui.available_width() - 3.0 * 14.0) / 4.0).max(180.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        for (k, (icon, title, sub, c2, en)) in [
                            (
                                Icon::New,
                                "New schematic",
                                "Empty sheet · opens in Schematic",
                                Some(Cmd::New),
                                true,
                            ),
                            (
                                Icon::Open,
                                "Open...",
                                "An .ecs schematic (Ctrl+O)",
                                Some(Cmd::Open),
                                true,
                            ),
                            (
                                Icon::Import,
                                "Import SPICE...",
                                ".model and .subckt files (phase P7)",
                                None,
                                false,
                            ),
                            (
                                Icon::Scope,
                                "Sample circuit",
                                "Astable multivibrator, running",
                                Some(Cmd::OpenSample("astable")),
                                true,
                            ),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let (r, resp) = ui.allocate_exact_size(
                                vec2(w, 82.0),
                                if en { Sense::click() } else { Sense::hover() },
                            );
                            let sel = k == 0;
                            let fill = if sel {
                                c::CARD_SEL
                            } else if resp.hovered() && en {
                                c::HOVER
                            } else {
                                c::CARD
                            };
                            ui.painter().rect_filled(r, 7.0, fill);
                            ui.painter().rect_stroke(
                                r,
                                7.0,
                                Stroke::new(
                                    if sel { 1.5 } else { 1.0 },
                                    if sel { c::ACCENT } else { c::PANEL_BORDER },
                                ),
                                StrokeKind::Inside,
                            );
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(
                                    r.left_center() + vec2(36.0, 0.0),
                                    vec2(34.0, 34.0),
                                ),
                                icon,
                                if en { c::TEXT } else { c::TEXT_FAINT },
                                !en,
                            );
                            let tc = if sel {
                                c::ACTIVE_TAB_TEXT
                            } else if en {
                                c::TEXT
                            } else {
                                c::TEXT_FAINT
                            };
                            ui.painter().text(
                                r.left_center() + vec2(70.0, -9.0),
                                Align2::LEFT_CENTER,
                                title,
                                theme::bold(14.0),
                                tc,
                            );
                            ui.painter().text(
                                r.left_center() + vec2(70.0, 11.0),
                                Align2::LEFT_CENTER,
                                sub,
                                theme::regular(12.0),
                                if en { c::TEXT_DIM } else { c::TEXT_FAINT },
                            );
                            if resp.clicked()
                                && let Some(c2) = c2
                            {
                                cmd = Some(c2);
                            }
                        }
                    });
                    ui.add_space(26.0);
                    ui.label(RichText::new("Recent").font(theme::bold(16.0)));
                    ui.add_space(4.0);
                    if self.recent.is_empty() {
                        ui.label(
                            RichText::new("Schematics you open or save appear here.")
                                .color(c::TEXT_DIM),
                        );
                    } else {
                        for p in self.recent.clone() {
                            let name = p
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let dir = p
                                .parent()
                                .map(|d| d.display().to_string())
                                .unwrap_or_default();
                            let (r, resp) = ui.allocate_exact_size(
                                vec2(ui.available_width().min(700.0), 30.0),
                                Sense::click(),
                            );
                            if resp.hovered() {
                                ui.painter().rect_filled(r, 5.0, c::CARD);
                            }
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(
                                    r.left_center() + vec2(14.0, 0.0),
                                    vec2(16.0, 16.0),
                                ),
                                Icon::New,
                                c::TEXT_DIM,
                                false,
                            );
                            ui.painter().text(
                                r.left_center() + vec2(32.0, 0.0),
                                Align2::LEFT_CENTER,
                                &name,
                                theme::bold(13.0),
                                c::TEXT,
                            );
                            ui.painter().text(
                                r.left_center() + vec2(48.0 + 7.5 * name.len() as f32, 0.0),
                                Align2::LEFT_CENTER,
                                dir,
                                theme::regular(12.0),
                                c::TEXT_FAINT,
                            );
                            if resp.clicked() {
                                cmd = Some(Cmd::OpenPath(p));
                            }
                        }
                    }
                    ui.add_space(26.0);
                    ui.label(RichText::new("Samples").font(theme::bold(16.0)));
                    ui.add_space(6.0);
                    let samples = crate::samples::all();
                    let n = samples.len() as f32;
                    let cw = ((ui.available_width() - (n - 1.0) * 14.0) / n).clamp(170.0, 240.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(14.0, 14.0);
                        for s in samples {
                            let (r, resp) = ui.allocate_exact_size(vec2(cw, 190.0), Sense::click());
                            let fill = if resp.hovered() { c::HOVER } else { c::CARD };
                            ui.painter().rect_filled(r, 7.0, fill);
                            ui.painter().rect_stroke(
                                r,
                                7.0,
                                Stroke::new(1.0, c::PANEL_BORDER),
                                StrokeKind::Inside,
                            );
                            let thumb = Rect::from_min_size(
                                r.min + vec2(10.0, 10.0),
                                vec2(cw - 20.0, 96.0),
                            );
                            if let Some(list) = self.thumbnails.get(s.id) {
                                let ppp = list.width as f32 / thumb.width();
                                let ppp = ppp.max(list.height as f32 / thumb.height());
                                let painter = ui.painter_at(thumb);
                                painter.rect_filled(thumb, 4.0, c::CANVAS);
                                let off = thumb.center()
                                    - vec2(
                                        list.width as f32 / ppp / 2.0,
                                        list.height as f32 / ppp / 2.0,
                                    );
                                crate::scene::paint_egui(&painter, list, off, ppp, false);
                            }
                            ui.painter().text(
                                pos2(r.min.x + 10.0, thumb.max.y + 16.0),
                                Align2::LEFT_CENTER,
                                s.title,
                                theme::bold(13.5),
                                c::TEXT,
                            );
                            let galley = ui.painter().layout(
                                s.subtitle.to_string(),
                                theme::regular(12.0),
                                c::TEXT_DIM,
                                cw - 20.0,
                            );
                            ui.painter().galley(
                                pos2(r.min.x + 10.0, thumb.max.y + 28.0),
                                galley,
                                c::TEXT_DIM,
                            );
                            if resp.clicked() {
                                cmd = Some(Cmd::OpenSample(s.id));
                            }
                        }
                    });
                });
            });
        cmd
    }

    /// Ribbon: workspace card, tabs, grouped big buttons and command search.
    pub fn ribbon(&mut self, ui: &mut Ui) -> Option<Cmd> {
        let mut cmd = None;
        egui::Panel::top("ribbon")
            .exact_size(124.0)
            .frame(egui::Frame::new().fill(c::TITLEBAR).inner_margin(Margin {
                left: 6,
                right: 6,
                top: 6,
                bottom: 4,
            }))
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    // Workspace card.
                    let (r, _) = ui.allocate_exact_size(vec2(88.0, 114.0), Sense::hover());
                    ui.painter().rect_filled(r, 7.0, c::PANEL);
                    ui.painter().line_segment(
                        [
                            r.left_top() + vec2(8.0, 1.0),
                            r.right_top() + vec2(-8.0, 1.0),
                        ],
                        Stroke::new(2.0, c::ACCENT),
                    );
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(r.center_top() + vec2(0.0, 34.0), vec2(30.0, 30.0)),
                        Icon::Wire,
                        c::TEXT,
                        false,
                    );
                    ui.painter().text(
                        r.center_top() + vec2(0.0, 66.0),
                        Align2::CENTER_CENTER,
                        "Schematic",
                        theme::bold(13.0),
                        c::TEXT,
                    );
                    ui.painter().text(
                        r.center_top() + vec2(0.0, 84.0),
                        Align2::CENTER_CENTER,
                        "Workspace",
                        theme::regular(11.0),
                        c::TEXT_DIM,
                    );
                    // Main ribbon card.
                    let (card, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 114.0), Sense::hover());
                    ui.painter().rect_filled(card, 7.0, c::PANEL);
                    let mut inner = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(card.shrink2(vec2(6.0, 4.0)))
                            .layout(Layout::top_down(Align::Min)),
                    );
                    self.ribbon_contents(&mut inner, &mut cmd);
                });
            });
        cmd
    }

    fn ribbon_contents(&mut self, ui: &mut Ui, cmd: &mut Option<Cmd>) {
        let tabs = ["Schematic", "Digital", "Simulate", "Analyze", "View"];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (k, t) in tabs.iter().enumerate() {
                let galley =
                    ui.painter()
                        .layout_no_wrap(t.to_string(), theme::regular(13.5), c::TEXT);
                let (r, resp) =
                    ui.allocate_exact_size(vec2(galley.size().x + 22.0, 24.0), Sense::click());
                let active = self.ribbon_tab == k;
                if active {
                    ui.painter().rect_filled(r, 5.0, c::ACTIVE_TAB_BG);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 5.0, c::HOVER);
                }
                let col = if active { c::ACTIVE_TAB_TEXT } else { c::TEXT };
                let font = if active {
                    theme::bold(13.5)
                } else {
                    theme::regular(13.5)
                };
                ui.painter()
                    .text(r.center(), Align2::CENTER_CENTER, *t, font, col);
                if resp.clicked() {
                    self.ribbon_tab = k;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (r, resp) = ui.allocate_exact_size(vec2(206.0, 24.0), Sense::click());
                ui.painter().rect_filled(r, 5.0, c::TITLEBAR);
                ui.painter().rect_stroke(
                    r,
                    5.0,
                    Stroke::new(
                        1.0,
                        if resp.hovered() {
                            c::ACCENT
                        } else {
                            c::PANEL_BORDER
                        },
                    ),
                    StrokeKind::Inside,
                );
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(r.left_center() + vec2(13.0, 0.0), vec2(15.0, 15.0)),
                    Icon::Search,
                    c::TEXT,
                    true,
                );
                ui.painter().text(
                    r.left_center() + vec2(26.0, 0.0),
                    Align2::LEFT_CENTER,
                    "Search commands",
                    theme::regular(12.5),
                    c::TEXT,
                );
                let chip =
                    Rect::from_center_size(r.right_center() - vec2(22.0, 0.0), vec2(36.0, 16.0));
                ui.painter().rect_filled(chip, 3.0, c::TAB);
                ui.painter().text(
                    chip.center(),
                    Align2::CENTER_CENTER,
                    "Ctrl K",
                    theme::mono(10.0),
                    c::TEXT_DIM,
                );
                if resp.clicked() {
                    *cmd = Some(Cmd::Palette);
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let place = |def: &'static str| {
                Cmd::Tool(Tool::Place {
                    def: def.into(),
                    rot: 0,
                    mirror: false,
                })
            };
            let is_place = |t: &Tool, d: &str| matches!(t, Tool::Place { def, .. } if def == d);
            let tool = self.tool.clone();
            match self.ribbon_tab {
                0 => {
                    group(
                        ui,
                        "Tools",
                        cmd,
                        &[
                            (
                                Btn::I(Icon::Select),
                                "Select",
                                Cmd::Tool(Tool::Select),
                                tool == Tool::Select,
                                true,
                            ),
                            (
                                Btn::I(Icon::Wire),
                                "Wire",
                                Cmd::Tool(Tool::Wire),
                                tool == Tool::Wire,
                                true,
                            ),
                            (
                                Btn::I(Icon::Label),
                                "Label",
                                Cmd::Tool(Tool::Label),
                                tool == Tool::Label,
                                true,
                            ),
                            (
                                Btn::I(Icon::Text),
                                "Text",
                                Cmd::Tool(Tool::Text),
                                tool == Tool::Text,
                                true,
                            ),
                        ],
                    );
                    group(
                        ui,
                        "Place",
                        cmd,
                        &[
                            (
                                Btn::S("resistor"),
                                "Resistor",
                                place("core.resistor"),
                                is_place(&tool, "core.resistor"),
                                true,
                            ),
                            (
                                Btn::S("capacitor"),
                                "Capacitor",
                                place("core.capacitor"),
                                is_place(&tool, "core.capacitor"),
                                true,
                            ),
                            (
                                Btn::S("inductor"),
                                "Inductor",
                                place("core.inductor"),
                                is_place(&tool, "core.inductor"),
                                true,
                            ),
                            (
                                Btn::S("ground"),
                                "Ground",
                                place("core.ground"),
                                is_place(&tool, "core.ground"),
                                true,
                            ),
                            (
                                Btn::S("vdc"),
                                "DC Source",
                                place("core.vdc"),
                                is_place(&tool, "core.vdc"),
                                true,
                            ),
                            (
                                Btn::S("vac"),
                                "Sine",
                                place("core.vsine"),
                                is_place(&tool, "core.vsine"),
                                true,
                            ),
                            (
                                Btn::S("switch"),
                                "Switch",
                                place("core.switch"),
                                is_place(&tool, "core.switch"),
                                true,
                            ),
                            (
                                Btn::S("led"),
                                "LED",
                                place("core.led"),
                                is_place(&tool, "core.led"),
                                true,
                            ),
                            (
                                Btn::S("npn"),
                                "NPN",
                                place("core.npn"),
                                is_place(&tool, "core.npn"),
                                true,
                            ),
                        ],
                    );
                    let sel = !self.selection.is_empty();
                    group(
                        ui,
                        "Modify",
                        cmd,
                        &[
                            (
                                Btn::I(Icon::Rotate),
                                "Rotate",
                                Cmd::Rotate,
                                false,
                                sel || matches!(tool, Tool::Place { .. }),
                            ),
                            (
                                Btn::I(Icon::Mirror),
                                "Mirror",
                                Cmd::Mirror,
                                false,
                                sel || matches!(tool, Tool::Place { .. }),
                            ),
                            (
                                Btn::I(Icon::Duplicate),
                                "Duplicate",
                                Cmd::Duplicate,
                                false,
                                sel,
                            ),
                            (Btn::I(Icon::Delete), "Delete", Cmd::Delete, false, sel),
                        ],
                    );
                    let single = match self.selection.iter().copied().collect::<Vec<_>>()[..] {
                        [Item::Part(id)] => {
                            self.doc.part(id).map(|p| (p.refdes.clone(), p.def.clone()))
                        }
                        _ => None,
                    };
                    let refdes = single.as_ref().map(|s| s.0.clone()).unwrap_or_default();
                    let inner = single.as_ref().is_some_and(|(_, def)| {
                        self.root_doc().components.iter().any(|c| {
                            &c.id == def
                                && matches!(
                                    c.implementation,
                                    ecs_schematic::Implementation::Schematic { .. }
                                        | ecs_schematic::Implementation::ExternalSchematic { .. }
                                )
                        })
                    });
                    group(
                        ui,
                        "Component",
                        cmd,
                        &[
                            (Btn::I(Icon::Plus), "New", Cmd::NewComponent, false, true),
                            (
                                Btn::I(Icon::Settings),
                                "Edit",
                                Cmd::EditComponent(refdes.clone()),
                                false,
                                single.is_some(),
                            ),
                            (
                                Btn::I(Icon::Chevron),
                                "Enter",
                                Cmd::EnterComponent(refdes),
                                false,
                                inner,
                            ),
                            (
                                Btn::I(Icon::Undo),
                                "Back",
                                Cmd::ExitComponent,
                                false,
                                !self.stack.is_empty(),
                            ),
                        ],
                    );
                }
                1 => {
                    group(
                        ui,
                        "Gates",
                        cmd,
                        &[
                            (
                                Btn::S("and"),
                                "AND",
                                place("core.and"),
                                is_place(&tool, "core.and"),
                                true,
                            ),
                            (
                                Btn::S("or"),
                                "OR",
                                place("core.or"),
                                is_place(&tool, "core.or"),
                                true,
                            ),
                            (
                                Btn::S("not"),
                                "NOT",
                                place("core.not"),
                                is_place(&tool, "core.not"),
                                true,
                            ),
                            (
                                Btn::S("nand"),
                                "NAND",
                                place("core.nand"),
                                is_place(&tool, "core.nand"),
                                true,
                            ),
                            (
                                Btn::S("nor"),
                                "NOR",
                                place("core.nor"),
                                is_place(&tool, "core.nor"),
                                true,
                            ),
                            (
                                Btn::S("xor"),
                                "XOR",
                                place("core.xor"),
                                is_place(&tool, "core.xor"),
                                true,
                            ),
                        ],
                    );
                    group(
                        ui,
                        "Sequential",
                        cmd,
                        &[
                            (
                                Btn::S("dff"),
                                "D FF",
                                place("core.dff"),
                                is_place(&tool, "core.dff"),
                                true,
                            ),
                            (
                                Btn::S("jkff"),
                                "JK FF",
                                place("core.jkff"),
                                is_place(&tool, "core.jkff"),
                                true,
                            ),
                            (
                                Btn::S("counter4"),
                                "Counter",
                                place("core.counter4"),
                                is_place(&tool, "core.counter4"),
                                true,
                            ),
                            (
                                Btn::S("shift4"),
                                "Shift Reg",
                                place("core.shift4"),
                                is_place(&tool, "core.shift4"),
                                true,
                            ),
                        ],
                    );
                    group(
                        ui,
                        "I/O",
                        cmd,
                        &[
                            (
                                Btn::S("logic_toggle"),
                                "Toggle",
                                place("core.logic_toggle"),
                                is_place(&tool, "core.logic_toggle"),
                                true,
                            ),
                            (
                                Btn::S("clock"),
                                "Clock",
                                place("core.clock"),
                                is_place(&tool, "core.clock"),
                                true,
                            ),
                            (
                                Btn::S("logic_probe"),
                                "Probe",
                                place("core.logic_probe"),
                                is_place(&tool, "core.logic_probe"),
                                true,
                            ),
                            (
                                Btn::S("bcd7seg"),
                                "BCD→7",
                                place("core.bcd7seg"),
                                is_place(&tool, "core.bcd7seg"),
                                true,
                            ),
                            (
                                Btn::S("sevenseg"),
                                "7-Segment",
                                place("core.sevenseg"),
                                is_place(&tool, "core.sevenseg"),
                                true,
                            ),
                            (
                                Btn::S("ne555"),
                                "555",
                                place("core.ne555"),
                                is_place(&tool, "core.ne555"),
                                true,
                            ),
                        ],
                    );
                }
                2 => {
                    let running = self.sim_running;
                    group(
                        ui,
                        "Transport",
                        cmd,
                        &[
                            (
                                Btn::I(if running { Icon::Pause } else { Icon::Play }),
                                if running { "Pause" } else { "Run" },
                                Cmd::ToggleRun,
                                false,
                                true,
                            ),
                            (Btn::I(Icon::Step), "Step", Cmd::Step, false, true),
                            (Btn::I(Icon::Reset), "Reset", Cmd::Reset, false, true),
                        ],
                    );
                    group(
                        ui,
                        "Instruments",
                        cmd,
                        &[
                            (
                                Btn::I(Icon::Probe),
                                "Probe",
                                Cmd::Tool(Tool::Probe),
                                tool == Tool::Probe,
                                true,
                            ),
                            (
                                Btn::S("voltmeter"),
                                "Voltmeter",
                                place("core.voltmeter"),
                                is_place(&tool, "core.voltmeter"),
                                true,
                            ),
                            (
                                Btn::S("ammeter"),
                                "Ammeter",
                                place("core.ammeter"),
                                is_place(&tool, "core.ammeter"),
                                true,
                            ),
                            (
                                Btn::I(Icon::Delete),
                                "Clear Scope",
                                Cmd::ClearProbes,
                                false,
                                !self.doc.scope.is_empty(),
                            ),
                        ],
                    );
                    group(
                        ui,
                        "Display",
                        cmd,
                        &[
                            (
                                Btn::I(Icon::Voltage),
                                "Voltage",
                                Cmd::ToggleVolts,
                                self.volt_colors,
                                true,
                            ),
                            (
                                Btn::I(Icon::Current),
                                "Current",
                                Cmd::ToggleDots,
                                self.current_dots,
                                true,
                            ),
                            (
                                Btn::I(Icon::Scope),
                                "Scopes",
                                Cmd::TogglePanel("scopes"),
                                self.show_scopes,
                                true,
                            ),
                        ],
                    );
                }
                3 => {
                    group(
                        ui,
                        "Operating Point",
                        cmd,
                        &[(
                            Btn::I(Icon::DcOp),
                            "DC Op Point",
                            Cmd::DcOp,
                            self.annotate.is_some(),
                            true,
                        )],
                    );
                    group(
                        ui,
                        "Analyses",
                        cmd,
                        &[
                            (
                                Btn::I(Icon::Info),
                                "Op Table",
                                Cmd::Analysis(Which::Op),
                                false,
                                true,
                            ),
                            (
                                Btn::I(Icon::Sweep),
                                "DC Sweep",
                                Cmd::Analysis(Which::Dc),
                                false,
                                true,
                            ),
                            (
                                Btn::I(Icon::Transient),
                                "Transient",
                                Cmd::Analysis(Which::Tran),
                                false,
                                true,
                            ),
                            (
                                Btn::I(Icon::Ac),
                                "AC Sweep",
                                Cmd::Analysis(Which::Ac),
                                false,
                                true,
                            ),
                        ],
                    );
                    group(
                        ui,
                        "SPICE",
                        cmd,
                        &[
                            (
                                Btn::I(Icon::Import),
                                "Import",
                                Cmd::ImportSpice,
                                false,
                                true,
                            ),
                            (Btn::I(Icon::Save), "Export", Cmd::ExportSpice, false, true),
                        ],
                    );
                    group(
                        ui,
                        "Models",
                        cmd,
                        &[
                            (
                                Btn::I(Icon::Sweep),
                                "Datasheet Fit",
                                Cmd::DatasheetTool,
                                false,
                                true,
                            ),
                            (
                                Btn::I(Icon::Open),
                                "JS Components",
                                Cmd::OpenComponentsFolder,
                                false,
                                true,
                            ),
                        ],
                    );
                }
                _ => {
                    group(
                        ui,
                        "Navigate",
                        cmd,
                        &[
                            (Btn::I(Icon::Fit), "Fit", Cmd::Fit, false, true),
                            (Btn::I(Icon::ZoomIn), "Zoom In", Cmd::ZoomIn, false, true),
                            (Btn::I(Icon::ZoomOut), "Zoom Out", Cmd::ZoomOut, false, true),
                            (
                                Btn::I(Icon::Pan),
                                "Pan",
                                Cmd::Tool(Tool::Pan),
                                tool == Tool::Pan,
                                true,
                            ),
                        ],
                    );
                    group(
                        ui,
                        "Show",
                        cmd,
                        &[
                            (Btn::I(Icon::Grid), "Grid", Cmd::ToggleGrid, self.grid, true),
                            (
                                Btn::I(Icon::Voltage),
                                "Voltage",
                                Cmd::ToggleVolts,
                                self.volt_colors,
                                true,
                            ),
                            (
                                Btn::I(Icon::Current),
                                "Current",
                                Cmd::ToggleDots,
                                self.current_dots,
                                true,
                            ),
                        ],
                    );
                    group(
                        ui,
                        "Panels",
                        cmd,
                        &[
                            (
                                Btn::S("resistor"),
                                "Components",
                                Cmd::TogglePanel("components"),
                                self.show_components,
                                true,
                            ),
                            (
                                Btn::I(Icon::Info),
                                "Inspector",
                                Cmd::TogglePanel("inspector"),
                                self.show_inspector,
                                true,
                            ),
                            (
                                Btn::I(Icon::Scope),
                                "Scopes",
                                Cmd::TogglePanel("scopes"),
                                self.show_scopes,
                                true,
                            ),
                        ],
                    );
                }
            }
        });
    }

    /// Floating component palette (SolidCraft's Browser position).
    pub fn components_panel(&mut self, ctx: &egui::Context) -> Option<Cmd> {
        let mut cmd = None;
        let r = self.canvas_rect;
        let height = (r.height() - 90.0).clamp(160.0, 620.0);
        egui::Area::new(egui::Id::unique("components"))
            .order(Order::Middle)
            .fixed_pos(r.min + vec2(8.0, 8.0))
            .show(ctx, |ui| {
                theme::card().inner_margin(Margin::same(6)).show(ui, |ui| {
                    ui.set_width(228.0);
                    ui.set_max_height(height);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Components").font(theme::bold(13.5)));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(format!("{} parts", self.doc.parts.len())).color(c::TEXT_DIM).small());
                        });
                    });
                    ui.add(egui::TextEdit::singleline(&mut self.component_query).hint_text("Search components").desired_width(f32::INFINITY));
                    ui.add_space(2.0);
                    let new_btn = egui::Button::new(RichText::new("＋  New component…").color(c::ACTIVE_TAB_TEXT))
                        .fill(c::ACTIVE_TAB_BG.gamma_multiply(0.5))
                        .min_size(vec2(ui.available_width(), 26.0));
                    if ui
                        .add(new_btn)
                        .on_hover_text("Draw a symbol and describe it with a schematic or JavaScript; it is saved in this schematic")
                        .clicked()
                    {
                        cmd = Some(Cmd::NewComponent);
                    }
                    ui.add_space(2.0);
                    let q = self.component_query.to_lowercase();
                    // Components defined in this schematic first.
                    let local: Vec<(String, String, String)> = self
                        .root_doc()
                        .components
                        .iter()
                        .filter(|c| q.is_empty() || c.name.to_lowercase().contains(&q))
                        .map(|c| (c.id.clone(), c.name.clone(), c.description.clone()))
                        .collect();
                    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                        if !local.is_empty() {
                            egui::CollapsingHeader::new(RichText::new("This schematic").color(c::TEXT_DIM).small())
                                .id_salt("local-components")
                                .default_open(true)
                                .show(ui, |ui| {
                                    for (id, name, desc) in &local {
                                        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                                        let active = matches!(&self.tool, Tool::Place { def, .. } if def == id);
                                        if active {
                                            ui.painter().rect_filled(row, 5.0, c::ACTIVE_TAB_BG);
                                        } else if resp.hovered() {
                                            ui.painter().rect_filled(row, 5.0, c::HOVER);
                                        }
                                        let thumb = Rect::from_min_size(row.min + vec2(4.0, 3.0), vec2(34.0, 22.0));
                                        paint_symbol(ui.painter(), thumb, &crate::components::symbol_id(id), if active { c::ACTIVE_TAB_TEXT } else { c::ICON });
                                        ui.painter().text(row.min + vec2(46.0, 14.0), Align2::LEFT_CENTER, name, theme::regular(13.0), c::TEXT);
                                        let resp = resp.on_hover_text(format!("{desc}\nClick to place; Ctrl+E on a placed one edits it").trim().to_string());
                                        if resp.clicked() {
                                            cmd = Some(Cmd::Tool(Tool::Place { def: id.clone(), rot: 0, mirror: false }));
                                        }
                                    }
                                });
                        }
                        for (k, cat) in self.cat.categories().iter().enumerate() {
                            let defs: Vec<_> = self
                                .cat
                                .category(cat)
                                .into_iter()
                                .filter(|d| !d.id.starts_with("local."))
                                .filter(|d| q.is_empty() || d.meta.name.to_lowercase().contains(&q) || d.id.contains(&q))
                                .collect();
                            if defs.is_empty() {
                                continue;
                            }
                            egui::CollapsingHeader::new(RichText::new(cat.as_str()).color(c::TEXT_DIM).size(12.0))
                                .id_salt(cat)
                                .default_open(k < 4)
                                .open(if q.is_empty() { None } else { Some(true) })
                                .show(ui, |ui| {
                                    let names: Vec<String> = defs.iter().map(|d| d.meta.name.clone()).collect();
                                    for d in defs {
                                        let twin = names.iter().filter(|n| **n == d.meta.name).count() > 1;
                                        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                                        let active = matches!(&self.tool, Tool::Place { def, .. } if *def == d.id);
                                        if active {
                                            ui.painter().rect_filled(row, 5.0, c::ACTIVE_TAB_BG);
                                        } else if resp.hovered() {
                                            ui.painter().rect_filled(row, 5.0, c::HOVER);
                                        }
                                        let thumb = Rect::from_min_size(row.min + vec2(4.0, 3.0), vec2(34.0, 22.0));
                                        paint_symbol(ui.painter(), thumb, &d.symbol, if active { c::ACTIVE_TAB_TEXT } else { c::ICON });
                                        let name_r = ui.painter().text(row.min + vec2(46.0, 14.0), Align2::LEFT_CENTER, &d.meta.name, theme::regular(13.0), c::TEXT);
                                        // Same name twice (a user copy of a reference): tell them apart by id.
                                        if twin {
                                            ui.painter().text(name_r.right_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, &d.id, theme::mono(10.0), c::TEXT_FAINT);
                                        }
                                        let resp = resp.on_hover_text(format!("{}\n{}", d.meta.description, d.id).trim().to_string());
                                        if resp.clicked() {
                                            cmd = Some(Cmd::Tool(Tool::Place { def: d.id.clone(), rot: 0, mirror: false }));
                                        }
                                    }
                                });
                        }
                    });
                });
            });
        cmd
    }

    /// Floating inspector for the selection.
    pub fn inspector(&mut self, ctx: &egui::Context) {
        if self.selection.is_empty() {
            return;
        }
        let r = self.canvas_rect;
        egui::Area::new(egui::Id::unique("inspector"))
            .order(Order::Middle)
            .fixed_pos(pos2(r.max.x - 284.0, r.min.y + 8.0))
            .show(ctx, |ui| {
                theme::card().show(ui, |ui| {
                    ui.set_width(260.0);
                    if self.selection.len() > 1 {
                        ui.label(RichText::new(format!("{} items selected", self.selection.len())).font(theme::bold(14.0)));
                        ui.label(RichText::new("R rotate · M mirror · Ctrl+D duplicate · Del delete").color(c::TEXT_DIM).small());
                        self.sourcing_multi(ui);
                        return;
                    }
                    let item = *self.selection.iter().next().unwrap();
                    match item {
                        Item::Part(id) => {
                            let max_h = (r.height() - 40.0).max(200.0);
                            ui.set_max_height(max_h);
                            egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                                self.part_inspector(ui, id);
                                self.sourcing_section(ui, id);
                                self.scope_section(ui, item);
                            });
                        }
                        Item::Wire(id) => {
                            ui.label(RichText::new("Wire").font(theme::bold(14.0)));
                            if let Some(net) = self.built.wires.get(&id) {
                                kv(ui, "Net", net);
                                if let Some(v) = self.live.net_v.get(net) {
                                    kv(ui, "Voltage", &ecs_units::format_digits(*v, "V", 4));
                                }
                            }
                            self.scope_section(ui, item);
                        }
                        Item::Label(id) => {
                            ui.label(RichText::new("Net label").font(theme::bold(14.0)));
                            if let Some(l) = self.doc.labels.iter().find(|l| l.id == id).cloned() {
                                let mut name = l.name.clone();
                                ui.horizontal(|ui| {
                                    ui.label("Name");
                                    let resp = ui.add(egui::TextEdit::singleline(&mut name).desired_width(170.0));
                                    if resp.changed() && !name.trim().is_empty() {
                                        self.edit(Command::SetLabel { label: id, name: name.clone() });
                                    }
                                });
                                ui.label(RichText::new("Labels with the same name are connected. GND and 0 are ground.").color(c::TEXT_DIM).small());
                            }
                        }
                        Item::Note(id) => {
                            ui.label(RichText::new("Text").font(theme::bold(14.0)));
                            if let Some(n) = self.doc.notes.iter_mut().find(|n| n.id == id)
                                && ui.add(egui::TextEdit::multiline(&mut n.text).desired_rows(2).desired_width(240.0)).changed()
                            {
                                self.dirty = true;
                            }
                        }
                    }
                });
            });
    }

    fn part_inspector(&mut self, ui: &mut Ui, id: ecs_schematic::Id) {
        let Some(part) = self.doc.part(id).cloned() else {
            return;
        };
        let Some(def) = self.cat.def(&part.def).cloned() else {
            ui.label(format!("Unknown component {}", part.def));
            return;
        };
        ui.horizontal(|ui| {
            let thumb = ui.allocate_exact_size(vec2(36.0, 26.0), Sense::hover()).0;
            paint_symbol(ui.painter(), thumb, &def.symbol, c::ICON);
            ui.vertical(|ui| {
                ui.label(RichText::new(&def.meta.name).font(theme::bold(14.0)));
                ui.label(RichText::new(&def.meta.category).color(c::TEXT_DIM).small());
            });
        });
        ui.add_space(4.0);
        egui::Grid::new("params")
            .num_columns(2)
            .spacing(vec2(10.0, 6.0))
            .show(ui, |ui| {
                ui.label(RichText::new("Name").color(c::TEXT_DIM));
                let mut refdes = part.refdes.clone();
                if ui
                    .add(egui::TextEdit::singleline(&mut refdes).desired_width(150.0))
                    .lost_focus()
                    && refdes != part.refdes
                    && !refdes.trim().is_empty()
                {
                    self.edit(Command::SetRefdes { part: id, refdes });
                }
                ui.end_row();
                self.part_number_row(ui, id, &def.id);
                for p in &def.params {
                    let label = if p.unit.is_empty() {
                        p.name.clone()
                    } else {
                        format!("{} ({})", p.name, p.unit)
                    };
                    ui.label(RichText::new(label).color(c::TEXT_DIM))
                        .on_hover_text(&p.description);
                    if let ecs_model::ParamKind::Enum(names) = &p.kind {
                        let cur = catalog::param_value(&part, &def, &p.name) as usize;
                        let mut sel = cur;
                        egui::ComboBox::from_id_salt(format!("enum-{}", p.name))
                            .width(150.0)
                            .selected_text(names.get(cur).cloned().unwrap_or_default())
                            .show_ui(ui, |ui| {
                                for (k, n) in names.iter().enumerate() {
                                    ui.selectable_value(&mut sel, k, n);
                                }
                            });
                        if sel != cur {
                            self.edit(Command::SetParam {
                                part: id,
                                name: p.name.clone(),
                                value: names[sel].clone(),
                            });
                        }
                    } else {
                        let key = format!("{id}:{}", p.name);
                        let mut text = self
                            .inspector_edit
                            .get(&key)
                            .cloned()
                            .unwrap_or_else(|| catalog::param_text(&part, &def, &p.name));
                        let ok = catalog::parse_param(&def, &p.name, &text).is_some();
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut text)
                                .desired_width(150.0)
                                .text_color(if ok { c::TEXT } else { c::DANGER }),
                        );
                        if resp.changed() {
                            self.inspector_edit.insert(key.clone(), text.clone());
                        }
                        if resp.lost_focus() {
                            self.inspector_edit.remove(&key);
                            if ok && text != catalog::param_text(&part, &def, &p.name) {
                                self.edit(Command::SetParam {
                                    part: id,
                                    name: p.name.clone(),
                                    value: text.clone(),
                                });
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        // Live controls.
        for ctrl in &def.controls {
            ui.add_space(6.0);
            match ctrl {
                ecs_model::ControlDef::Slider { param, label } => {
                    let mut v = catalog::param_value(&part, &def, param);
                    ui.label(RichText::new(label).color(c::TEXT_DIM));
                    if ui
                        .add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(true))
                        .changed()
                    {
                        if let Some(p) = self.doc.part_mut(id) {
                            p.params.insert(param.clone(), format!("{v:.3}"));
                        }
                        self.dirty = true;
                        self.sim.send(ecs_sim::live::Command::SetParam {
                            inst: part.refdes.clone(),
                            name: param.clone(),
                            value: v,
                        });
                    }
                }
                ecs_model::ControlDef::Toggle { state, label }
                | ecs_model::ControlDef::Momentary { state, label } => {
                    let mut on = part.states.get(state).copied().unwrap_or(0.0) > 0.5;
                    if ui.checkbox(&mut on, label.as_str()).changed() {
                        let v = if on { 1.0 } else { 0.0 };
                        if let Some(p) = self.doc.part_mut(id) {
                            p.states.insert(state.clone(), v);
                        }
                        self.dirty = true;
                        self.sim.send(ecs_sim::live::Command::SetState {
                            inst: part.refdes.clone(),
                            name: state.clone(),
                            value: v,
                        });
                    }
                }
            }
        }
        if let Some(vals) = self.live.part_probe.get(&id).cloned()
            && !vals.is_empty()
        {
            ui.add_space(6.0);
            ui.separator();
            ui.label(RichText::new("Live values").color(c::TEXT_DIM).small());
            for (p, v) in def.probes.iter().zip(vals) {
                let txt = if p.name == "brightness" {
                    format!("{:.0} %", v * 100.0)
                } else {
                    ecs_units::format_digits(v, &p.unit, 4)
                };
                kv(ui, &p.name, &txt);
            }
        }
        if ecs_mcu::is_mcu(&part.def) {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Edit Program…").strong())
                    .on_hover_text("Assembly source of this chip")
                    .clicked()
                {
                    self.open_program(id);
                }
                let lines = part.code.lines().filter(|l| !l.trim().is_empty()).count();
                ui.label(
                    RichText::new(if lines == 0 {
                        "no program: every pin is an input".to_string()
                    } else {
                        format!("{lines} lines")
                    })
                    .color(c::TEXT_DIM)
                    .small(),
                );
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .button("Edit Component…")
                .on_hover_text("Symbol, ports and implementation (Ctrl+E)")
                .clicked()
            {
                self.edit_component(&part.refdes);
            }
            let inner = self.root_doc().components.iter().any(|c| {
                c.id == part.def
                    && matches!(
                        c.implementation,
                        ecs_schematic::Implementation::Schematic { .. }
                            | ecs_schematic::Implementation::ExternalSchematic { .. }
                    )
            });
            if inner
                && ui
                    .button("Enter")
                    .on_hover_text("Edit its schematic (Ctrl+Enter)")
                    .clicked()
            {
                self.enter_component(&part.refdes);
            }
        });
        if !def.meta.description.is_empty() {
            ui.add_space(4.0);
            ui.label(
                RichText::new(&def.meta.description)
                    .color(c::TEXT_FAINT)
                    .small(),
            );
        }
    }

    /// Floating navigation toolbar at the bottom centre of the canvas.
    pub fn nav_toolbar(&mut self, ctx: &egui::Context) -> Option<Cmd> {
        let mut cmd = None;
        let r = self.canvas_rect;
        let w = 9.0 * 32.0 + 40.0;
        egui::Area::new(egui::Id::unique("nav"))
            .order(Order::Middle)
            .fixed_pos(pos2(r.center().x - w / 2.0, r.max.y - 52.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(c::NAV)
                    .stroke(Stroke::new(1.0, c::PANEL_BORDER))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for (icon, tip, c2, active) in [
                                (
                                    Icon::Select,
                                    "Select (Esc)",
                                    Cmd::Tool(Tool::Select),
                                    self.tool == Tool::Select,
                                ),
                                (
                                    Icon::Pan,
                                    "Pan (H, middle drag, Space+drag)",
                                    Cmd::Tool(Tool::Pan),
                                    self.tool == Tool::Pan,
                                ),
                                (
                                    Icon::Wire,
                                    "Wire (W)",
                                    Cmd::Tool(Tool::Wire),
                                    self.tool == Tool::Wire,
                                ),
                            ] {
                                if icon_button(ui, icon, 28.0, tip, true, active) {
                                    cmd = Some(c2);
                                }
                            }
                            sep_v(ui);
                            for (icon, tip, c2) in [
                                (Icon::Fit, "Fit (F)", Cmd::Fit),
                                (Icon::ZoomIn, "Zoom in", Cmd::ZoomIn),
                                (Icon::ZoomOut, "Zoom out", Cmd::ZoomOut),
                            ] {
                                if icon_button(ui, icon, 28.0, tip, true, false) {
                                    cmd = Some(c2);
                                }
                            }
                            sep_v(ui);
                            for (icon, tip, c2, active) in [
                                (Icon::Grid, "Grid (G)", Cmd::ToggleGrid, self.grid),
                                (
                                    Icon::Voltage,
                                    "Voltage colours",
                                    Cmd::ToggleVolts,
                                    self.volt_colors,
                                ),
                                (
                                    Icon::Current,
                                    "Current dots",
                                    Cmd::ToggleDots,
                                    self.current_dots,
                                ),
                            ] {
                                if icon_button(ui, icon, 28.0, tip, true, active) {
                                    cmd = Some(c2);
                                }
                            }
                        });
                    });
            });
        cmd
    }

    /// The Simulation bar, in the place of SolidCraft's History bar.
    pub fn sim_bar(&mut self, ui: &mut Ui) -> Option<Cmd> {
        let mut cmd = None;
        egui::Panel::bottom("simbar")
            .exact_size(42.0)
            .frame(egui::Frame::new().fill(c::TITLEBAR).inner_margin(Margin { left: 6, right: 6, top: 2, bottom: 6 }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().rect_filled(r, 7.0, c::BAR);
                ui.painter().rect_stroke(r, 7.0, Stroke::new(1.0, c::PANEL_BORDER), StrokeKind::Inside);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(10.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
                let ui = &mut child;
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new("⠿").color(c::TEXT_FAINT));
                ui.label(RichText::new("Simulation").font(theme::bold(13.5)));
                ui.add_space(6.0);
                if icon_button(ui, Icon::Reset, 26.0, "Reset to the operating point (F7)", true, false) {
                    cmd = Some(Cmd::Reset);
                }
                if icon_button(ui, Icon::Step, 26.0, "Step (F6)", true, false) {
                    cmd = Some(Cmd::Step);
                }
                let icon = if self.sim_running { Icon::Pause } else { Icon::Play };
                if icon_button(ui, icon, 26.0, "Run / pause (F5)", true, self.sim_running) {
                    cmd = Some(Cmd::ToggleRun);
                }
                ui.add_space(4.0);
                // Time cursor like the History marker.
                let (tr, _) = ui.allocate_exact_size(vec2(130.0, 26.0), Sense::hover());
                ui.painter().text(tr.left_center(), Align2::LEFT_CENTER, format!("t = {}", ecs_units::format_digits(self.sim_time, "s", 4)), theme::mono(12.5), c::TEXT);
                ui.label(RichText::new("Speed").color(c::TEXT_DIM));
                let speeds = [1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 0.1, 1.0];
                let mut speed = self.doc.simulation.speed;
                egui::ComboBox::from_id_salt("speed").width(92.0).selected_text(format!("{}/s", ecs_units::format(speed, "s"))).show_ui(ui, |ui| {
                    for s in speeds {
                        ui.selectable_value(&mut speed, s, format!("{}/s", ecs_units::format(s, "s")));
                    }
                });
                if speed != self.doc.simulation.speed {
                    self.doc.simulation.speed = speed;
                    self.scope.window = 0.0;
                    self.dirty = true;
                    self.sim.send(ecs_sim::live::Command::SetSpeed(speed));
                }
                ui.label(RichText::new("Step").color(c::TEXT_DIM));
                let steps = [1e-8, 1e-7, 1e-6, 1e-5, 1e-4];
                let mut step = self.doc.simulation.time_step;
                egui::ComboBox::from_id_salt("step").width(72.0).selected_text(ecs_units::format(step, "s")).show_ui(ui, |ui| {
                    for s in steps {
                        ui.selectable_value(&mut step, s, ecs_units::format(s, "s"));
                    }
                });
                if step != self.doc.simulation.time_step {
                    self.doc.simulation.time_step = step;
                    self.dirty = true;
                    self.sim.send(ecs_sim::live::Command::SetTimeStep(step));
                }
                let actual = if self.sim_running { format!("{}/s actual", ecs_units::format(self.actual_speed, "s")) } else { "paused".into() };
                ui.label(RichText::new(actual).color(c::TEXT_DIM).font(theme::mono(11.5)));
                self.analysis.jobs_ui(ui);
                let problems = self.problems();
                if !problems.is_empty() {
                    let (pr, resp) = ui.allocate_exact_size(vec2(120.0, 24.0), Sense::click());
                    ui.painter().rect_filled(pr, 5.0, if resp.hovered() { c::HOVER } else { c::TAB });
                    icons::paint(ui.painter(), Rect::from_center_size(pr.left_center() + vec2(14.0, 0.0), vec2(16.0, 16.0)), Icon::Warning, c::WARN, false);
                    ui.painter().text(pr.left_center() + vec2(28.0, 0.0), Align2::LEFT_CENTER, format!("{} problem{}", problems.len(), if problems.len() == 1 { "" } else { "s" }), theme::regular(12.5), c::TEXT);
                    if resp.clicked() {
                        self.dialog = Some("problems");
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let hint = match &self.tool {
                        Tool::Wire => "Click to route · / flips the elbow · double-click or click a pin to finish · Esc cancels",
                        Tool::Place { .. } => "Click to place · R rotate · M mirror · Esc stops placing",
                        Tool::Probe => "Click a wire to scope its voltage, a part for its current (Shift: its voltage)",
                        _ => "Wheel: zoom · Middle drag: pan · Shift: add · Ctrl: remove",
                    };
                    let mut text = if self.status.is_empty() { hint.to_string() } else { self.status.clone() };
                    if self.software_canvas || self.renderer.is_none() {
                        text.push_str("   ·   software canvas");
                    } else if self.frame_ms > 0.0 {
                        text.push_str(&format!("   ·   raylib {:.1} ms (read-back {:.1} ms)", self.frame_ms, self.readback_ms));
                    }
                    ui.add(egui::Label::new(RichText::new(text).color(c::TEXT_DIM).size(12.0)).truncate());
                });
            });
        cmd
    }

    /// All current problems for the problems list.
    pub fn problems(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(e) = &self.sim_error {
            v.push(e.clone());
        }
        v.extend(self.local_problems.iter().cloned());
        v.extend(self.built.problems.iter().cloned());
        v.extend(self.sim_warnings.iter().cloned());
        for (id, k) in &self.built.unconnected {
            if let Some(p) = self.doc.part(*id)
                && !p.def.starts_with("mcu.")
                && !p.def.starts_with("ic.")
            {
                let pin = p
                    .symbol()
                    .pins
                    .get(*k)
                    .map(|x| x.name.clone())
                    .unwrap_or_default();
                v.push(format!("{} pin {pin} is not connected", p.refdes));
            }
        }
        for id in &self.live.trouble {
            if let Some(p) = self.doc.part(*id) {
                v.push(format!(
                    "{}: convergence trouble; the step was accepted anyway",
                    p.refdes
                ));
            }
        }
        // Script hook errors, user components, packages, analyses.
        for d in self.cat.lib.iter() {
            if let Some(e) = d.behavior.as_ref().and_then(|b| b.diagnostics()) {
                v.push(format!("{}: {e}", d.meta.name));
            }
        }
        v.extend(self.user_lib.problems.iter().cloned());
        v.extend(self.package_problems.iter().cloned());
        v.extend(self.analysis.errors.iter().cloned());
        v
    }

    /// Command palette (Ctrl+K).
    pub fn palette(&mut self, ctx: &egui::Context) -> Option<Cmd> {
        if !self.palette_open {
            return None;
        }
        // Every argument-free command from the registry, plus analyses and parts.
        let keys: std::collections::HashMap<&str, &str> = [
            ("file.new", "Ctrl+N"),
            ("file.open", "Ctrl+O"),
            ("file.save", "Ctrl+S"),
            ("file.save_as", "Ctrl+Shift+S"),
            ("edit.undo", "Ctrl+Z"),
            ("edit.redo", "Ctrl+Y"),
            ("edit.delete", "Del"),
            ("edit.rotate", "R"),
            ("edit.mirror", "M"),
            ("edit.copy", "Ctrl+C"),
            ("edit.paste", "Ctrl+V"),
            ("edit.duplicate", "Ctrl+D"),
            ("edit.select_all", "Ctrl+A"),
            ("tool.select", "Esc"),
            ("tool.wire", "W"),
            ("tool.label", "L"),
            ("tool.text", "T"),
            ("tool.probe", "P"),
            ("tool.pan", "H"),
            ("sim.toggle_run", "F5"),
            ("sim.step", "F6"),
            ("sim.reset", "F7"),
            ("view.fit", "F"),
            ("view.grid", "G"),
            ("app.palette", "Ctrl+K"),
        ]
        .into_iter()
        .collect();
        let mut entries: Vec<(String, String, Cmd)> = crate::commands::catalog()
            .into_iter()
            .filter(|i| i.args.as_object().is_some_and(|o| o.is_empty()) && i.id != "app.palette")
            .filter_map(|i| {
                crate::commands::from_id(i.id, &serde_json::json!({}))
                    .ok()
                    .map(|c2| {
                        (
                            i.title.to_string(),
                            keys.get(i.id).unwrap_or(&"").to_string(),
                            c2,
                        )
                    })
            })
            .collect();
        for kind in ["op", "dc", "tran", "ac"] {
            if let Ok(c2) =
                crate::commands::from_id("analysis.dialog", &serde_json::json!({ "kind": kind }))
            {
                let title = match &c2 {
                    Cmd::Analysis(w) => format!("Analysis: {}", w.title()),
                    _ => continue,
                };
                entries.push((title, String::new(), c2));
            }
        }
        for d in self.cat.lib.iter() {
            entries.push((
                format!("Place {}", d.meta.name),
                String::new(),
                Cmd::Tool(Tool::Place {
                    def: d.id.clone(),
                    rot: 0,
                    mirror: false,
                }),
            ));
        }
        let q = self.palette_query.to_lowercase();
        let filtered: Vec<&(String, String, Cmd)> = entries
            .iter()
            .filter(|(n, _, _)| q.split_whitespace().all(|w| n.to_lowercase().contains(w)))
            .collect();
        let mut chosen = None;
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("palette"))
            .order(Order::Foreground)
            .fixed_pos(pos2(screen.center().x - 260.0, screen.min.y + 90.0))
            .show(ctx, |ui| {
                theme::card().show(ui, |ui| {
                    ui.set_width(520.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.palette_query)
                            .hint_text("Type a command or a component")
                            .desired_width(f32::INFINITY)
                            .font(theme::regular(15.0)),
                    );
                    resp.request_focus();
                    if resp.changed() {
                        self.palette_sel = 0;
                    }
                    let (down, up, enter) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::ArrowDown),
                            i.key_pressed(egui::Key::ArrowUp),
                            i.key_pressed(egui::Key::Enter),
                        )
                    });
                    if down {
                        self.palette_sel =
                            (self.palette_sel + 1).min(filtered.len().saturating_sub(1));
                    }
                    if up {
                        self.palette_sel = self.palette_sel.saturating_sub(1);
                    }
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            for (k, (name, key, cmd)) in filtered.iter().enumerate() {
                                let (r, row) = ui.allocate_exact_size(
                                    vec2(ui.available_width(), 28.0),
                                    Sense::click(),
                                );
                                if k == self.palette_sel || row.hovered() {
                                    ui.painter().rect_filled(
                                        r,
                                        5.0,
                                        if k == self.palette_sel {
                                            c::ACTIVE_TAB_BG
                                        } else {
                                            c::HOVER
                                        },
                                    );
                                }
                                ui.painter().text(
                                    r.left_center() + vec2(10.0, 0.0),
                                    Align2::LEFT_CENTER,
                                    name,
                                    theme::regular(13.5),
                                    c::TEXT,
                                );
                                ui.painter().text(
                                    r.right_center() - vec2(10.0, 0.0),
                                    Align2::RIGHT_CENTER,
                                    key,
                                    theme::mono(11.0),
                                    c::TEXT_DIM,
                                );
                                if k == self.palette_sel && (down || up) {
                                    ui.scroll_to_rect(r, None);
                                }
                                if row.clicked() || (enter && k == self.palette_sel) {
                                    chosen = Some(cmd.clone());
                                }
                            }
                        });
                });
            });
        if chosen.is_some() {
            self.palette_open = false;
        }
        chosen
    }

    /// About, shortcuts and problems dialogs.
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(which) = self.dialog else { return };
        let mut open = true;
        let title = match which {
            "about" => "About Electrocraft",
            "shortcuts" => "Keyboard Shortcuts",
            "settings" => "Settings",
            _ => "Problems",
        };
        let mut window = egui::Window::new(RichText::new(title).font(theme::bold(14.0)))
            .open(&mut open)
            .collapsible(false)
            .resizable(which == "settings")
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0));
        if which == "settings" {
            window = window
                .default_size(vec2(640.0, 640.0))
                .min_size(vec2(520.0, 360.0));
        }
        window.show(ctx, |ui| match which {
                "settings" => {
                    #[derive(Clone, Copy, Default, PartialEq)]
                    enum SettingsPage {
                        #[default]
                        Simulation,
                        Catalogs,
                        Mcp,
                    }
                    let page_id = ui.make_persistent_id("settings_page");
                    let mut page = ui.data(|data| data.get_temp::<SettingsPage>(page_id).unwrap_or_default());
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut page, SettingsPage::Simulation, "Live simulation");
                        ui.selectable_value(&mut page, SettingsPage::Catalogs, "Online part catalogs");
                        ui.selectable_value(&mut page, SettingsPage::Mcp, "MCP server");
                    });
                    ui.data_mut(|data| data.insert_temp(page_id, page));
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt(("settings-page", page as u8))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                    match page {
                        SettingsPage::Catalogs => return self.catalog_settings(ui),
                        SettingsPage::Mcp => return self.mcp_settings(ui),
                        SettingsPage::Simulation => {}
                    }
                    ui.label(RichText::new("Live simulation").font(theme::bold(13.0)));
                    let before = (self.settings.clone(), self.grid, self.volt_colors, self.current_dots);
                    egui::Grid::new("settings").num_columns(2).spacing(vec2(14.0, 8.0)).show(ui, |ui| {
                        ui.label("Current dot speed");
                        ui.horizontal(|ui| {
                            ui.add(egui::Slider::new(&mut self.settings.dot_speed, 0.05..=4.0).logarithmic(true).show_value(false));
                            ui.label(RichText::new(format!("{:.2}×", self.settings.dot_speed)).font(theme::mono(11.5)));
                            if ui.small_button("Reset").clicked() {
                                self.settings.dot_speed = crate::settings::DOT_SPEED;
                            }
                        });
                        ui.end_row();
                        ui.label("");
                        ui.label(RichText::new("Dots speed up by one step per decade of current (1 nA to 1 A); this scales all of them.").color(c::TEXT_DIM).small());
                        ui.end_row();
                        ui.label("Current direction");
                        ui.vertical(|ui| {
                            ui.radio_value(&mut self.settings.electron_flow, false, "Conventional: + to − (engineering)");
                            ui.radio_value(&mut self.settings.electron_flow, true, "Electron flow: − to + (physical)");
                        });
                        ui.end_row();
                        ui.label("Resistor symbol");
                        ui.vertical(|ui| {
                            ui.radio_value(&mut self.settings.iec_resistors, false, "Zigzag (ANSI / IEEE 315)");
                            ui.radio_value(&mut self.settings.iec_resistors, true, "Rectangle (IEC 60617)");
                        });
                        ecs_schematic::symbols::set_resistor_style(self.settings.iec_resistors);
                        ui.end_row();
                        ui.label("Show");
                        ui.vertical(|ui| {
                            ui.checkbox(&mut self.current_dots, "Current dots");
                            ui.checkbox(&mut self.volt_colors, "Voltage colours on wires");
                            ui.checkbox(&mut self.grid, "Grid");
                        });
                        ui.end_row();
                    });
                    self.settings.grid = self.grid;
                    self.settings.volt_colors = self.volt_colors;
                    self.settings.current_dots = self.current_dots;
                    if (self.settings.clone(), self.grid, self.volt_colors, self.current_dots) != before {
                        self.settings.save();
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("Saved for next time.").color(c::TEXT_FAINT).small());
                    });
                }
                "about" => {
                    ui.set_width(380.0);
                    ui.horizontal(|ui| {
                        icons::show(ui, Icon::Logo, 40.0, c::TEXT);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Electrocraft").font(theme::bold(20.0)));
                            ui.label(RichText::new(format!("Version {VERSION}")).color(c::TEXT_DIM));
                        });
                    });
                    ui.add_space(6.0);
                    ui.label("Draw electronic schematics and simulate them live: wire colours show voltage, dots show current, and switches and knobs act while it runs.");
                    ui.add_space(6.0);
                    ui.label(RichText::new("Engine: modified nodal analysis with autodiff tapes, Newton-Raphson and trapezoidal integration. Canvas: Raylib off-screen, shown in egui (OpenGL).").color(c::TEXT_DIM).small());
                    ui.label(RichText::new("Inter and Noto Sans Mono fonts under the SIL Open Font License.").color(c::TEXT_FAINT).small());
                }
                "shortcuts" => {
                    egui::Grid::new("keys").num_columns(2).spacing(vec2(24.0, 4.0)).show(ui, |ui| {
                        for (k, d) in [
                            ("Wheel", "Zoom at the pointer"),
                            ("Middle drag / Space+drag / H", "Pan"),
                            ("Drag left → right", "Select enclosed items"),
                            ("Drag right → left", "Select touched items"),
                            ("Shift / Ctrl + click", "Add to / remove from selection"),
                            ("W  L  T  P", "Wire, label, text, probe tools"),
                            ("R  M", "Rotate, mirror (also while placing)"),
                            ("/", "Flip the wire elbow"),
                            ("Del", "Delete"),
                            ("Ctrl+C  Ctrl+V  Ctrl+D", "Copy, paste, duplicate"),
                            ("Ctrl+Z  Ctrl+Y", "Undo, redo"),
                            ("F  G", "Fit, toggle grid"),
                            ("F5  F6  F7", "Run/pause, step, reset"),
                            ("Ctrl+K", "Command palette"),
                            ("Esc", "Cancel, select tool, back from Start"),
                        ] {
                            ui.label(RichText::new(k).font(theme::mono(12.0)).color(c::ACTIVE_TAB_TEXT));
                            ui.label(d);
                            ui.end_row();
                        }
                    });
                }
                _ => {
                    ui.set_width(460.0);
                    let probs = self.problems();
                    if probs.is_empty() {
                        ui.label("No problems.");
                    }
                    for p in probs {
                        ui.horizontal(|ui| {
                            icons::show(ui, Icon::Warning, 16.0, c::WARN);
                            ui.label(p);
                        });
                    }
                }
            });
        if !open {
            self.dialog = None;
        }
    }
}

fn kv(ui: &mut Ui, k: &str, v: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(k).color(c::TEXT_DIM));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(v).font(theme::mono(12.5)));
        });
    });
}

fn sep_v(ui: &mut Ui) {
    let r = ui.allocate_exact_size(vec2(9.0, 22.0), Sense::hover()).0;
    ui.painter().line_segment(
        [r.center_top(), r.center_bottom()],
        Stroke::new(1.0, c::PANEL_BORDER),
    );
}

/// What a ribbon button shows.
enum Btn {
    I(Icon),
    S(&'static str),
}

/// A ribbon group: big buttons over a group caption with a chevron.
fn group(
    ui: &mut Ui,
    title: &str,
    cmd: &mut Option<Cmd>,
    buttons: &[(Btn, &str, Cmd, bool, bool)],
) {
    let widths: Vec<f32> = buttons
        .iter()
        .map(|(_, l, ..)| {
            (ui.painter()
                .layout_no_wrap(l.to_string(), theme::regular(12.0), c::TEXT)
                .size()
                .x
                + 16.0)
                .max(54.0)
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 8.0;
    let (r, _) = ui.allocate_exact_size(vec2(total, 78.0), Sense::hover());
    let mut x = r.min.x + 4.0;
    for ((btn, label, c2, active, enabled), w) in buttons.iter().zip(&widths) {
        let br = Rect::from_min_size(pos2(x, r.min.y), vec2(*w, 54.0));
        let resp = ui.interact(
            br,
            ui.scope_id().with((title, label)),
            if *enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if *active {
            ui.painter().rect_filled(br, 5.0, c::ACTIVE_TAB_BG);
        } else if resp.hovered() && *enabled {
            ui.painter().rect_filled(br, 5.0, c::HOVER);
        }
        let ir = Rect::from_center_size(br.center_top() + vec2(0.0, 18.0), vec2(26.0, 26.0));
        let line = if *enabled { c::TEXT } else { c::TEXT_FAINT };
        match btn {
            Btn::I(icon) => icons::paint(ui.painter(), ir, *icon, line, !enabled),
            Btn::S(sym) => paint_symbol(
                ui.painter(),
                ir.expand2(vec2(6.0, 0.0)),
                sym,
                if *enabled { c::ICON } else { c::TEXT_FAINT },
            ),
        }
        ui.painter().text(
            br.center_top() + vec2(0.0, 43.0),
            Align2::CENTER_CENTER,
            *label,
            theme::regular(12.0),
            line,
        );
        if resp.clicked() && *enabled {
            *cmd = Some(c2.clone());
        }
        x += w;
    }
    ui.painter().text(
        pos2(r.center().x - 5.0, r.max.y - 9.0),
        Align2::CENTER_CENTER,
        title,
        theme::regular(11.5),
        c::TEXT_DIM,
    );
    let chev = Rect::from_center_size(
        pos2(
            r.center().x
                + ui.painter()
                    .layout_no_wrap(title.into(), theme::regular(11.5), c::TEXT)
                    .size()
                    .x
                    / 2.0
                + 2.0,
            r.max.y - 9.0,
        ),
        vec2(10.0, 10.0),
    );
    icons::paint(ui.painter(), chev, Icon::Chevron, c::TEXT_DIM, true);
    // Group divider.
    ui.painter().line_segment(
        [pos2(r.max.x, r.min.y + 4.0), pos2(r.max.x, r.max.y - 4.0)],
        Stroke::new(1.0, c::PANEL_BORDER),
    );
}
