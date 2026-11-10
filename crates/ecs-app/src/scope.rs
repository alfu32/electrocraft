//! The oscilloscope (spec 9.5): channels are formulas over the codes of nets
//! and parts. Every terminal has a voltage and a current; parts add their
//! own probes. Formulas compile to tapes over the raw signals they need.

use crate::app::App;
use crate::catalog::{Built, Catalog};
use crate::theme::{self, c};
use ecs_expr::{Leaf, Resolved, Sym, Tape, TapeBuilder, Workspace};
use ecs_model::PinKind;
use ecs_schematic::{Command, Id, Item, Schematic, ScopeChannel};
use ecs_sim::live::{MAX_CHANNELS, ProbeTarget};
use egui::{Align2, Color32, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use std::collections::{HashMap, VecDeque};

/// One raw quantity the simulation thread samples.
#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    Node(usize),
    Branch(String, usize),
    Probe(String, usize),
}

/// A linear combination of raw terms.
type Terms = Vec<(Term, f64)>;

/// A named variable usable in scope formulas.
#[derive(Debug, Clone)]
pub struct Var {
    pub code: String,
    pub desc: String,
    pub unit: String,
    pub terms: Terms,
    /// Owner: a part or a net (for the inspector and canvas badges).
    pub owner: Owner,
    /// The terminal the quantity belongs to (for canvas badges).
    pub pin: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Owner {
    Part(Id),
    Net(String),
}

/// Every variable of the current circuit.
#[derive(Debug, Clone, Default)]
pub struct Vars {
    pub list: Vec<Var>,
    index: HashMap<String, usize>,
}

impl Vars {
    pub fn get(&self, code: &str) -> Option<&Var> {
        self.index.get(code).map(|&k| &self.list[k])
    }
    fn push(&mut self, v: Var) {
        self.index.insert(v.code.clone(), self.list.len());
        self.list.push(v);
    }
}

/// Build the variable table for a schematic.
pub fn variables(cat: &Catalog, doc: &Schematic, built: &Built) -> Vars {
    let mut vars = Vars::default();
    let node = |net: &str| -> Vec<(Term, f64)> {
        match built.nodes.get(net) {
            Some(&k) if k != 0 => vec![(Term::Node(k), 1.0)],
            _ => Vec::new(),
        }
    };
    let mut nets: Vec<&String> = built.nodes.keys().filter(|n| n.as_str() != "0").collect();
    nets.sort();
    for net in nets {
        // Internal nodes of flattened subcircuits are not nets of this sheet.
        if net.contains('.') {
            continue;
        }
        vars.push(Var {
            code: net.clone(),
            desc: "net voltage".into(),
            unit: "V".into(),
            terms: node(net),
            owner: Owner::Net(net.clone()),
            pin: None,
        });
    }
    for part in &doc.parts {
        let Some(def) = cat.def(&part.def) else {
            continue;
        };
        if def.kind == ecs_model::DefKind::Ground || !built.parts.contains_key(&part.refdes) {
            continue;
        }
        let Some(nets) = built.pins.get(&part.id) else {
            continue;
        };
        let sym_pins: Vec<String> = part.symbol().pins.iter().map(|p| p.name.clone()).collect();
        let r = &part.refdes;
        let owner = Owner::Part(part.id);
        let subckt = matches!(def.kind, ecs_model::DefKind::Subcircuit(_));
        // Digital pins get engine branches after the definition's own, in pin order.
        let mut digital_branch: HashMap<&str, usize> = HashMap::new();
        let mut k = def.branches.len();
        for p in &def.pins {
            if !matches!(p.kind, PinKind::Analog) {
                digital_branch.insert(p.name.as_str(), k);
                k += 1;
            }
        }
        let mut pin_terms: Vec<(Terms, Terms)> = Vec::new();
        for p in &def.pins {
            let net = sym_pins
                .iter()
                .position(|n| *n == p.name)
                .and_then(|i| nets.get(i))
                .cloned()
                .unwrap_or_default();
            let v = node(&net);
            let mut i = Vec::new();
            for (bi, b) in def.branches.iter().enumerate() {
                let (a, bb) = b.nodes();
                if a == p.name {
                    i.push((Term::Branch(r.clone(), bi), 1.0));
                }
                if bb == p.name {
                    i.push((Term::Branch(r.clone(), bi), -1.0));
                }
            }
            if let Some(&bi) = digital_branch.get(p.name.as_str()) {
                i.push((Term::Branch(r.clone(), bi), 1.0));
            }
            vars.push(Var {
                code: format!("{r}.{}.V", p.name),
                desc: format!("voltage at terminal {}", p.name),
                unit: "V".into(),
                terms: v.clone(),
                owner: owner.clone(),
                pin: Some(p.name.clone()),
            });
            if !subckt {
                vars.push(Var {
                    code: format!("{r}.{}.I", p.name),
                    desc: format!("current into terminal {}", p.name),
                    unit: "A".into(),
                    terms: i.clone(),
                    owner: owner.clone(),
                    pin: Some(p.name.clone()),
                });
            }
            pin_terms.push((v, i));
        }
        if def.pins.len() == 2 {
            let mut v = pin_terms[0].0.clone();
            v.extend(pin_terms[1].0.iter().map(|(t, k)| (t.clone(), -k)));
            vars.push(Var {
                code: format!("{r}.V"),
                desc: format!("voltage {} to {}", def.pins[0].name, def.pins[1].name),
                unit: "V".into(),
                terms: v,
                owner: owner.clone(),
                pin: None,
            });
            if !subckt {
                vars.push(Var {
                    code: format!("{r}.I"),
                    desc: format!("current through, entering {}", def.pins[0].name),
                    unit: "A".into(),
                    terms: pin_terms[0].1.clone(),
                    owner: owner.clone(),
                    pin: None,
                });
            }
        }
        for (k, p) in def.probes.iter().enumerate() {
            let code = format!("{r}.{}", p.name);
            if vars.get(&code).is_none() {
                vars.push(Var {
                    code,
                    desc: format!("probe {}", p.name),
                    unit: p.unit.clone(),
                    terms: vec![(Term::Probe(r.clone(), k), 1.0)],
                    owner: owner.clone(),
                    pin: None,
                });
            }
        }
    }
    vars
}

/// A compiled channel.
pub struct ChannelRt {
    pub name: String,
    pub expr: String,
    pub color: Color32,
    pub unit: String,
    pub tape: Option<Tape>,
    pub error: Option<String>,
    pub owner: Option<Owner>,
    pub pin: Option<String>,
}

/// Compile channels; returns them and the raw signals to sample.
pub fn compile(channels: &[ScopeChannel], vars: &Vars) -> (Vec<ChannelRt>, Vec<ProbeTarget>) {
    let mut signals: Vec<ProbeTarget> = Vec::new();
    let slot = |t: &Term, signals: &mut Vec<ProbeTarget>| -> Option<usize> {
        let target = match t {
            Term::Node(n) => ProbeTarget::Node(*n),
            Term::Branch(inst, b) => ProbeTarget::Current {
                inst: inst.clone(),
                branch: *b,
            },
            Term::Probe(inst, k) => ProbeTarget::Probe {
                inst: inst.clone(),
                index: *k,
            },
        };
        if let Some(k) = signals.iter().position(|s| *s == target) {
            return Some(k);
        }
        (signals.len() < MAX_CHANNELS).then(|| {
            signals.push(target);
            signals.len() - 1
        })
    };
    let mut out = Vec::new();
    for (k, ch) in channels.iter().enumerate() {
        let color = c::TRACES[k % c::TRACES.len()];
        let single = vars.get(ch.expr.trim());
        let unit = single.map(|v| v.unit.clone()).unwrap_or_default();
        let owner = single.map(|v| v.owner.clone());
        let pin = single.and_then(|v| v.pin.clone());
        let compiled = ecs_expr::parse(&ch.expr)
            .map_err(|e| e.to_string())
            .and_then(|e| {
                let mut b = TapeBuilder::new(0);
                let mut missing: Option<String> = None;
                let id = b
                    .add_expr(&e, &mut |s| match s {
                        Sym::Name("t") => Ok(Resolved::Leaf(Leaf::Live(0))),
                        Sym::Name(n) if n.starts_with("__s") => n[3..]
                            .parse()
                            .map(|k: usize| Resolved::Leaf(Leaf::Live(k + 1)))
                            .map_err(|_| "bad slot".to_string()),
                        Sym::Name(n) => {
                            let v = vars.get(n).ok_or_else(|| format!("unknown code {n}"))?;
                            let mut expr = ecs_expr::Expr::Num(0.0);
                            for (t, coef) in &v.terms {
                                match slot(t, &mut signals) {
                                    Some(k) => {
                                        expr = expr
                                            + ecs_expr::Expr::Num(*coef)
                                                * ecs_expr::Expr::Name(format!("__s{k}"))
                                    }
                                    None => missing = Some("too many signals on the scope".into()),
                                }
                            }
                            Ok(Resolved::Expr(expr))
                        }
                        _ => Err(
                            "use codes such as R1.A.V, R1.I or a net name instead of V() and I()"
                                .into(),
                        ),
                    })
                    .map_err(|e| e.0)?;
                if let Some(m) = missing {
                    return Err(m);
                }
                Ok(b.finish(vec![id]))
            });
        let (tape, error) = match compiled {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(e)),
        };
        out.push(ChannelRt {
            name: ch.name.clone(),
            expr: ch.expr.clone(),
            color,
            unit,
            tape,
            error,
            owner,
            pin,
        });
    }
    (out, signals)
}

/// Scope state.
#[derive(Default)]
pub struct Scope {
    pub generation: u32,
    pub rt: Vec<ChannelRt>,
    pub signals: Vec<ProbeTarget>,
    pub vars: Vars,
    /// Channel values per sample.
    pub samples: VecDeque<(f64, Vec<f64>)>,
    pub frozen: Vec<(f64, Vec<f64>)>,
    /// Seconds shown (0 = automatic, 4 × speed).
    pub window: f64,
    /// Edge trigger on channel 1: 0 off, 1 rising, 2 falling.
    pub trigger: u8,
    pub hold: bool,
    pub xy: bool,
    pub overlay: bool,
    pub floating: bool,
    pub cursors: [Option<f64>; 2],
    pub formula: String,
    pub show_codes: bool,
    pub code_filter: String,
    ws: Workspace,
    live: Vec<f64>,
}

impl Scope {
    /// Evaluate every channel for one raw sample.
    pub fn push(&mut self, time: f64, raw: &[f64]) {
        self.live.clear();
        self.live.push(time);
        self.live.extend_from_slice(raw);
        let mut vals = Vec::with_capacity(self.rt.len());
        for ch in &self.rt {
            vals.push(match &ch.tape {
                Some(t) => {
                    t.eval(&[], &self.live, &mut self.ws);
                    t.value(&self.ws, 0)
                }
                None => f64::NAN,
            });
        }
        if let Some((t, _)) = self.samples.back()
            && time < *t
        {
            self.samples.clear();
        }
        self.samples.push_back((time, vals));
    }
}

fn nice(x: f64) -> f64 {
    if x <= 0.0 || !x.is_finite() {
        return 1.0;
    }
    let e = 10f64.powf(x.log10().floor());
    let m = x / e;
    e * if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    }
}

impl App {
    /// Whether the dock panel or the floating window should show.
    fn scope_wanted(&self) -> bool {
        self.show_scopes && (!self.scope.rt.is_empty() || self.scopes_pinned)
    }

    /// Docked scope panel (bottom). Floating mode shows a window instead.
    pub fn scopes(&mut self, ui: &mut Ui) {
        if !self.scope_wanted() || self.scope.floating {
            return;
        }
        egui::Panel::bottom("scopes")
            .resizable(true)
            .default_size(340.0)
            .min_size(220.0)
            .frame(
                egui::Frame::new()
                    .fill(c::TITLEBAR)
                    .inner_margin(egui::Margin {
                        left: 6,
                        right: 6,
                        top: 4,
                        bottom: 2,
                    }),
            )
            .show(ui, |ui| {
                let r = ui.max_rect();
                self.scope_view(ui, r);
                // Claim the whole panel so it keeps its size.
                ui.expand_to_include_rect(r);
            });
    }

    /// Floating scope window: tall enough to read the Y axes.
    pub fn scope_window(&mut self, ctx: &egui::Context) {
        if !self.scope_wanted() || !self.scope.floating {
            return;
        }
        let mut open = true;
        egui::Window::new(RichText::new("Oscilloscope").font(theme::bold(13.5)))
            .id(egui::Id::unique("scope-window"))
            .open(&mut open)
            .default_size(vec2(820.0, 560.0))
            .pivot(Align2::RIGHT_BOTTOM)
            .default_pos(pos2(
                self.canvas_rect.max.x - 12.0,
                ctx.content_rect().max.y - 60.0,
            ))
            .min_size(vec2(480.0, 300.0))
            .resizable(true)
            .show(ctx, |ui| {
                let r = ui.available_rect_before_wrap();
                let r = Rect::from_min_size(r.min, vec2(r.width(), r.height().max(260.0)));
                ui.allocate_rect(r, Sense::hover());
                self.scope_view(ui, r);
            });
        if !open {
            self.scope.floating = false;
        }
    }

    fn scope_view(&mut self, ui: &mut Ui, full: Rect) {
        ui.painter().rect_filled(full, 7.0, c::PANEL);
        let legend = Rect::from_min_size(
            full.min + vec2(10.0, 8.0),
            vec2(230.0, full.height() - 16.0),
        );
        let area = Rect::from_min_max(
            pos2(legend.max.x + 10.0, full.min.y + 36.0),
            full.max - vec2(10.0, 20.0),
        );
        // Controls.
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(area.min.x, full.min.y + 6.0),
                    pos2(area.max.x, full.min.y + 30.0),
                ))
                .layout(Layout::left_to_right(egui::Align::Center)),
        );
        bar.spacing_mut().item_spacing.x = 6.0;
        bar.label(RichText::new("Time/div").color(c::TEXT_DIM));
        let auto = (self.doc.simulation.speed * 4.0).max(1e-6) / 10.0;
        let cur = if self.scope.window > 0.0 {
            self.scope.window / 10.0
        } else {
            0.0
        };
        let mut pick = cur;
        egui::ComboBox::from_id_salt("tdiv")
            .width(96.0)
            .selected_text(if cur == 0.0 {
                format!("Auto ({})", ecs_units::format(auto, "s"))
            } else {
                ecs_units::format(cur, "s")
            })
            .show_ui(&mut bar, |ui| {
                ui.selectable_value(&mut pick, 0.0, "Auto");
                for d in [
                    1e-6, 2e-6, 5e-6, 1e-5, 2e-5, 5e-5, 1e-4, 2e-4, 5e-4, 1e-3, 2e-3, 5e-3, 1e-2,
                    2e-2, 5e-2, 0.1, 0.2, 0.5, 1.0,
                ] {
                    ui.selectable_value(&mut pick, d, ecs_units::format(d, "s"));
                }
            });
        if pick != cur {
            self.scope.window = pick * 10.0;
        }
        bar.label(RichText::new("Trigger").color(c::TEXT_DIM));
        egui::ComboBox::from_id_salt("trig")
            .width(78.0)
            .selected_text(["Off", "CH1 ↑", "CH1 ↓"][self.scope.trigger as usize])
            .show_ui(&mut bar, |ui| {
                for (k, t) in ["Off", "CH1 ↑", "CH1 ↓"].iter().enumerate() {
                    ui.selectable_value(&mut self.scope.trigger, k as u8, *t);
                }
            });
        if bar.selectable_label(self.scope.hold, "Hold").clicked() {
            self.scope.hold = !self.scope.hold;
            self.scope.frozen = if self.scope.hold {
                self.scope.samples.iter().cloned().collect()
            } else {
                Vec::new()
            };
        }
        if bar
            .selectable_label(self.scope.overlay, "Overlay")
            .on_hover_text("All channels on one grid instead of a lane each")
            .clicked()
        {
            self.scope.overlay = !self.scope.overlay;
        }
        if bar
            .selectable_label(self.scope.xy, "XY")
            .on_hover_text("Channel 2 against channel 1")
            .clicked()
        {
            self.scope.xy = !self.scope.xy;
        }
        let float_label = if self.scope.floating { "Dock" } else { "Float" };
        if bar
            .button(float_label)
            .on_hover_text("Detach the scope into a resizable window")
            .clicked()
        {
            self.scope.floating = !self.scope.floating;
        }
        if bar.available_width() > 220.0 {
            bar.label(
                RichText::new("Click: cursor A · Right-click: cursor B")
                    .color(c::TEXT_FAINT)
                    .small(),
            );
        }

        // Legend with formulas.
        let mut legend_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(legend)
                .layout(Layout::top_down(egui::Align::Min)),
        );
        self.scope_legend(&mut legend_ui);

        let painter = ui.painter_at(full);
        if self.scope.rt.is_empty() {
            painter.text(area.center(), Align2::CENTER_CENTER, "Select a wire or part and tick quantities under Scope in the inspector,\nclick with the Probe tool (P), or type a formula on the left.", theme::regular(13.0), c::TEXT_DIM);
            return;
        }
        if area.height() < 40.0 || area.width() < 80.0 {
            return;
        }
        let samples: Vec<(f64, Vec<f64>)> = if self.scope.hold {
            self.scope.frozen.clone()
        } else {
            self.scope.samples.iter().cloned().collect()
        };
        let window = if self.scope.window > 0.0 {
            self.scope.window
        } else {
            auto * 10.0
        };
        let t_end = samples.last().map_or(0.0, |s| s.0);
        let mut t0 = t_end - window;
        if self.scope.trigger > 0 && samples.len() > 2 {
            let (lo, hi) = samples
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), s| {
                    (a.min(s.1[0]), b.max(s.1[0]))
                });
            let level = 0.5 * (lo + hi);
            let rising = self.scope.trigger == 1;
            if let Some(w) = samples.windows(2).rev().find(|w| {
                let (a, b) = (w[0].1[0], w[1].1[0]);
                w[1].0 <= t_end - window * 0.9
                    && if rising {
                        a < level && b >= level
                    } else {
                        a > level && b <= level
                    }
            }) {
                t0 = w[1].0 - window * 0.1;
            }
        }
        let visible: Vec<&(f64, Vec<f64>)> = samples
            .iter()
            .filter(|(t, _)| *t >= t0 && *t <= t0 + window)
            .collect();
        let n = self.scope.rt.len();
        // Y-axis label column on the left of the traces.
        let label_w = 70.0;
        let plot = Rect::from_min_max(pos2(area.min.x + label_w, area.min.y), area.max);
        painter.rect_filled(plot, 4.0, Color32::from_rgb(0x15, 0x19, 0x20));
        for k in 0..=10 {
            let x = plot.min.x + plot.width() * k as f32 / 10.0;
            painter.line_segment(
                [pos2(x, plot.min.y), pos2(x, plot.max.y)],
                Stroke::new(1.0, Color32::from_rgb(0x24, 0x2A, 0x34)),
            );
        }
        painter.text(
            pos2(plot.max.x, plot.max.y + 1.0),
            Align2::RIGHT_TOP,
            format!("{}/div", ecs_units::format(window / 10.0, "s")),
            theme::mono(10.5),
            c::TEXT_DIM,
        );
        if self.scope.xy && n >= 2 {
            let range = |k: usize| {
                visible
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), s| {
                        (a.min(s.1[k]), b.max(s.1[k]))
                    })
            };
            let ((x0, x1), (y0, y1)) = (range(0), range(1));
            let sx = (x1 - x0).max(1e-12);
            let sy = (y1 - y0).max(1e-12);
            let pts: Vec<Pos2> = visible
                .iter()
                .map(|s| {
                    pos2(
                        plot.min.x + ((s.1[0] - x0) / sx) as f32 * plot.width(),
                        plot.max.y - ((s.1[1] - y0) / sy) as f32 * plot.height(),
                    )
                })
                .collect();
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(
                    pts,
                    Stroke::new(1.6, self.scope.rt[1].color),
                ));
            }
            painter.text(
                pos2(area.min.x, plot.min.y),
                Align2::LEFT_TOP,
                ecs_units::format_digits(y1, &self.scope.rt[1].unit, 3),
                theme::mono(10.5),
                self.scope.rt[1].color,
            );
            painter.text(
                pos2(area.min.x, plot.max.y),
                Align2::LEFT_BOTTOM,
                ecs_units::format_digits(y0, &self.scope.rt[1].unit, 3),
                theme::mono(10.5),
                self.scope.rt[1].color,
            );
            return;
        }
        // Lanes: one per channel, or one shared lane in overlay mode.
        let lanes: Vec<Vec<usize>> = if self.scope.overlay {
            vec![(0..n).collect()]
        } else {
            (0..n).map(|k| vec![k]).collect()
        };
        let lane_h = plot.height() / lanes.len() as f32;
        let to_x = |t: f64| plot.min.x + ((t - t0) / window) as f32 * plot.width();
        for (li, members) in lanes.iter().enumerate() {
            let lane = Rect::from_min_size(
                pos2(plot.min.x, plot.min.y + lane_h * li as f32),
                vec2(plot.width(), lane_h),
            );
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for s in &visible {
                for &k in members {
                    let v = s.1[k];
                    if v.is_finite() {
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
            }
            if !lo.is_finite() {
                lo = -1.0;
                hi = 1.0;
            }
            if hi - lo < 1e-12 * (1.0 + hi.abs()) {
                let pad = if hi.abs() > 0.0 { hi.abs() * 0.1 } else { 1.0 };
                lo -= pad;
                hi += pad;
            }
            // Gridlines at a 1-2-5 step with labels in the label column.
            let divs = ((lane_h / 28.0).floor() as f64).clamp(2.0, 8.0);
            let step = nice((hi - lo) / divs);
            let (glo, ghi) = ((lo / step).floor() * step, (hi / step).ceil() * step);
            let to_y = |v: f64| {
                lane.max.y - 10.0 - ((v - glo) / (ghi - glo)) as f32 * (lane.height() - 20.0)
            };
            let unit = self.scope.rt[members[0]].unit.clone();
            let mut v = glo;
            let mut guard = 0;
            while v <= ghi + step * 0.5 && guard < 40 {
                let y = to_y(v);
                let strong = v.abs() < step * 1e-6;
                painter.line_segment(
                    [pos2(lane.min.x, y), pos2(lane.max.x, y)],
                    Stroke::new(
                        1.0,
                        if strong {
                            Color32::from_rgb(0x44, 0x4D, 0x5C)
                        } else {
                            Color32::from_rgb(0x24, 0x2A, 0x34)
                        },
                    ),
                );
                let edge = y < lane.min.y + 5.0 || y > lane.max.y - 5.0;
                if !edge || lanes.len() == 1 {
                    painter.text(
                        pos2(plot.min.x - 6.0, y),
                        Align2::RIGHT_CENTER,
                        ecs_units::format_digits(
                            if v.abs() < step * 1e-6 { 0.0 } else { v },
                            &unit,
                            3,
                        ),
                        theme::mono(10.5),
                        if members.len() == 1 {
                            self.scope.rt[members[0]].color
                        } else {
                            c::TEXT_DIM
                        },
                    );
                }
                v += step;
                guard += 1;
            }
            if li > 0 {
                painter.line_segment(
                    [pos2(area.min.x, lane.min.y), pos2(lane.max.x, lane.min.y)],
                    Stroke::new(1.0, c::PANEL_BORDER),
                );
            }
            for &k in members {
                let ch = &self.scope.rt[k];
                let stride = (visible.len() / (plot.width() as usize * 2).max(1)).max(1);
                let pts: Vec<Pos2> = visible
                    .iter()
                    .step_by(stride)
                    .filter(|s| s.1[k].is_finite())
                    .map(|s| pos2(to_x(s.0), to_y(s.1[k]).clamp(lane.min.y, lane.max.y)))
                    .collect();
                if pts.len() >= 2 {
                    painter.add(egui::Shape::line(pts, Stroke::new(1.6, ch.color)));
                }
                if members.len() == 1 {
                    painter.text(
                        lane.min + vec2(6.0, 4.0),
                        Align2::LEFT_TOP,
                        &ch.name,
                        theme::regular(11.5),
                        ch.color,
                    );
                }
            }
            // Cursor readouts for this lane.
            for (ci, cur) in self.scope.cursors.iter().enumerate() {
                let Some(dt) = cur else { continue };
                let t = t0 + dt;
                let near = visible
                    .iter()
                    .min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs()));
                if let Some(s) = near {
                    let text: Vec<String> = members
                        .iter()
                        .map(|&k| ecs_units::format_digits(s.1[k], &self.scope.rt[k].unit, 4))
                        .collect();
                    painter.text(
                        pos2(lane.max.x - 6.0, lane.min.y + 4.0 + 14.0 * ci as f32),
                        Align2::RIGHT_TOP,
                        format!("{}: {}", if ci == 0 { "A" } else { "B" }, text.join("  ")),
                        theme::mono(10.5),
                        if ci == 0 { c::WARN } else { c::OK },
                    );
                }
            }
        }
        let resp = ui.interact(plot, ui.scope_id().with("scope-plot"), Sense::click());
        if let Some(p) = resp.interact_pointer_pos() {
            let dt = ((p.x - plot.min.x) / plot.width()) as f64 * window;
            if resp.clicked() {
                self.scope.cursors[0] = Some(dt);
            }
            if resp.secondary_clicked() {
                self.scope.cursors[1] = Some(dt);
            }
        }
        for (k, cur) in self.scope.cursors.iter().enumerate() {
            if let Some(dt) = cur {
                let x = plot.min.x + (*dt / window) as f32 * plot.width();
                painter.line_segment(
                    [pos2(x, plot.min.y), pos2(x, plot.max.y)],
                    Stroke::new(1.0, if k == 0 { c::WARN } else { c::OK }),
                );
            }
        }
        if let [Some(a), Some(b)] = self.scope.cursors {
            let d = (b - a).abs();
            painter.text(
                pos2(plot.min.x, plot.max.y + 1.0),
                Align2::LEFT_TOP,
                format!(
                    "Δt = {}   1/Δt = {}",
                    ecs_units::format_digits(d, "s", 4),
                    if d > 0.0 {
                        ecs_units::format_digits(1.0 / d, "Hz", 4)
                    } else {
                        "—".into()
                    }
                ),
                theme::mono(10.5),
                c::TEXT,
            );
        }
    }

    fn scope_legend(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Oscilloscope").font(theme::bold(13.5)));
        let latest = self
            .scope
            .samples
            .back()
            .map(|s| s.1.clone())
            .unwrap_or_default();
        let mut remove = None;
        egui::ScrollArea::vertical()
            .id_salt("scope-legend")
            .max_height((ui.available_height() - 64.0).max(40.0))
            .show(ui, |ui| {
                for (k, ch) in self.scope.rt.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(12.0, 10.0), Sense::hover());
                        ui.painter().rect_filled(r, 2.0, ch.color);
                        ui.label(RichText::new(&ch.name).size(12.5));
                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("×")
                                .on_hover_text("Remove from the scope")
                                .clicked()
                            {
                                remove = Some(k);
                            }
                            let v = latest.get(k).copied().unwrap_or(f64::NAN);
                            ui.label(
                                RichText::new(if v.is_finite() {
                                    ecs_units::format_digits(v, &ch.unit, 3)
                                } else {
                                    "—".into()
                                })
                                .font(theme::mono(11.0))
                                .color(ch.color),
                            );
                        });
                    });
                    match &ch.error {
                        Some(e) => {
                            ui.label(RichText::new(e).color(c::DANGER).small());
                        }
                        None if ch.expr != ch.name => {
                            ui.label(
                                RichText::new(&ch.expr)
                                    .font(theme::mono(10.0))
                                    .color(c::TEXT_FAINT),
                            );
                        }
                        None => {}
                    }
                }
            });
        if let Some(k) = remove {
            let mut list = self.doc.scope.clone();
            if k < list.len() {
                list.remove(k);
                self.edit(Command::SetScope(list));
            }
        }
        ui.horizontal(|ui| {
            let codes: Vec<crate::code_editor::Item> = self
                .scope
                .vars
                .list
                .iter()
                .map(|v| {
                    crate::code_editor::Item::new(
                        &v.code,
                        crate::code_editor::Kind::Code,
                        &format!("{} ({})", v.desc, v.unit),
                    )
                })
                .collect();
            let resp = crate::code_editor::code_editor(
                ui,
                "scope-formula",
                &mut self.scope.formula,
                crate::code_editor::Lang::Expr,
                &codes,
                crate::code_editor::Opts {
                    single_line: true,
                    width: 150.0,
                    hint: "formula, e.g. R1.P or out - in",
                    help: false,
                    ..Default::default()
                },
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Add").clicked() || enter) && !self.scope.formula.trim().is_empty() {
                let expr = self.scope.formula.trim().to_string();
                self.scope_add(&expr, &expr);
                self.scope.formula.clear();
            }
        });
        if ui
            .selectable_label(self.scope.show_codes, "Codes…")
            .on_hover_text("Every code you can use in a formula")
            .clicked()
        {
            self.scope.show_codes = !self.scope.show_codes;
        }
        if self.scope.show_codes {
            let mut insert = None;
            egui::Window::new("Scope codes")
                .id(egui::Id::unique("scope-codes"))
                .default_size(vec2(360.0, 420.0))
                .show(ui.ctx(), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.scope.code_filter).hint_text("filter"),
                    );
                    ui.label(
                        RichText::new("Click a code to add it to the formula; t is time.")
                            .color(c::TEXT_DIM)
                            .small(),
                    );
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let f = self.scope.code_filter.to_lowercase();
                        for v in self
                            .scope
                            .vars
                            .list
                            .iter()
                            .filter(|v| f.is_empty() || v.code.to_lowercase().contains(&f))
                        {
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Label::new(
                                            RichText::new(&v.code)
                                                .font(theme::mono(12.0))
                                                .color(c::ACTIVE_TAB_TEXT),
                                        )
                                        .sense(Sense::click()),
                                    )
                                    .clicked()
                                {
                                    insert = Some(v.code.clone());
                                }
                                ui.label(RichText::new(&v.desc).color(c::TEXT_DIM).small());
                            });
                        }
                    });
                });
            if let Some(code) = insert {
                if !self.scope.formula.is_empty() && !self.scope.formula.ends_with(' ') {
                    self.scope.formula.push(' ');
                }
                self.scope.formula.push_str(&code);
            }
        }
    }

    /// Add a channel (one undo step).
    pub fn scope_add(&mut self, name: &str, expr: &str) {
        if self.doc.scope.iter().any(|c| c.expr == expr) {
            return;
        }
        let mut list = self.doc.scope.clone();
        list.push(ScopeChannel {
            name: name.into(),
            expr: expr.into(),
        });
        self.edit(Command::SetScope(list));
        self.show_scopes = true;
    }

    /// Remove every channel whose formula is `expr`.
    pub fn scope_remove(&mut self, expr: &str) {
        let list: Vec<ScopeChannel> = self
            .doc
            .scope
            .iter()
            .filter(|c| c.expr != expr)
            .cloned()
            .collect();
        if list.len() != self.doc.scope.len() {
            self.edit(Command::SetScope(list));
        }
    }

    /// Scope checkboxes for the selected part or wire (inspector section).
    pub fn scope_section(&mut self, ui: &mut Ui, item: Item) {
        let owner = match item {
            Item::Part(id) => Owner::Part(id),
            Item::Wire(id) => match self.built.wires.get(&id) {
                Some(n) => Owner::Net(n.clone()),
                None => return,
            },
            _ => return,
        };
        let vars: Vec<Var> = self
            .scope
            .vars
            .list
            .iter()
            .filter(|v| v.owner == owner)
            .cloned()
            .collect();
        if vars.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.separator();
        let code = match &owner {
            Owner::Part(id) => self
                .doc
                .part(*id)
                .map(|p| p.refdes.clone())
                .unwrap_or_default(),
            Owner::Net(n) => n.clone(),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("Scope").font(theme::bold(12.5)));
            ui.label(
                RichText::new(format!("code {code}"))
                    .font(theme::mono(11.0))
                    .color(c::ACTIVE_TAB_TEXT),
            )
            .on_hover_text("Use this code in scope formulas");
        });
        egui::Grid::new(("scope-checks", code.clone()))
            .num_columns(2)
            .spacing(vec2(8.0, 2.0))
            .show(ui, |ui| {
                for v in vars {
                    let mut on = self.doc.scope.iter().any(|c| c.expr == v.code);
                    if ui
                        .checkbox(&mut on, RichText::new(&v.code).font(theme::mono(11.5)))
                        .changed()
                    {
                        if on {
                            self.scope_add(&v.code, &v.code);
                        } else {
                            self.scope_remove(&v.code);
                        }
                    }
                    ui.label(RichText::new(&v.desc).color(c::TEXT_DIM).small());
                    ui.end_row();
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(c: &mut ecs_sim::Circuit, signals: &[ProbeTarget]) -> Vec<f64> {
        signals
            .iter()
            .map(|s| match s {
                ProbeTarget::Node(n) => c.voltage_of_netlist_node(*n),
                ProbeTarget::Diff(a, b) => {
                    c.voltage_of_netlist_node(*a) - c.voltage_of_netlist_node(*b)
                }
                ProbeTarget::Current { inst, branch } => {
                    let k = c.instance_index(inst).unwrap();
                    c.branch_currents(k)[*branch]
                }
                ProbeTarget::Probe { inst, index } => {
                    let k = c.instance_index(inst).unwrap();
                    c.probes(k)[*index]
                }
            })
            .collect()
    }

    #[test]
    fn every_code_of_every_sample_compiles() {
        let mut cat = Catalog::builtin();
        for s in crate::samples::all() {
            let doc = (s.build)(&cat);
            crate::components::compile_all(&mut cat, &doc, None);
            let built = crate::catalog::build(&cat, &doc);
            let vars = variables(&cat, &doc, &built);
            assert!(!vars.list.is_empty(), "{}", s.id);
            for ch in &doc.scope {
                assert!(
                    vars.get(&ch.expr).is_some() || ecs_expr::parse(&ch.expr).is_ok(),
                    "{}: {}",
                    s.id,
                    ch.expr
                );
            }
            let (rt, _) = compile(&doc.scope, &vars);
            for ch in &rt {
                assert!(ch.error.is_none(), "{}: {} {:?}", s.id, ch.expr, ch.error);
            }
            for v in vars.list.iter().take(MAX_CHANNELS) {
                let (rt, _) = compile(
                    &[ScopeChannel {
                        name: v.code.clone(),
                        expr: v.code.clone(),
                    }],
                    &vars,
                );
                assert!(
                    rt[0].error.is_none(),
                    "{}: {} {:?}",
                    s.id,
                    v.code,
                    rt[0].error
                );
            }
        }
    }

    #[test]
    fn codes_read_the_right_quantities() {
        let cat = Catalog::builtin();
        let doc = (crate::samples::all()
            .into_iter()
            .find(|s| s.id == "divider")
            .unwrap()
            .build)(&cat);
        let built = crate::catalog::build(&cat, &doc);
        let vars = variables(&cat, &doc, &built);
        let chans: Vec<ScopeChannel> =
            ["V1.V", "V1.I", "V1.P.V", "V1.P.I + V1.N.I", "2 * V1.V + t"]
                .iter()
                .map(|e| ScopeChannel {
                    name: e.to_string(),
                    expr: e.to_string(),
                })
                .collect();
        let (rt, signals) = compile(&chans, &vars);
        for ch in &rt {
            assert!(ch.error.is_none(), "{}: {:?}", ch.expr, ch.error);
        }
        let mut c = ecs_sim::Circuit::compile(&built.netlist).unwrap();
        c.dc_op().unwrap();
        let mut scope = Scope {
            rt,
            signals: signals.clone(),
            ..Default::default()
        };
        let r = raw(&mut c, &signals);
        scope.push(0.0, &r);
        let v = &scope.samples.back().unwrap().1;
        assert!((v[0] - 10.0).abs() < 1e-6, "{v:?}");
        // The source delivers 1 mA into 10 kΩ: current enters P negative.
        assert!((v[1] + 1e-3).abs() < 1e-6, "{v:?}");
        assert!((v[2] - 10.0).abs() < 1e-6, "{v:?}");
        assert!(v[3].abs() < 1e-9, "KCL {v:?}");
        assert!((v[4] - 20.0).abs() < 1e-6, "{v:?}");
        assert_eq!(scope.rt[0].unit, "V");
        assert_eq!(scope.rt[1].unit, "A");
    }

    #[test]
    fn unknown_codes_are_reported() {
        let (rt, _) = compile(
            &[ScopeChannel {
                name: "x".into(),
                expr: "R9.I".into(),
            }],
            &Vars::default(),
        );
        assert!(rt[0].error.as_deref().unwrap_or("").contains("R9.I"));
    }
}
