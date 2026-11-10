//! Component editor (spec 9.7): name and ports, a symbol drawn with simple
//! tools (or started from an existing symbol), parameters, and the
//! implementation: nested or external schematic, inline or external
//! JavaScript. Also the stack of schematics entered for editing.

use crate::app::{App, Tool};
use crate::components;
use crate::theme::{self, c};
use ecs_schematic::symbols::{self, GRID, Prim, SymPin, Symbol};
use ecs_schematic::{
    Command, History, Id, Implementation, Item, LocalComponent, LocalParam, NetLabel, P2, Schematic,
};
use egui::{Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// A schematic set aside while editing a component's inner schematic.
pub struct Frame {
    pub doc: Schematic,
    pub history: History,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub selection: BTreeSet<Item>,
    /// The component whose schematic is open above this frame.
    pub comp_id: String,
    /// The open schematic is an external file (saved on its own).
    pub external: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Symbol,
    Implementation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymTool {
    Select,
    Line,
    Rect,
    Circle,
    Polyline,
    Text,
    Pin,
}

/// State of the component editor window.
pub struct CompEditor {
    pub comp: LocalComponent,
    /// The prototype as it was when the editor opened (None: new).
    pub original: Option<LocalComponent>,
    /// The part the editor was opened from.
    pub part: Option<Id>,
    /// Definition the part used before (to re-point every instance).
    pub from_def: Option<String>,
    pub apply_all: bool,
    pub tab: Tab,
    pub tool: SymTool,
    drag: Option<P2>,
    poly: Vec<P2>,
    selected: Option<Sel>,
    pub note: Option<String>,
    pub check: Option<Result<String, String>>,
    pin_name: String,
    text: String,
    base: String,
    /// Implementations kept while switching kinds.
    stash_js: String,
    stash_sch: Option<Box<Schematic>>,
    stash_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sel {
    Prim(usize),
    Pin(usize),
}

impl CompEditor {
    pub fn new(
        comp: LocalComponent,
        original: Option<LocalComponent>,
        part: Option<Id>,
        from_def: Option<String>,
    ) -> Self {
        let stash_js = match &comp.implementation {
            Implementation::Script { source } => source.clone(),
            _ => {
                let pins: Vec<String> = comp.symbol.pins.iter().map(|p| p.name.clone()).collect();
                components::template_js(&comp.id, &comp.name, &pins)
            }
        };
        Self {
            comp,
            original,
            part,
            from_def,
            apply_all: false,
            tab: Tab::General,
            tool: SymTool::Select,
            drag: None,
            poly: Vec::new(),
            selected: None,
            note: None,
            check: None,
            pin_name: String::new(),
            text: "IC".into(),
            base: String::new(),
            stash_js,
            stash_sch: None,
            stash_path: String::new(),
        }
    }

    fn next_pin_name(&self) -> String {
        if !self.pin_name.trim().is_empty()
            && !self
                .comp
                .symbol
                .pins
                .iter()
                .any(|p| p.name == self.pin_name.trim())
        {
            return self.pin_name.trim().to_string();
        }
        let mut k = self.comp.symbol.pins.len() + 1;
        loop {
            let n = format!("P{k}");
            if !self.comp.symbol.pins.iter().any(|p| p.name == n) {
                return n;
            }
            k += 1;
        }
    }
}

/// Bounding box of a symbol's drawing and pins, padded a little.
pub fn fit_bbox(s: &mut Symbol) {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut add = |p: P2, r: f64| {
        lo[0] = lo[0].min(p[0] - r);
        lo[1] = lo[1].min(p[1] - r);
        hi[0] = hi[0].max(p[0] + r);
        hi[1] = hi[1].max(p[1] + r);
    };
    for prim in s.prims.iter().chain(&s.prims_off).chain(&s.prims_on) {
        match prim {
            Prim::Line { a, b } => {
                add(*a, 0.0);
                add(*b, 0.0);
            }
            Prim::Polyline { points, .. } => points.iter().for_each(|p| add(*p, 0.0)),
            Prim::Circle { c, r, .. } | Prim::Arc { c, r, .. } => add(*c, *r),
            Prim::Text { at, size, text } => {
                let w = *size * 0.3 * text.chars().count() as f64;
                add([at[0] - w, at[1]], *size * 0.6);
                add([at[0] + w, at[1]], *size * 0.6);
            }
        }
    }
    for p in &s.pins {
        add(p.at, 2.0);
    }
    if !lo[0].is_finite() {
        lo = [-10.0, -10.0];
        hi = [10.0, 10.0];
    }
    s.bbox = [lo, hi];
}

fn snap_to(p: P2, step: f64) -> P2 {
    [(p[0] / step).round() * step, (p[1] / step).round() * step]
}

fn seg_dist(p: P2, a: P2, b: P2) -> f64 {
    ecs_schematic::seg_distance(p, a, b)
}

fn prim_distance(prim: &Prim, p: P2) -> f64 {
    match prim {
        Prim::Line { a, b } => seg_dist(p, *a, *b),
        Prim::Polyline { points, closed, .. } => {
            let mut d = f64::INFINITY;
            for w in points.windows(2) {
                d = d.min(seg_dist(p, w[0], w[1]));
            }
            if *closed && points.len() > 2 {
                d = d.min(seg_dist(p, points[points.len() - 1], points[0]));
            }
            d
        }
        Prim::Circle { c, r, .. } | Prim::Arc { c, r, .. } => {
            (((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt() - r).abs()
        }
        Prim::Text { at, .. } => ((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2)).sqrt() - 4.0,
    }
}

fn move_prim(prim: &mut Prim, d: P2) {
    let mv = |q: &mut P2| {
        q[0] += d[0];
        q[1] += d[1];
    };
    match prim {
        Prim::Line { a, b } => {
            mv(a);
            mv(b);
        }
        Prim::Polyline { points, .. } => points.iter_mut().for_each(mv),
        Prim::Circle { c, .. } | Prim::Arc { c, .. } => mv(c),
        Prim::Text { at, .. } => mv(at),
    }
}

/// Draw symbol primitives with a world→screen mapping.
fn draw_prims(
    painter: &egui::Painter,
    prims: &[Prim],
    to: &dyn Fn(P2) -> Pos2,
    scale: f32,
    color: Color32,
    width: f32,
) {
    let stroke = Stroke::new(width, color);
    for prim in prims {
        match prim {
            Prim::Line { a, b } => {
                painter.line_segment([to(*a), to(*b)], stroke);
            }
            Prim::Polyline {
                points,
                closed,
                fill,
            } => {
                let pts: Vec<Pos2> = points.iter().map(|q| to(*q)).collect();
                if *fill && pts.len() > 2 {
                    painter.add(egui::Shape::convex_polygon(
                        pts.clone(),
                        color.gamma_multiply(0.6),
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
                    painter.circle_filled(to(*centre), *r as f32 * scale, color);
                } else {
                    painter.circle_stroke(to(*centre), *r as f32 * scale, stroke);
                }
            }
            Prim::Arc {
                c: centre,
                r,
                a0,
                a1,
            } => {
                let pts: Vec<Pos2> = (0..=16)
                    .map(|k| {
                        let a = a0 + (a1 - a0) * k as f64 / 16.0;
                        to([centre[0] + r * a.cos(), centre[1] + r * a.sin()])
                    })
                    .collect();
                painter.add(egui::Shape::line(pts, stroke));
            }
            Prim::Text { at, text, size } => {
                painter.text(
                    to(*at),
                    Align2::CENTER_CENTER,
                    text,
                    theme::bold((*size as f32 * scale).max(6.0)),
                    color,
                );
            }
        }
    }
}

/// Port labels for a new inner schematic: one per symbol pin, on the side
/// the pin is on.
pub fn port_labels(doc: &mut Schematic, symbol: &Symbol) {
    for (k, p) in symbol.pins.iter().enumerate() {
        if doc.labels.iter().any(|l| l.name == p.name) {
            continue;
        }
        let left = p.at[0] <= 0.0;
        let id = doc.alloc_id();
        doc.labels.push(NetLabel {
            id,
            at: [if left { -200.0 } else { 200.0 }, -100.0 + 40.0 * k as f64],
            name: p.name.clone(),
        });
    }
}

impl App {
    /// The outermost document (the file on disk).
    pub fn root_doc(&self) -> &Schematic {
        self.stack.first().map_or(&self.doc, |f| &f.doc)
    }

    /// Folder external component files are relative to.
    pub fn root_dir(&self) -> Option<PathBuf> {
        let p = self
            .stack
            .first()
            .map_or(self.path.as_ref(), |f| f.path.as_ref());
        p.and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    /// Apply a command to the outermost document (component prototypes live there).
    pub fn edit_root(&mut self, cmd: Command) {
        match self.stack.first_mut() {
            None => {
                self.edit(cmd);
            }
            Some(f) => {
                f.history.apply(&mut f.doc, cmd);
                f.dirty = true;
                self.rev += 1;
            }
        }
    }

    /// Open the component editor for a part's definition.
    pub fn edit_component(&mut self, refdes: &str) {
        let Some(part) = self.doc.parts.iter().find(|p| p.refdes == refdes).cloned() else {
            self.status = format!("No part {refdes}");
            return;
        };
        if let Some(c) = self
            .root_doc()
            .components
            .iter()
            .find(|c| c.id == part.def)
            .cloned()
        {
            self.comp_editor = Some(CompEditor::new(
                c.clone(),
                Some(c),
                Some(part.id),
                Some(part.def.clone()),
            ));
            return;
        }
        let Some(def) = self.cat.def(&part.def).cloned() else {
            self.status = format!("Unknown component {}", part.def);
            return;
        };
        let (copy, note) = components::local_copy(self.root_doc(), &def);
        let mut ed = CompEditor::new(copy, None, Some(part.id), Some(part.def.clone()));
        ed.note = Some(match note {
            Some(why) => format!("{why}. Applying makes an editable copy for {refdes}."),
            None => format!(
                "{} is a library component: applying makes an editable copy for {refdes}.",
                def.meta.name
            ),
        });
        ed.tab = Tab::Implementation;
        self.comp_editor = Some(ed);
    }

    /// Open the editor on a new, blank component.
    pub fn new_component(&mut self) {
        let comp = components::blank(self.root_doc());
        let mut ed = CompEditor::new(comp, None, None, None);
        ed.note = Some("Draw the symbol and its pins, then choose how it works.".into());
        self.comp_editor = Some(ed);
    }

    /// Open a part's inner schematic for editing.
    pub fn enter_component(&mut self, refdes: &str) {
        let Some(part) = self.doc.parts.iter().find(|p| p.refdes == refdes).cloned() else {
            self.status = format!("No part {refdes}");
            return;
        };
        let Some(comp) = self
            .root_doc()
            .components
            .iter()
            .find(|c| c.id == part.def)
            .cloned()
        else {
            self.status =
                format!("{refdes} is a library part: use Edit Component to make an editable copy");
            return;
        };
        let (inner, external) = match &comp.implementation {
            Implementation::Schematic { schematic } => ((**schematic).clone(), None),
            Implementation::ExternalSchematic { path } => {
                match components::load_schematic(self.root_dir().as_deref(), path) {
                    Ok(s) => {
                        let full = self
                            .root_dir()
                            .map_or(PathBuf::from(path), |d| d.join(path));
                        (s, Some(full))
                    }
                    Err(e) => {
                        self.status = e;
                        return;
                    }
                }
            }
            _ => {
                self.comp_editor = Some(CompEditor::new(
                    comp.clone(),
                    Some(comp),
                    Some(part.id),
                    Some(part.def),
                ));
                if let Some(ed) = &mut self.comp_editor {
                    ed.tab = Tab::Implementation;
                }
                return;
            }
        };
        let mut inner = inner;
        port_labels(&mut inner, &comp.symbol);
        inner.metadata.title = comp.name.clone();
        let frame = Frame {
            doc: std::mem::replace(&mut self.doc, inner),
            history: std::mem::take(&mut self.history),
            path: self.path.take(),
            dirty: self.dirty,
            selection: std::mem::take(&mut self.selection),
            comp_id: comp.id.clone(),
            external,
        };
        self.stack.push(frame);
        self.dirty = false;
        self.tool = Tool::Select;
        self.rev += 1;
        self.fit_pending = true;
        self.fit_frames = 4;
        self.sync_sim(true);
        self.status = format!(
            "Editing {} — ports are the net labels {}",
            comp.name,
            comp.symbol
                .pins
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    /// Leave the inner schematic, writing it back into its component.
    pub fn exit_component(&mut self) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        let inner = std::mem::replace(&mut self.doc, frame.doc);
        let inner_dirty = self.dirty;
        self.history = frame.history;
        self.path = frame.path;
        self.dirty = frame.dirty;
        self.selection = frame.selection;
        self.status = format!("Back in {}", self.doc.metadata.title);
        if inner_dirty {
            match &frame.external {
                Some(path) => match std::fs::write(path, inner.to_json()) {
                    Ok(()) => self.status = format!("Saved {}", path.display()),
                    Err(e) => self.status = format!("Could not save {}: {e}", path.display()),
                },
                None => {
                    if let Some(mut comp) = self
                        .doc
                        .components
                        .iter()
                        .find(|c| c.id == frame.comp_id)
                        .cloned()
                    {
                        comp.implementation = Implementation::Schematic {
                            schematic: Box::new(inner),
                        };
                        self.edit(Command::SetComponent(Box::new(comp)));
                    }
                }
            }
        }
        // External files changed: recompile.
        self.local_cache = None;
        self.rev += 1;
        self.fit_pending = true;
        self.fit_frames = 4;
        self.sync_sim(true);
    }

    /// The document as saved: inner schematics being edited written back.
    pub fn composed_root(&self) -> Schematic {
        let mut child = self.doc.clone();
        for f in self.stack.iter().rev() {
            let mut parent = f.doc.clone();
            if f.external.is_none()
                && let Some(c) = parent.components.iter_mut().find(|c| c.id == f.comp_id)
            {
                c.implementation = Implementation::Schematic {
                    schematic: Box::new(child),
                };
            }
            child = parent;
        }
        child
    }

    /// Breadcrumb bar while inside a component.
    pub fn breadcrumb(&mut self, ctx: &egui::Context) {
        if self.stack.is_empty() {
            return;
        }
        let r = self.canvas_rect;
        let mut back = false;
        egui::Area::new(egui::Id::unique("breadcrumb"))
            .order(egui::Order::Middle)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(pos2(r.center().x, r.min.y + 8.0))
            .show(ctx, |ui| {
                theme::card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .button("◀ Back")
                            .on_hover_text("Return to the enclosing schematic")
                            .clicked()
                        {
                            back = true;
                        }
                        let mut names: Vec<String> = vec![self.stack[0].doc.metadata.title.clone()];
                        for f in &self.stack {
                            names.push(
                                self.root_doc()
                                    .components
                                    .iter()
                                    .find(|c| c.id == f.comp_id)
                                    .map_or(f.comp_id.clone(), |c| c.name.clone()),
                            );
                        }
                        ui.label(RichText::new(names.join("  ›  ")).color(c::TEXT_DIM));
                        ui.label(
                            RichText::new("component schematic")
                                .color(c::ACTIVE_TAB_TEXT)
                                .small(),
                        );
                    });
                });
            });
        if back {
            self.exit_component();
        }
    }

    /// The component editor window.
    pub fn component_editor(&mut self, ctx: &egui::Context) {
        let Some(mut ed) = self.comp_editor.take() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut enter = false;
        egui::Window::new(
            RichText::new(format!("Component — {}", ed.comp.name)).font(theme::bold(14.0)),
        )
        .id(egui::Id::unique("component-editor"))
        .open(&mut open)
        .default_size(vec2(760.0, 560.0))
        .min_size(vec2(600.0, 440.0))
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            if let Some(n) = &ed.note {
                ui.label(RichText::new(n).color(c::WARN).small());
            }
            ui.horizontal(|ui| {
                for (t, label) in [
                    (Tab::General, "General"),
                    (Tab::Symbol, "Symbol"),
                    (Tab::Implementation, "Implementation"),
                ] {
                    if ui.selectable_label(ed.tab == t, label).clicked() {
                        ed.tab = t;
                    }
                }
            });
            ui.separator();
            let footer = 40.0;
            let body = ui.available_height() - footer;
            ui.allocate_ui(vec2(ui.available_width(), body.max(200.0)), |ui| {
                match ed.tab {
                    Tab::General => self.comp_general(ui, &mut ed),
                    Tab::Symbol => comp_symbol(ui, &mut ed),
                    Tab::Implementation => {
                        if self.comp_implementation(ui, &mut ed) {
                            enter = true;
                        }
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if let Some(r) = &ed.check {
                    match r {
                        Ok(m) => ui.label(RichText::new(m).color(c::OK).small()),
                        Err(e) => ui.label(RichText::new(e).color(c::DANGER).small()),
                    };
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("Apply").strong()).clicked() {
                        apply = true;
                    }
                    if ui
                        .button("Check")
                        .on_hover_text("Compile the component without applying it")
                        .clicked()
                    {
                        ed.check = Some(self.check_component(&ed.comp));
                    }
                    if ed.part.is_some() && ed.original.is_none() {
                        ui.checkbox(&mut ed.apply_all, "Use for every part of the same type");
                    }
                });
            });
        });
        if apply || enter {
            self.apply_component(&mut ed);
            ed.check = Some(self.check_component(&ed.comp));
            if enter
                && let Some(id) = ed.part
                && let Some(p) = self.doc.part(id)
            {
                let refdes = p.refdes.clone();
                self.sync_sim(false);
                self.enter_component(&refdes);
                return;
            }
        }
        if open {
            self.comp_editor = Some(ed);
        }
    }

    fn check_component(&self, comp: &LocalComponent) -> Result<String, String> {
        // As Apply would: new pins get their port labels inside.
        let mut comp = comp.clone();
        if let Implementation::Schematic { schematic } = &mut comp.implementation {
            port_labels(schematic, &comp.symbol);
        }
        let comp = &comp;
        let mut s = comp.symbol.clone();
        s.id = components::symbol_id(&comp.id);
        if !symbols::exists(&s.id) || symbols::get(&s.id) != &s {
            symbols::register(s);
        }
        components::compile(&self.cat, comp, self.root_dir().as_deref()).map(|d| {
            format!(
                "OK: {} pins, {} parameters{}",
                d.pins.len(),
                d.params.len(),
                if matches!(d.kind, ecs_model::DefKind::Subcircuit(_)) {
                    ", subcircuit"
                } else {
                    ""
                }
            )
        })
    }

    fn apply_component(&mut self, ed: &mut CompEditor) {
        fit_bbox(&mut ed.comp.symbol);
        // New pins of a schematic implementation get their port labels inside.
        if let Implementation::Schematic { schematic } = &mut ed.comp.implementation {
            port_labels(schematic, &ed.comp.symbol);
        }
        ed.comp.symbol.id = components::symbol_id(&ed.comp.id);
        let first = ed.original.is_none();
        self.edit_root(Command::SetComponent(Box::new(ed.comp.clone())));
        let sym = components::symbol_id(&ed.comp.id);
        if first {
            if let Some(pid) = ed.part {
                let parts: Vec<Id> = match (&ed.from_def, ed.apply_all) {
                    (Some(from), true) => self
                        .doc
                        .parts
                        .iter()
                        .filter(|p| &p.def == from)
                        .map(|p| p.id)
                        .collect(),
                    _ => vec![pid],
                };
                for p in parts {
                    self.edit(Command::SetPartDef {
                        part: p,
                        def: ed.comp.id.clone(),
                        symbol: sym.clone(),
                    });
                }
            } else {
                self.tool = Tool::Place {
                    def: ed.comp.id.clone(),
                    rot: 0,
                    mirror: false,
                };
                self.status = format!("Click to place {}", ed.comp.name);
            }
        }
        ed.original = Some(ed.comp.clone());
        ed.note = None;
        self.local_cache = None;
        self.rev += 1;
    }

    fn comp_general(&mut self, ui: &mut Ui, ed: &mut CompEditor) {
        egui::Grid::new("comp-general")
            .num_columns(2)
            .spacing(vec2(10.0, 6.0))
            .show(ui, |ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut ed.comp.name).desired_width(260.0));
                ui.end_row();
                ui.label("Prefix");
                ui.add(egui::TextEdit::singleline(&mut ed.comp.prefix).desired_width(60.0))
                    .on_hover_text("Reference designator prefix: R, U, X...");
                ui.end_row();
                ui.label("Category");
                ui.add(egui::TextEdit::singleline(&mut ed.comp.category).desired_width(160.0));
                ui.end_row();
                ui.label("Id");
                ui.label(
                    RichText::new(&ed.comp.id)
                        .font(theme::mono(11.5))
                        .color(c::TEXT_DIM),
                );
                ui.end_row();
                ui.label("Description");
                ui.add(
                    egui::TextEdit::multiline(&mut ed.comp.description)
                        .desired_rows(2)
                        .desired_width(420.0),
                );
                ui.end_row();
                ui.label("Ports");
                ui.label(
                    RichText::new(
                        ed.comp
                            .symbol
                            .pins
                            .iter()
                            .map(|p| p.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                    .font(theme::mono(11.5)),
                )
                .on_hover_text("Edit pins on the Symbol tab");
                ui.end_row();
            });
        ui.add_space(8.0);
        ui.label(RichText::new("Parameters").font(theme::bold(12.5)));
        ui.label(RichText::new("Used by a schematic implementation: write {name} (or any {expression}) in an inner part's value. Script components declare their own.").color(c::TEXT_DIM).small());
        let mut remove = None;
        egui::Grid::new("comp-params")
            .num_columns(5)
            .spacing(vec2(8.0, 4.0))
            .show(ui, |ui| {
                for h in ["Name", "Default", "Unit", "Description", ""] {
                    ui.label(RichText::new(h).color(c::TEXT_FAINT).small());
                }
                ui.end_row();
                for (k, p) in ed.comp.params.iter_mut().enumerate() {
                    ui.add_sized([80.0, 22.0], egui::TextEdit::singleline(&mut p.name));
                    ui.add_sized([80.0, 22.0], egui::TextEdit::singleline(&mut p.default));
                    ui.add_sized([50.0, 22.0], egui::TextEdit::singleline(&mut p.unit));
                    ui.add_sized(
                        [220.0, 22.0],
                        egui::TextEdit::singleline(&mut p.description),
                    );
                    if ui.small_button("×").clicked() {
                        remove = Some(k);
                    }
                    ui.end_row();
                }
            });
        if let Some(k) = remove {
            ed.comp.params.remove(k);
        }
        if ui.button("+ Parameter").clicked() {
            let n = ed.comp.params.len() + 1;
            ed.comp.params.push(LocalParam {
                name: format!("p{n}"),
                default: "1".into(),
                unit: String::new(),
                description: String::new(),
            });
        }
    }

    /// Returns true when the user asks to open the inner schematic.
    fn comp_implementation(&mut self, ui: &mut Ui, ed: &mut CompEditor) -> bool {
        let kind = match &ed.comp.implementation {
            Implementation::Schematic { .. } => 0,
            Implementation::ExternalSchematic { .. } => 1,
            Implementation::Script { .. } => 2,
            Implementation::ExternalScript { .. } => 3,
        };
        let mut pick = kind;
        ui.horizontal(|ui| {
            ui.radio_value(&mut pick, 0, "Schematic");
            ui.radio_value(&mut pick, 1, "External schematic");
            ui.radio_value(&mut pick, 2, "JavaScript");
            ui.radio_value(&mut pick, 3, "External JavaScript");
        });
        if pick != kind {
            // Keep what was there to switch back without losing it.
            match std::mem::replace(
                &mut ed.comp.implementation,
                Implementation::Script {
                    source: String::new(),
                },
            ) {
                Implementation::Schematic { schematic } => ed.stash_sch = Some(schematic),
                Implementation::Script { source } => ed.stash_js = source,
                Implementation::ExternalSchematic { path }
                | Implementation::ExternalScript { path } => ed.stash_path = path,
            }
            ed.comp.implementation = match pick {
                0 => Implementation::Schematic {
                    schematic: ed.stash_sch.take().unwrap_or_else(|| {
                        let mut s = Schematic::default();
                        port_labels(&mut s, &ed.comp.symbol);
                        Box::new(s)
                    }),
                },
                1 => Implementation::ExternalSchematic {
                    path: ed.stash_path.clone(),
                },
                2 => Implementation::Script {
                    source: ed.stash_js.clone(),
                },
                _ => Implementation::ExternalScript {
                    path: ed.stash_path.clone(),
                },
            };
            ed.check = None;
        }
        ui.add_space(6.0);
        let mut enter = false;
        let base = self.root_dir();
        match &mut ed.comp.implementation {
            Implementation::Schematic { schematic } => {
                ui.label(format!(
                    "{} parts, {} wires, {} labels inside.",
                    schematic.parts.len(),
                    schematic.wires.len(),
                    schematic.labels.len()
                ));
                ui.label(RichText::new("Net labels named like the symbol's pins are the ports. Ground inside is the global ground.").color(c::TEXT_DIM).small());
                if ed.part.is_some()
                    && ui
                        .button("Open the schematic…")
                        .on_hover_text("Apply and edit the inner schematic on the canvas")
                        .clicked()
                {
                    enter = true;
                }
                if ed.part.is_none() {
                    ui.label(RichText::new("Place the component, then use Enter Component on it to draw its schematic.").color(c::TEXT_DIM).small());
                }
            }
            Implementation::ExternalSchematic { path }
            | Implementation::ExternalScript { path } => {
                let js = kind == 3 || pick == 3;
                ui.horizontal(|ui| {
                    ui.label("File");
                    ui.add(
                        egui::TextEdit::singleline(path)
                            .desired_width(380.0)
                            .hint_text(if js { "component.js" } else { "block.ecs" }),
                    );
                    if ui.button("Browse…").clicked() {
                        let mut dlg = rfd::FileDialog::new();
                        dlg = if js {
                            dlg.add_filter("JavaScript component", &["js"])
                        } else {
                            dlg.add_filter("Electrocraft schematic", &["ecs"])
                        };
                        if let Some(d) = &base {
                            dlg = dlg.set_directory(d);
                        }
                        if let Some(p) = dlg.pick_file() {
                            *path = match &base {
                                Some(b) => p
                                    .strip_prefix(b)
                                    .map(|r| r.display().to_string())
                                    .unwrap_or_else(|_| p.display().to_string()),
                                None => p.display().to_string(),
                            };
                        }
                    }
                });
                ui.label(RichText::new(if js { "The file is read whenever the schematic is compiled; its pins must be on the symbol." } else { "Net labels named like the symbol's pins are the ports. Relative paths start at this schematic's folder." }).color(c::TEXT_DIM).small());
                if !js && ed.part.is_some() && ui.button("Open the schematic…").clicked() {
                    enter = true;
                }
            }
            Implementation::Script { source } => {
                ui.horizontal(|ui| {
                    if ui.button("Template").on_hover_text("Replace with a starting point using the symbol's pins").clicked() {
                        let pins: Vec<String> = ed.comp.symbol.pins.iter().map(|p| p.name.clone()).collect();
                        *source = components::template_js(&ed.comp.id, &ed.comp.name, &pins);
                    }
                    ui.label(RichText::new("component({...}) with pins, params and equations(c): c.current, c.voltage, c.let, c.probe (spec 6).").color(c::TEXT_DIM).small());
                });
                egui::ScrollArea::vertical()
                    .id_salt("js-src")
                    .max_height(ui.available_height() - 4.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(source)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(22)
                                .font(theme::mono(12.5)),
                        );
                    });
            }
        }
        enter
    }
}

/// Symbol tab: drawing canvas, tools and the pin list.
fn comp_symbol(ui: &mut Ui, ed: &mut CompEditor) {
    ui.horizontal(|ui| {
        for (t, label, tip) in [
            (
                SymTool::Select,
                "Select",
                "Click to select, drag to move, Del to delete",
            ),
            (SymTool::Line, "Line", "Drag"),
            (SymTool::Rect, "Rect", "Drag"),
            (SymTool::Circle, "Circle", "Drag from the centre"),
            (
                SymTool::Polyline,
                "Polyline",
                "Click points; double-click or right-click to finish",
            ),
            (SymTool::Text, "Text", "Click to place the text below"),
            (SymTool::Pin, "Pin", "Click on the 20-unit grid"),
        ] {
            if ui
                .selectable_label(ed.tool == t, label)
                .on_hover_text(tip)
                .clicked()
            {
                ed.tool = t;
                ed.poly.clear();
            }
        }
        ui.separator();
        match ed.tool {
            SymTool::Text => {
                ui.add(egui::TextEdit::singleline(&mut ed.text).desired_width(90.0));
            }
            SymTool::Pin => {
                let hint = ed.next_pin_name();
                ui.add(
                    egui::TextEdit::singleline(&mut ed.pin_name)
                        .desired_width(70.0)
                        .hint_text(hint),
                );
            }
            _ => {}
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Start from").color(c::TEXT_DIM));
        let mut ids: Vec<String> = symbols::library().keys().map(|k| k.to_string()).collect();
        ids.sort();
        egui::ComboBox::from_id_salt("sym-base")
            .width(150.0)
            .selected_text(if ed.base.is_empty() {
                "existing symbol…".to_string()
            } else {
                ed.base.clone()
            })
            .show_ui(ui, |ui| {
                for id in &ids {
                    ui.selectable_value(&mut ed.base, id.clone(), id);
                }
            });
        if !ed.base.is_empty()
            && ui
                .button("Use")
                .on_hover_text("Replace the drawing and pins with this symbol's")
                .clicked()
        {
            let s = symbols::get(&ed.base).clone();
            ed.comp.symbol.prims = s.prims.iter().chain(&s.prims_off).cloned().collect();
            ed.comp.symbol.pins = s.pins.clone();
            ed.comp.symbol.glow = s.glow;
            ed.comp.symbol.lit = s.lit.clone();
            ed.selected = None;
        }
        if !ed.base.is_empty()
            && ui
                .button("Add drawing")
                .on_hover_text("Add this symbol's lines to the drawing, keeping the pins")
                .clicked()
        {
            let s = symbols::get(&ed.base);
            ed.comp
                .symbol
                .prims
                .extend(s.prims.iter().chain(&s.prims_off).cloned());
        }
        if ui
            .button("Box")
            .on_hover_text("A box with the current pins on its sides")
            .clicked()
        {
            let names: Vec<String> = ed.comp.symbol.pins.iter().map(|p| p.name.clone()).collect();
            let half = names.len().div_ceil(2);
            let l: Vec<&str> = names[..half].iter().map(String::as_str).collect();
            let r: Vec<&str> = names[half..].iter().map(String::as_str).collect();
            let title: String = ed.comp.name.chars().take(10).collect();
            let s = symbols::boxed("tmp", &title, &l, &r);
            ed.comp.symbol.prims = s.prims;
            ed.comp.symbol.pins = s.pins;
        }
        if ui.button("Clear").clicked() {
            ed.comp.symbol.prims.clear();
            ed.selected = None;
        }
    });
    let avail = ui.available_size();
    let side = 200.0;
    ui.horizontal(|ui| {
        let (rect, resp) = ui.allocate_exact_size(
            vec2(
                (avail.x - side - 12.0).max(200.0),
                (avail.y - 6.0).max(200.0),
            ),
            Sense::click_and_drag(),
        );
        symbol_canvas(ui, ed, rect, &resp);
        ui.vertical(|ui| {
            ui.set_width(side);
            ui.label(RichText::new("Pins").font(theme::bold(12.5)));
            let mut remove = None;
            egui::ScrollArea::vertical()
                .id_salt("sym-pins")
                .max_height(avail.y - 80.0)
                .show(ui, |ui| {
                    for (k, p) in ed.comp.symbol.pins.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let sel = ed.selected == Some(Sel::Pin(k));
                            if ui
                                .selectable_label(sel, format!("{:>4},{:>4}", p.at[0], p.at[1]))
                                .clicked()
                            {
                                ed.selected = Some(Sel::Pin(k));
                            }
                            ui.add_sized([80.0, 22.0], egui::TextEdit::singleline(&mut p.name));
                            if ui.small_button("×").clicked() {
                                remove = Some(k);
                            }
                        });
                    }
                });
            if let Some(k) = remove {
                ed.comp.symbol.pins.remove(k);
                ed.selected = None;
            }
            ui.label(
                RichText::new(
                    "Pins sit on the 20-unit grid; their names are the component's ports.",
                )
                .color(c::TEXT_DIM)
                .small(),
            );
        });
    });
}

fn symbol_canvas(ui: &mut Ui, ed: &mut CompEditor, rect: Rect, resp: &egui::Response) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, c::CANVAS);
    let span = [200.0f32, 160.0f32];
    let scale = (rect.width() / span[0]).min(rect.height() / span[1]);
    let ctr = rect.center();
    let to = |p: P2| pos2(ctr.x + p[0] as f32 * scale, ctr.y + p[1] as f32 * scale);
    let from = |q: Pos2| -> P2 {
        [
            ((q.x - ctr.x) / scale) as f64,
            ((q.y - ctr.y) / scale) as f64,
        ]
    };
    // Grid: 10-unit minor, 20-unit pin grid.
    let nx = (rect.width() / scale / 20.0) as i32 / 2 + 1;
    let ny = (rect.height() / scale / 20.0) as i32 / 2 + 1;
    for i in -nx..=nx {
        let x = to([i as f64 * GRID, 0.0]).x;
        painter.line_segment(
            [pos2(x, rect.min.y), pos2(x, rect.max.y)],
            Stroke::new(1.0, if i == 0 { c::GRID_MAJOR } else { c::GRID_MINOR }),
        );
    }
    for j in -ny..=ny {
        let y = to([0.0, j as f64 * GRID]).y;
        painter.line_segment(
            [pos2(rect.min.x, y), pos2(rect.max.x, y)],
            Stroke::new(1.0, if j == 0 { c::GRID_MAJOR } else { c::GRID_MINOR }),
        );
    }
    let sym = &ed.comp.symbol;
    draw_prims(&painter, &sym.prims, &to, scale, c::INK, 1.6);
    if let Some(Sel::Prim(k)) = ed.selected
        && let Some(p) = sym.prims.get(k)
    {
        draw_prims(
            &painter,
            std::slice::from_ref(p),
            &to,
            scale,
            c::SELECT,
            2.6,
        );
    }
    for (k, p) in sym.pins.iter().enumerate() {
        let col = if ed.selected == Some(Sel::Pin(k)) {
            c::SELECT
        } else {
            c::PIN
        };
        painter.circle_filled(to(p.at), 4.0, col);
        painter.text(
            to(p.at) + vec2(0.0, -10.0),
            Align2::CENTER_BOTTOM,
            &p.name,
            theme::regular(11.0),
            col,
        );
    }
    let hover = resp.hover_pos().map(from);
    let fine = |p: P2| snap_to(p, 10.0 / 2.0);
    // Previews.
    if let (Some(a), Some(h)) = (ed.drag, hover) {
        let b = fine(h);
        let preview = match ed.tool {
            SymTool::Line => Some(Prim::Line { a, b }),
            SymTool::Rect => Some(Prim::Polyline {
                points: vec![a, [b[0], a[1]], b, [a[0], b[1]]],
                closed: true,
                fill: false,
            }),
            SymTool::Circle => Some(Prim::Circle {
                c: a,
                r: ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt(),
                fill: false,
            }),
            _ => None,
        };
        if let Some(p) = preview {
            draw_prims(&painter, &[p], &to, scale, c::ACCENT, 1.4);
        }
    }
    if !ed.poly.is_empty() {
        let mut pts = ed.poly.clone();
        if let Some(h) = hover {
            pts.push(fine(h));
        }
        draw_prims(
            &painter,
            &[Prim::Polyline {
                points: pts,
                closed: false,
                fill: false,
            }],
            &to,
            scale,
            c::ACCENT,
            1.4,
        );
    }
    if let Some(h) = hover {
        let snap = if ed.tool == SymTool::Pin {
            snap_to(h, GRID)
        } else {
            fine(h)
        };
        painter.circle_stroke(to(snap), 3.0, Stroke::new(1.0, c::TEXT_DIM));
        painter.text(
            rect.max - vec2(6.0, 6.0),
            Align2::RIGHT_BOTTOM,
            format!("{}, {}", snap[0], snap[1]),
            theme::mono(10.5),
            c::TEXT_DIM,
        );
    }
    // Input.
    let press = resp
        .drag_started()
        .then(|| resp.interact_pointer_pos())
        .flatten()
        .map(from);
    match ed.tool {
        SymTool::Select => {
            if resp.clicked()
                && let Some(p) = hover
            {
                let pin = ed.comp.symbol.pins.iter().position(|q| {
                    ((q.at[0] - p[0]).powi(2) + (q.at[1] - p[1]).powi(2)).sqrt() < 6.0
                });
                ed.selected = match pin {
                    Some(k) => Some(Sel::Pin(k)),
                    None => ed
                        .comp
                        .symbol
                        .prims
                        .iter()
                        .enumerate()
                        .map(|(k, q)| (k, prim_distance(q, p)))
                        .filter(|(_, d)| *d < 5.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(k, _)| Sel::Prim(k)),
                };
            }
            if let Some(p) = press {
                ed.drag = Some(fine(p));
            }
            if resp.dragged()
                && let (Some(a), Some(h)) = (ed.drag, hover)
            {
                let step = if matches!(ed.selected, Some(Sel::Pin(_))) {
                    GRID
                } else {
                    5.0
                };
                let b = snap_to(h, step);
                let d = [b[0] - snap_to(a, step)[0], b[1] - snap_to(a, step)[1]];
                if d != [0.0, 0.0] {
                    match ed.selected {
                        Some(Sel::Prim(k)) => {
                            if let Some(q) = ed.comp.symbol.prims.get_mut(k) {
                                move_prim(q, d);
                            }
                        }
                        Some(Sel::Pin(k)) => {
                            if let Some(q) = ed.comp.symbol.pins.get_mut(k) {
                                q.at = snap_to([q.at[0] + d[0], q.at[1] + d[1]], GRID);
                            }
                        }
                        None => {}
                    }
                    ed.drag = Some(b);
                }
            }
            if resp.drag_stopped() {
                ed.drag = None;
            }
            if resp.has_focus() || resp.hovered() {
                let del = ui.input(|i| {
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)
                });
                if del {
                    match ed.selected.take() {
                        Some(Sel::Prim(k)) if k < ed.comp.symbol.prims.len() => {
                            ed.comp.symbol.prims.remove(k);
                        }
                        Some(Sel::Pin(k)) if k < ed.comp.symbol.pins.len() => {
                            ed.comp.symbol.pins.remove(k);
                        }
                        _ => {}
                    }
                }
            }
        }
        SymTool::Line | SymTool::Rect | SymTool::Circle => {
            if let Some(p) = press {
                ed.drag = Some(fine(p));
            }
            if resp.drag_stopped()
                && let (Some(a), Some(h)) = (ed.drag.take(), hover)
            {
                let b = fine(h);
                if a != b {
                    ed.comp.symbol.prims.push(match ed.tool {
                        SymTool::Line => Prim::Line { a, b },
                        SymTool::Rect => Prim::Polyline {
                            points: vec![a, [b[0], a[1]], b, [a[0], b[1]]],
                            closed: true,
                            fill: false,
                        },
                        _ => Prim::Circle {
                            c: a,
                            r: ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt(),
                            fill: false,
                        },
                    });
                }
            }
        }
        SymTool::Polyline => {
            let finish = resp.double_clicked()
                || resp.secondary_clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Enter));
            if resp.clicked()
                && let Some(p) = hover
            {
                let q = fine(p);
                if ed.poly.last() != Some(&q) {
                    ed.poly.push(q);
                }
            }
            if finish && ed.poly.len() >= 2 {
                let mut pts = std::mem::take(&mut ed.poly);
                let closed = pts.len() > 2 && pts.first() == pts.last();
                if closed {
                    pts.pop();
                }
                ed.comp.symbol.prims.push(Prim::Polyline {
                    points: pts,
                    closed,
                    fill: false,
                });
            }
        }
        SymTool::Text => {
            if resp.clicked()
                && let Some(p) = hover
                && !ed.text.trim().is_empty()
            {
                ed.comp.symbol.prims.push(Prim::Text {
                    at: fine(p),
                    text: ed.text.clone(),
                    size: 10.0,
                });
            }
        }
        SymTool::Pin => {
            if resp.clicked()
                && let Some(p) = hover
            {
                let at = snap_to(p, GRID);
                if !ed.comp.symbol.pins.iter().any(|q| q.at == at) {
                    let name = ed.next_pin_name();
                    ed.comp.symbol.pins.push(SymPin { name, at });
                    ed.pin_name.clear();
                }
            }
        }
    }
}
