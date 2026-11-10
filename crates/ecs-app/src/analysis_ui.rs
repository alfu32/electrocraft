//! Analysis commands in the UI (spec 4.6, 9.5, phase P7): settings dialogs,
//! background jobs with progress and cancel, and plot windows.

use crate::catalog::Built;
use crate::theme::{self, c};
use ecs_sim::analysis::{self, Kind, Progress, ResultSet};
use ecs_sim::{Method, Netlist, Options};
use egui::{Color32, RichText};
use egui_plot::{Legend, Line, Plot, PlotPoints, VLine};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

/// Which analysis a dialog configures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Op,
    Dc,
    Tran,
    Ac,
}

impl Which {
    pub fn title(self) -> &'static str {
        match self {
            Which::Op => "DC operating point",
            Which::Dc => "DC sweep",
            Which::Tran => "Transient",
            Which::Ac => "AC sweep",
        }
    }
}

/// Dialog fields, typed in engineering notation.
#[derive(Debug, Clone)]
pub struct Settings {
    pub dc_inst: String,
    pub dc_param: String,
    pub dc_start: String,
    pub dc_stop: String,
    pub dc_points: String,
    pub tran_stop: String,
    pub tran_hmax: String,
    pub ac_start: String,
    pub ac_stop: String,
    pub ac_ppd: String,
    pub step: bool,
    pub step_inst: String,
    pub step_param: String,
    pub step_values: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dc_inst: String::new(),
            dc_param: String::new(),
            dc_start: "0".into(),
            dc_stop: "5".into(),
            dc_points: "51".into(),
            tran_stop: "10m".into(),
            tran_hmax: "10u".into(),
            ac_start: "1".into(),
            ac_stop: "1meg".into(),
            ac_ppd: "20".into(),
            step: false,
            step_inst: String::new(),
            step_param: String::new(),
            step_values: "1k 2k 5k".into(),
        }
    }
}

struct Job {
    title: String,
    which: Which,
    progress: Arc<Progress>,
    handle: Option<JoinHandle<Vec<Result<ResultSet, String>>>>,
}

/// An open plot window.
pub struct PlotWin {
    pub id: u64,
    pub title: String,
    pub results: Vec<ResultSet>,
    pub selected: BTreeSet<String>,
    pub open: bool,
    pub cursors: [Option<f64>; 2],
    pub fft: bool,
    rect: egui::Rect,
    pub status: String,
}

/// Analysis UI state.
#[derive(Default)]
pub struct AnalysisUi {
    pub dialog: Option<Which>,
    pub settings: Settings,
    jobs: Vec<Job>,
    pub plots: Vec<PlotWin>,
    next_id: u64,
    pub errors: Vec<String>,
}

fn val(t: &str) -> Result<f64, String> {
    ecs_units::parse(t).map_err(|e| e.to_string())
}

fn run_one(which: Which, n: &Netlist, s: &Settings, p: &Progress) -> Result<ResultSet, String> {
    let o = Options::analysis();
    let r = match which {
        Which::Op => analysis::op(n, o).map(|r| r.0),
        Which::Dc => {
            let (a, b) = (val(&s.dc_start)?, val(&s.dc_stop)?);
            let k = val(&s.dc_points)?.max(2.0) as usize;
            let values: Vec<f64> = (0..k)
                .map(|i| a + (b - a) * i as f64 / (k - 1) as f64)
                .collect();
            analysis::dc_sweep(n, &s.dc_inst, &s.dc_param, &values, o, p)
        }
        Which::Tran => analysis::transient(
            n,
            val(&s.tran_stop)?,
            val(&s.tran_hmax)?,
            o,
            Method::Trapezoidal,
            p,
        ),
        Which::Ac => analysis::ac(
            n,
            &analysis::log_points(
                val(&s.ac_start)?,
                val(&s.ac_stop)?,
                val(&s.ac_ppd)? as usize,
            ),
            o,
            p,
        ),
    };
    r.map_err(|e| e.to_string())
}

/// Default traces: named nets first, then other node voltages, at most six.
fn default_selection(r: &ResultSet) -> BTreeSet<String> {
    let mut names: Vec<&String> = r
        .traces
        .iter()
        .map(|t| &t.name)
        .filter(|n| n.starts_with("V(") && !n.contains('.'))
        .collect();
    names.sort_by_key(|n| (n.starts_with("V(N0"), n.len()));
    names.into_iter().take(6).cloned().collect()
}

impl AnalysisUi {
    /// Start an analysis on a background thread.
    pub fn start(&mut self, which: Which, netlist: Netlist) {
        let s = self.settings.clone();
        let progress = Arc::new(Progress::default());
        let p2 = progress.clone();
        let title = which.title().to_string();
        let handle = std::thread::Builder::new()
            .name("electrocraft-analysis".into())
            .spawn(move || {
                if s.step && !s.step_inst.is_empty() {
                    let values: Result<Vec<f64>, String> =
                        s.step_values.split_whitespace().map(val).collect();
                    match values {
                        Ok(values) => analysis::param_step(
                            &netlist,
                            &s.step_inst,
                            &s.step_param,
                            &values,
                            |n| {
                                run_one(which, n, &s, &Progress::default()).map_err(|e| {
                                    ecs_sim::SimError::Component {
                                        instance: "analysis".into(),
                                        message: e,
                                    }
                                })
                            },
                        )
                        .into_iter()
                        .map(|r| r.map_err(|e| e.to_string()))
                        .collect(),
                        Err(e) => vec![Err(e)],
                    }
                } else {
                    vec![run_one(which, &netlist, &s, &p2)]
                }
            })
            .ok();
        self.jobs.push(Job {
            title,
            which,
            progress,
            handle,
        });
    }

    /// Collect finished jobs into plot windows. Returns true while jobs run.
    pub fn poll(&mut self) -> bool {
        let mut done = Vec::new();
        for (k, j) in self.jobs.iter().enumerate() {
            if j.handle.as_ref().is_some_and(|h| h.is_finished()) {
                done.push(k);
            }
        }
        for k in done.into_iter().rev() {
            let mut j = self.jobs.remove(k);
            let results = j
                .handle
                .take()
                .and_then(|h| h.join().ok())
                .unwrap_or_default();
            let (ok, errs): (Vec<_>, Vec<_>) = results.into_iter().partition(|r| r.is_ok());
            self.errors.extend(
                errs.into_iter()
                    .filter_map(|e| e.err())
                    .map(|e| format!("{}: {e}", j.title)),
            );
            let ok: Vec<ResultSet> = ok.into_iter().filter_map(|r| r.ok()).collect();
            if !ok.is_empty() && j.which != Which::Op {
                self.open_plot(j.title.clone(), ok, false);
            } else if let Some(r) = ok.first() {
                self.open_plot(j.title.clone(), vec![r.clone()], false);
            }
        }
        !self.jobs.is_empty()
    }

    fn open_plot(&mut self, title: String, results: Vec<ResultSet>, fft: bool) {
        self.next_id += 1;
        let selected = results.first().map(default_selection).unwrap_or_default();
        self.plots.push(PlotWin {
            id: self.next_id,
            title,
            results,
            selected,
            open: true,
            cursors: [None, None],
            fft,
            rect: egui::Rect::NOTHING,
            status: String::new(),
        });
    }

    /// Progress strip for running jobs (shown in the Simulation bar).
    pub fn jobs_ui(&mut self, ui: &mut egui::Ui) {
        for j in &self.jobs {
            let f = j.progress.done.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0;
            ui.add(egui::ProgressBar::new(f).desired_width(110.0).text(format!(
                "{} {:.0}%",
                j.title,
                f * 100.0
            )));
            if ui.small_button("Cancel").clicked() {
                j.progress
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// The settings dialog. Returns the analysis to start.
    pub fn dialog_ui(&mut self, ctx: &egui::Context, built: &Built) -> Option<Which> {
        let which = self.dialog?;
        let mut open = true;
        let mut run = None;
        // Instances and their parameters for the pickers.
        let insts: Vec<(String, Vec<String>)> = built
            .netlist
            .instances
            .iter()
            .map(|i| {
                (
                    i.name.clone(),
                    i.def
                        .params
                        .iter()
                        .filter(|p| !matches!(p.kind, ecs_model::ParamKind::Enum(_)))
                        .map(|p| p.name.clone())
                        .collect(),
                )
            })
            .collect();
        let s = &mut self.settings;
        egui::Window::new(RichText::new(which.title()).font(theme::bold(14.0)))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                let pick = |ui: &mut egui::Ui, id: &str, inst: &mut String, param: &mut String| {
                    egui::ComboBox::from_id_salt(format!("{id}-inst")).width(110.0).selected_text(inst.clone()).show_ui(ui, |ui| {
                        for (n, _) in &insts {
                            ui.selectable_value(inst, n.clone(), n);
                        }
                    });
                    let params = insts.iter().find(|(n, _)| n == inst).map(|(_, p)| p.clone()).unwrap_or_default();
                    if !params.contains(param) {
                        *param = params.first().cloned().unwrap_or_default();
                    }
                    egui::ComboBox::from_id_salt(format!("{id}-param")).width(90.0).selected_text(param.clone()).show_ui(ui, |ui| {
                        for p in &params {
                            ui.selectable_value(param, p.clone(), p);
                        }
                    });
                };
                egui::Grid::new("analysis-settings").num_columns(2).spacing(egui::vec2(10.0, 6.0)).show(ui, |ui| {
                    match which {
                        Which::Op => {
                            ui.label(RichText::new("Solve with capacitors open and inductors shorted; every node voltage and branch current is listed.").color(c::TEXT_DIM));
                            ui.end_row();
                        }
                        Which::Dc => {
                            ui.label("Sweep");
                            ui.horizontal(|ui| pick(ui, "dc", &mut s.dc_inst, &mut s.dc_param));
                            ui.end_row();
                            for (l, v) in [("From", &mut s.dc_start), ("To", &mut s.dc_stop), ("Points", &mut s.dc_points)] {
                                ui.label(l);
                                ui.text_edit_singleline(v);
                                ui.end_row();
                            }
                        }
                        Which::Tran => {
                            for (l, v) in [("Stop time", &mut s.tran_stop), ("Maximum step", &mut s.tran_hmax)] {
                                ui.label(l);
                                ui.text_edit_singleline(v);
                                ui.end_row();
                            }
                        }
                        Which::Ac => {
                            for (l, v) in [("Start frequency", &mut s.ac_start), ("Stop frequency", &mut s.ac_stop), ("Points per decade", &mut s.ac_ppd)] {
                                ui.label(l);
                                ui.text_edit_singleline(v);
                                ui.end_row();
                            }
                            ui.label("");
                            ui.label(RichText::new("Sources are driven by their AC parameter (the sine source has AC = 1 V).").color(c::TEXT_DIM).small());
                            ui.end_row();
                        }
                    }
                    if which != Which::Op {
                        ui.checkbox(&mut s.step, "Parameter step");
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(s.step, |ui| pick(ui, "step", &mut s.step_inst, &mut s.step_param));
                        });
                        ui.end_row();
                        if s.step {
                            ui.label("Values");
                            ui.text_edit_singleline(&mut s.step_values);
                            ui.end_row();
                        }
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Run").color(Color32::WHITE)).fill(c::ACCENT)).clicked() {
                        run = Some(which);
                    }
                    ui.label(RichText::new("Runs in the background; results open in a plot window.").color(c::TEXT_DIM).small());
                });
            });
        if !open || run.is_some() {
            self.dialog = None;
        }
        run
    }

    /// Plot windows.
    pub fn windows(&mut self, ctx: &egui::Context) {
        let mut ffts = Vec::new();
        for win in &mut self.plots {
            let mut open = win.open;
            let resp = egui::Window::new(RichText::new(&win.title).font(theme::bold(13.0)))
                .id(egui::Id::unique(("plot", win.id)))
                .open(&mut open)
                .default_size(egui::vec2(720.0, 440.0))
                .resizable(true)
                .show(ctx, |ui| plot_body(ui, win, &mut ffts));
            if let Some(r) = resp {
                win.rect = r.response.rect;
            }
            win.open = open;
        }
        self.plots.retain(|p| p.open);
        for (title, set) in ffts {
            self.open_plot(title, vec![set], true);
        }
    }
}

fn si_axis(
    unit: &'static str,
    log: bool,
) -> impl Fn(egui_plot::GridMark, &std::ops::RangeInclusive<f64>) -> String {
    move |m, _| {
        let v = if log { 10f64.powf(m.value) } else { m.value };
        if log && (m.value - m.value.round()).abs() > 1e-6 {
            return String::new();
        }
        ecs_units::format_digits(v, unit, 3)
    }
}

fn plot_body(ui: &mut egui::Ui, win: &mut PlotWin, ffts: &mut Vec<(String, ResultSet)>) {
    let first = win.results[0].clone();
    let ac = first.kind == Kind::Ac;
    let log_x = ac || win.fft;
    ui.horizontal(|ui| {
        if ui.button("CSV…").clicked()
            && let Some(p) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_file_name(format!("{}.csv", win.title.replace(' ', "-"))).save_file()
        {
            let mut text = String::new();
            for r in &win.results {
                if !r.label.is_empty() {
                    text += &format!("# {}\n", r.label);
                }
                text += &r.to_csv();
            }
            win.status = match std::fs::write(&p, text) {
                Ok(()) => format!("Saved {}", p.display()),
                Err(e) => e.to_string(),
            };
        }
        if ui.button("PNG…").clicked()
            && let Some(p) = rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(format!("{}.png", win.title.replace(' ', "-"))).save_file()
        {
            save_png(ui.ctx(), win.rect, p);
            win.status = "Saved PNG".into();
        }
        if first.kind == Kind::Transient && !win.fft && ui.button("FFT").on_hover_text("Spectrum of the selected traces").clicked() {
            for name in &win.selected {
                if let Some(t) = first.traces.iter().find(|t| &t.name == name) {
                    let (f, m) = analysis::fft(&first.axis, &t.re);
                    let set = ResultSet {
                        kind: Kind::Transient,
                        axis_name: "frequency".into(),
                        axis_unit: "Hz".into(),
                        axis: f.iter().skip(1).copied().collect(),
                        traces: vec![analysis::Trace { name: name.clone(), unit: t.unit.clone(), re: m.iter().skip(1).copied().collect(), im: None }],
                        label: String::new(),
                    };
                    ffts.push((format!("FFT of {name}"), set));
                }
            }
        }
        ui.label(RichText::new("Click: cursor A · Right-click: cursor B · Drag: pan · Wheel: zoom · Double-click: fit").color(c::TEXT_DIM).small());
        if !win.status.is_empty() {
            ui.label(RichText::new(&win.status).color(c::TEXT_DIM).small());
        }
    });
    ui.horizontal_top(|ui| {
        // Trace picker.
        ui.vertical(|ui| {
            ui.set_width(150.0);
            egui::ScrollArea::vertical().id_salt(("traces", win.id)).max_height(ui.available_height().max(200.0)).show(ui, |ui| {
                for t in &first.traces {
                    let mut on = win.selected.contains(&t.name);
                    if ui.checkbox(&mut on, RichText::new(&t.name).font(theme::mono(11.0))).changed() {
                        if on {
                            win.selected.insert(t.name.clone());
                        } else {
                            win.selected.remove(&t.name);
                        }
                    }
                }
            });
        });
        ui.vertical(|ui| {
            let xs = |x: f64| if log_x { x.max(1e-30).log10() } else { x };
            let palette = &c::TRACES;
            let series = |value: &dyn Fn(&analysis::Trace) -> Vec<f64>| -> Vec<(String, Color32, Vec<[f64; 2]>)> {
                let mut out = Vec::new();
                let mut k = 0;
                for r in &win.results {
                    for t in r.traces.iter().filter(|t| win.selected.contains(&t.name)) {
                        let ys = value(t);
                        let name = if r.label.is_empty() { t.name.clone() } else { format!("{} [{}]", t.name, r.label) };
                        out.push((name, palette[k % palette.len()], r.axis.iter().zip(ys).map(|(x, y)| [xs(*x), y]).collect()));
                        k += 1;
                    }
                }
                out
            };
            let mut cursor_click: Option<(usize, f64)> = None;
            let mut draw = |ui: &mut egui::Ui, id: &str, height: f32, unit: &'static str, data: Vec<(String, Color32, Vec<[f64; 2]>)>| {
                let cursors = win.cursors;
                let resp = Plot::new((id, win.id))
                    .height(height)
                    .legend(Legend::default())
                    .link_axis(egui::Id::unique(("x", win.id)), [true, false])
                    .x_axis_formatter(si_axis(if ac || win.fft { "Hz" } else if first.axis_unit == "s" { "s" } else { "" }, log_x))
                    .y_axis_formatter(move |m, _| ecs_units::format_digits(m.value, unit, 3))
                    .show(ui, |p| {
                        for (name, col, pts) in data {
                            p.line(Line::new(name, PlotPoints::from(pts)).color(col).width(1.6));
                        }
                        for (k, cur) in cursors.iter().enumerate() {
                            if let Some(x) = cur {
                                p.vline(VLine::new(if k == 0 { "A" } else { "B" }, *x).color(if k == 0 { c::WARN } else { c::OK }));
                            }
                        }
                        p.pointer_coordinate()
                    });
                if let Some(pt) = resp.inner {
                    if resp.response.clicked() {
                        cursor_click = Some((0, pt.x));
                    }
                    if resp.response.secondary_clicked() {
                        cursor_click = Some((1, pt.x));
                    }
                }
            };
            let h = ui.available_height().max(260.0);
            if ac {
                draw(ui, "mag", h * 0.58, "dB", series(&|t| t.magnitude().iter().map(|m| 20.0 * m.max(1e-30).log10()).collect()));
                draw(ui, "phase", h * 0.36, "°", series(&|t| t.phase_deg()));
            } else {
                let unit = match first.traces.iter().find(|t| win.selected.contains(&t.name)).map(|t| t.unit.as_str()) {
                    Some("A") => "A",
                    _ => "V",
                };
                draw(ui, "main", h, unit, series(&|t| t.re.clone()));
            }
            if let Some((k, x)) = cursor_click {
                win.cursors[k] = Some(x);
            }
            // Cursor readout.
            if win.cursors.iter().any(Option::is_some) {
                let at = |x: f64, t: &analysis::Trace, r: &ResultSet| -> f64 {
                    let ys = if ac { t.magnitude().iter().map(|m| 20.0 * m.max(1e-30).log10()).collect() } else { t.re.clone() };
                    let xs_: Vec<f64> = r.axis.iter().map(|v| xs(*v)).collect();
                    let k = xs_.partition_point(|v| *v < x).clamp(1, xs_.len().max(2) - 1);
                    if xs_.len() < 2 {
                        return ys.first().copied().unwrap_or(0.0);
                    }
                    let f = ((x - xs_[k - 1]) / (xs_[k] - xs_[k - 1])).clamp(0.0, 1.0);
                    ys[k - 1] + (ys[k] - ys[k - 1]) * f
                };
                let unx = |x: f64| if log_x { 10f64.powf(x) } else { x };
                let xu = if log_x { "Hz" } else { first.axis_unit.as_str() };
                ui.horizontal_wrapped(|ui| {
                    for (k, cur) in win.cursors.iter().enumerate() {
                        if let Some(x) = cur {
                            ui.label(RichText::new(format!("{}: {}", if k == 0 { "A" } else { "B" }, ecs_units::format_digits(unx(*x), xu, 4))).font(theme::mono(11.5)).color(if k == 0 { c::WARN } else { c::OK }));
                            for t in first.traces.iter().filter(|t| win.selected.contains(&t.name)).take(4) {
                                ui.label(RichText::new(format!("{} {}", t.name, ecs_units::format_digits(at(*x, t, &first), if ac { "dB" } else { &t.unit }, 4))).font(theme::mono(11.0)));
                            }
                        }
                    }
                    if let [Some(a), Some(b)] = win.cursors {
                        let d = unx(b) - unx(a);
                        let mut text = format!("Δ = {}", ecs_units::format_digits(d, xu, 4));
                        if xu == "s" && d != 0.0 {
                            text += &format!("  (1/Δ = {})", ecs_units::format_digits(1.0 / d.abs(), "Hz", 4));
                        }
                        ui.label(RichText::new(text).font(theme::mono(11.5)));
                    }
                });
            }
        });
    });
}

/// Save the window region of the next frame as PNG.
fn save_png(ctx: &egui::Context, rect: egui::Rect, path: PathBuf) {
    let ppp = ctx.pixels_per_point();
    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
        egui::ScreenshotCallback::new(move |img| {
            let region = img.region(&rect, Some(ppp));
            let [w, h] = region.size;
            let bytes: Vec<u8> = region.pixels.iter().flat_map(|p| p.to_array()).collect();
            let _ = image::save_buffer(
                &path,
                &bytes,
                w as u32,
                h as u32,
                image::ExtendedColorType::Rgba8,
            );
        }),
    ));
}
