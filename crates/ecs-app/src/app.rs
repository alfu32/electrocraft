//! Application state, commands and canvas interaction.

use crate::catalog::{self, Built, Catalog};
use crate::scene::{self, Live, SceneOpts, Viewport};
use crate::theme::c;
use ecs_render::{Renderer, Status};
use ecs_schematic::{Clipboard, Command, History, Id, Item, P2, Probe, Schematic, snap};
use ecs_sim::live::{Command as SimCmd, ScopeSample, SimHandle};
use egui::{
    Color32, Key, Modifiers, PointerButton, Pos2, Rect, Sense, TextureHandle, TextureOptions,
};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Instant;

/// Active canvas tool.
#[derive(Debug, Clone, PartialEq)]
pub enum Tool {
    Select,
    Pan,
    Wire,
    Label,
    Text,
    Probe,
    Place { def: String, rot: u8, mirror: bool },
}

/// A pointer gesture in progress on the canvas.
#[derive(Debug, Clone, PartialEq)]
enum Gesture {
    None,
    Pan {
        start: Pos2,
        offset: [f32; 2],
    },
    /// Pressed on a selected item; becomes a drag past the 3 px threshold.
    Press {
        at: P2,
        screen: Pos2,
        item: Item,
    },
    Drag {
        from: P2,
        delta: P2,
    },
    Window {
        from: P2,
        to: P2,
        base: BTreeSet<Item>,
        op: SelOp,
    },
    /// Momentary button held.
    Hold {
        part: Id,
        state: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SelOp {
    Replace,
    Add,
    Remove,
}

fn sel_op(m: Modifiers) -> SelOp {
    if m.ctrl || m.command {
        SelOp::Remove
    } else if m.shift {
        SelOp::Add
    } else {
        SelOp::Replace
    }
}

/// Every command reachable from menus, ribbon, palette and shortcuts.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Home,
    New,
    Open,
    OpenPath(PathBuf),
    OpenSample(&'static str),
    Save,
    SaveAs,
    Undo,
    Redo,
    Delete,
    Rotate,
    Mirror,
    Copy,
    Cut,
    Paste,
    Duplicate,
    SelectAll,
    Tool(Tool),
    ToggleRun,
    Step,
    Reset,
    Fit,
    ZoomIn,
    ZoomOut,
    ToggleGrid,
    ToggleVolts,
    ToggleDots,
    TogglePanel(&'static str),
    DcOp,
    ClearProbes,
    /// Show the document in tab k.
    SwitchTab(usize),
    /// Close a tab (the active one when None), asking to save changes.
    CloseTab(Option<usize>),
    /// Show the next tab.
    NextTab,
    /// Save every modified document.
    SaveAll,
    /// The settings dialog.
    Settings,
    McpStart,
    McpStop,
    /// Open a microcontroller's program editor.
    EditProgram(String),
    /// Set a part's program (assembly source).
    SetProgram {
        refdes: String,
        code: String,
    },
    /// Set a part number (known numbers set model parameters).
    SetPartNumber {
        refdes: String,
        number: String,
    },
    /// Current-dot speed factor.
    DotSpeed(f64),
    /// Add a scope channel: a formula over net and part codes.
    ScopeAdd {
        name: String,
        expr: String,
    },
    /// Remove scope channels with this formula.
    ScopeRemove(String),
    /// Open the component editor on a new component.
    NewComponent,
    /// Open the component editor for a part's definition.
    EditComponent(String),
    /// Edit a part's inner schematic on the canvas.
    EnterComponent(String),
    /// Return to the enclosing schematic.
    ExitComponent,
    Palette,
    Shortcuts,
    About,
    Quit,
    /// Open an analysis settings dialog.
    Analysis(crate::analysis_ui::Which),
    /// Run an analysis with the current settings (no dialog).
    RunAnalysis(crate::analysis_ui::Which),
    ImportSpice,
    ExportSpice,
    OpenComponentsFolder,
    /// Place a part (document command; MCP-ready).
    Place {
        def: String,
        at: P2,
        rot: u8,
    },
    /// Add a wire through points.
    AddWire(Vec<P2>),
    /// Set a parameter of a part by reference designator.
    SetParam {
        refdes: String,
        name: String,
        value: String,
    },
    /// Operate a control (switch, logic toggle) while running.
    Control {
        refdes: String,
        state: String,
        value: f64,
    },
    /// Delete a part by reference designator.
    DeletePart(String),
    /// Start the datasheet fitting tool (a separate program, spec 8.2).
    DatasheetTool,
    /// Show or hide the Parts panel (online catalogs).
    PartsPanel,
    /// Search the catalogs (opens the Parts panel); provider key or all enabled.
    PartsSearch {
        query: String,
        provider: Option<String>,
    },
    /// Set sourcing fields of parts (comma-separated designators; None keeps a field).
    SetSourcing {
        refdes: String,
        mpn: Option<String>,
        manufacturer: Option<String>,
        preferred_vendor: Option<String>,
    },
    /// Look parts up at every enabled vendor (the selection's parts without
    /// vendor entries when empty).
    FindAtVendors(String),
    /// Refresh a part's vendor entries.
    RefreshSourcing(String),
    /// Clear a part's sourcing.
    ClearSourcing(String),
    /// Export the bill of materials (dialog).
    ExportBom,
    /// Export the bill of materials to a file (plus one per preferred vendor).
    ExportBomPath(PathBuf),
    /// Add a net label.
    Label {
        name: String,
        at: P2,
    },
    /// Add a text note.
    Text {
        text: String,
        at: P2,
    },
    /// Move a part to a position (attached wires stretch).
    MovePart {
        refdes: String,
        at: P2,
    },
    /// Set a part's rotation (quarter turns clockwise) and optionally its mirroring.
    OrientPart {
        refdes: String,
        rot: u8,
        mirror: Option<bool>,
    },
    /// Delete a wire by id.
    DeleteWire(Id),
    /// Select parts by designator (comma-separated; empty clears).
    SelectParts(String),
    /// Save the active document to a path (`.ecs`).
    SavePath(PathBuf),
}

/// A document open in a tab. The active tab's state lives in [`App`]'s own
/// fields; the others wait here (their simulation is paused, and restarts
/// from its operating point when the tab is shown again).
#[derive(Default)]
pub struct DocTab {
    pub doc: Schematic,
    pub history: History,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub selection: BTreeSet<Item>,
    pub vp: Viewport,
    pub stack: Vec<crate::component_ui::Frame>,
    pub scope: crate::scope::Scope,
    pub sim_running: bool,
    /// Fit-to-view frames still due when the tab was left.
    pub fit_frames: u8,
}

impl DocTab {
    /// Tab caption.
    pub fn title(&self) -> String {
        let root = self.stack.first().map_or(&self.doc, |f| &f.doc);
        match self
            .stack
            .first()
            .map_or(self.path.as_ref(), |f| f.path.as_ref())
        {
            Some(p) => p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            None => root.metadata.title.clone(),
        }
    }
    pub fn modified(&self) -> bool {
        self.dirty || self.stack.iter().any(|f| f.dirty)
    }
}

pub struct App {
    pub cat: Catalog,
    /// Open documents; `tabs[active]` is a placeholder for the one in use.
    pub tabs: Vec<DocTab>,
    pub active: usize,
    /// A tab waiting for "Save changes?" before it closes.
    pub confirm_tab: Option<usize>,
    pub doc: Schematic,
    pub history: History,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub home: bool,
    pub has_doc: bool,
    pub tool: Tool,
    pub selection: BTreeSet<Item>,
    pub vp: Viewport,
    gesture: Gesture,
    pub hover: Option<Item>,
    pub hover_world: Option<P2>,
    pub hover_pin: Option<P2>,
    wire_pts: Vec<P2>,
    pub wire_hfirst: bool,
    pub clipboard: Clipboard,
    pub built: Built,
    built_rev: u64,
    /// Generation of the last netlist sent to the simulation thread: its
    /// snapshots match `built` from then on (parameter edits update in place).
    sim_gen: u64,
    pub rev: u64,
    pub sim: SimHandle,
    pub live: Live,
    pub sim_running: bool,
    pub sim_time: f64,
    pub actual_speed: f64,
    pub sim_error: Option<String>,
    pub sim_warnings: Vec<String>,
    pub newton: usize,
    pub linear: bool,
    pub scope: crate::scope::Scope,
    /// Schematics set aside while editing a component's inner schematic.
    pub stack: Vec<crate::component_ui::Frame>,
    pub comp_editor: Option<crate::component_ui::CompEditor>,
    pub program_editor: Option<crate::program_ui::ProgramEditor>,
    /// Problems compiling the document's own components.
    pub local_problems: Vec<String>,
    /// Component prototypes last compiled into the catalog.
    pub local_cache: Option<Vec<ecs_schematic::LocalComponent>>,
    pub renderer: Option<Renderer>,
    texture: Option<TextureHandle>,
    last_list: Option<ecs_render::DisplayList>,
    pub frame_ms: f32,
    /// Smoothed seconds between canvas frames actually shown, and the last one.
    canvas_dt: f64,
    last_canvas_frame: Option<Instant>,
    pub readback_ms: f32,
    pub grid: bool,
    pub volt_colors: bool,
    pub current_dots: bool,
    pub settings: crate::settings::Settings,
    pub show_components: bool,
    pub show_inspector: bool,
    pub show_scopes: bool,
    /// The user opened the scope panel with nothing in it yet.
    pub scopes_pinned: bool,
    pub ribbon_tab: usize,
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_sel: usize,
    pub component_query: String,
    pub dialog: Option<&'static str>,
    pub annotate: Option<(HashMap<String, f64>, HashMap<Id, f64>)>,
    pub status: String,
    pub recent: Vec<PathBuf>,
    started: Instant,
    pub canvas_rect: Rect,
    pub fit_pending: bool,
    /// Frames left to refit while panels settle after loading.
    pub fit_frames: u8,
    pub software_canvas: bool,
    pub inspector_edit: HashMap<String, String>,
    pub thumbnails: HashMap<&'static str, ecs_render::DisplayList>,
    pub home_page: usize,
    pub confirm_close: bool,
    pub allow_close: bool,
    pub analysis: crate::analysis_ui::AnalysisUi,
    pub import: crate::import_ui::ImportUi,
    pub user_lib: crate::packages::UserLibrary,
    pub package_problems: Vec<String>,
    /// Online part catalogs (Parts panel, sourcing lookups).
    pub parts: crate::parts_ui::PartsUi,
    /// Tool calls from an MCP client (`--mcp`), run between frames.
    pub mcp: Option<std::sync::mpsc::Receiver<crate::mcp::Job>>,
    pub mcp_network: crate::mcp_http::NetworkMcp,
}

const MAX_SCOPE: usize = 60_000;

use crate::packages::config_dir;

impl App {
    pub fn new(software_canvas: bool) -> Self {
        let renderer = if software_canvas {
            None
        } else {
            Some(Renderer::spawn(crate::theme::INTER))
        };
        let recent = config_dir()
            .and_then(|d| std::fs::read_to_string(d.join("recent.json")).ok())
            .and_then(|t| serde_json::from_str::<Vec<PathBuf>>(&t).ok())
            .unwrap_or_default();
        let mut app = Self {
            cat: Catalog::builtin(),
            tabs: Vec::new(),
            active: 0,
            confirm_tab: None,
            doc: Schematic::default(),
            history: History::default(),
            path: None,
            dirty: false,
            home: true,
            has_doc: false,
            tool: Tool::Select,
            selection: BTreeSet::new(),
            vp: Viewport::default(),
            gesture: Gesture::None,
            hover: None,
            hover_world: None,
            hover_pin: None,
            wire_pts: Vec::new(),
            wire_hfirst: true,
            clipboard: Clipboard::default(),
            built: Built::default(),
            built_rev: u64::MAX,
            sim_gen: u64::MAX,
            rev: 0,
            sim: SimHandle::spawn(),
            live: Live::default(),
            sim_running: false,
            sim_time: 0.0,
            actual_speed: 0.0,
            sim_error: None,
            sim_warnings: Vec::new(),
            newton: 0,
            linear: true,
            scope: crate::scope::Scope::default(),
            stack: Vec::new(),
            comp_editor: None,
            program_editor: None,
            local_problems: Vec::new(),
            local_cache: None,
            renderer,
            texture: None,
            last_list: None,
            frame_ms: 0.0,
            canvas_dt: 1.0 / 60.0,
            last_canvas_frame: None,
            readback_ms: 0.0,
            grid: true,
            volt_colors: true,
            current_dots: true,
            settings: crate::settings::Settings::default(),
            show_components: true,
            show_inspector: true,
            show_scopes: true,
            scopes_pinned: false,
            ribbon_tab: 0,
            palette_open: false,
            palette_query: String::new(),
            palette_sel: 0,
            component_query: String::new(),
            dialog: None,
            annotate: None,
            status: String::new(),
            recent,
            started: Instant::now(),
            canvas_rect: Rect::NOTHING,
            fit_pending: false,
            fit_frames: 0,
            software_canvas,
            inspector_edit: HashMap::new(),
            thumbnails: HashMap::new(),
            home_page: 0,
            confirm_close: false,
            allow_close: false,
            analysis: Default::default(),
            import: Default::default(),
            user_lib: Default::default(),
            package_problems: Vec::new(),
            parts: Default::default(),
            mcp: None,
            mcp_network: crate::mcp_http::NetworkMcp::default(),
        };
        app.settings = crate::settings::Settings::load();
        app.grid = app.settings.grid;
        app.volt_colors = app.settings.volt_colors;
        app.current_dots = app.settings.current_dots;
        ecs_schematic::symbols::set_resistor_style(app.settings.iec_resistors);
        app.parts.set_consent(&app.settings.catalogs);
        app.package_problems = crate::packages::load_packages(&mut app.cat);
        app.user_lib.poll(&mut app.cat);
        let cat = Catalog::builtin();
        for s in crate::samples::all() {
            let doc = (s.build)(&cat);
            let built = catalog::build(&cat, &doc);
            let mut vp = Viewport::default();
            if let Some(b) = doc.extent() {
                vp.fit(b, [200.0, 120.0], 12.0);
            }
            let opts = SceneOpts {
                grid: false,
                thumbnail: true,
                ..Default::default()
            };
            let mut list =
                scene::build(&doc, &cat, &built, &Live::default(), vp, [200, 120], &opts);
            list.clear = [0x2E, 0x34, 0x40, 255];
            app.thumbnails.insert(s.id, list);
        }
        app
    }

    pub fn title(&self) -> String {
        match self
            .stack
            .first()
            .map_or(self.path.as_ref(), |f| f.path.as_ref())
        {
            Some(p) => p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            None => self.root_doc().metadata.title.clone(),
        }
    }

    /// Whether the active document (or a schematic it encloses) has unsaved changes.
    pub fn modified(&self) -> bool {
        self.dirty || self.stack.iter().any(|f| f.dirty)
    }

    /// Caption and modified flag of tab `k`.
    pub fn tab_info(&self, k: usize) -> (String, bool) {
        if k == self.active {
            (self.title(), self.modified())
        } else {
            (self.tabs[k].title(), self.tabs[k].modified())
        }
    }

    /// File of tab `k`, if saved.
    pub fn tab_path(&self, k: usize) -> Option<PathBuf> {
        if k == self.active {
            self.stack
                .first()
                .map_or(self.path.clone(), |f| f.path.clone())
        } else {
            let t = &self.tabs[k];
            t.stack.first().map_or(t.path.clone(), |f| f.path.clone())
        }
    }

    /// Any open document with unsaved changes.
    pub fn any_modified(&self) -> bool {
        self.has_doc && (0..self.tabs.len()).any(|k| self.tab_info(k).1)
    }

    /// Move the active document out of the working fields.
    fn take_active(&mut self) -> DocTab {
        self.comp_editor = None;
        // The simulation thread counts probe lists globally: keep the counter.
        let generation = self.scope.generation;
        let tab = DocTab {
            doc: std::mem::take(&mut self.doc),
            history: std::mem::take(&mut self.history),
            path: self.path.take(),
            dirty: self.dirty,
            selection: std::mem::take(&mut self.selection),
            vp: self.vp,
            stack: std::mem::take(&mut self.stack),
            scope: std::mem::take(&mut self.scope),
            sim_running: self.sim_running,
            fit_frames: if self.fit_pending {
                self.fit_frames.max(1)
            } else {
                0
            },
        };
        self.scope.generation = generation;
        tab
    }

    /// Make a stored document the working one and restart its simulation.
    fn put_active(&mut self, t: DocTab) {
        self.doc = t.doc;
        self.history = t.history;
        self.path = t.path;
        self.dirty = t.dirty;
        self.selection = t.selection;
        self.vp = t.vp;
        self.fit_pending = t.fit_frames > 0;
        self.fit_frames = t.fit_frames;
        self.stack = t.stack;
        let generation = self.scope.generation;
        self.scope = t.scope;
        self.scope.generation = generation;
        // The simulation thread starts over: resend the scope signals.
        self.scope.signals.clear();
        self.scope.samples.clear();
        self.scope.frozen.clear();
        self.tool = Tool::Select;
        self.gesture = Gesture::None;
        self.wire_pts.clear();
        self.hover = None;
        self.annotate = None;
        self.inspector_edit.clear();
        self.home = false;
        self.has_doc = true;
        self.rev += 1;
        let sim = self.root_doc().simulation.clone();
        self.sim.send(SimCmd::SetSpeed(sim.speed));
        self.sim.send(SimCmd::SetTimeStep(sim.time_step));
        self.sync_sim(true);
        self.sim.send(if t.sim_running {
            SimCmd::Run
        } else {
            SimCmd::Pause
        });
        self.sim_running = t.sim_running;
    }

    /// Show tab `k`.
    pub fn switch_tab(&mut self, k: usize) {
        if !self.has_doc || k >= self.tabs.len() {
            return;
        }
        if k == self.active {
            self.home = false;
            return;
        }
        let cur = self.take_active();
        self.tabs[self.active] = cur;
        let next = std::mem::take(&mut self.tabs[k]);
        self.active = k;
        self.put_active(next);
    }

    /// Make room for a new document in a new tab (the caller loads it).
    fn new_tab(&mut self) {
        if self.has_doc {
            let cur = self.take_active();
            self.tabs[self.active] = cur;
            self.tabs.push(DocTab::default());
            self.active = self.tabs.len() - 1;
        } else {
            self.tabs = vec![DocTab::default()];
            self.active = 0;
        }
    }

    /// Close tab `k`, asking first when it has unsaved changes (unless `force`).
    pub fn close_tab(&mut self, k: usize, force: bool) {
        if !self.has_doc || k >= self.tabs.len() {
            return;
        }
        if !force && self.tab_info(k).1 {
            self.confirm_tab = Some(k);
            return;
        }
        if k != self.active {
            self.tabs.remove(k);
            if k < self.active {
                self.active -= 1;
            }
            return;
        }
        drop(self.take_active());
        self.tabs.remove(k);
        if self.tabs.is_empty() {
            self.has_doc = false;
            self.home = true;
            self.active = 0;
            self.dirty = false;
            self.sim.send(SimCmd::Pause);
            self.sim_running = false;
            self.sim.send(SimCmd::Rebuild {
                netlist: Default::default(),
                generation: self.rev + 1,
            });
            self.rev += 1;
            self.sim_gen = self.rev;
            return;
        }
        self.active = k.min(self.tabs.len() - 1);
        let next = std::mem::take(&mut self.tabs[self.active]);
        self.put_active(next);
    }

    /// Tab already showing this file, if any.
    fn tab_with_path(&self, p: &std::path::Path) -> Option<usize> {
        let same = |q: &Option<PathBuf>| {
            q.as_deref()
                .is_some_and(|q| q == p || q.canonicalize().ok() == p.canonicalize().ok())
        };
        (0..self.tabs.len()).find(|&k| {
            if k == self.active {
                same(
                    &self
                        .stack
                        .first()
                        .map_or(self.path.clone(), |f| f.path.clone()),
                )
            } else {
                same(
                    &self.tabs[k]
                        .stack
                        .first()
                        .map_or(self.tabs[k].path.clone(), |f| f.path.clone()),
                )
            }
        })
    }

    fn remember(&mut self, path: &std::path::Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(8);
        // Tests must not touch the user's recent files.
        if cfg!(test) {
            return;
        }
        if let Some(d) = config_dir() {
            let _ = std::fs::create_dir_all(&d);
            let _ = std::fs::write(
                d.join("recent.json"),
                serde_json::to_string_pretty(&self.recent).unwrap_or_default(),
            );
        }
    }

    /// Open a document in a new tab and start simulating it.
    pub fn load_doc(&mut self, doc: Schematic, path: Option<PathBuf>) {
        self.new_tab();
        self.stack.clear();
        self.comp_editor = None;
        self.doc = doc;
        self.path = path;
        self.history.clear();
        self.selection.clear();
        self.dirty = false;
        self.home = false;
        self.has_doc = true;
        self.tool = Tool::Select;
        self.annotate = None;
        self.rev += 1;
        self.fit_pending = true;
        self.fit_frames = 4;
        self.scope.samples.clear();
        self.sim.send(SimCmd::SetSpeed(self.doc.simulation.speed));
        self.sim
            .send(SimCmd::SetTimeStep(self.doc.simulation.time_step));
        self.sync_sim(true);
        self.migrate_probes();
        self.sim.send(SimCmd::Run);
        self.sim_running = true;
    }

    /// Apply an editing command as one undo step.
    pub fn edit(&mut self, cmd: Command) -> Vec<Item> {
        let created = self.history.apply(&mut self.doc, cmd);
        self.changed();
        created
    }

    fn changed(&mut self) {
        self.rev += 1;
        self.dirty = true;
        self.annotate = None;
        let doc = &self.doc;
        self.selection.retain(|i| match i {
            Item::Part(id) => doc.part(*id).is_some(),
            Item::Wire(id) => doc.wires.iter().any(|w| w.id == *id),
            Item::Label(id) => doc.labels.iter().any(|l| l.id == *id),
            Item::Note(id) => doc.notes.iter().any(|n| n.id == *id),
        });
    }

    /// Rebuild the netlist and tell the simulation thread when the document changed.
    pub fn sync_sim(&mut self, force: bool) {
        if self.built_rev == self.rev && !force {
            return;
        }
        self.built_rev = self.rev;
        let mut force = force;
        if self.local_cache.as_ref() != Some(&self.root_doc().components) {
            let root = self.root_doc().clone();
            let base = self.root_dir();
            self.local_problems =
                crate::components::compile_all(&mut self.cat, &root, base.as_deref());
            self.local_cache = Some(root.components);
            force = true;
        }
        let built = catalog::build(&self.cat, &self.doc);
        let topology_same = !force
            && built.netlist.nodes == self.built.netlist.nodes
            && built.netlist.instances.len() == self.built.netlist.instances.len()
            && built
                .netlist
                .instances
                .iter()
                .zip(&self.built.netlist.instances)
                .all(|(a, b)| {
                    a.name == b.name && std::sync::Arc::ptr_eq(&a.def, &b.def) && a.pins == b.pins
                });
        if topology_same {
            // Parameter edits only: update values in place.
            for (a, b) in built
                .netlist
                .instances
                .iter()
                .zip(&self.built.netlist.instances)
            {
                for (k, v) in &a.params {
                    if b.params.get(k) != Some(v) {
                        self.sim.send(SimCmd::SetParam {
                            inst: a.name.clone(),
                            name: k.clone(),
                            value: *v,
                        });
                    }
                }
                for k in b.params.keys() {
                    if !a.params.contains_key(k)
                        && let Some(d) = a.def.params.iter().find(|p| &p.name == k)
                    {
                        self.sim.send(SimCmd::SetParam {
                            inst: a.name.clone(),
                            name: k.clone(),
                            value: d.default,
                        });
                    }
                }
            }
        } else {
            self.sim.send(SimCmd::Rebuild {
                netlist: built.netlist.clone(),
                generation: self.rev,
            });
            self.sim_gen = self.rev;
            for p in &self.doc.parts {
                for (k, v) in &p.states {
                    self.sim.send(SimCmd::SetState {
                        inst: p.refdes.clone(),
                        name: k.clone(),
                        value: *v,
                    });
                }
            }
        }
        self.built = built;
        self.send_probes();
    }

    /// Recompile the scope channels; resend the signal list when it changed.
    fn send_probes(&mut self) {
        let vars = crate::scope::variables(&self.cat, &self.doc, &self.built);
        let (rt, signals) = crate::scope::compile(&self.doc.scope, &vars);
        let same_exprs = rt.len() == self.scope.rt.len()
            && rt.iter().zip(&self.scope.rt).all(|(a, b)| a.expr == b.expr);
        if signals != self.scope.signals {
            self.scope.signals = signals.clone();
            self.scope.generation = self.scope.generation.wrapping_add(1);
            self.sim.send(SimCmd::SetProbes(signals));
            self.scope.samples.clear();
        } else if !same_exprs {
            self.scope.samples.clear();
        }
        self.scope.rt = rt;
        self.scope.vars = vars;
    }

    /// Canvas badges for channels that show a single net or part quantity.
    fn scope_badges(&self) -> Vec<(P2, Color32, usize)> {
        let mut out = Vec::new();
        for (k, ch) in self.scope.rt.iter().enumerate() {
            let at = match &ch.owner {
                Some(crate::scope::Owner::Net(net)) => self
                    .doc
                    .wires
                    .iter()
                    .find(|w| self.built.wires.get(&w.id) == Some(net))
                    .and_then(|w| w.points.first().copied())
                    .or_else(|| {
                        self.doc
                            .labels
                            .iter()
                            .find(|l| &l.name == net)
                            .map(|l| l.at)
                    })
                    .or_else(|| {
                        self.doc.parts.iter().find_map(|p| {
                            let nets = self.built.pins.get(&p.id)?;
                            let k = nets.iter().position(|n| n == net)?;
                            p.pins().get(k).map(|x| x.1)
                        })
                    }),
                Some(crate::scope::Owner::Part(id)) => self.doc.part(*id).map(|p| match &ch.pin {
                    Some(pin) => p
                        .pins()
                        .into_iter()
                        .find(|x| &x.0 == pin)
                        .map_or(p.pos, |x| x.1),
                    None => {
                        let [lo, hi] = p.bbox();
                        [hi[0], lo[1]]
                    }
                }),
                None => None,
            };
            if let Some(at) = at {
                out.push((at, ch.color, k + 1));
            }
        }
        out
    }

    /// Turn probes saved by older versions into scope channels.
    fn migrate_probes(&mut self) {
        if self.doc.probes.is_empty() {
            return;
        }
        let probes = std::mem::take(&mut self.doc.probes);
        for p in probes {
            let code = match p {
                Probe::Node { at } => self.net_at(at),
                Probe::Current { part } => self.doc.part(part).map(|p| format!("{}.I", p.refdes)),
            };
            if let Some(code) = code
                && !self.doc.scope.iter().any(|c| c.expr == code)
            {
                self.doc.scope.push(ecs_schematic::ScopeChannel {
                    name: code.clone(),
                    expr: code,
                });
            }
        }
        self.send_probes();
    }

    /// Net name at a world point (wire, pin or label).
    pub fn net_at(&self, p: P2) -> Option<String> {
        for w in &self.doc.wires {
            if w.points
                .windows(2)
                .any(|s| ecs_schematic::seg_distance(p, s[0], s[1]) < 1e-3)
            {
                return self.built.wires.get(&w.id).cloned();
            }
        }
        for part in &self.doc.parts {
            for (k, (_, at)) in part.pins().iter().enumerate() {
                if (at[0] - p[0]).abs() < 1e-3 && (at[1] - p[1]).abs() < 1e-3 {
                    return self
                        .built
                        .pins
                        .get(&part.id)
                        .and_then(|v| v.get(k))
                        .cloned();
                }
            }
        }
        None
    }

    /// Pull the newest snapshot and scope samples from the simulation thread.
    pub fn poll_sim(&mut self) {
        let snap = self.sim.snapshot().clone();
        self.sim_running = snap.running;
        self.sim_time = snap.time;
        self.actual_speed = snap.actual_speed;
        self.sim_error = snap.error.clone();
        self.sim_warnings = snap.warnings.clone();
        self.newton = snap.newton_iterations;
        self.linear = snap.linear;
        let mut live = Live {
            running: snap.running,
            ..Default::default()
        };
        if snap.generation == self.sim_gen && !snap.node_voltages.is_empty() {
            for (name, &k) in &self.built.nodes {
                if let Some(v) = snap.node_voltages.get(k) {
                    live.net_v.insert(name.clone(), *v);
                }
            }
            live.v_range = snap
                .node_voltages
                .iter()
                .fold(1.0f64, |m, v| m.max(v.abs()));
            for (k, (name, currents)) in snap.currents.iter().enumerate() {
                if let Some(&id) = self.built.parts.get(name) {
                    live.part_i.insert(id, currents.clone());
                    if let Some(p) = snap.probes.get(k) {
                        live.part_probe.insert(id, p.clone());
                    }
                    if snap.trouble.get(k).copied().unwrap_or(false) {
                        live.trouble.insert(id);
                    }
                }
            }
        }
        self.live = live;
        let mut samples: Vec<ScopeSample> = Vec::new();
        self.sim.drain_samples(&mut samples);
        let n = self.scope.signals.len();
        for s in samples {
            if s.probes == self.scope.generation {
                self.scope.push(s.time, &s.values[..n]);
            }
        }
        let window = if self.scope.window > 0.0 {
            self.scope.window
        } else {
            (self.doc.simulation.speed * 4.0).max(1e-6)
        };
        if let Some(&(t_end, _)) = self.scope.samples.back() {
            while self
                .scope
                .samples
                .front()
                .is_some_and(|(t, _)| *t < t_end - window * 2.2)
            {
                self.scope.samples.pop_front();
            }
        }
        while self.scope.samples.len() > MAX_SCOPE {
            self.scope.samples.pop_front();
        }
    }

    pub fn run(&mut self, cmd: Cmd, ctx: &egui::Context) {
        match cmd {
            Cmd::Home => self.home = !self.home || !self.has_doc,
            Cmd::New => {
                let mut d = Schematic::default();
                d.metadata.title = "Untitled".into();
                self.load_doc(d, None);
                self.vp = Viewport::default();
                self.fit_pending = false;
                if self.canvas_rect.is_positive() {
                    self.vp.offset = [
                        self.canvas_rect.width() / 2.0,
                        self.canvas_rect.height() / 2.0,
                    ];
                }
            }
            Cmd::Open => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Electrocraft schematic", &["ecs"])
                    .pick_file()
                {
                    self.run(Cmd::OpenPath(p), ctx);
                }
            }
            Cmd::OpenPath(p) if self.has_doc && self.tab_with_path(&p).is_some() => {
                if let Some(k) = self.tab_with_path(&p) {
                    self.switch_tab(k);
                    self.status = format!("{} is already open", p.display());
                }
            }
            Cmd::OpenPath(p) => match std::fs::read_to_string(&p)
                .map_err(|e| e.to_string())
                .and_then(|t| Schematic::from_json(&t).map_err(|e| e.to_string()))
            {
                Ok(doc) => {
                    self.remember(&p);
                    self.load_doc(doc, Some(p));
                }
                Err(e) => self.status = format!("Could not open {}: {e}", p.display()),
            },
            Cmd::OpenSample(id) => {
                if let Some(s) = crate::samples::all().into_iter().find(|s| s.id == id) {
                    let doc = (s.build)(&self.cat);
                    self.load_doc(doc, None);
                }
            }
            Cmd::SwitchTab(k) => self.switch_tab(k),
            Cmd::CloseTab(k) => self.close_tab(k.unwrap_or(self.active), false),
            Cmd::NextTab => {
                if self.has_doc && !self.tabs.is_empty() {
                    self.switch_tab((self.active + 1) % self.tabs.len());
                }
            }
            Cmd::SaveAll => {
                for k in 0..self.tabs.len() {
                    if self.tab_info(k).1 {
                        self.switch_tab(k);
                        self.run(Cmd::Save, ctx);
                    }
                }
            }
            Cmd::Save => match self
                .stack
                .first()
                .map_or(self.path.clone(), |f| f.path.clone())
            {
                Some(p) => self.save_to(p),
                None => self.run(Cmd::SaveAs, ctx),
            },
            Cmd::SaveAs => {
                if !self.has_doc {
                    return;
                }
                let name = format!("{}.ecs", self.title());
                if let Some(mut p) = rfd::FileDialog::new()
                    .add_filter("Electrocraft schematic", &["ecs"])
                    .set_file_name(name)
                    .save_file()
                {
                    if p.extension().is_none() {
                        p.set_extension("ecs");
                    }
                    self.save_to(p);
                }
            }
            Cmd::Undo => {
                if self.history.undo(&mut self.doc) {
                    self.changed();
                }
            }
            Cmd::Redo => {
                if self.history.redo(&mut self.doc) {
                    self.changed();
                }
            }
            Cmd::Delete => {
                if !self.selection.is_empty() {
                    let s = std::mem::take(&mut self.selection);
                    self.edit(Command::Delete(s));
                }
            }
            Cmd::Rotate => {
                if let Tool::Place { rot, .. } = &mut self.tool {
                    *rot = (*rot + 1) % 4;
                } else if !self.selection.is_empty() {
                    self.edit(Command::Rotate(self.selection.clone()));
                }
            }
            Cmd::Mirror => {
                if let Tool::Place { mirror, .. } = &mut self.tool {
                    *mirror = !*mirror;
                } else if !self.selection.is_empty() {
                    self.edit(Command::Mirror(self.selection.clone()));
                }
            }
            Cmd::Copy => {
                self.clipboard = Clipboard::copy(&self.doc, &self.selection);
                if let Ok(text) = serde_json::to_string(&self.clipboard) {
                    ctx.copy_text(text);
                }
            }
            Cmd::Cut => {
                self.run(Cmd::Copy, ctx);
                self.run(Cmd::Delete, ctx);
            }
            Cmd::Paste => {
                if !self.clipboard.is_empty() {
                    let at = self.hover_world.unwrap_or([0.0, 0.0]);
                    let anchor = self
                        .clipboard
                        .parts
                        .first()
                        .map(|p| p.pos)
                        .or_else(|| self.clipboard.wires.first().map(|w| w.points[0]))
                        .unwrap_or([0.0, 0.0]);
                    let delta = snap([at[0] - anchor[0], at[1] - anchor[1]]);
                    let created = self.edit(Command::Paste {
                        clip: self.clipboard.clone(),
                        delta,
                    });
                    self.selection = created.into_iter().collect();
                }
            }
            Cmd::Duplicate => {
                let clip = Clipboard::copy(&self.doc, &self.selection);
                if !clip.is_empty() {
                    let created = self.edit(Command::Paste {
                        clip,
                        delta: [40.0, 40.0],
                    });
                    self.selection = created.into_iter().collect();
                }
            }
            Cmd::SelectAll => {
                self.selection = self
                    .doc
                    .parts
                    .iter()
                    .map(|p| Item::Part(p.id))
                    .chain(self.doc.wires.iter().map(|w| Item::Wire(w.id)))
                    .chain(self.doc.labels.iter().map(|l| Item::Label(l.id)))
                    .chain(self.doc.notes.iter().map(|n| Item::Note(n.id)))
                    .collect();
            }
            Cmd::Tool(t) => {
                self.wire_pts.clear();
                self.gesture = Gesture::None;
                self.tool = t;
            }
            Cmd::ToggleRun => self.sim.send(if self.sim_running {
                SimCmd::Pause
            } else {
                SimCmd::Run
            }),
            Cmd::Step => {
                self.sim.send(SimCmd::Pause);
                self.sim.send(SimCmd::Step);
            }
            Cmd::Reset => {
                self.scope.samples.clear();
                self.sim.send(SimCmd::Reset);
                for p in &self.doc.parts {
                    for (k, v) in &p.states {
                        self.sim.send(SimCmd::SetState {
                            inst: p.refdes.clone(),
                            name: k.clone(),
                            value: *v,
                        });
                    }
                }
            }
            Cmd::Fit => self.fit_pending = true,
            Cmd::ZoomIn => {
                let ctr = [
                    self.canvas_rect.width() / 2.0,
                    self.canvas_rect.height() / 2.0,
                ];
                self.vp.zoom_at(ctr, 1.25);
            }
            Cmd::ZoomOut => {
                let ctr = [
                    self.canvas_rect.width() / 2.0,
                    self.canvas_rect.height() / 2.0,
                ];
                self.vp.zoom_at(ctr, 0.8);
            }
            Cmd::ToggleGrid | Cmd::ToggleVolts | Cmd::ToggleDots => {
                match cmd {
                    Cmd::ToggleGrid => self.grid = !self.grid,
                    Cmd::ToggleVolts => self.volt_colors = !self.volt_colors,
                    _ => self.current_dots = !self.current_dots,
                }
                self.settings.grid = self.grid;
                self.settings.volt_colors = self.volt_colors;
                self.settings.current_dots = self.current_dots;
                self.settings.save();
            }
            Cmd::TogglePanel(p) => match p {
                "components" => self.show_components = !self.show_components,
                "inspector" => self.show_inspector = !self.show_inspector,
                _ => {
                    if self.scope.rt.is_empty() && !self.scopes_pinned {
                        self.scopes_pinned = true;
                        self.show_scopes = true;
                    } else {
                        self.show_scopes = !self.show_scopes;
                    }
                }
            },
            Cmd::DcOp => self.dc_op(),
            Cmd::ClearProbes => {
                if !self.doc.scope.is_empty() {
                    self.edit(Command::SetScope(Vec::new()));
                }
            }
            Cmd::ScopeAdd { name, expr } => self.scope_add(&name, &expr),
            Cmd::ScopeRemove(expr) => self.scope_remove(&expr),
            Cmd::NewComponent => self.new_component(),
            Cmd::EditComponent(r) => self.edit_component(&r),
            Cmd::EnterComponent(r) => self.enter_component(&r),
            Cmd::ExitComponent => self.exit_component(),
            Cmd::Palette => {
                self.palette_open = !self.palette_open;
                self.palette_query.clear();
                self.palette_sel = 0;
            }
            Cmd::Shortcuts => self.dialog = Some("shortcuts"),
            Cmd::About => self.dialog = Some("about"),
            Cmd::Settings => self.dialog = Some("settings"),
            Cmd::McpStart => self.mcp_network.start(&self.settings.mcp, ctx),
            Cmd::McpStop => self.mcp_network.stop(),
            Cmd::PartsPanel => self.parts.open = !self.parts.open,
            Cmd::PartsSearch { query, provider } => {
                self.parts.open = true;
                self.parts.text = query;
                self.parts.provider = provider
                    .as_deref()
                    .and_then(ecs_catalog::ProviderId::from_key);
                if let Some(msg) = self.parts.search(ctx) {
                    self.status = msg;
                }
            }
            Cmd::SetSourcing {
                refdes,
                mpn,
                manufacturer,
                preferred_vendor,
            } => {
                let mut changes = Vec::new();
                for r in refdes.split(',').map(str::trim).filter(|r| !r.is_empty()) {
                    match self.doc.parts.iter().find(|p| p.refdes == r) {
                        Some(p) => {
                            let mut s = p.sourcing.clone();
                            if let Some(m) = &mpn {
                                s.mpn = m.trim().to_string();
                            }
                            if let Some(m) = &manufacturer {
                                s.manufacturer = m.trim().to_string();
                            }
                            if let Some(v) = &preferred_vendor {
                                s.preferred_vendor = v.trim().to_string();
                            }
                            changes.push((p.id, s));
                        }
                        None => self.status = format!("No part {r}"),
                    }
                }
                if !changes.is_empty() {
                    self.edit(Command::SetSourcing(changes));
                }
            }
            Cmd::FindAtVendors(refdes) => {
                let ids: Vec<Id> = if refdes.trim().is_empty() {
                    self.selection
                        .iter()
                        .filter_map(|i| match i {
                            Item::Part(id) => Some(*id),
                            _ => None,
                        })
                        .filter(|id| {
                            self.doc
                                .part(*id)
                                .is_some_and(|p| p.sourcing.vendors.is_empty())
                        })
                        .collect()
                } else {
                    refdes
                        .split(',')
                        .filter_map(|r| {
                            self.doc
                                .parts
                                .iter()
                                .find(|p| p.refdes == r.trim())
                                .map(|p| p.id)
                        })
                        .collect()
                };
                self.find_at_vendors(ctx, ids);
            }
            Cmd::RefreshSourcing(r) => {
                match self.doc.parts.iter().find(|p| p.refdes == r).map(|p| p.id) {
                    Some(id) => self.refresh_sourcing(ctx, id),
                    None => self.status = format!("No part {r}"),
                }
            }
            Cmd::ClearSourcing(r) => {
                match self.doc.parts.iter().find(|p| p.refdes == r).map(|p| p.id) {
                    Some(id) => {
                        self.edit(Command::SetSourcing(vec![(
                            id,
                            ecs_schematic::Sourcing::default(),
                        )]));
                    }
                    None => self.status = format!("No part {r}"),
                }
            }
            Cmd::ExportBom => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .set_file_name(format!("{}-bom.csv", self.title()))
                    .save_file()
                {
                    self.run(Cmd::ExportBomPath(p), ctx);
                }
            }
            Cmd::ExportBomPath(p) => {
                self.status = match self.export_bom(&p) {
                    Ok(files) => format!("Exported {} file(s): {}", files.len(), p.display()),
                    Err(e) => format!("Could not export: {e}"),
                };
            }
            Cmd::EditProgram(r) => {
                match self.doc.parts.iter().find(|p| p.refdes == r).map(|p| p.id) {
                    Some(id) => self.open_program(id),
                    None => self.status = format!("No part {r}"),
                }
            }
            Cmd::SetProgram { refdes, code } => match self
                .doc
                .parts
                .iter()
                .find(|p| p.refdes == refdes)
                .map(|p| p.id)
            {
                Some(id) => {
                    self.edit(Command::SetPartCode { part: id, code });
                }
                None => self.status = format!("No part {refdes}"),
            },
            Cmd::SetPartNumber { refdes, number } => match self
                .doc
                .parts
                .iter()
                .find(|p| p.refdes == refdes)
                .map(|p| p.id)
            {
                Some(id) => self.set_part_number(id, &number),
                None => self.status = format!("No part {refdes}"),
            },
            Cmd::DotSpeed(v) => {
                self.settings.dot_speed = v.clamp(0.05, 4.0);
                self.settings.save();
            }
            Cmd::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Cmd::Analysis(which) => self.analysis.dialog = Some(which),
            Cmd::RunAnalysis(which) => {
                let built = catalog::build(&self.cat, &self.doc);
                self.analysis.start(which, built.netlist);
                self.status = format!("{} running…", which.title());
            }
            Cmd::ImportSpice => self.import.show_dialog(),
            Cmd::ExportSpice => {
                let built = catalog::build(&self.cat, &self.doc);
                let text = ecs_spice::export(&built.netlist, &self.title());
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("SPICE netlist", &["cir"])
                    .set_file_name(format!("{}.cir", self.title()))
                    .save_file()
                {
                    self.status = match std::fs::write(&p, text) {
                        Ok(()) => format!("Exported {}", p.display()),
                        Err(e) => format!("Could not export: {e}"),
                    };
                }
            }
            Cmd::OpenComponentsFolder => {
                if let Some(dir) = crate::packages::components_dir() {
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = std::fs::write(dir.join("electrocraft.d.ts"), ecs_script::DTS);
                    for (name, src) in ecs_script::REFERENCE {
                        let p = dir.join(format!("example-{name}"));
                        if !p.exists() {
                            // Reference copies with their own ids so they do not clash.
                            let src = src.replace("id: \"user.", "id: \"example.");
                            // ...and their own names, so the palette shows both.
                            let src = match src.find("name: \"").map(|k| k + 7) {
                                Some(k) => match src[k..].find('"') {
                                    Some(e) => format!(
                                        "{}{} (example){}",
                                        &src[..k],
                                        &src[k..k + e],
                                        &src[k + e..]
                                    ),
                                    None => src,
                                },
                                None => src,
                            };
                            let _ = std::fs::write(p, src);
                        }
                    }
                    let _ = std::process::Command::new(if cfg!(target_os = "windows") {
                        "explorer"
                    } else if cfg!(target_os = "macos") {
                        "open"
                    } else {
                        "xdg-open"
                    })
                    .arg(&dir)
                    .spawn();
                    self.status = format!(
                        "Components folder: {} (saved .js files reload automatically)",
                        dir.display()
                    );
                }
            }
            Cmd::Place { def, at, rot } => match self.cat.def(&def).cloned() {
                Some(d) => {
                    let mut p = self.cat.new_part(&self.doc, &d, at);
                    p.rot = rot % 4;
                    let refdes = p.refdes.clone();
                    let created = self.edit(Command::AddPart(Box::new(p)));
                    self.selection = created.into_iter().collect();
                    self.status = format!("Placed {refdes}");
                }
                None => self.status = format!("Unknown component {def}"),
            },
            Cmd::AddWire(points) => {
                self.edit(Command::AddWire(points.into_iter().map(snap).collect()));
            }
            Cmd::SetParam {
                refdes,
                name,
                value,
            } => match self.doc.parts.iter().find(|p| p.refdes == refdes) {
                Some(p) => {
                    let id = p.id;
                    self.edit(Command::SetParam {
                        part: id,
                        name,
                        value,
                    });
                }
                None => self.status = format!("No part {refdes}"),
            },
            Cmd::Control {
                refdes,
                state,
                value,
            } => {
                if let Some(p) = self.doc.parts.iter_mut().find(|p| p.refdes == refdes) {
                    p.states.insert(state.clone(), value);
                    self.dirty = true;
                    self.sim.send(SimCmd::SetState {
                        inst: refdes,
                        name: state,
                        value,
                    });
                }
            }
            Cmd::DatasheetTool => {
                let exe = std::env::current_exe().ok().and_then(|p| {
                    p.parent().map(|d| {
                        d.join(if cfg!(windows) {
                            "electrocraft-datasheet.exe"
                        } else {
                            "electrocraft-datasheet"
                        })
                    })
                });
                self.status = match exe.map(|e| std::process::Command::new(e).spawn()) {
                    Some(Ok(_)) => "Datasheet tool started; exported components land in the components folder".into(),
                    _ => "The datasheet tool (electrocraft-datasheet) was not found beside Electrocraft".into(),
                };
            }
            Cmd::DeletePart(refdes) => match self.doc.parts.iter().find(|p| p.refdes == refdes) {
                Some(p) => {
                    let id = p.id;
                    self.edit(Command::Delete([Item::Part(id)].into()));
                    self.status = format!("Deleted {refdes}");
                }
                None => self.status = format!("No part {refdes}"),
            },
            Cmd::Label { name, at } => {
                self.edit(Command::AddLabel {
                    at: snap(at),
                    name: name.clone(),
                });
                self.status = format!("Label {name} added");
            }
            Cmd::Text { text, at } => {
                self.edit(Command::AddNote { at: snap(at), text });
                self.status = "Text added".into();
            }
            Cmd::MovePart { refdes, at } => {
                match self.doc.parts.iter().find(|p| p.refdes == refdes) {
                    Some(p) => {
                        let (id, to) = (p.id, snap(at));
                        let delta = [to[0] - p.pos[0], to[1] - p.pos[1]];
                        self.edit(Command::Move {
                            items: [Item::Part(id)].into(),
                            delta,
                        });
                        self.status = format!("Moved {refdes} to {}, {}", to[0], to[1]);
                    }
                    None => self.status = format!("No part {refdes}"),
                }
            }
            Cmd::OrientPart {
                refdes,
                rot,
                mirror,
            } => match self.doc.parts.iter().find(|p| p.refdes == refdes) {
                Some(p) => {
                    let (id, cur, flipped) = (p.id, p.rot % 4, p.mirror);
                    for _ in 0..(rot % 4 + 4 - cur) % 4 {
                        self.edit(Command::Rotate([Item::Part(id)].into()));
                    }
                    if mirror.is_some_and(|m| m != flipped) {
                        self.edit(Command::Mirror([Item::Part(id)].into()));
                    }
                    self.status = format!("Oriented {refdes}");
                }
                None => self.status = format!("No part {refdes}"),
            },
            Cmd::DeleteWire(id) => {
                if self.doc.wires.iter().any(|w| w.id == id) {
                    self.edit(Command::Delete([Item::Wire(id)].into()));
                    self.status = format!("Deleted wire {id}");
                } else {
                    self.status = format!("No wire {id}");
                }
            }
            Cmd::SelectParts(list) => {
                let wanted: Vec<&str> = list
                    .split(',')
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .collect();
                self.selection = self
                    .doc
                    .parts
                    .iter()
                    .filter(|p| wanted.contains(&p.refdes.as_str()))
                    .map(|p| Item::Part(p.id))
                    .collect();
                self.status = format!("{} part(s) selected", self.selection.len());
            }
            Cmd::SavePath(p) => {
                if p.extension().is_some_and(|e| e == "ecs") {
                    self.save_to(p);
                } else {
                    self.status = "Schematics are saved as .ecs files".into();
                }
            }
        }
    }

    fn save_to(&mut self, p: PathBuf) {
        // Inside a component: the outermost document is what is saved.
        while self.stack.len() > 1 {
            self.exit_component();
        }
        if !self.stack.is_empty() {
            let mut root = self.composed_root();
            root.metadata.title = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            match std::fs::write(&p, root.to_json()) {
                Ok(()) => {
                    self.stack[0].dirty = false;
                    self.stack[0].path = Some(p.clone());
                    self.status = format!("Saved {}", p.display());
                    self.remember(&p);
                }
                Err(e) => self.status = format!("Could not save: {e}"),
            }
            return;
        }
        self.doc.metadata.title = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        match std::fs::write(&p, self.doc.to_json()) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", p.display());
                self.remember(&p);
                self.path = Some(p);
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    /// DC operating point, annotated on the schematic until the next edit.
    fn dc_op(&mut self) {
        let built = catalog::build(&self.cat, &self.doc);
        let mut circuit = match ecs_sim::Circuit::compile(&built.netlist) {
            Ok(c) => c,
            Err(e) => {
                self.status = format!("Operating point: {e}");
                return;
            }
        };
        for p in &self.doc.parts {
            if let Some(k) = circuit.instance_index(&p.refdes) {
                for (s, v) in &p.states {
                    circuit.set_state(k, s, *v);
                }
            }
        }
        match circuit.dc_op() {
            Ok(method) => {
                let mut nv = HashMap::new();
                for (name, &k) in &built.nodes {
                    nv.insert(name.clone(), circuit.voltage_of_netlist_node(k));
                }
                let mut pi = HashMap::new();
                for (name, &id) in &built.parts {
                    if let Some(k) = circuit.instance_index(name)
                        && let Some(i) = circuit.branch_currents(k).first()
                    {
                        pi.insert(id, *i);
                    }
                }
                self.annotate = Some((nv, pi));
                self.status = format!(
                    "Operating point solved ({method:?}); annotations stay until the next edit"
                );
            }
            Err(e) => self.status = format!("Operating point: {e}"),
        }
    }

    /// Keyboard shortcuts.
    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let editor_open = self.comp_editor.is_some();
        let single_part = match self.selection.iter().copied().collect::<Vec<_>>()[..] {
            [Item::Part(id)] => self.doc.part(id).map(|p| p.refdes.clone()),
            _ => None,
        };
        let mut cmds = Vec::new();
        ctx.input_mut(|i| {
            let m = Modifiers::COMMAND;
            let ms = Modifiers::COMMAND | Modifiers::SHIFT;
            let mut take = |mods: Modifiers, key: Key, cmd: Cmd, cmds: &mut Vec<Cmd>| {
                if i.consume_key(mods, key) {
                    cmds.push(cmd);
                }
            };
            take(m, Key::K, Cmd::Palette, &mut cmds);
            take(m, Key::N, Cmd::New, &mut cmds);
            take(m, Key::O, Cmd::Open, &mut cmds);
            take(ms, Key::S, Cmd::SaveAs, &mut cmds);
            take(m, Key::S, Cmd::Save, &mut cmds);
            take(m, Key::W, Cmd::CloseTab(None), &mut cmds);
            take(m, Key::Tab, Cmd::NextTab, &mut cmds);
            if typing {
                return;
            }
            take(ms, Key::Z, Cmd::Redo, &mut cmds);
            take(m, Key::Z, Cmd::Undo, &mut cmds);
            take(m, Key::Y, Cmd::Redo, &mut cmds);
            take(m, Key::C, Cmd::Copy, &mut cmds);
            take(m, Key::X, Cmd::Cut, &mut cmds);
            take(m, Key::V, Cmd::Paste, &mut cmds);
            take(m, Key::D, Cmd::Duplicate, &mut cmds);
            take(m, Key::A, Cmd::SelectAll, &mut cmds);
            if let Some(r) = &single_part {
                take(m, Key::E, Cmd::EditComponent(r.clone()), &mut cmds);
                take(m, Key::Enter, Cmd::EnterComponent(r.clone()), &mut cmds);
            }
            if editor_open {
                return;
            }
            let none = Modifiers::NONE;
            take(none, Key::Delete, Cmd::Delete, &mut cmds);
            take(none, Key::Backspace, Cmd::Delete, &mut cmds);
            take(none, Key::R, Cmd::Rotate, &mut cmds);
            take(none, Key::M, Cmd::Mirror, &mut cmds);
            take(none, Key::W, Cmd::Tool(Tool::Wire), &mut cmds);
            take(none, Key::L, Cmd::Tool(Tool::Label), &mut cmds);
            take(none, Key::T, Cmd::Tool(Tool::Text), &mut cmds);
            take(none, Key::P, Cmd::Tool(Tool::Probe), &mut cmds);
            take(none, Key::H, Cmd::Tool(Tool::Pan), &mut cmds);
            take(none, Key::F, Cmd::Fit, &mut cmds);
            take(none, Key::G, Cmd::ToggleGrid, &mut cmds);
            take(none, Key::F5, Cmd::ToggleRun, &mut cmds);
            take(none, Key::F6, Cmd::Step, &mut cmds);
            take(none, Key::F7, Cmd::Reset, &mut cmds);
            take(none, Key::Plus, Cmd::ZoomIn, &mut cmds);
            take(none, Key::Equals, Cmd::ZoomIn, &mut cmds);
            take(none, Key::Minus, Cmd::ZoomOut, &mut cmds);
            if i.consume_key(none, Key::Slash) {
                self.wire_hfirst = !self.wire_hfirst;
            }
            // Read Escape without consuming it so open menus and popups close too.
            if i.key_pressed(Key::Escape) {
                cmds.push(Cmd::Tool(Tool::Select));
            }
        });
        if ctx.input(|i| i.key_pressed(Key::Escape)) && self.palette_open {
            self.palette_open = false;
        }
        for c in cmds {
            if c == Cmd::Tool(Tool::Select) {
                if self.palette_open || self.dialog.is_some() {
                    self.palette_open = false;
                    self.dialog = None;
                    continue;
                }
                if !self.wire_pts.is_empty() || self.gesture != Gesture::None {
                    self.wire_pts.clear();
                    self.gesture = Gesture::None;
                    continue;
                }
                if self.tool == Tool::Select {
                    if self.home && self.has_doc {
                        self.home = false;
                    } else {
                        self.selection.clear();
                    }
                    continue;
                }
            }
            if self.home
                && !matches!(
                    c,
                    Cmd::New
                        | Cmd::Open
                        | Cmd::Palette
                        | Cmd::Save
                        | Cmd::SaveAs
                        | Cmd::CloseTab(_)
                        | Cmd::NextTab
                )
            {
                continue;
            }
            self.run(c, ctx);
        }
    }

    /// The schematic canvas: interaction and drawing.
    pub fn canvas(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        self.canvas_rect = rect;
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        if self.fit_frames > 0 && rect.width() > 50.0 {
            // Panels (scopes, inspector) appear over the first frames: refit.
            self.fit_frames -= 1;
            self.fit_pending = true;
        }
        if self.fit_pending && rect.width() > 50.0 {
            self.fit_pending = false;
            match self.doc.extent() {
                Some(b) => self.vp.fit(b, [rect.width(), rect.height() - 60.0], 90.0),
                None => {
                    self.vp = Viewport {
                        offset: [rect.width() / 2.0, rect.height() / 2.0],
                        zoom: 1.0,
                    }
                }
            }
        }
        let local = |p: Pos2| [p.x - rect.min.x, p.y - rect.min.y];
        let pointer = resp.hover_pos().or(if resp.dragged() {
            ctx.pointer_interact_pos()
        } else {
            None
        });
        let world = pointer.map(|p| self.vp.world(local(p)));
        self.hover_world = world.map(snap);
        let tol = 6.0 / self.vp.zoom as f64;
        self.hover = world.and_then(|w| self.doc.hit(w, tol));
        self.hover_pin =
            world.and_then(|w| self.doc.pin_at(w, 10.0 / self.vp.zoom as f64).map(|x| x.2));

        // Wheel zooms at the pointer (Wirecraft).
        if resp.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x * 0.0);
            let zoom_delta = ctx.input(|i| i.zoom_delta());
            if let Some(p) = pointer {
                if scroll != 0.0 {
                    self.vp.zoom_at(local(p), (scroll * 0.004).exp());
                }
                if zoom_delta != 1.0 {
                    self.vp.zoom_at(local(p), zoom_delta);
                }
            }
        }
        let (pressed_middle, space) = ctx.input(|i| {
            (
                i.pointer.button_pressed(PointerButton::Middle),
                i.key_down(Key::Space),
            )
        });
        let mods = ctx.input(|i| i.modifiers);
        let primary_pressed =
            resp.hovered() && ctx.input(|i| i.pointer.button_pressed(PointerButton::Primary));
        let primary_down = ctx.input(|i| i.pointer.button_down(PointerButton::Primary));
        let primary_released = ctx.input(|i| i.pointer.button_released(PointerButton::Primary));
        let middle_down = ctx.input(|i| i.pointer.button_down(PointerButton::Middle));

        // Panning: middle drag, space+drag, or the Pan tool.
        if resp.hovered()
            && (pressed_middle || (primary_pressed && (space || self.tool == Tool::Pan)))
            && let Some(p) = pointer
        {
            self.gesture = Gesture::Pan {
                start: p,
                offset: self.vp.offset,
            };
        }
        if let Gesture::Pan { start, offset } = self.gesture.clone() {
            if let Some(p) = ctx.pointer_latest_pos() {
                self.vp.offset = [offset[0] + p.x - start.x, offset[1] + p.y - start.y];
            }
            if !middle_down && !primary_down {
                self.gesture = Gesture::None;
            }
        } else if let Some(w) = world {
            let snapped = snap(w);
            if primary_pressed {
                self.press(w, snapped, pointer.unwrap(), mods, &ctx);
            }
            if primary_down {
                self.drag(w, pointer.unwrap());
            }
        }
        if primary_released {
            self.release();
        }
        if resp.secondary_clicked() {
            if self.tool != Tool::Select {
                self.run(Cmd::Tool(Tool::Select), &ctx);
            } else if let Some(h) = self.hover
                && !self.selection.contains(&h)
            {
                self.selection = [h].into();
            }
        }
        if resp.double_clicked() && self.tool == Tool::Wire && self.wire_pts.len() >= 2 {
            let pts = std::mem::take(&mut self.wire_pts);
            self.edit(Command::AddWire(pts));
        }
        self.context_menu(&resp, &ctx);

        // Build the scene.
        let mut opts = SceneOpts {
            grid: self.grid,
            volt_colors: self.volt_colors,
            current_dots: self.current_dots,
            anim_t: self.started.elapsed().as_secs_f64(),
            dot_speed: self.settings.dot_speed,
            electron_flow: self.settings.electron_flow,
            // Shown-frame cadence: egui frames, or the slower canvas pipeline.
            frame_dt: {
                let ui_dt = ui.input(|i| i.unstable_dt) as f64;
                let render_dt = (self.frame_ms + self.readback_ms) as f64 / 1000.0;
                let canvas_dt = if self.renderer.is_some() {
                    self.canvas_dt
                } else {
                    0.0
                };
                ui_dt.max(render_dt).max(canvas_dt).clamp(1.0 / 120.0, 0.25)
            },
            ppp,
            hover: self.hover,
            hover_pin: if matches!(self.tool, Tool::Wire | Tool::Probe) {
                self.hover_pin
            } else {
                None
            },
            selection: self.selection.clone(),
            annotate: self.annotate.clone(),
            probes: self.scope_badges(),
            ..Default::default()
        };
        let mut doc_view = std::borrow::Cow::Borrowed(&self.doc);
        match &self.gesture {
            Gesture::Window { from, to, .. } => opts.window = Some((*from, *to)),
            Gesture::Drag { delta, .. } if *delta != [0.0, 0.0] => {
                let mut preview = self.doc.clone();
                preview.apply(Command::Move {
                    items: self.selection.clone(),
                    delta: *delta,
                });
                doc_view = std::borrow::Cow::Owned(preview);
            }
            _ => {}
        }
        if self.tool == Tool::Wire
            && let (Some(last), Some(h)) = (self.wire_pts.last(), self.hover_world)
        {
            let target = self.hover_pin.unwrap_or(h);
            let mut pts = self.wire_pts.clone();
            pts.extend(
                ecs_schematic::route(*last, target, self.wire_hfirst)
                    .into_iter()
                    .skip(1),
            );
            opts.wire_preview = pts;
        }
        if let (Tool::Place { def, rot, mirror }, Some(h)) = (&self.tool, self.hover_world)
            && resp.hovered()
            && let Some(d) = self.cat.def(def)
        {
            let mut g = self.cat.new_part(&self.doc, d, h);
            g.rot = *rot;
            g.mirror = *mirror;
            opts.ghost = Some(g);
        }
        if self.annotate.is_some() || !self.live.net_v.is_empty() {
            // keep built nets current for annotations
        }
        let size_px = [
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        ];
        let list = scene::build(
            &doc_view,
            &self.cat,
            &self.built,
            &self.live,
            self.vp,
            size_px,
            &opts,
        );
        let painter = ui.painter_at(rect);
        let raylib_ready = self
            .renderer
            .as_ref()
            .is_some_and(|r| r.status() == Status::Ready);
        if let Some(r) = &self.renderer
            && let Status::Failed(e) = r.status()
        {
            self.status = format!("Raylib canvas unavailable ({e}); using the software canvas");
            self.renderer = None;
        }
        if raylib_ready {
            let r = self.renderer.as_ref().unwrap();
            if self.last_list.as_ref() != Some(&list) {
                r.submit(list.clone());
                self.last_list = Some(list.clone());
            }
            if let Some(frame) = r.take_frame() {
                let now = Instant::now();
                if let Some(prev) = self.last_canvas_frame {
                    let dt = now.duration_since(prev).as_secs_f64().min(0.5);
                    self.canvas_dt = 0.8 * self.canvas_dt + 0.2 * dt;
                }
                self.last_canvas_frame = Some(now);
                self.frame_ms = frame.render_ms;
                self.readback_ms = frame.readback_ms;
                let img = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &frame.rgba,
                );
                match &mut self.texture {
                    Some(t) => t.set(img, TextureOptions::NEAREST),
                    None => {
                        self.texture =
                            Some(ctx.load_texture("canvas", img, TextureOptions::NEAREST))
                    }
                }
            }
            match &self.texture {
                Some(t) => {
                    let uv = Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));
                    let size = t.size_vec2() / ppp;
                    painter.image(
                        t.id(),
                        Rect::from_min_size(rect.min, size),
                        uv,
                        Color32::WHITE,
                    );
                }
                None => scene::paint_egui(&painter, &list, rect.min, ppp, true),
            }
            // Keep animating while the frame is behind or dots move.
            ctx.request_repaint();
        } else {
            scene::paint_egui(&painter, &list, rect.min, ppp, true);
            if self.sim_running || self.gesture != Gesture::None {
                ctx.request_repaint();
            }
        }
        if self.sim_running {
            ctx.request_repaint();
        }
        // Hover tooltip with live values.
        if let (Some(h), Some(p)) = (self.hover, pointer)
            && self.gesture == Gesture::None
            && let Some(text) = self.hover_text(h)
        {
            let at = p + egui::vec2(16.0, 14.0);
            let galley = painter.layout_no_wrap(text, crate::theme::regular(12.0), c::TEXT);
            let r = Rect::from_min_size(at, galley.size()).expand2(egui::vec2(8.0, 5.0));
            painter.rect_filled(r, 5.0, c::TITLEBAR.gamma_multiply(0.96));
            painter.rect_stroke(
                r,
                5.0,
                egui::Stroke::new(1.0, c::PANEL_BORDER),
                egui::StrokeKind::Inside,
            );
            painter.galley(at, galley, c::TEXT);
        }
    }

    fn hover_text(&self, item: Item) -> Option<String> {
        match item {
            Item::Wire(id) => {
                let net = self.built.wires.get(&id)?;
                let v = self
                    .live
                    .net_v
                    .get(net)
                    .or(self.annotate.as_ref().and_then(|a| a.0.get(net)));
                Some(match v {
                    Some(v) => format!(
                        "{} = {}",
                        if net.starts_with('N') { "net" } else { net },
                        ecs_units::format_digits(*v, "V", 4)
                    ),
                    None => format!("net {net}"),
                })
            }
            Item::Part(id) => {
                let part = self.doc.part(id)?;
                let def = self.cat.def(&part.def)?;
                let mut s = format!("{}  {}", part.refdes, def.meta.name);
                if let Some(nets) = self.built.pins.get(&id)
                    && nets.len() == 2
                {
                    let va = self.live.net_v.get(&nets[0]).copied().unwrap_or(0.0);
                    let vb = self.live.net_v.get(&nets[1]).copied().unwrap_or(0.0);
                    if self.live.running || !self.live.net_v.is_empty() {
                        s.push_str(&format!(
                            "\nV = {}",
                            ecs_units::format_digits(va - vb, "V", 4)
                        ));
                    }
                }
                if let Some(vals) = self.live.part_probe.get(&id) {
                    for (p, v) in def.probes.iter().zip(vals) {
                        if p.name == "brightness" {
                            s.push_str(&format!("\nbrightness = {:.0}%", v * 100.0));
                        } else {
                            s.push_str(&format!(
                                "\n{} = {}",
                                p.name,
                                ecs_units::format_digits(*v, &p.unit, 4)
                            ));
                        }
                    }
                }
                Some(s)
            }
            Item::Label(id) => self.doc.labels.iter().find(|l| l.id == id).map(|l| {
                let v = self.live.net_v.get(&l.name);
                match v {
                    Some(v) => format!("{} = {}", l.name, ecs_units::format_digits(*v, "V", 4)),
                    None => l.name.clone(),
                }
            }),
            Item::Note(_) => None,
        }
    }

    fn press(&mut self, w: P2, snapped: P2, screen: Pos2, mods: Modifiers, ctx: &egui::Context) {
        match self.tool.clone() {
            Tool::Select | Tool::Pan => {
                // Clicking a switch or button operates it (iCircuit feel).
                if let Some(Item::Part(id)) = self.hover
                    && !mods.shift
                    && !mods.ctrl
                    && let Some(part) = self.doc.part(id)
                    && let Some(def) = self.cat.def(&part.def)
                    && let Some(ctrl) = def.controls.first()
                    && self.sim_running
                {
                    match ctrl {
                        ecs_model::ControlDef::Toggle { state, .. } => {
                            let v = if part.states.get(state).copied().unwrap_or(0.0) > 0.5 {
                                0.0
                            } else {
                                1.0
                            };
                            let (inst, state) = (part.refdes.clone(), state.clone());
                            // Operating a control is not an edit: no undo step, no rebuild.
                            if let Some(p) = self.doc.part_mut(id) {
                                p.states.insert(state.clone(), v);
                            }
                            self.dirty = true;
                            self.sim.send(SimCmd::SetState {
                                inst,
                                name: state,
                                value: v,
                            });
                            self.selection = [Item::Part(id)].into();
                            return;
                        }
                        ecs_model::ControlDef::Momentary { state, .. } => {
                            self.sim.send(SimCmd::SetState {
                                inst: part.refdes.clone(),
                                name: state.clone(),
                                value: 1.0,
                            });
                            self.gesture = Gesture::Hold {
                                part: id,
                                state: state.clone(),
                            };
                            if let Some(p) = self.doc.part_mut(id) {
                                p.states.insert(state.clone(), 1.0);
                            }
                            return;
                        }
                        _ => {}
                    }
                }
                match self.hover {
                    Some(item) => {
                        let op = sel_op(mods);
                        match op {
                            SelOp::Add => {
                                self.selection.insert(item);
                            }
                            SelOp::Remove => {
                                self.selection.remove(&item);
                            }
                            SelOp::Replace => {
                                if !self.selection.contains(&item) {
                                    self.selection = [item].into();
                                }
                                self.gesture = Gesture::Press {
                                    at: snapped,
                                    screen,
                                    item,
                                };
                            }
                        }
                    }
                    None => {
                        let op = sel_op(mods);
                        let base = self.selection.clone();
                        if op == SelOp::Replace {
                            self.selection.clear();
                        }
                        self.gesture = Gesture::Window {
                            from: w,
                            to: w,
                            base,
                            op,
                        };
                    }
                }
            }
            Tool::Wire => {
                let target = self.hover_pin.unwrap_or(snapped);
                if let Some(&last) = self.wire_pts.last() {
                    let seg = ecs_schematic::route(last, target, self.wire_hfirst);
                    self.wire_pts.extend(seg.into_iter().skip(1));
                    // Ending on a pin or another wire finishes the wire.
                    let on_wire = self.doc.wires.iter().any(|wr| {
                        wr.points
                            .windows(2)
                            .any(|s| ecs_schematic::seg_distance(target, s[0], s[1]) < 1e-6)
                    });
                    if self.hover_pin.is_some() || on_wire {
                        let pts = std::mem::take(&mut self.wire_pts);
                        self.edit(Command::AddWire(pts));
                    }
                } else {
                    self.wire_pts.push(target);
                }
            }
            Tool::Label => {
                let n = self.doc.labels.len() + 1;
                let created = self.edit(Command::AddLabel {
                    at: snapped,
                    name: format!("NET{n}"),
                });
                self.selection = created.into_iter().collect();
                self.tool = Tool::Select;
            }
            Tool::Text => {
                let created = self.edit(Command::AddNote {
                    at: snapped,
                    text: "Note".into(),
                });
                self.selection = created.into_iter().collect();
                self.tool = Tool::Select;
            }
            Tool::Probe => {
                let code = match self.hover {
                    Some(Item::Part(id)) => self.doc.part(id).and_then(|p| {
                        [format!("{}.I", p.refdes), format!("{}.V", p.refdes)]
                            .into_iter()
                            .find(|c| self.scope.vars.get(c).is_some())
                    }),
                    Some(Item::Wire(_)) | None => self.net_at(self.hover_pin.unwrap_or(snapped)),
                    _ => None,
                };
                match code {
                    Some(c) if self.scope.vars.get(&c).is_some() => {
                        if self.doc.scope.iter().any(|ch| ch.expr == c) {
                            self.status = format!("{c} is already on the scope");
                        } else {
                            self.scope_add(&c, &c);
                        }
                    }
                    Some(c) => {
                        self.status = format!("{c} cannot be scoped (ground or not simulated)")
                    }
                    None => self.status = "Click a wire, pin or part to scope it".into(),
                }
            }
            Tool::Place { def, rot, mirror } => {
                if let Some(d) = self.cat.def(&def).cloned() {
                    let mut p = self.cat.new_part(&self.doc, &d, snapped);
                    p.rot = rot;
                    p.mirror = mirror;
                    let created = self.edit(Command::AddPart(Box::new(p)));
                    self.selection = created.into_iter().collect();
                }
                let _ = ctx;
            }
        }
    }

    fn drag(&mut self, w: P2, screen: Pos2) {
        match self.gesture.clone() {
            Gesture::Press {
                at,
                screen: s0,
                item,
            } => {
                if (screen - s0).length() >= 3.0 {
                    if !self.selection.contains(&item) {
                        self.selection = [item].into();
                    }
                    let d = snap([w[0] - at[0], w[1] - at[1]]);
                    self.gesture = Gesture::Drag { from: at, delta: d };
                }
            }
            Gesture::Drag { from, .. } => {
                self.gesture = Gesture::Drag {
                    from,
                    delta: snap([w[0] - from[0], w[1] - from[1]]),
                };
            }
            Gesture::Window { from, base, op, .. } => {
                let hits = self.doc.window(from, w);
                self.selection = match op {
                    SelOp::Replace => hits,
                    SelOp::Add => base.union(&hits).copied().collect(),
                    SelOp::Remove => base.difference(&hits).copied().collect(),
                };
                self.gesture = Gesture::Window {
                    from,
                    to: w,
                    base,
                    op,
                };
            }
            _ => {}
        }
    }

    fn release(&mut self) {
        match std::mem::replace(&mut self.gesture, Gesture::None) {
            Gesture::Drag { delta, .. } if delta != [0.0, 0.0] => {
                self.edit(Command::Move {
                    items: self.selection.clone(),
                    delta,
                });
            }
            Gesture::Press { item, .. } => {
                self.selection = [item].into();
            }
            Gesture::Hold { part, state } => {
                if let Some(p) = self.doc.part_mut(part) {
                    p.states.insert(state.clone(), 0.0);
                    let inst = p.refdes.clone();
                    self.sim.send(SimCmd::SetState {
                        inst,
                        name: state,
                        value: 0.0,
                    });
                }
            }
            Gesture::Pan { .. } => {}
            other => {
                if !matches!(
                    other,
                    Gesture::Window { .. } | Gesture::None | Gesture::Drag { .. }
                ) {
                    self.gesture = other;
                }
            }
        }
    }

    fn context_menu(&mut self, resp: &egui::Response, ctx: &egui::Context) {
        let mut chosen: Option<Cmd> = None;
        resp.context_menu(|ui| {
            ui.set_min_width(210.0);
            if !self.selection.is_empty() {
                for (label, cmd, key) in [
                    ("Rotate", Cmd::Rotate, "R"),
                    ("Mirror", Cmd::Mirror, "M"),
                    ("Duplicate", Cmd::Duplicate, "Ctrl+D"),
                    ("Copy", Cmd::Copy, "Ctrl+C"),
                    ("Delete", Cmd::Delete, "Del"),
                ] {
                    if crate::chrome::menu_item(ui, label, key) {
                        chosen = Some(cmd);
                        ui.close();
                    }
                }
                if let [Item::Part(id)] = self.selection.iter().copied().collect::<Vec<_>>()[..]
                    && let Some(p) = self.doc.part(id)
                {
                    let refdes = p.refdes.clone();
                    if crate::chrome::menu_item(ui, "Edit Component…", "Ctrl+E") {
                        chosen = Some(Cmd::EditComponent(refdes.clone()));
                        ui.close();
                    }
                    let inner = self.root_doc().components.iter().any(|c| {
                        c.id == p.def
                            && matches!(
                                c.implementation,
                                ecs_schematic::Implementation::Schematic { .. }
                                    | ecs_schematic::Implementation::ExternalSchematic { .. }
                            )
                    });
                    if inner && crate::chrome::menu_item(ui, "Enter Component", "Ctrl+Enter") {
                        chosen = Some(Cmd::EnterComponent(refdes));
                        ui.close();
                    }
                }
                ui.separator();
            }
            if crate::chrome::menu_item(ui, "New Component…", "") {
                chosen = Some(Cmd::NewComponent);
                ui.close();
            }
            if !self.stack.is_empty()
                && crate::chrome::menu_item(ui, "Back to enclosing schematic", "")
            {
                chosen = Some(Cmd::ExitComponent);
                ui.close();
            }
            if !self.clipboard.is_empty() && crate::chrome::menu_item(ui, "Paste", "Ctrl+V") {
                chosen = Some(Cmd::Paste);
                ui.close();
            }
            ui.label(egui::RichText::new("Place").color(c::TEXT_FAINT).small());
            for id in [
                "core.resistor",
                "core.capacitor",
                "core.inductor",
                "core.ground",
                "core.vdc",
                "core.vsine",
                "core.switch",
                "core.led",
                "core.diode",
                "core.npn",
            ] {
                if let Some(d) = self.cat.def(id) {
                    let name = d.meta.name.clone();
                    if crate::chrome::menu_item(ui, &name, "") {
                        chosen = Some(Cmd::Tool(Tool::Place {
                            def: id.into(),
                            rot: 0,
                            mirror: false,
                        }));
                        ui.close();
                    }
                }
            }
            ui.separator();
            for (label, cmd, key) in [
                ("Wire", Cmd::Tool(Tool::Wire), "W"),
                ("Net label", Cmd::Tool(Tool::Label), "L"),
                ("Probe", Cmd::Tool(Tool::Probe), "P"),
                ("Fit to view", Cmd::Fit, "F"),
            ] {
                if crate::chrome::menu_item(ui, label, key) {
                    chosen = Some(cmd);
                    ui.close();
                }
            }
        });
        if let Some(cmd) = chosen {
            self.run(cmd, ctx);
        }
    }
}

#[cfg(test)]
mod tab_tests {
    use super::*;

    fn sample(app: &App, id: &str) -> Schematic {
        let s = crate::samples::all()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap();
        (s.build)(&app.cat)
    }

    #[test]
    fn documents_open_in_tabs() {
        let ctx = egui::Context::default();
        let mut app = App::new(true);
        let a = sample(&app, "astable");
        app.load_doc(a, None);
        let b = sample(&app, "rc-lowpass");
        app.load_doc(b, None);
        assert_eq!((app.tabs.len(), app.active), (2, 1));
        assert_eq!(app.title(), "RC Low-pass");
        assert_eq!(app.tab_info(0).0, "Astable Multivibrator");
        // Edit the second, switch away and back: edits and undo history stay.
        let n = app.doc.parts.len();
        let first = app.doc.parts[0].id;
        app.edit(Command::Delete([Item::Part(first)].into()));
        app.switch_tab(0);
        assert_eq!(app.title(), "Astable Multivibrator");
        assert!(!app.modified() && app.tab_info(1).1);
        app.switch_tab(1);
        assert_eq!(app.doc.parts.len(), n - 1);
        app.run(Cmd::Undo, &ctx);
        assert_eq!(app.doc.parts.len(), n);
        // The + menu's New opens a third tab.
        app.run(Cmd::New, &ctx);
        assert_eq!((app.tabs.len(), app.active), (3, 2));
        // Closing a modified tab asks first.
        app.switch_tab(1);
        app.edit(Command::Delete([Item::Part(first)].into()));
        app.close_tab(1, false);
        assert_eq!(app.confirm_tab, Some(1));
        assert_eq!(app.tabs.len(), 3);
        app.confirm_tab = None;
        app.close_tab(1, true);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.title(), "Untitled");
        // A file that is already open is switched to, not opened twice.
        let dir = std::env::temp_dir().join(format!("ecs-tabs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("divider.ecs");
        std::fs::write(&path, sample(&app, "divider").to_json()).unwrap();
        app.run(Cmd::OpenPath(path.clone()), &ctx);
        assert_eq!(app.tabs.len(), 3);
        app.switch_tab(0);
        app.run(Cmd::OpenPath(path.clone()), &ctx);
        assert_eq!((app.tabs.len(), app.active), (3, 2));
        std::fs::remove_dir_all(&dir).ok();
        // Closing every tab returns to the Start screen.
        for _ in 0..3 {
            app.close_tab(app.active, true);
        }
        assert!(!app.has_doc && app.home && app.tabs.is_empty());
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Poll the simulation thread until live values arrive (or give up).
    fn wait_live(app: &mut App) -> bool {
        for _ in 0..300 {
            app.sync_sim(false);
            app.poll_sim();
            if !app.live.net_v.is_empty() && !app.live.part_i.is_empty() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn live_values_survive_parameter_edits_and_new_parts() {
        let mut app = App::new(true);
        let s = crate::samples::all()
            .into_iter()
            .find(|s| s.id == "astable")
            .unwrap();
        let doc = (s.build)(&app.cat);
        app.load_doc(doc, None);
        assert!(wait_live(&mut app), "no live values after loading");
        // A parameter edit updates the running circuit in place.
        let r1 = app.doc.parts.iter().find(|p| p.refdes == "R1").unwrap().id;
        app.edit(Command::SetParam {
            part: r1,
            name: "R".into(),
            value: "560".into(),
        });
        app.live = Live::default();
        assert!(
            wait_live(&mut app),
            "live values stopped after a parameter edit"
        );
        // A part that is not wired in leaves the netlist as it was.
        let def = app.cat.def("core.resistor").unwrap().clone();
        let p = app.cat.new_part(&app.doc, &def, [2000.0, 2000.0]);
        app.edit(Command::AddPart(Box::new(p)));
        app.live = Live::default();
        assert!(
            wait_live(&mut app),
            "live values stopped after adding a part"
        );
    }
}
