//! Schematic document (spec 9.3 and 10): parts, wires, net labels and text,
//! with editing commands, undo/redo, hit testing and netlist extraction. It
//! has no UI or rendering dependency, so everything here is tested headless.

pub mod symbols;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
pub use symbols::{GRID, P2, Prim, Symbol};
use thiserror::Error;

/// Current `.ecs` format version.
pub const FORMAT_VERSION: u32 = 1;

/// Stable object id within a document.
pub type Id = u64;

/// A placed component instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub id: Id,
    /// Component definition id, e.g. `core.resistor`.
    pub def: String,
    /// Definition version the part was placed with.
    pub def_version: u32,
    /// Package id; `core` for built-ins.
    pub package: String,
    /// Symbol id in the symbol library.
    pub symbol: String,
    /// Reference designator, e.g. `R1`.
    pub refdes: String,
    pub pos: P2,
    /// Quarter turns clockwise, 0..=3.
    pub rot: u8,
    pub mirror: bool,
    /// Parameter overrides as typed (engineering notation), by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    /// Saved control states (switch positions), by state name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub states: BTreeMap<String, f64>,
    /// Manufacturer part number, e.g. `2N2222A`, `74HC00`, `PIC16F84A`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub part_number: String,
    /// Program source of a microcontroller (assembly).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
}

/// A wire as an orthogonal polyline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wire {
    pub id: Id,
    pub points: Vec<P2>,
}

/// A net label: connects every wire or pin it touches to the net of that name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetLabel {
    pub id: Id,
    pub at: P2,
    pub name: String,
}

/// A free text annotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: Id,
    pub at: P2,
    pub text: String,
}

/// A scope probe saved with the schematic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Probe {
    /// Voltage of the net at a point.
    Node { at: P2 },
    /// Current through a part's first branch.
    Current { part: Id },
}

/// Simulation settings saved with the schematic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimSettings {
    /// Simulated seconds per real second.
    pub speed: f64,
    /// Nominal time step in seconds.
    pub time_step: f64,
}

impl Default for SimSettings {
    fn default() -> Self {
        Self {
            speed: 1e-3,
            time_step: 1e-6,
        }
    }
}

/// A scope channel: a formula over part and net codes, such as `R1.I`,
/// `n12`, `out - in` or `R1.A.V * R1.A.I`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopeChannel {
    pub name: String,
    pub expr: String,
}

/// A parameter of a schematic-defined component. Inner parts use it as
/// `{name}` (or any expression in braces) in their parameter fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalParam {
    pub name: String,
    pub default: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub description: String,
}

/// How a component prototype is realised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Implementation {
    /// A nested schematic; net labels named like the symbol's pins are its ports.
    Schematic { schematic: Box<Schematic> },
    /// Another `.ecs` file, relative to this schematic's folder.
    ExternalSchematic { path: String },
    /// JavaScript source (spec 6), kept in this schematic.
    Script { source: String },
    /// A `.js` file, relative to this schematic's folder or absolute.
    ExternalScript { path: String },
}

/// A component prototype defined inside a schematic: its own symbol and
/// ports, and an implementation. Parts refer to it by `id` (`local.<name>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalComponent {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub category: String,
    pub prefix: String,
    #[serde(default)]
    pub description: String,
    pub symbol: Symbol,
    #[serde(default)]
    pub params: Vec<LocalParam>,
    pub implementation: Implementation,
}

/// Document metadata.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A schematic document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schematic {
    pub format_version: u32,
    pub metadata: Metadata,
    pub parts: Vec<Part>,
    pub wires: Vec<Wire>,
    #[serde(default)]
    pub labels: Vec<NetLabel>,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub probes: Vec<Probe>,
    /// Scope channels: formulas over part and net codes (spec 9.5).
    #[serde(default)]
    pub scope: Vec<ScopeChannel>,
    /// Component prototypes defined in this schematic (spec 5, 9.7).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<LocalComponent>,
    #[serde(default)]
    pub simulation: SimSettings,
    pub next_id: Id,
}

impl Default for Schematic {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            metadata: Metadata {
                title: "Untitled".into(),
                description: String::new(),
            },
            parts: Vec::new(),
            wires: Vec::new(),
            labels: Vec::new(),
            notes: Vec::new(),
            probes: Vec::new(),
            scope: Vec::new(),
            components: Vec::new(),
            simulation: SimSettings::default(),
            next_id: 1,
        }
    }
}

/// File errors.
#[derive(Debug, Error)]
pub enum FileError {
    #[error("not a schematic: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schematic format {0} is newer than this version of Electrocraft supports")]
    TooNew(u32),
}

/// Something in the document that can be selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Item {
    Part(Id),
    Wire(Id),
    Label(Id),
    Note(Id),
}

/// Snap a point to the grid.
pub fn snap(p: P2) -> P2 {
    [(p[0] / GRID).round() * GRID, (p[1] / GRID).round() * GRID]
}

fn close(a: P2, b: P2) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

/// Distance from `p` to segment `a`-`b`.
pub fn seg_distance(p: P2, a: P2, b: P2) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len2 = d[0] * d[0] + d[1] * d[1];
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0)
    };
    let q = [a[0] + t * d[0], a[1] + t * d[1]];
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
}

/// True when `p` lies strictly inside segment `a`-`b` (not at an end).
fn on_segment_interior(p: P2, a: P2, b: P2) -> bool {
    seg_distance(p, a, b) < 1e-6 && !close(p, a) && !close(p, b)
}

impl Part {
    /// Transform a symbol-local point to world coordinates.
    pub fn to_world(&self, p: P2) -> P2 {
        let mut x = if self.mirror { -p[0] } else { p[0] };
        let mut y = p[1];
        for _ in 0..self.rot % 4 {
            // Quarter turn clockwise on screen (y down).
            let nx = -y;
            y = x;
            x = nx;
        }
        [self.pos[0] + x, self.pos[1] + y]
    }

    /// The part's symbol.
    pub fn symbol(&self) -> &'static Symbol {
        symbols::get(&self.symbol)
    }

    /// World positions of the pins, with names, in symbol pin order.
    pub fn pins(&self) -> Vec<(String, P2)> {
        self.symbol()
            .pins
            .iter()
            .map(|p| (p.name.clone(), self.to_world(p.at)))
            .collect()
    }

    /// World bounding box `[min, max]`.
    pub fn bbox(&self) -> [P2; 2] {
        let [a, b] = self.symbol().bbox;
        let corners = [a, [b[0], a[1]], b, [a[0], b[1]]].map(|c| self.to_world(c));
        let mut lo = corners[0];
        let mut hi = corners[0];
        for c in corners {
            lo = [lo[0].min(c[0]), lo[1].min(c[1])];
            hi = [hi[0].max(c[0]), hi[1].max(c[1])];
        }
        [lo, hi]
    }
}

/// Window selection: `enclosing` (drag left to right) needs the whole item
/// inside; crossing (right to left) takes anything the rectangle touches.
fn rect_contains(r: [P2; 2], p: P2) -> bool {
    p[0] >= r[0][0] && p[0] <= r[1][0] && p[1] >= r[0][1] && p[1] <= r[1][1]
}

fn rects_touch(a: [P2; 2], b: [P2; 2]) -> bool {
    a[0][0] <= b[1][0] && a[1][0] >= b[0][0] && a[0][1] <= b[1][1] && a[1][1] >= b[0][1]
}

fn segment_touches_rect(a: P2, b: P2, r: [P2; 2]) -> bool {
    if rect_contains(r, a) || rect_contains(r, b) {
        return true;
    }
    // Wires are orthogonal, so a bounding-box overlap is exact.
    let lo = [a[0].min(b[0]), a[1].min(b[1])];
    let hi = [a[0].max(b[0]), a[1].max(b[1])];
    rects_touch([lo, hi], r)
}

/// Pick tolerance for wires in world units.
pub const WIRE_PICK: f64 = 5.0;

impl Schematic {
    /// Allocate a new id.
    pub fn alloc_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Serialize as pretty JSON with stable key order.
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("schematic serializes");
        s.push('\n');
        s
    }

    /// Load from JSON, migrating older format versions.
    pub fn from_json(text: &str) -> Result<Self, FileError> {
        let doc: Schematic = serde_json::from_str(text)?;
        if doc.format_version > FORMAT_VERSION {
            return Err(FileError::TooNew(doc.format_version));
        }
        Ok(doc)
    }

    /// A part by id.
    pub fn part(&self, id: Id) -> Option<&Part> {
        self.parts.iter().find(|p| p.id == id)
    }
    /// A mutable part by id.
    pub fn part_mut(&mut self, id: Id) -> Option<&mut Part> {
        self.parts.iter_mut().find(|p| p.id == id)
    }

    /// Next free reference designator for a prefix: `R1`, `R2`...
    pub fn next_refdes(&self, prefix: &str) -> String {
        let used: BTreeSet<u32> = self
            .parts
            .iter()
            .filter_map(|p| p.refdes.strip_prefix(prefix).and_then(|n| n.parse().ok()))
            .collect();
        let n = (1..).find(|n| !used.contains(n)).unwrap();
        format!("{prefix}{n}")
    }

    /// Topmost item under `p`. Pins and wires beat part bodies within `tol`.
    pub fn hit(&self, p: P2, tol: f64) -> Option<Item> {
        for l in self.labels.iter().rev() {
            if (p[0] - l.at[0]).abs() < 30.0 + tol
                && (p[1] - l.at[1] + 8.0).abs() < 10.0 + tol
                && p[0] >= l.at[0] - tol
            {
                return Some(Item::Label(l.id));
            }
        }
        for n in self.notes.iter().rev() {
            let w = 7.0 * n.text.chars().count() as f64;
            if p[0] >= n.at[0] - tol
                && p[0] <= n.at[0] + w + tol
                && (p[1] - n.at[1]).abs() < 10.0 + tol
            {
                return Some(Item::Note(n.id));
            }
        }
        for w in self.wires.iter().rev() {
            if w.points
                .windows(2)
                .any(|s| seg_distance(p, s[0], s[1]) <= tol)
            {
                return Some(Item::Wire(w.id));
            }
        }
        for part in self.parts.iter().rev() {
            let [lo, hi] = part.bbox();
            if p[0] >= lo[0] - tol
                && p[0] <= hi[0] + tol
                && p[1] >= lo[1] - tol
                && p[1] <= hi[1] + tol
            {
                return Some(Item::Part(part.id));
            }
        }
        None
    }

    /// Items selected by a window from `a` to `b` (see [`Item`]).
    pub fn window(&self, a: P2, b: P2) -> BTreeSet<Item> {
        let r = [
            [a[0].min(b[0]), a[1].min(b[1])],
            [a[0].max(b[0]), a[1].max(b[1])],
        ];
        let crossing = b[0] < a[0];
        let mut out = BTreeSet::new();
        for p in &self.parts {
            let bb = p.bbox();
            let hit = if crossing {
                rects_touch(bb, r)
            } else {
                rect_contains(r, bb[0]) && rect_contains(r, bb[1])
            };
            if hit {
                out.insert(Item::Part(p.id));
            }
        }
        for w in &self.wires {
            let hit = if crossing {
                w.points
                    .windows(2)
                    .any(|s| segment_touches_rect(s[0], s[1], r))
            } else {
                w.points.iter().all(|&q| rect_contains(r, q))
            };
            if hit {
                out.insert(Item::Wire(w.id));
            }
        }
        for l in &self.labels {
            if rect_contains(r, l.at) {
                out.insert(Item::Label(l.id));
            }
        }
        for n in &self.notes {
            if rect_contains(r, n.at) {
                out.insert(Item::Note(n.id));
            }
        }
        out
    }

    /// The nearest pin within `tol` of `p`: (part id, pin name, position).
    pub fn pin_at(&self, p: P2, tol: f64) -> Option<(Id, String, P2)> {
        let mut best: Option<(f64, Id, String, P2)> = None;
        for part in &self.parts {
            for (name, at) in part.pins() {
                let d = ((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2)).sqrt();
                if d <= tol && best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((d, part.id, name, at));
                }
            }
        }
        best.map(|(_, id, n, at)| (id, n, at))
    }

    /// Bounding box of everything, or `None` when empty.
    pub fn extent(&self) -> Option<[P2; 2]> {
        let mut pts: Vec<P2> = Vec::new();
        for p in &self.parts {
            let b = p.bbox();
            pts.push(b[0]);
            pts.push(b[1]);
        }
        for w in &self.wires {
            pts.extend(&w.points);
        }
        for l in &self.labels {
            pts.push(l.at);
            pts.push([l.at[0] + 40.0, l.at[1] - 16.0]);
        }
        for n in &self.notes {
            pts.push(n.at);
            pts.push([n.at[0] + 7.0 * n.text.chars().count() as f64, n.at[1]]);
        }
        let first = *pts.first()?;
        let mut lo = first;
        let mut hi = first;
        for p in pts {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        Some([lo, hi])
    }

    /// Junction dots: points where three or more wire ends or pins meet, or
    /// where a wire end or pin lands on the interior of another wire.
    pub fn junctions(&self) -> Vec<P2> {
        let mut ends: Vec<P2> = Vec::new();
        for w in &self.wires {
            if let (Some(a), Some(b)) = (w.points.first(), w.points.last()) {
                ends.push(*a);
                ends.push(*b);
            }
        }
        let pins: Vec<P2> = self
            .parts
            .iter()
            .flat_map(|p| p.pins().into_iter().map(|x| x.1))
            .collect();
        let mut out: Vec<P2> = Vec::new();
        let push = |p: P2, out: &mut Vec<P2>| {
            if !out.iter().any(|q| close(*q, p)) {
                out.push(p);
            }
        };
        for &e in &ends {
            let degree = ends.iter().filter(|q| close(**q, e)).count()
                + pins.iter().filter(|q| close(**q, e)).count();
            let tee = self.wires.iter().any(|w| {
                w.points
                    .windows(2)
                    .any(|s| on_segment_interior(e, s[0], s[1]))
            });
            if degree >= 3 || tee {
                push(e, &mut out);
            }
        }
        for &p in &pins {
            if self.wires.iter().any(|w| {
                w.points
                    .windows(2)
                    .any(|s| on_segment_interior(p, s[0], s[1]))
            }) {
                push(p, &mut out);
            }
        }
        out
    }

    /// Compute nets (spec 9.3 connectivity rules).
    pub fn connectivity(&self, is_ground: &dyn Fn(&Part) -> bool) -> Connectivity {
        // Union-find over "terminals": every wire vertex and every pin.
        let mut points: Vec<P2> = Vec::new();
        let index = |p: P2, points: &mut Vec<P2>| -> usize {
            if let Some(i) = points.iter().position(|q| close(*q, p)) {
                i
            } else {
                points.push(p);
                points.len() - 1
            }
        };
        let mut wire_pts: Vec<Vec<usize>> = Vec::new();
        for w in &self.wires {
            wire_pts.push(w.points.iter().map(|&p| index(p, &mut points)).collect());
        }
        let mut pin_pts: Vec<Vec<usize>> = Vec::new();
        for part in &self.parts {
            pin_pts.push(
                part.pins()
                    .iter()
                    .map(|(_, p)| index(*p, &mut points))
                    .collect(),
            );
        }
        let label_pts: Vec<usize> = self
            .labels
            .iter()
            .map(|l| index(l.at, &mut points))
            .collect();
        let mut uf: Vec<usize> = (0..points.len()).collect();
        fn find(uf: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while uf[r] != r {
                r = uf[r];
            }
            let mut i = i;
            while uf[i] != r {
                let n = uf[i];
                uf[i] = r;
                i = n;
            }
            r
        }
        fn union(uf: &mut [usize], a: usize, b: usize) {
            let (ra, rb) = (find(uf, a), find(uf, b));
            if ra != rb {
                uf[ra] = rb;
            }
        }
        for wp in &wire_pts {
            for k in 1..wp.len() {
                union(&mut uf, wp[0], wp[k]);
            }
        }
        // A terminal on the interior of a wire segment joins that wire (T junction).
        for (i, &p) in points.iter().enumerate() {
            for (w, wp) in self.wires.iter().zip(&wire_pts) {
                if w.points
                    .windows(2)
                    .any(|s| on_segment_interior(p, s[0], s[1]))
                {
                    union(&mut uf, i, wp[0]);
                }
            }
        }
        // Labels with the same name join; ground parts and GND labels are net 0.
        let mut by_name: HashMap<String, usize> = HashMap::new();
        let mut ground: Option<usize> = None;
        for (l, &pi) in self.labels.iter().zip(&label_pts) {
            let name = l.name.trim().to_string();
            if matches!(name.as_str(), "0" | "GND" | "gnd") {
                match ground {
                    Some(g) => union(&mut uf, g, pi),
                    None => ground = Some(pi),
                }
            } else if let Some(&other) = by_name.get(&name) {
                union(&mut uf, other, pi);
            } else {
                by_name.insert(name, pi);
            }
        }
        for (part, pp) in self.parts.iter().zip(&pin_pts) {
            if is_ground(part)
                && let Some(&pi) = pp.first()
            {
                match ground {
                    Some(g) => union(&mut uf, g, pi),
                    None => ground = Some(pi),
                }
            }
        }
        // Name nets: ground "0", labelled nets by label, others N001...
        let ground_root = ground.map(|g| find(&mut uf, g));
        let mut root_name: HashMap<usize, String> = HashMap::new();
        if let Some(g) = ground_root {
            root_name.insert(g, "0".into());
        }
        for (l, &pi) in self.labels.iter().zip(&label_pts) {
            let r = find(&mut uf, pi);
            root_name
                .entry(r)
                .or_insert_with(|| l.name.trim().to_string());
        }
        // Unlabelled nets get stable codes: `n<id>` of their oldest wire, or
        // `p<part>_<pin>` when only pins meet. Codes survive unrelated edits,
        // so scope formulas keep referring to the same net.
        let mut oldest_wire: HashMap<usize, Id> = HashMap::new();
        for (w, wp) in self.wires.iter().zip(&wire_pts) {
            if let Some(&i) = wp.first() {
                let r = find(&mut uf, i);
                let e = oldest_wire.entry(r).or_insert(w.id);
                *e = (*e).min(w.id);
            }
        }
        let mut pin_code: HashMap<usize, String> = HashMap::new();
        for (part, pp) in self.parts.iter().zip(&pin_pts) {
            for (k, &i) in pp.iter().enumerate() {
                let r = find(&mut uf, i);
                pin_code
                    .entry(r)
                    .or_insert_with(|| format!("p{}_{k}", part.id));
            }
        }
        let net_of_point: Vec<String> = (0..points.len())
            .map(|i| {
                let r = find(&mut uf, i);
                root_name
                    .entry(r)
                    .or_insert_with(|| match oldest_wire.get(&r) {
                        Some(w) => format!("n{w}"),
                        None => pin_code.get(&r).cloned().unwrap_or_else(|| format!("q{i}")),
                    })
                    .clone()
            })
            .collect();
        let mut pins = BTreeMap::new();
        for (part, pp) in self.parts.iter().zip(&pin_pts) {
            let names: Vec<String> = pp.iter().map(|&i| net_of_point[i].clone()).collect();
            pins.insert(part.id, names);
        }
        let mut wires = BTreeMap::new();
        for (w, wp) in self.wires.iter().zip(&wire_pts) {
            if let Some(&i) = wp.first() {
                wires.insert(w.id, net_of_point[i].clone());
            }
        }
        let mut labels = BTreeMap::new();
        for (l, &pi) in self.labels.iter().zip(&label_pts) {
            labels.insert(l.id, net_of_point[pi].clone());
        }
        // Pins that touch nothing else are unconnected.
        let mut uses: HashMap<String, usize> = HashMap::new();
        for names in pins.values() {
            for n in names {
                *uses.entry(n.clone()).or_default() += 1;
            }
        }
        for n in wires.values().chain(labels.values()) {
            *uses.entry(n.clone()).or_default() += 2;
        }
        let unconnected = pins
            .iter()
            .flat_map(|(id, names)| {
                names
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| uses.get(*n).copied().unwrap_or(0) < 2 && n.as_str() != "0")
                    .map(move |(k, _)| (*id, k))
            })
            .collect();
        Connectivity {
            pins,
            wires,
            labels,
            unconnected,
        }
    }
}

/// Result of [`Schematic::connectivity`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Connectivity {
    /// Net name for each pin of each part, in symbol pin order.
    pub pins: BTreeMap<Id, Vec<String>>,
    /// Net name of each wire.
    pub wires: BTreeMap<Id, String>,
    /// Net name of each label.
    pub labels: BTreeMap<Id, String>,
    /// Pins connected to nothing: (part, pin index).
    pub unconnected: Vec<(Id, usize)>,
}

/// An editing command. Every command is one undo step.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    AddPart(Part),
    AddWire(Vec<P2>),
    AddLabel {
        at: P2,
        name: String,
    },
    AddNote {
        at: P2,
        text: String,
    },
    Delete(BTreeSet<Item>),
    /// Move items by a delta; wires attached to moved pins stretch.
    Move {
        items: BTreeSet<Item>,
        delta: P2,
    },
    /// Rotate parts a quarter turn clockwise about the selection centre.
    Rotate(BTreeSet<Item>),
    /// Mirror parts horizontally.
    Mirror(BTreeSet<Item>),
    SetParam {
        part: Id,
        name: String,
        value: String,
    },
    SetRefdes {
        part: Id,
        refdes: String,
    },
    SetLabel {
        label: Id,
        name: String,
    },
    SetState {
        part: Id,
        name: String,
        value: f64,
    },
    /// Paste copies of parts, wires, labels and notes, offset by `delta`.
    Paste {
        clip: Clipboard,
        delta: P2,
    },
    /// Replace the scope probes.
    SetProbes(Vec<Probe>),
    /// Replace the scope channels.
    SetScope(Vec<ScopeChannel>),
    /// Add or replace a component prototype.
    SetComponent(Box<LocalComponent>),
    /// Remove a component prototype.
    RemoveComponent(String),
    /// Point a part at another definition (and symbol).
    SetPartDef {
        part: Id,
        def: String,
        symbol: String,
    },
    /// Set a part number and the parameter values it brings, in one step.
    SetPartNumber {
        part: Id,
        number: String,
        params: Vec<(String, String)>,
    },
    /// Replace a microcontroller's program.
    SetPartCode {
        part: Id,
        code: String,
    },
}

/// Copied items for paste and duplicate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Clipboard {
    pub parts: Vec<Part>,
    pub wires: Vec<Wire>,
    pub labels: Vec<NetLabel>,
    pub notes: Vec<Note>,
}

impl Clipboard {
    /// Copy the selected items.
    pub fn copy(doc: &Schematic, items: &BTreeSet<Item>) -> Self {
        let has = |i: Item| items.contains(&i);
        Self {
            parts: doc
                .parts
                .iter()
                .filter(|p| has(Item::Part(p.id)))
                .cloned()
                .collect(),
            wires: doc
                .wires
                .iter()
                .filter(|w| has(Item::Wire(w.id)))
                .cloned()
                .collect(),
            labels: doc
                .labels
                .iter()
                .filter(|l| has(Item::Label(l.id)))
                .cloned()
                .collect(),
            notes: doc
                .notes
                .iter()
                .filter(|n| has(Item::Note(n.id)))
                .cloned()
                .collect(),
        }
    }
    /// Whether nothing is copied.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
            && self.wires.is_empty()
            && self.labels.is_empty()
            && self.notes.is_empty()
    }
}

fn add(p: P2, d: P2) -> P2 {
    [p[0] + d[0], p[1] + d[1]]
}

/// Remove zero-length segments and merge collinear runs.
fn simplify(points: &mut Vec<P2>) {
    points.dedup_by(|a, b| close(*a, *b));
    let mut i = 1;
    while i + 1 < points.len() {
        let (a, b, c) = (points[i - 1], points[i], points[i + 1]);
        let collinear = ((a[0] - b[0]).abs() < 1e-9 && (b[0] - c[0]).abs() < 1e-9)
            || ((a[1] - b[1]).abs() < 1e-9 && (b[1] - c[1]).abs() < 1e-9);
        if collinear {
            points.remove(i);
        } else {
            i += 1;
        }
    }
}

impl Schematic {
    /// Apply a command. Returns the ids of items it created.
    pub fn apply(&mut self, cmd: Command) -> Vec<Item> {
        let mut created = Vec::new();
        match cmd {
            Command::AddPart(mut p) => {
                if p.id == 0 || self.part(p.id).is_some() {
                    p.id = self.alloc_id();
                } else {
                    self.next_id = self.next_id.max(p.id + 1);
                }
                created.push(Item::Part(p.id));
                self.parts.push(p);
            }
            Command::AddWire(mut points) => {
                simplify(&mut points);
                if points.len() >= 2 {
                    let id = self.alloc_id();
                    self.wires.push(Wire { id, points });
                    created.push(Item::Wire(id));
                }
            }
            Command::AddLabel { at, name } => {
                let id = self.alloc_id();
                self.labels.push(NetLabel { id, at, name });
                created.push(Item::Label(id));
            }
            Command::AddNote { at, text } => {
                let id = self.alloc_id();
                self.notes.push(Note { id, at, text });
                created.push(Item::Note(id));
            }
            Command::Delete(items) => {
                self.parts.retain(|p| !items.contains(&Item::Part(p.id)));
                self.wires.retain(|w| !items.contains(&Item::Wire(w.id)));
                self.labels.retain(|l| !items.contains(&Item::Label(l.id)));
                self.notes.retain(|n| !items.contains(&Item::Note(n.id)));
                self.probes.retain(|p| match p {
                    Probe::Current { part } => !items.contains(&Item::Part(*part)),
                    Probe::Node { .. } => true,
                });
            }
            Command::Move { items, delta } => self.move_items(&items, delta),
            Command::Rotate(items) => self.transform_parts(&items, |p| p.rot = (p.rot + 1) % 4),
            Command::Mirror(items) => self.transform_parts(&items, |p| p.mirror = !p.mirror),
            Command::SetParam { part, name, value } => {
                if let Some(p) = self.part_mut(part) {
                    if value.trim().is_empty() {
                        p.params.remove(&name);
                    } else {
                        p.params.insert(name, value);
                    }
                }
            }
            Command::SetRefdes { part, refdes } => {
                if let Some(p) = self.part_mut(part) {
                    p.refdes = refdes;
                }
            }
            Command::SetLabel { label, name } => {
                if let Some(l) = self.labels.iter_mut().find(|l| l.id == label) {
                    l.name = name;
                }
            }
            Command::SetState { part, name, value } => {
                if let Some(p) = self.part_mut(part) {
                    p.states.insert(name, value);
                }
            }
            Command::SetProbes(probes) => self.probes = probes,
            Command::SetScope(scope) => self.scope = scope,
            Command::SetComponent(c) => match self.components.iter_mut().find(|x| x.id == c.id) {
                Some(slot) => *slot = *c,
                None => self.components.push(*c),
            },
            Command::RemoveComponent(id) => self.components.retain(|c| c.id != id),
            Command::SetPartNumber {
                part,
                number,
                params,
            } => {
                if let Some(p) = self.part_mut(part) {
                    p.part_number = number;
                    for (k, v) in params {
                        p.params.insert(k, v);
                    }
                }
            }
            Command::SetPartCode { part, code } => {
                if let Some(p) = self.part_mut(part) {
                    p.code = code;
                }
            }
            Command::SetPartDef { part, def, symbol } => {
                if let Some(p) = self.part_mut(part) {
                    // Keep only overrides the new definition may still use.
                    p.def = def;
                    p.symbol = symbol;
                }
            }
            Command::Paste { clip, delta } => {
                for mut p in clip.parts {
                    p.id = self.alloc_id();
                    p.pos = add(p.pos, delta);
                    let prefix: String = p
                        .refdes
                        .chars()
                        .take_while(|c| !c.is_ascii_digit())
                        .collect();
                    p.refdes = self.next_refdes(&prefix);
                    created.push(Item::Part(p.id));
                    self.parts.push(p);
                }
                for mut w in clip.wires {
                    w.id = self.alloc_id();
                    w.points.iter_mut().for_each(|q| *q = add(*q, delta));
                    created.push(Item::Wire(w.id));
                    self.wires.push(w);
                }
                for mut l in clip.labels {
                    l.id = self.alloc_id();
                    l.at = add(l.at, delta);
                    created.push(Item::Label(l.id));
                    self.labels.push(l);
                }
                for mut n in clip.notes {
                    n.id = self.alloc_id();
                    n.at = add(n.at, delta);
                    created.push(Item::Note(n.id));
                    self.notes.push(n);
                }
            }
        }
        created
    }

    fn transform_parts(&mut self, items: &BTreeSet<Item>, f: impl Fn(&mut Part)) {
        let ids: Vec<Id> = items
            .iter()
            .filter_map(|i| {
                if let Item::Part(id) = i {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        // Rotate about the selection centre, snapped, keeping wires attached.
        let before: Vec<(Id, Vec<P2>)> = ids
            .iter()
            .filter_map(|id| {
                self.part(*id)
                    .map(|p| (*id, p.pins().into_iter().map(|x| x.1).collect()))
            })
            .collect();
        let n = ids.len() as f64;
        let centre = snap([
            ids.iter()
                .filter_map(|id| self.part(*id))
                .map(|p| p.pos[0])
                .sum::<f64>()
                / n,
            ids.iter()
                .filter_map(|id| self.part(*id))
                .map(|p| p.pos[1])
                .sum::<f64>()
                / n,
        ]);
        for id in &ids {
            let rotate_about = ids.len() > 1;
            if let Some(p) = self.part_mut(*id) {
                let old_rot = p.rot;
                f(p);
                if rotate_about && p.rot != old_rot {
                    let d = [p.pos[0] - centre[0], p.pos[1] - centre[1]];
                    p.pos = snap([centre[0] - d[1], centre[1] + d[0]]);
                }
            }
        }
        // Detach-free: wire ends that sat on old pins follow to the new pins.
        for (id, old) in before {
            let new: Vec<P2> = self
                .part(id)
                .map(|p| p.pins().into_iter().map(|x| x.1).collect())
                .unwrap_or_default();
            for (o, nw) in old.iter().zip(&new) {
                for w in &mut self.wires {
                    let last = w.points.len() - 1;
                    for k in [0, last] {
                        if close(w.points[k], *o) {
                            w.points[k] = *nw;
                            let j = if k == 0 { 1 } else { last - 1 };
                            if w.points.len() == 2 {
                                // Insert an elbow so the wire stays orthogonal.
                                let other = w.points[1 - k];
                                let elbow = [nw[0], other[1]];
                                w.points.insert(1, elbow);
                            } else {
                                fix_orthogonal(&mut w.points, k, j);
                            }
                        }
                    }
                    simplify(&mut w.points);
                }
            }
        }
    }

    fn move_items(&mut self, items: &BTreeSet<Item>, delta: P2) {
        let moved_parts: Vec<Id> = items
            .iter()
            .filter_map(|i| {
                if let Item::Part(id) = i {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        let mut pin_points: Vec<P2> = Vec::new();
        for id in &moved_parts {
            if let Some(p) = self.part(*id) {
                pin_points.extend(p.pins().into_iter().map(|x| x.1));
            }
        }
        for p in &mut self.parts {
            if items.contains(&Item::Part(p.id)) {
                p.pos = add(p.pos, delta);
            }
        }
        // Probes on moved pins follow.
        for probe in &mut self.probes {
            if let Probe::Node { at } = probe
                && pin_points.iter().any(|pp| close(*pp, *at))
            {
                *at = add(*at, delta);
            }
        }
        for l in &mut self.labels {
            if items.contains(&Item::Label(l.id)) {
                l.at = add(l.at, delta);
            }
        }
        for n in &mut self.notes {
            if items.contains(&Item::Note(n.id)) {
                n.at = add(n.at, delta);
            }
        }
        for w in &mut self.wires {
            if items.contains(&Item::Wire(w.id)) {
                w.points.iter_mut().for_each(|q| *q = add(*q, delta));
                continue;
            }
            // Rubber band: ends on moved pins follow, keeping the wire orthogonal.
            let last = w.points.len() - 1;
            let mut touched = false;
            for k in [0, last] {
                if pin_points.iter().any(|pp| close(*pp, w.points[k])) {
                    w.points[k] = add(w.points[k], delta);
                    touched = true;
                    if w.points.len() == 2 {
                        let other = w.points[1 - k];
                        let moved = w.points[k];
                        if (moved[0] - other[0]).abs() > 1e-9 && (moved[1] - other[1]).abs() > 1e-9
                        {
                            let elbow = if k == 0 {
                                [other[0], moved[1]]
                            } else {
                                [moved[0], other[1]]
                            };
                            w.points.insert(1, elbow);
                        }
                    } else {
                        let j = if k == 0 { 1 } else { w.points.len() - 2 };
                        let k2 = if k == 0 { 0 } else { w.points.len() - 1 };
                        fix_orthogonal(&mut w.points, k2, j);
                    }
                }
            }
            if touched {
                simplify(&mut w.points);
            }
        }
    }
}

/// After moving end `k`, slide its neighbour `j` so segment k-j stays
/// horizontal or vertical as it was.
fn fix_orthogonal(points: &mut [P2], k: usize, j: usize) {
    let n = points.len();
    if n < 3 {
        return;
    }
    let beyond = if j > k { j + 1 } else { j.wrapping_sub(1) };
    let neighbour = points[j];
    let anchor = if beyond < n {
        points[beyond]
    } else {
        neighbour
    };
    // The segment j-beyond decides: if it is vertical, k-j must be horizontal.
    if (neighbour[0] - anchor[0]).abs() < 1e-9 {
        points[j] = [neighbour[0], points[k][1]];
    } else {
        points[j] = [points[k][0], neighbour[1]];
    }
}

/// Undo and redo over whole-document snapshots. Every [`Command`] is one step.
#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Schematic>,
    redo: Vec<Schematic>,
}

impl History {
    /// Apply `cmd` to `doc`, recording the previous state.
    pub fn apply(&mut self, doc: &mut Schematic, cmd: Command) -> Vec<Item> {
        let before = doc.clone();
        let created = doc.apply(cmd);
        if *doc != before {
            self.undo.push(before);
            self.redo.clear();
            if self.undo.len() > 500 {
                self.undo.remove(0);
            }
        }
        created
    }
    /// Undo the last step. Returns false when there is nothing to undo.
    pub fn undo(&mut self, doc: &mut Schematic) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(doc, prev));
                true
            }
            None => false,
        }
    }
    /// Redo the last undone step.
    pub fn redo(&mut self, doc: &mut Schematic) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(doc, next));
                true
            }
            None => false,
        }
    }
    /// Whether undo is possible.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    /// Whether redo is possible.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// Forget everything (after loading a file).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// Orthogonal route from `a` to `b`: horizontal first, or vertical first.
pub fn route(a: P2, b: P2, horizontal_first: bool) -> Vec<P2> {
    let elbow = if horizontal_first {
        [b[0], a[1]]
    } else {
        [a[0], b[1]]
    };
    let mut pts = vec![a, elbow, b];
    simplify(&mut pts);
    pts
}

/// Create a part from its definition data.
pub fn new_part(def: &str, symbol: &str, refdes: String, pos: P2) -> Part {
    Part {
        id: 0,
        def: def.into(),
        def_version: 1,
        package: "core".into(),
        symbol: symbol.into(),
        refdes,
        pos: snap(pos),
        rot: 0,
        mirror: false,
        params: BTreeMap::new(),
        states: BTreeMap::new(),
        part_number: String::new(),
        code: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground(p: &Part) -> bool {
        p.def == "core.ground"
    }

    fn divider() -> Schematic {
        let mut d = Schematic::default();
        let mut h = History::default();
        h.apply(
            &mut d,
            Command::AddPart(new_part("core.vdc", "vdc", "V1".into(), [0.0, 0.0])),
        );
        let mut r1 = new_part("core.resistor", "resistor", "R1".into(), [80.0, -40.0]);
        r1.params.insert("R".into(), "3k".into());
        h.apply(&mut d, Command::AddPart(r1));
        let mut r2 = new_part("core.resistor", "resistor", "R2".into(), [160.0, 0.0]);
        r2.rot = 1;
        h.apply(&mut d, Command::AddPart(r2));
        h.apply(
            &mut d,
            Command::AddPart(new_part(
                "core.ground",
                "ground",
                "GND1".into(),
                [0.0, 60.0],
            )),
        );
        // V1+ (0,-40) to R1.A (40,-40); R1.B (120,-40) to R2 top (160,-40).
        h.apply(&mut d, Command::AddWire(vec![[0.0, -40.0], [40.0, -40.0]]));
        h.apply(
            &mut d,
            Command::AddWire(route([120.0, -40.0], [160.0, -40.0], true)),
        );
        // R2 bottom (160,40) and V1- (0,40) to ground (0,60).
        h.apply(
            &mut d,
            Command::AddWire(vec![[160.0, 40.0], [160.0, 60.0], [0.0, 60.0]]),
        );
        h.apply(&mut d, Command::AddWire(vec![[0.0, 40.0], [0.0, 60.0]]));
        d
    }

    #[test]
    fn rotation_maps_pins() {
        let mut p = new_part("core.resistor", "resistor", "R1".into(), [100.0, 100.0]);
        assert_eq!(p.pins()[0].1, [60.0, 100.0]);
        p.rot = 1;
        assert_eq!(p.pins()[0].1, [100.0, 60.0]);
        p.mirror = true;
        p.rot = 0;
        assert_eq!(p.pins()[0].1, [140.0, 100.0]);
    }

    #[test]
    fn divider_netlist() {
        let d = divider();
        let c = d.connectivity(&ground);
        let v1 = d.parts.iter().find(|p| p.refdes == "V1").unwrap().id;
        let r1 = d.parts.iter().find(|p| p.refdes == "R1").unwrap().id;
        let r2 = d.parts.iter().find(|p| p.refdes == "R2").unwrap().id;
        assert_eq!(c.pins[&v1][1], "0");
        assert_eq!(c.pins[&r2][1], "0");
        assert_eq!(c.pins[&v1][0], c.pins[&r1][0]);
        assert_eq!(c.pins[&r1][1], c.pins[&r2][0]);
        assert_ne!(c.pins[&r1][0], c.pins[&r1][1]);
        assert!(c.unconnected.is_empty(), "{:?}", c.unconnected);
    }

    #[test]
    fn tee_junction_connects_and_is_dotted() {
        let mut d = Schematic::default();
        d.apply(Command::AddWire(vec![[0.0, 0.0], [100.0, 0.0]]));
        d.apply(Command::AddWire(vec![[40.0, 0.0], [40.0, 60.0]]));
        // A crossing wire without an endpoint on the first does not connect.
        d.apply(Command::AddWire(vec![[60.0, -40.0], [60.0, 40.0]]));
        let c = d.connectivity(&ground);
        let nets: Vec<&String> = c.wires.values().collect();
        assert_eq!(nets[0], nets[1]);
        assert_ne!(nets[0], nets[2]);
        assert_eq!(d.junctions(), vec![[40.0, 0.0]]);
    }

    #[test]
    fn labels_join_nets_by_name() {
        let mut d = Schematic::default();
        d.apply(Command::AddWire(vec![[0.0, 0.0], [40.0, 0.0]]));
        d.apply(Command::AddWire(vec![[200.0, 0.0], [240.0, 0.0]]));
        d.apply(Command::AddLabel {
            at: [0.0, 0.0],
            name: "VCC".into(),
        });
        d.apply(Command::AddLabel {
            at: [240.0, 0.0],
            name: "VCC".into(),
        });
        let c = d.connectivity(&ground);
        assert!(c.wires.values().all(|n| n == "VCC"));
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut d = Schematic::default();
        let mut h = History::default();
        let created = h.apply(
            &mut d,
            Command::AddPart(new_part(
                "core.resistor",
                "resistor",
                "R1".into(),
                [0.0, 0.0],
            )),
        );
        let snapshot = d.clone();
        h.apply(
            &mut d,
            Command::Move {
                items: created.iter().copied().collect(),
                delta: [20.0, 0.0],
            },
        );
        assert_eq!(d.parts[0].pos, [20.0, 0.0]);
        assert!(h.undo(&mut d));
        assert_eq!(d, snapshot);
        assert!(h.redo(&mut d));
        assert_eq!(d.parts[0].pos, [20.0, 0.0]);
        assert!(h.undo(&mut d) && h.undo(&mut d));
        assert!(d.parts.is_empty());
        assert!(!h.undo(&mut d));
    }

    #[test]
    fn wires_stretch_when_parts_move() {
        let mut d = divider();
        let r1 = Item::Part(d.parts.iter().find(|p| p.refdes == "R1").unwrap().id);
        let before = d.connectivity(&ground);
        d.apply(Command::Move {
            items: [r1].into(),
            delta: [0.0, -40.0],
        });
        let after = d.connectivity(&ground);
        assert_eq!(
            before.pins.values().collect::<Vec<_>>(),
            after.pins.values().collect::<Vec<_>>()
        );
        for w in &d.wires {
            for s in w.points.windows(2) {
                assert!(
                    s[0][0] == s[1][0] || s[0][1] == s[1][1],
                    "diagonal segment {s:?}"
                );
            }
        }
    }

    #[test]
    fn rotate_keeps_connections() {
        let mut d = divider();
        let r2 = Item::Part(d.parts.iter().find(|p| p.refdes == "R2").unwrap().id);
        let before = d.connectivity(&ground);
        d.apply(Command::Rotate([r2].into()));
        let after = d.connectivity(&ground);
        assert_eq!(before.pins.len(), after.pins.len());
        assert!(after.unconnected.is_empty(), "{:?}", after.unconnected);
    }

    #[test]
    fn save_load_save_is_byte_identical() {
        let d = divider();
        let a = d.to_json();
        let b = Schematic::from_json(&a).unwrap().to_json();
        assert_eq!(a, b);
    }

    #[test]
    fn window_and_crossing_selection() {
        let d = divider();
        // Left to right around R1 only.
        let enclosed = d.window([30.0, -60.0], [130.0, -20.0]);
        assert!(enclosed.iter().any(|i| matches!(i, Item::Part(_))));
        assert_eq!(
            enclosed
                .iter()
                .filter(|i| matches!(i, Item::Part(_)))
                .count(),
            1
        );
        // Right to left touching R1 and the wires on either side.
        let crossing = d.window([100.0, -50.0], [30.0, -30.0]);
        assert!(
            crossing
                .iter()
                .filter(|i| matches!(i, Item::Wire(_)))
                .count()
                >= 1
        );
    }

    #[test]
    fn refdes_fills_gaps() {
        let mut d = Schematic::default();
        d.apply(Command::AddPart(new_part(
            "core.resistor",
            "resistor",
            "R1".into(),
            [0.0, 0.0],
        )));
        d.apply(Command::AddPart(new_part(
            "core.resistor",
            "resistor",
            "R3".into(),
            [0.0, 0.0],
        )));
        assert_eq!(d.next_refdes("R"), "R2");
    }
}
