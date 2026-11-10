//! Electrocraft datasheet tool (spec 8.2, phase P8): fit a model family to
//! datasheet points, typed in or digitized from a curve image, review the
//! fit, and export a JavaScript component with a fit report.

mod fit;
#[path = "../../ecs-app/src/theme.rs"]
#[allow(dead_code)]
mod theme;

use egui::{Color32, RichText, TextureHandle};
use egui_plot::{Legend, Line, Plot, PlotPoints, Points};
use fit::{Dataset, Family, Fit};
use theme::c;

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Digitizer,
    Review,
}

/// Axis calibration of a curve image: two pixel/value pairs per axis.
#[derive(Default, Clone)]
struct Calib {
    x: [Option<(f32, f64)>; 2],
    y: [Option<(f32, f64)>; 2],
    log_x: bool,
    log_y: bool,
}

impl Calib {
    fn map(&self, axis: [Option<(f32, f64)>; 2], log: bool, px: f32) -> Option<f64> {
        let (a, b) = (axis[0]?, axis[1]?);
        let f = ((px - a.0) / (b.0 - a.0)) as f64;
        Some(if log {
            (a.1.ln() + f * (b.1.ln() - a.1.ln())).exp()
        } else {
            a.1 + f * (b.1 - a.1)
        })
    }
    fn ready(&self) -> bool {
        self.x.iter().chain(&self.y).all(Option::is_some)
    }
}

struct Tool {
    name: String,
    ds: Dataset,
    active: usize,
    fit: Option<Fit>,
    status: String,
    tab: Tab,
    image: Option<(TextureHandle, [usize; 2])>,
    calib: Calib,
    /// Next calibration click: 0..4 (X1, X2, Y1, Y2), 4 = adding points.
    step: usize,
    pending: String,
}

impl Tool {
    fn new() -> Self {
        Self {
            name: "MyDiode".into(),
            ds: Dataset::new(Family::Diode),
            active: 0,
            fit: None,
            status: String::new(),
            tab: Tab::Review,
            image: None,
            calib: Calib::default(),
            step: 0,
            pending: String::new(),
        }
    }

    fn export(&mut self) {
        let Some(f) = &self.fit else { return };
        let dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
            })
            .map(|d| d.join("electrocraft").join("components"));
        let picked = rfd::FileDialog::new().set_title("Export the component (the Electrocraft components folder reloads it automatically)").set_directory(dir.clone().unwrap_or_default()).pick_folder().or(dir);
        let Some(dir) = picked else { return };
        let stem: String = self
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        let res = std::fs::create_dir_all(&dir)
            .and_then(|_| {
                std::fs::write(
                    dir.join(format!("{stem}.js")),
                    fit::export_js(self.ds.family, &self.name, f),
                )
            })
            .and_then(|_| {
                std::fs::write(
                    dir.join(format!("{stem}.fit.md")),
                    fit::report(&self.ds, &self.name, f)
                        + "\n```\n"
                        + &fit::export_spice(self.ds.family, &self.name, f)
                        + "\n```\n",
                )
            });
        self.status = match res {
            Ok(()) => format!("Exported {stem}.js and {stem}.fit.md to {}", dir.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }
}

impl eframe::App for Tool {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("top")
            .exact_size(46.0)
            .frame(
                egui::Frame::new()
                    .fill(c::TITLEBAR)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("Electrocraft Datasheet").font(theme::bold(15.0)));
                    ui.add_space(16.0);
                    ui.label("Family");
                    let mut fam = self.ds.family;
                    egui::ComboBox::from_id_salt("fam")
                        .selected_text(fam.name())
                        .show_ui(ui, |ui| {
                            for f in Family::ALL {
                                ui.selectable_value(&mut fam, f, f.name());
                            }
                        });
                    if fam != self.ds.family {
                        self.ds = Dataset::new(fam);
                        self.active = 0;
                        self.fit = None;
                    }
                    ui.label("Part");
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(120.0));
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Fit").color(Color32::WHITE))
                                .fill(c::ACCENT),
                        )
                        .clicked()
                    {
                        match fit::fit(&self.ds) {
                            Ok(f) => {
                                self.status = format!(
                                    "Fitted in {} iterations; largest error {:.2} %",
                                    f.iterations,
                                    f.max_error * 100.0
                                );
                                self.fit = Some(f);
                                self.tab = Tab::Review;
                            }
                            Err(e) => self.status = e,
                        }
                    }
                    if ui
                        .add_enabled(self.fit.is_some(), egui::Button::new("Export…"))
                        .clicked()
                    {
                        self.export();
                    }
                    ui.label(RichText::new(&self.status).color(c::TEXT_DIM));
                });
            });
        egui::Panel::left("points")
            .exact_size(300.0)
            .frame(
                egui::Frame::new()
                    .fill(c::PANEL)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| {
                let series = self.ds.family.series();
                for (k, s) in series.iter().enumerate() {
                    if ui
                        .selectable_label(
                            self.active == k,
                            RichText::new(s.0).font(theme::bold(12.5)),
                        )
                        .clicked()
                    {
                        self.active = k;
                    }
                }
                if matches!(self.ds.family, Family::Npn | Family::Nmos) {
                    ui.horizontal(|ui| {
                        ui.label(if self.ds.family == Family::Nmos {
                            "Vds"
                        } else {
                            "Vce"
                        });
                        ui.add(
                            egui::DragValue::new(&mut self.ds.bias)
                                .speed(0.1)
                                .suffix(" V"),
                        );
                    });
                }
                ui.separator();
                let (_, xl, xu, yl, yu, _) = series[self.active];
                ui.label(RichText::new(format!("{xl} ({xu})   {yl} ({yu})")).color(c::TEXT_DIM));
                let mut remove = None;
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height() - 40.0)
                    .show(ui, |ui| {
                        for (i, p) in self.ds.series[self.active].iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                let sp = p[0].abs() * 0.01 + 1e-9;
                                ui.add(
                                    egui::DragValue::new(&mut p[0])
                                        .speed(sp)
                                        .custom_formatter(|v, _| ecs_units::format_digits(v, "", 4))
                                        .custom_parser(|t| ecs_units::parse(t).ok()),
                                );
                                let sp = p[1].abs() * 0.01 + 1e-9;
                                ui.add(
                                    egui::DragValue::new(&mut p[1])
                                        .speed(sp)
                                        .custom_formatter(|v, _| ecs_units::format_digits(v, "", 4))
                                        .custom_parser(|t| ecs_units::parse(t).ok()),
                                );
                                if let Some(f) = &self.fit
                                    && let Some(e) =
                                        f.errors.get(self.active).and_then(|e| e.get(i))
                                {
                                    ui.label(
                                        RichText::new(format!("{:.1} %", e * 100.0))
                                            .color(if *e < 0.05 { c::OK } else { c::WARN })
                                            .small(),
                                    );
                                }
                                if ui.small_button("×").clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                    });
                if let Some(i) = remove {
                    self.ds.series[self.active].remove(i);
                }
                if ui.button("Add point").clicked() {
                    let last = self.ds.series[self.active]
                        .last()
                        .copied()
                        .unwrap_or([1e-3, 0.6]);
                    self.ds.series[self.active].push(last);
                }
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(c::CANVAS)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Review, "Review");
                    ui.selectable_value(&mut self.tab, Tab::Digitizer, "Digitizer");
                });
                match self.tab {
                    Tab::Review => self.review(ui),
                    Tab::Digitizer => self.digitizer(ui, &ctx),
                }
            });
    }
}

impl Tool {
    fn review(&mut self, ui: &mut egui::Ui) {
        let (title, _, xu, _, yu, log_x) = self.ds.family.series()[self.active];
        let pts = self.ds.series[self.active].clone();
        let tx = |x: f64| if log_x { x.max(1e-30).log10() } else { x };
        let mut curve = Vec::new();
        if let (Some(f), false) = (&self.fit, pts.is_empty()) {
            let (lo, hi) = pts
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                    (a.min(p[0]), b.max(p[0]))
                });
            for k in 0..=40 {
                let t = k as f64 / 40.0;
                let x = if log_x {
                    (lo.ln() + t * (hi / lo).ln()).exp()
                } else {
                    lo + t * (hi - lo)
                };
                if let Some(y) =
                    fit::model_value(self.ds.family, &f.params, self.active, x, self.ds.bias)
                {
                    curve.push([tx(x), y]);
                }
            }
        }
        Plot::new("review")
            .legend(Legend::default())
            .x_axis_formatter(move |m, _| {
                ecs_units::format_digits(if log_x { 10f64.powf(m.value) } else { m.value }, xu, 3)
            })
            .y_axis_formatter(move |m, _| ecs_units::format_digits(m.value, yu, 3))
            .show(ui, |p| {
                p.points(
                    Points::new(
                        format!("{title} (datasheet)"),
                        PlotPoints::from(pts.iter().map(|q| [tx(q[0]), q[1]]).collect::<Vec<_>>()),
                    )
                    .radius(4.0)
                    .color(c::WARN),
                );
                if !curve.is_empty() {
                    p.line(
                        Line::new("model", PlotPoints::from(curve))
                            .color(c::ACCENT)
                            .width(2.0),
                    );
                }
            });
    }

    fn digitizer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if ui.button("Load curve image…").clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .add_filter("Image", &["png", "jpg", "jpeg"])
                    .pick_file()
            {
                match image::open(&p) {
                    Ok(img) => {
                        let rgba = img.to_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        let ci = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                        self.image = Some((
                            ctx.load_texture("curve", ci, egui::TextureOptions::LINEAR),
                            size,
                        ));
                        self.calib = Calib::default();
                        self.step = 0;
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
            ui.checkbox(&mut self.calib.log_x, "log X");
            ui.checkbox(&mut self.calib.log_y, "log Y");
            if ui.button("Recalibrate").clicked() {
                self.calib = Calib {
                    log_x: self.calib.log_x,
                    log_y: self.calib.log_y,
                    ..Default::default()
                };
                self.step = 0;
            }
            let prompt = [
                "Click a known X value on the X axis",
                "Click a second X value",
                "Click a known Y value on the Y axis",
                "Click a second Y value",
                "Click along the curve to add points to the selected series",
            ];
            ui.label(RichText::new(prompt[self.step.min(4)]).color(c::ACTIVE_TAB_TEXT));
            if self.step < 4 {
                ui.label("Value");
                ui.add(
                    egui::TextEdit::singleline(&mut self.pending)
                        .desired_width(70.0)
                        .hint_text("e.g. 10m"),
                );
            }
        });
        let Some((tex, size)) = &self.image else {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load a screenshot of a datasheet graph, calibrate two points on each axis, then click along the curve.").color(c::TEXT_DIM)));
            return;
        };
        let avail = ui.available_size();
        let scale = (avail.x / size[0] as f32)
            .min(avail.y / size[1] as f32)
            .min(2.0);
        let disp = egui::vec2(size[0] as f32 * scale, size[1] as f32 * scale);
        let (rect, resp) = ui.allocate_exact_size(disp, egui::Sense::click());
        ui.painter().image(
            tex.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        // Image pixel coordinates (independent of the display scale).
        let to_px = |p: egui::Pos2| ((p.x - rect.min.x) / scale, (p.y - rect.min.y) / scale);
        if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let (px, py) = to_px(pos);
            if self.step < 4 {
                match ecs_units::parse(&self.pending) {
                    Ok(v) => {
                        match self.step {
                            0 | 1 => self.calib.x[self.step] = Some((px, v)),
                            _ => self.calib.y[self.step - 2] = Some((py, v)),
                        }
                        self.step += 1;
                        self.pending.clear();
                    }
                    Err(_) => self.status = "Type the axis value first, then click it".into(),
                }
            } else if self.calib.ready() {
                let x = self.calib.map(self.calib.x, self.calib.log_x, px);
                let y = self.calib.map(self.calib.y, self.calib.log_y, py);
                if let (Some(x), Some(y)) = (x, y) {
                    self.ds.series[self.active].push([x, y]);
                    self.ds.series[self.active].sort_by(|a, b| a[0].total_cmp(&b[0]));
                }
            }
        }
        // Calibration marks and digitized points.
        let painter = ui.painter_at(rect);
        for (k, m) in self.calib.x.iter().enumerate() {
            if let Some((px, _)) = m {
                let x = rect.min.x + px * scale;
                painter.line_segment(
                    [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                    egui::Stroke::new(1.0, c::OK),
                );
                painter.text(
                    egui::pos2(x + 3.0, rect.max.y - 14.0),
                    egui::Align2::LEFT_CENTER,
                    format!("X{}", k + 1),
                    theme::mono(11.0),
                    c::OK,
                );
            }
        }
        for (k, m) in self.calib.y.iter().enumerate() {
            if let Some((py, _)) = m {
                let y = rect.min.y + py * scale;
                painter.line_segment(
                    [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
                    egui::Stroke::new(1.0, c::WARN),
                );
                painter.text(
                    egui::pos2(rect.min.x + 4.0, y - 8.0),
                    egui::Align2::LEFT_CENTER,
                    format!("Y{}", k + 1),
                    theme::mono(11.0),
                    c::WARN,
                );
            }
        }
        if self.calib.ready() {
            // Inverse map to draw existing points.
            let inv = |axis: [Option<(f32, f64)>; 2], log: bool, v: f64| -> Option<f32> {
                let (a, b) = (axis[0]?, axis[1]?);
                let f = if log {
                    (v.ln() - a.1.ln()) / (b.1.ln() - a.1.ln())
                } else {
                    (v - a.1) / (b.1 - a.1)
                };
                Some(a.0 + (b.0 - a.0) * f as f32)
            };
            for p in &self.ds.series[self.active] {
                if let (Some(px), Some(py)) = (
                    inv(self.calib.x, self.calib.log_x, p[0]),
                    inv(self.calib.y, self.calib.log_y, p[1]),
                ) {
                    painter.circle_filled(
                        egui::pos2(rect.min.x + px * scale, rect.min.y + py * scale),
                        4.0,
                        c::ACCENT,
                    );
                }
            }
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Electrocraft Datasheet")
            .with_app_id("electrocraft-datasheet")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Electrocraft Datasheet",
        options,
        Box::new(|cc| {
            theme::install(&cc.egui_ctx);
            let mut tool = Tool::new();
            // --example diode | npn: typical datasheet points, fitted.
            let args: Vec<String> = std::env::args().collect();
            if let Some(k) = args.iter().position(|a| a == "--example") {
                let (name, ds) = match args.get(k + 1).map(String::as_str) {
                    Some("npn") => {
                        let mut ds = Dataset::new(Family::Npn);
                        ds.bias = 1.0;
                        ds.series[0] = vec![[1e-3, 110.0], [3e-3, 115.0], [10e-3, 118.0]];
                        ds.series[1] = vec![[0.1e-3, 0.56], [1e-3, 0.65], [10e-3, 0.74]];
                        ("2N3904", ds)
                    }
                    _ => {
                        let mut ds = Dataset::new(Family::Diode);
                        ds.series[0] = vec![
                            [0.1e-3, 0.52],
                            [0.3e-3, 0.565],
                            [1e-3, 0.62],
                            [3e-3, 0.665],
                            [10e-3, 0.72],
                        ];
                        ("1N4148", ds)
                    }
                };
                tool.name = name.into();
                tool.ds = ds;
                if let Ok(f) = fit::fit(&tool.ds) {
                    tool.status = format!(
                        "Fitted in {} iterations; largest error {:.2} %",
                        f.iterations,
                        f.max_error * 100.0
                    );
                    tool.fit = Some(f);
                }
            }
            Ok(Box::new(tool))
        }),
    )
}
