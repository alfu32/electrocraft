//! Builds the canvas [`DisplayList`] from the schematic and live state, and
//! paints a display list with egui when Raylib is unavailable (and for the
//! Start screen thumbnails).

use crate::catalog::{self, Catalog};
use crate::theme::c;
use ecs_render::{Align, DisplayList, Px, Rgba, Shape};
use ecs_schematic::{GRID, Id, Item, P2, Prim, Schematic};
use egui::{Color32, Pos2, Stroke, pos2};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// World ↔ screen mapping (screen in points relative to the canvas origin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Screen position (points) of world origin.
    pub offset: [f32; 2],
    /// Points per world unit.
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            offset: [400.0, 300.0],
            zoom: 1.0,
        }
    }
}

impl Viewport {
    pub fn screen(&self, p: P2) -> [f32; 2] {
        [
            p[0] as f32 * self.zoom + self.offset[0],
            p[1] as f32 * self.zoom + self.offset[1],
        ]
    }
    pub fn world(&self, s: [f32; 2]) -> P2 {
        [
            ((s[0] - self.offset[0]) / self.zoom) as f64,
            ((s[1] - self.offset[1]) / self.zoom) as f64,
        ]
    }
    /// Zoom by `factor` keeping the screen point `at` fixed (Wirecraft rule).
    pub fn zoom_at(&mut self, at: [f32; 2], factor: f32) {
        let w = self.world(at);
        self.zoom = (self.zoom * factor).clamp(0.1, 8.0);
        self.offset = [
            at[0] - w[0] as f32 * self.zoom,
            at[1] - w[1] as f32 * self.zoom,
        ];
    }
    /// Fit a world box into a screen of `size` points with a margin.
    pub fn fit(&mut self, b: [P2; 2], size: [f32; 2], margin: f32) {
        let w = ((b[1][0] - b[0][0]) as f32).max(GRID as f32);
        let h = ((b[1][1] - b[0][1]) as f32).max(GRID as f32);
        let z = ((size[0] - 2.0 * margin) / w)
            .min((size[1] - 2.0 * margin) / h)
            .clamp(0.1, 3.0);
        self.zoom = z;
        let cx = (b[0][0] + b[1][0]) as f32 / 2.0;
        let cy = (b[0][1] + b[1][1]) as f32 / 2.0;
        self.offset = [size[0] / 2.0 - cx * z, size[1] / 2.0 - cy * z];
    }
}

/// Live simulation values mapped onto the schematic.
#[derive(Debug, Clone, Default)]
pub struct Live {
    /// Voltage per net name.
    pub net_v: HashMap<String, f64>,
    /// Branch currents per part.
    pub part_i: HashMap<Id, Vec<f64>>,
    /// Probe values per part, in definition order.
    pub part_probe: HashMap<Id, Vec<f64>>,
    pub trouble: BTreeSet<Id>,
    /// Largest |V| for the colour scale.
    pub v_range: f64,
    pub running: bool,
}

/// Options for the scene.
#[derive(Debug, Clone)]
pub struct SceneOpts {
    pub grid: bool,
    pub volt_colors: bool,
    pub current_dots: bool,
    /// Seconds of animation time for the current dots.
    pub anim_t: f64,
    /// Current-dot speed factor (settings).
    pub dot_speed: f64,
    pub ppp: f32,
    pub hover: Option<Item>,
    pub hover_pin: Option<P2>,
    pub selection: BTreeSet<Item>,
    /// Window selection rectangle in world coords (start, end).
    pub window: Option<(P2, P2)>,
    /// Wire being drawn.
    pub wire_preview: Vec<P2>,
    /// Ghost part for placement.
    pub ghost: Option<ecs_schematic::Part>,
    /// DC operating point annotations: net voltage, part current.
    pub annotate: Option<(HashMap<String, f64>, HashMap<Id, f64>)>,
    /// Scope badges: anchor point, trace colour, channel number.
    pub probes: Vec<(P2, Color32, usize)>,
    /// Hide labels (thumbnails).
    pub thumbnail: bool,
}

impl Default for SceneOpts {
    fn default() -> Self {
        Self {
            grid: true,
            volt_colors: true,
            current_dots: true,
            anim_t: 0.0,
            dot_speed: crate::settings::DOT_SPEED,
            ppp: 1.0,
            hover: None,
            hover_pin: None,
            selection: BTreeSet::new(),
            window: None,
            wire_preview: Vec::new(),
            ghost: None,
            annotate: None,
            probes: Vec::new(),
            thumbnail: false,
        }
    }
}

fn rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    [r, g, b, a]
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Diverging voltage colour: green positive, grey at 0 V, red negative.
pub fn volt_color(v: f64, range: f64) -> Color32 {
    let t = (v / range.max(1e-9)).clamp(-1.0, 1.0) as f32;
    let zero = Color32::from_rgb(0x8A, 0x93, 0xA3);
    if t >= 0.0 {
        mix(zero, Color32::from_rgb(0x52, 0xE0, 0x7E), t.powf(0.7))
    } else {
        mix(zero, Color32::from_rgb(0xF0, 0x5A, 0x50), (-t).powf(0.7))
    }
}

fn led_color(index: f64) -> Color32 {
    match index as usize {
        1 => Color32::from_rgb(0x4C, 0xF0, 0x6A),
        2 => Color32::from_rgb(0x4C, 0x8C, 0xFF),
        3 => Color32::from_rgb(0xFF, 0xD8, 0x40),
        4 => Color32::from_rgb(0xF4, 0xF4, 0xFF),
        _ => Color32::from_rgb(0xFF, 0x40, 0x3A),
    }
}

/// Dots speed in px/s for a current: log scale so µA and A both move.
fn dot_speed(i: f64) -> f64 {
    let a = i.abs();
    if a < 1e-9 {
        return 0.0;
    }
    let decades = (a / 1e-9).log10(); // 0 at 1 nA, 9 at 1 A
    i.signum() * (8.0 + 14.0 * decades).min(160.0)
}

struct Out<'a> {
    list: &'a mut Vec<Shape>,
    vp: Viewport,
    ppp: f32,
}

impl Out<'_> {
    fn px(&self, p: P2) -> Px {
        let s = self.vp.screen(p);
        [s[0] * self.ppp, s[1] * self.ppp]
    }
    fn w(&self, world_width: f32) -> f32 {
        (world_width * self.vp.zoom * self.ppp).max(self.ppp * 0.75)
    }
    fn line(&mut self, a: P2, b: P2, width: f32, color: Color32) {
        let (a, b, width) = (self.px(a), self.px(b), self.w(width));
        self.list.push(Shape::Line {
            a,
            b,
            width,
            color: rgba(color),
        });
    }
    fn polyline(&mut self, pts: &[P2], width: f32, color: Color32, closed: bool) {
        let points = pts.iter().map(|p| self.px(*p)).collect();
        let width = self.w(width);
        self.list.push(Shape::Polyline {
            points,
            width,
            color: rgba(color),
            closed,
        });
    }
    fn polygon(&mut self, pts: &[P2], color: Color32) {
        let points = pts.iter().map(|p| self.px(*p)).collect();
        self.list.push(Shape::Polygon {
            points,
            color: rgba(color),
        });
    }
    fn circle(&mut self, centre: P2, r: f32, color: Color32, fill: bool, width: f32) {
        let c = self.px(centre);
        let r = r * self.vp.zoom * self.ppp;
        let width = self.w(width);
        self.list.push(Shape::Circle {
            c,
            r,
            color: rgba(color),
            fill,
            width,
        });
    }
    fn dot_px(&mut self, centre: P2, r_px: f32, color: Color32) {
        let c = self.px(centre);
        self.list.push(Shape::Circle {
            c,
            r: r_px * self.ppp,
            color: rgba(color),
            fill: true,
            width: 0.0,
        });
    }
    fn text(&mut self, at: P2, text: &str, size: f32, color: Color32, align: Align) {
        let size_px = size * self.vp.zoom.clamp(0.55, 2.5) * self.ppp;
        if size_px < 5.0 {
            return;
        }
        let at = self.px(at);
        self.list.push(Shape::Text {
            at,
            text: text.into(),
            size: size_px,
            color: rgba(color),
            align,
        });
    }
}

fn prim(
    out: &mut Out<'_>,
    part: &ecs_schematic::Part,
    p: &Prim,
    color: Color32,
    fill_color: Color32,
) {
    const W: f32 = 1.6;
    match p {
        Prim::Line { a, b } => out.line(part.to_world(*a), part.to_world(*b), W, color),
        Prim::Polyline {
            points,
            closed,
            fill,
        } => {
            let pts: Vec<P2> = points.iter().map(|q| part.to_world(*q)).collect();
            if *fill {
                out.polygon(&pts, fill_color);
            }
            out.polyline(&pts, W, color, *closed);
        }
        Prim::Circle { c: centre, r, fill } => {
            out.circle(
                part.to_world(*centre),
                *r as f32,
                if *fill { fill_color } else { color },
                *fill,
                W,
            );
        }
        Prim::Arc {
            c: centre,
            r,
            a0,
            a1,
        } => {
            let n = 12;
            let pts: Vec<P2> = (0..=n)
                .map(|k| {
                    let a = a0 + (a1 - a0) * k as f64 / n as f64;
                    part.to_world([centre[0] + r * a.cos(), centre[1] + r * a.sin()])
                })
                .collect();
            out.polyline(&pts, W, color, false);
        }
        Prim::Text { at, text, size } => {
            out.text(part.to_world(*at), text, *size as f32, color, Align::Center);
        }
    }
}

/// Current along wire pieces, from pin injections, solved on a spanning tree
/// of each net's wire graph (loops carry no flow). Pieces are wire segments
/// split at T points and pins: `(from, to, current from→to)`.
fn wire_flows(
    doc: &Schematic,
    built: &catalog::Built,
    live: &Live,
    cat: &Catalog,
) -> Vec<(P2, P2, f64)> {
    type K = (i64, i64);
    let key = |p: P2| -> K { ((p[0] * 8.0).round() as i64, (p[1] * 8.0).round() as i64) };
    // Current flowing from the net into each pin.
    let mut inject: Vec<(P2, String, f64)> = Vec::new();
    for part in &doc.parts {
        let (Some(def), Some(currents), Some(nets)) = (
            cat.def(&part.def),
            live.part_i.get(&part.id),
            built.pins.get(&part.id),
        ) else {
            continue;
        };
        for (k, (name, at)) in part.pins().iter().enumerate() {
            let mut into = 0.0;
            for (b, i) in def.branches.iter().zip(currents) {
                let (a, bb) = b.nodes();
                if a == name {
                    into += i;
                }
                if bb == name {
                    into -= i;
                }
            }
            if into != 0.0 {
                inject.push((*at, nets[k].clone(), into));
            }
        }
    }
    let mut by_net: BTreeMap<String, Vec<(P2, P2)>> = BTreeMap::new();
    for w in &doc.wires {
        if let Some(net) = built.wires.get(&w.id) {
            for s in w.points.windows(2) {
                by_net.entry(net.clone()).or_default().push((s[0], s[1]));
            }
        }
    }
    let mut pieces = Vec::new();
    for (net, segs) in by_net {
        let mut cuts: Vec<P2> = segs.iter().flat_map(|(a, b)| [*a, *b]).collect();
        cuts.extend(
            inject
                .iter()
                .filter(|(_, n, _)| *n == net)
                .map(|(p, _, _)| *p),
        );
        // Edges between consecutive cut points along each segment.
        let mut edges: Vec<(P2, P2)> = Vec::new();
        for (a, b) in &segs {
            let d = [b[0] - a[0], b[1] - a[1]];
            let mut pts: Vec<P2> = cuts
                .iter()
                .copied()
                .filter(|p| ecs_schematic::seg_distance(*p, *a, *b) < 1e-6)
                .collect();
            let t = |p: &P2| (p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1];
            pts.sort_by(|p, q| t(p).total_cmp(&t(q)));
            pts.dedup_by(|p, q| key(*p) == key(*q));
            edges.extend(pts.windows(2).map(|w| (w[0], w[1])));
        }
        let mut adj: HashMap<K, Vec<usize>> = HashMap::new();
        for (e, (a, b)) in edges.iter().enumerate() {
            adj.entry(key(*a)).or_default().push(e);
            adj.entry(key(*b)).or_default().push(e);
        }
        let mut source: HashMap<K, f64> = HashMap::new();
        for (at, n, i) in &inject {
            if *n == net {
                *source.entry(key(*at)).or_default() -= i;
            }
        }
        // BFS forest: parent edge per vertex, then accumulate leaves upward.
        let mut parent: HashMap<K, Option<usize>> = HashMap::new();
        let mut order: Vec<K> = Vec::new();
        let mut starts: Vec<K> = adj.keys().copied().collect();
        starts.sort();
        for s0 in starts {
            if parent.contains_key(&s0) {
                continue;
            }
            parent.insert(s0, None);
            let mut queue = std::collections::VecDeque::from([s0]);
            while let Some(v) = queue.pop_front() {
                order.push(v);
                for &e in &adj[&v] {
                    let (a, b) = edges[e];
                    let u = if key(a) == v { key(b) } else { key(a) };
                    if let std::collections::hash_map::Entry::Vacant(slot) = parent.entry(u) {
                        slot.insert(Some(e));
                        queue.push_back(u);
                    }
                }
            }
        }
        let mut subtotal: HashMap<K, f64> = HashMap::new();
        for v in order.iter().rev() {
            let flow =
                source.get(v).copied().unwrap_or(0.0) + subtotal.get(v).copied().unwrap_or(0.0);
            if let Some(Some(e)) = parent.get(v) {
                let (a, b) = edges[*e];
                // `flow` leaves v toward its parent.
                let (from, to) = if key(a) == *v { (a, b) } else { (b, a) };
                if flow.abs() > 0.0 {
                    pieces.push((from, to, flow));
                }
                *subtotal.entry(key(to)).or_default() += flow;
            }
        }
    }
    pieces
}

/// Build the display list for the canvas.
#[allow(clippy::too_many_arguments)]
pub fn build(
    doc: &Schematic,
    cat: &Catalog,
    built: &catalog::Built,
    live: &Live,
    vp: Viewport,
    size_px: [u32; 2],
    opts: &SceneOpts,
) -> DisplayList {
    let mut shapes = Vec::new();
    let mut out = Out {
        list: &mut shapes,
        vp,
        ppp: opts.ppp,
    };
    let size_pt = [size_px[0] as f32 / opts.ppp, size_px[1] as f32 / opts.ppp];
    // Grid: minor lines every grid, major every 5, faded out when dense.
    if opts.grid {
        let w0 = vp.world([0.0, 0.0]);
        let w1 = vp.world(size_pt);
        let step = GRID * if vp.zoom < 0.35 { 5.0 } else { 1.0 };
        let mut x = (w0[0] / step).floor() * step;
        while x <= w1[0] {
            let major = ((x / (GRID * 5.0)).round() * GRID * 5.0 - x).abs() < 1e-6;
            let col = if major { c::GRID_MAJOR } else { c::GRID_MINOR };
            out.line([x, w0[1]], [x, w1[1]], 0.0, col);
            x += step;
        }
        let mut y = (w0[1] / step).floor() * step;
        while y <= w1[1] {
            let major = ((y / (GRID * 5.0)).round() * GRID * 5.0 - y).abs() < 1e-6;
            let col = if major { c::GRID_MAJOR } else { c::GRID_MINOR };
            out.line([w0[0], y], [w1[0], y], 0.0, col);
            y += step;
        }
    }
    let selected = |i: Item| opts.selection.contains(&i);
    let hovered = |i: Item| opts.hover == Some(i);
    let sim_on = live.running || !live.net_v.is_empty();
    // Glows under everything else.
    for part in &doc.parts {
        let sym = part.symbol();
        let Some((centre, r)) = sym.glow else {
            continue;
        };
        let Some(def) = cat.def(&part.def) else {
            continue;
        };
        let Some(probes) = live.part_probe.get(&part.id) else {
            continue;
        };
        let Some(k) = def.probes.iter().position(|p| p.name == "brightness") else {
            continue;
        };
        let b = probes.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
        if b > 0.02 {
            let col = if def.id == "core.lamp" {
                Color32::from_rgb(0xFF, 0xD8, 0x70)
            } else if def.id == "core.logic_probe" {
                Color32::from_rgb(0x4C, 0xF0, 0x6A)
            } else {
                led_color(catalog::param_value(part, def, "color"))
            };
            let mut inner = col;
            inner = Color32::from_rgba_unmultiplied(
                inner.r(),
                inner.g(),
                inner.b(),
                (230.0 * b.sqrt()) as u8,
            );
            let cpx = out.px(part.to_world(centre));
            let rr = (r as f32 * (0.8 + 0.9 * b)) * vp.zoom * opts.ppp;
            out.list.push(Shape::Glow {
                c: cpx,
                r: rr,
                inner: rgba(inner),
            });
        }
    }
    // Wires.
    let flows = if opts.current_dots && sim_on {
        wire_flows(doc, built, live, cat)
    } else {
        Vec::new()
    };
    for w in &doc.wires {
        let item = Item::Wire(w.id);
        let net = built.wires.get(&w.id);
        let mut col = c::WIRE;
        if opts.volt_colors
            && sim_on
            && let Some(v) = net.and_then(|n| live.net_v.get(n))
        {
            col = volt_color(*v, live.v_range);
        }
        if selected(item) {
            out.polyline(&w.points, 6.0, c::SELECT.gamma_multiply(0.35), false);
            col = c::SELECT;
        } else if hovered(item) {
            out.polyline(&w.points, 5.0, Color32::WHITE.gamma_multiply(0.12), false);
        }
        out.polyline(&w.points, 2.0, col, false);
    }
    // Current dots along wires.
    for (from, to, i) in &flows {
        dots_along(
            &mut out,
            *from,
            *to,
            dot_speed(*i) * opts.dot_speed,
            opts.anim_t,
        );
    }
    // Junctions.
    for j in doc.junctions() {
        out.circle(j, 3.6, c::WIRE, true, 0.0);
    }
    // Parts.
    for part in &doc.parts {
        let item = Item::Part(part.id);
        let def = cat.def(&part.def);
        let sym = part.symbol();
        let ink = if selected(item) {
            c::SELECT
        } else if hovered(item) {
            Color32::WHITE
        } else {
            c::INK
        };
        let fill = if selected(item) {
            c::SELECT
        } else {
            Color32::from_rgb(0xB8, 0xC4, 0xD6)
        };
        if selected(item) {
            let [lo, hi] = part.bbox();
            let pad = 4.0;
            out.list.push(Shape::Rect {
                min: out.px([lo[0] - pad, lo[1] - pad]),
                max: out.px([hi[0] + pad, hi[1] + pad]),
                color: rgba(c::SELECT.gamma_multiply(0.18)),
                fill: true,
                width: 0.0,
            });
        }
        // Lit regions (7-segment display): bright when the probe is high.
        if !sym.lit.is_empty() {
            let vals = live.part_probe.get(&part.id);
            for (probe_name, poly) in &sym.lit {
                let on = def
                    .and_then(|d| d.probes.iter().position(|p| &p.name == probe_name))
                    .and_then(|k| vals.and_then(|v| v.get(k)))
                    .is_some_and(|v| *v > 0.5);
                let pts: Vec<P2> = poly.iter().map(|q| part.to_world(*q)).collect();
                if on {
                    let c0 = pts.iter().fold([0.0, 0.0], |a, p| {
                        [
                            a[0] + p[0] / pts.len() as f64,
                            a[1] + p[1] / pts.len() as f64,
                        ]
                    });
                    out.list.push(Shape::Glow {
                        c: out.px(c0),
                        r: 16.0 * vp.zoom * opts.ppp,
                        inner: rgba(Color32::from_rgba_unmultiplied(0xFF, 0x40, 0x30, 120)),
                    });
                }
                out.polygon(
                    &pts,
                    if on {
                        Color32::from_rgb(0xFF, 0x45, 0x35)
                    } else {
                        Color32::from_rgb(0x3A, 0x26, 0x26)
                    },
                );
            }
        }
        // Lead colouring by pin voltage: tint the symbol's pin stubs.
        for p in &sym.prims {
            prim(&mut out, part, p, ink, fill);
        }
        let state = part.states.values().next().copied().unwrap_or(0.0);
        let variant = if state > 0.5 {
            &sym.prims_on
        } else {
            &sym.prims_off
        };
        for p in variant {
            prim(&mut out, part, p, ink, fill);
        }
        // Pin dots on hover in wiring.
        if let Some(pp) = opts.hover_pin {
            for (_, at) in part.pins() {
                if (at[0] - pp[0]).abs() < 1e-6 && (at[1] - pp[1]).abs() < 1e-6 {
                    out.circle(at, 5.0, c::PIN, false, 1.6);
                }
            }
        }
        // Current dots through two-pin parts.
        if opts.current_dots
            && sim_on
            && let (Some(d), Some(cur)) = (def, live.part_i.get(&part.id))
            && part.pins().len() == 2
            && d.branches.len() == 1
            && let Some(&i) = cur.first()
        {
            let pins = part.pins();
            let (an, _) = d.branches[0].nodes();
            let (from, to) = if pins[0].0 == an {
                (pins[0].1, pins[1].1)
            } else {
                (pins[1].1, pins[0].1)
            };
            dots_along(
                &mut out,
                from,
                to,
                dot_speed(i) * opts.dot_speed,
                opts.anim_t,
            );
        }
        if opts.thumbnail {
            continue;
        }
        // Labels: refdes and value beside the body.
        let [lo, hi] = part.bbox();
        let horizontal = (hi[0] - lo[0]) >= (hi[1] - lo[1]);
        let value = def
            .map(|d| catalog::value_label(part, d))
            .unwrap_or_default();
        let is_ground = def.is_some_and(|d| d.kind == ecs_model::DefKind::Ground);
        if !is_ground {
            let (a1, a2, align) = if horizontal {
                let x = (lo[0] + hi[0]) / 2.0;
                ([x, lo[1] - 9.0], [x, hi[1] + 10.0], Align::Center)
            } else {
                let x = hi[0] + 6.0;
                let y = (lo[1] + hi[1]) / 2.0;
                ([x, y - 8.0], [x, y + 8.0], Align::Left)
            };
            out.text(
                a1,
                &part.refdes,
                11.0,
                Color32::from_rgb(0xC7, 0xCE, 0xDA),
                align,
            );
            if !value.is_empty() {
                out.text(a2, &value, 10.5, c::ACTIVE_TAB_TEXT, align);
            }
            // Live meter readings.
            if let (Some(d), Some(pv)) = (def, live.part_probe.get(&part.id))
                && (d.id == "core.voltmeter" || d.id == "core.ammeter")
                && let Some(v) = pv.first()
            {
                let unit = &d.probes[0].unit;
                let at = if horizontal {
                    [(lo[0] + hi[0]) / 2.0, hi[1] + 24.0]
                } else {
                    [hi[0] + 6.0, (lo[1] + hi[1]) / 2.0 + 24.0]
                };
                out.text(at, &ecs_units::format(*v, unit), 12.0, c::WARN, align);
            }
        }
        if live.trouble.contains(&part.id) {
            let at = [hi[0] + 4.0, lo[1] - 4.0];
            out.circle(at, 7.0, c::WARN, true, 0.0);
            out.text(at, "!", 11.0, c::TITLEBAR, Align::Center);
        }
    }
    // Net labels: tag shapes.
    for l in &doc.labels {
        let item = Item::Label(l.id);
        let col = if selected(item) {
            c::SELECT
        } else {
            Color32::from_rgb(0xE8, 0xC0, 0x6A)
        };
        let w = 10.0 + 7.0 * l.name.chars().count() as f64;
        let (x, y) = (l.at[0], l.at[1] - 10.0);
        out.line(l.at, [x, y + 4.0], 1.4, col);
        let tag = [
            [x, y - 7.0],
            [x + w, y - 7.0],
            [x + w + 6.0, y],
            [x + w, y + 7.0],
            [x, y + 7.0],
        ];
        out.polygon(&tag, Color32::from_rgb(0x3A, 0x34, 0x26));
        out.polyline(&tag, 1.3, col, true);
        out.text([x + 5.0, y], &l.name, 11.0, col, Align::Left);
        out.circle(l.at, 2.6, col, true, 0.0);
    }
    for n in &doc.notes {
        let col = if selected(Item::Note(n.id)) {
            c::SELECT
        } else {
            c::TEXT_DIM
        };
        out.text(n.at, &n.text, 12.0, col, Align::Left);
    }
    // Probe markers.
    for &(at, ref col, k) in &opts.probes {
        // Flag up and to the left, clear of net-label tags that point right.
        let flag = [
            [at[0], at[1]],
            [at[0] - 10.0, at[1] - 16.0],
            [at[0] - 24.0, at[1] - 16.0],
            [at[0] - 24.0, at[1] - 30.0],
            [at[0] - 4.0, at[1] - 30.0],
            [at[0] - 4.0, at[1] - 16.0],
        ];
        out.polygon(&[flag[1], flag[5], flag[4], flag[3], flag[2]], *col);
        out.line(flag[0], flag[1], 1.6, *col);
        out.text(
            [at[0] - 14.0, at[1] - 23.0],
            &k.to_string(),
            10.0,
            c::TITLEBAR,
            Align::Center,
        );
    }
    // DC operating point annotations.
    if let Some((nv, pi)) = &opts.annotate {
        let mut placed: BTreeSet<String> = BTreeSet::new();
        for w in &doc.wires {
            if let Some(net) = built.wires.get(&w.id)
                && placed.insert(net.clone())
                && let Some(v) = nv.get(net)
            {
                let s = &w.points;
                let mid = [(s[0][0] + s[1][0]) / 2.0, (s[0][1] + s[1][1]) / 2.0 - 8.0];
                let txt = ecs_units::format(*v, "V");
                out.text(mid, &txt, 10.5, c::WARN, Align::Center);
            }
        }
        for part in &doc.parts {
            if let Some(i) = pi.get(&part.id) {
                let [lo, hi] = part.bbox();
                out.text(
                    [hi[0] + 4.0, lo[1] - 2.0],
                    &ecs_units::format(*i, "A"),
                    10.0,
                    Color32::from_rgb(0x8C, 0xD9, 0xF5),
                    Align::Left,
                );
            }
        }
    }
    // Wire being drawn.
    if opts.wire_preview.len() >= 2 {
        out.polyline(&opts.wire_preview, 2.0, c::ACCENT, false);
        for p in &opts.wire_preview {
            out.circle(*p, 2.5, c::ACCENT, true, 0.0);
        }
    }
    // Placement shadow follows the pointer.
    if let Some(ghost) = &opts.ghost {
        let sym = ghost.symbol();
        let col = c::ACCENT.gamma_multiply(0.9);
        for p in sym.prims.iter().chain(&sym.prims_off) {
            prim(&mut out, ghost, p, col, col.gamma_multiply(0.5));
        }
        for (_, at) in ghost.pins() {
            out.circle(at, 3.0, c::PIN, true, 0.0);
        }
    }
    // Window selection: solid blue enclosing, dashed green crossing.
    if let Some((a, b)) = opts.window {
        let crossing = b[0] < a[0];
        let col = if crossing { c::OK } else { c::SELECT };
        let lo = [a[0].min(b[0]), a[1].min(b[1])];
        let hi = [a[0].max(b[0]), a[1].max(b[1])];
        out.list.push(Shape::Rect {
            min: out.px(lo),
            max: out.px(hi),
            color: rgba(col.gamma_multiply(0.12)),
            fill: true,
            width: 0.0,
        });
        if crossing {
            let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]], lo];
            for s in corners.windows(2) {
                dashed(&mut out, s[0], s[1], col);
            }
        } else {
            out.list.push(Shape::Rect {
                min: out.px(lo),
                max: out.px(hi),
                color: rgba(col),
                fill: false,
                width: opts.ppp,
            });
        }
    }
    DisplayList {
        width: size_px[0],
        height: size_px[1],
        clear: rgba(c::CANVAS),
        shapes,
        revision: 0,
    }
}

fn dashed(out: &mut Out<'_>, a: P2, b: P2, col: Color32) {
    let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let dash = 6.0 / out.vp.zoom as f64;
    let n = (len / dash).floor() as usize;
    for k in (0..n).step_by(2) {
        let t0 = k as f64 / n.max(1) as f64;
        let t1 = ((k + 1) as f64 / n.max(1) as f64).min(1.0);
        let p = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let (pa, pb) = (out.px(p(t0)), out.px(p(t1)));
        out.list.push(Shape::Line {
            a: pa,
            b: pb,
            width: out.ppp,
            color: rgba(col),
        });
    }
}

fn dots_along(out: &mut Out<'_>, a: P2, b: P2, speed_px: f64, t: f64) {
    if speed_px == 0.0 {
        return;
    }
    let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    if len < 1e-9 {
        return;
    }
    let spacing = 16.0;
    let phase = (t * speed_px).rem_euclid(spacing);
    let mut s = phase;
    while s < len {
        let f = s / len;
        out.dot_px(
            [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f],
            2.3 * out.vp.zoom.clamp(0.6, 1.6),
            Color32::from_rgb(0xFF, 0xE0, 0x66),
        );
        s += spacing;
    }
}

/// Paint a display list with egui (fallback and thumbnails). `origin` is the
/// screen position of the list's (0, 0); coordinates are divided by `ppp`.
pub fn paint_egui(
    painter: &egui::Painter,
    list: &DisplayList,
    origin: Pos2,
    ppp: f32,
    fonts_ok: bool,
) {
    let col = |c: Rgba| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    let p = |q: Px| pos2(origin.x + q[0] / ppp, origin.y + q[1] / ppp);
    let rect = egui::Rect::from_min_size(
        origin,
        egui::vec2(list.width as f32 / ppp, list.height as f32 / ppp),
    );
    painter.rect_filled(rect, 0.0, col(list.clear));
    for s in &list.shapes {
        match s {
            Shape::Line { a, b, width, color } => {
                painter.line_segment([p(*a), p(*b)], Stroke::new(width / ppp, col(*color)));
            }
            Shape::Polyline {
                points,
                width,
                color,
                closed,
            } => {
                let pts: Vec<Pos2> = points.iter().map(|q| p(*q)).collect();
                let stroke = Stroke::new(width / ppp, col(*color));
                if *closed {
                    painter.add(egui::Shape::closed_line(pts, stroke));
                } else {
                    painter.add(egui::Shape::line(pts, stroke));
                }
            }
            Shape::Polygon { points, color } => {
                let pts: Vec<Pos2> = points.iter().map(|q| p(*q)).collect();
                painter.add(egui::Shape::convex_polygon(pts, col(*color), Stroke::NONE));
            }
            Shape::Circle {
                c: centre,
                r,
                color,
                fill,
                width,
            } => {
                if *fill {
                    painter.circle_filled(p(*centre), r / ppp, col(*color));
                } else {
                    painter.circle_stroke(
                        p(*centre),
                        r / ppp,
                        Stroke::new(width / ppp, col(*color)),
                    );
                }
            }
            Shape::Rect {
                min,
                max,
                color,
                fill,
                width,
            } => {
                let r = egui::Rect::from_min_max(p(*min), p(*max));
                if *fill {
                    painter.rect_filled(r, 0.0, col(*color));
                } else {
                    painter.rect_stroke(
                        r,
                        0.0,
                        Stroke::new(width / ppp, col(*color)),
                        egui::StrokeKind::Middle,
                    );
                }
            }
            Shape::Glow {
                c: centre,
                r,
                inner,
            } => {
                let steps = 10;
                for k in (1..=steps).rev() {
                    let f = k as f32 / steps as f32;
                    let mut cc = col(*inner);
                    cc = Color32::from_rgba_unmultiplied(
                        cc.r(),
                        cc.g(),
                        cc.b(),
                        ((inner[3] as f32) * (1.0 - f) * 0.35) as u8,
                    );
                    painter.circle_filled(p(*centre), r / ppp * f, cc);
                }
            }
            Shape::Text {
                at,
                text,
                size,
                color,
                align,
            } => {
                if !fonts_ok {
                    continue;
                }
                let anchor = match align {
                    Align::Left => egui::Align2::LEFT_CENTER,
                    Align::Center => egui::Align2::CENTER_CENTER,
                    Align::Right => egui::Align2::RIGHT_CENTER,
                };
                painter.text(
                    p(*at),
                    anchor,
                    text,
                    egui::FontId::proportional(size / ppp),
                    col(*color),
                );
            }
        }
    }
}
