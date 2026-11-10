//! SPICE import dialog (spec 8.1): paste or open a file, review the models and
//! subcircuits found, pick symbols and pin mappings, save as a package.

use crate::catalog::Catalog;
use crate::packages::{self, PackageComponent};
use crate::theme::{self, c};
use ecs_model::DefKind;
use ecs_schematic::symbols;
use egui::RichText;
use std::path::PathBuf;

#[derive(Default)]
pub struct ImportUi {
    pub open: bool,
    pub text: String,
    pub file_name: String,
    base: PathBuf,
    parsed: Option<Result<ecs_spice::Import, String>>,
    choices: Vec<PackageComponent>,
    pub package: String,
    pub status: String,
}

impl ImportUi {
    pub fn show_dialog(&mut self) {
        self.open = true;
    }

    fn parse(&mut self) {
        let base = self.base.clone();
        let r = ecs_spice::import(
            &self.text,
            if self.file_name.is_empty() {
                "pasted.lib"
            } else {
                &self.file_name
            },
            &base,
            &|p| std::fs::read_to_string(p).ok(),
        )
        .map_err(|e| e.to_string());
        self.choices = match &r {
            Ok(imp) => imp
                .defs
                .iter()
                .map(|d| {
                    // Guess a symbol: three-port subcircuits named like op-amps.
                    let lower = d.meta.name.to_lowercase();
                    let symbol = if matches!(d.kind, DefKind::Subcircuit(_))
                        && d.pins.len() == 3
                        && (lower.contains("op") || lower.contains("amp"))
                    {
                        "opamp".to_string()
                    } else if matches!(d.kind, DefKind::Subcircuit(_)) {
                        String::new()
                    } else {
                        d.symbol.clone()
                    };
                    let pins = if symbol == "opamp" {
                        vec!["INP".into(), "INN".into(), "OUT".into()]
                    } else {
                        Vec::new()
                    };
                    PackageComponent {
                        id: d.id.clone(),
                        symbol,
                        pins,
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        if self.package.is_empty() {
            self.package = std::path::Path::new(&self.file_name)
                .file_stem()
                .map_or("imported".into(), |s| s.to_string_lossy().to_string());
        }
        self.parsed = Some(r);
    }

    /// The dialog. Returns true when the catalog changed.
    pub fn ui(&mut self, ctx: &egui::Context, cat: &mut Catalog) -> bool {
        if !self.open {
            return false;
        }
        let mut changed = false;
        let mut open = true;
        egui::Window::new(RichText::new("Import SPICE models").font(theme::bold(14.0)))
            .open(&mut open)
            .default_size(egui::vec2(720.0, 560.0))
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Open file…").clicked()
                        && let Some(p) = rfd::FileDialog::new().add_filter("SPICE", &["lib", "mod", "sub", "cir", "sp", "txt", "spi"]).pick_file()
                    {
                        match std::fs::read_to_string(&p) {
                            Ok(t) => {
                                self.text = t;
                                self.file_name = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                                self.base = p.parent().map(PathBuf::from).unwrap_or_default();
                                self.package.clear();
                                self.parse();
                            }
                            Err(e) => self.status = e.to_string(),
                        }
                    }
                    if ui.button("Parse").clicked() {
                        self.parse();
                    }
                    ui.label(RichText::new(".model (D, NPN, PNP, NMOS, PMOS, NJF, PJF) and .subckt with R C L V I E F G H B D Q M J X").color(c::TEXT_DIM).small());
                });
                crate::code_editor::code_editor(ui, "spice-import", &mut self.text, crate::code_editor::Lang::Spice, &[], crate::code_editor::Opts { rows: 9, hint: "Paste .model / .subckt text here", ..Default::default() });
                match &self.parsed {
                    None => {}
                    Some(Err(e)) => {
                        ui.colored_label(c::DANGER, e);
                    }
                    Some(Ok(imp)) => {
                        let imp = imp.clone();
                        ui.separator();
                        ui.label(RichText::new(format!("{} component{} found", imp.defs.len(), if imp.defs.len() == 1 { "" } else { "s" })).font(theme::bold(13.0)));
                        let symbol_ids: Vec<String> = symbols::library().keys().cloned().collect();
                        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                            for (d, choice) in imp.defs.iter().zip(self.choices.iter_mut()) {
                                ui.horizontal(|ui| {
                                    let kind = if matches!(d.kind, DefKind::Subcircuit(_)) { "subckt" } else { "model" };
                                    ui.label(RichText::new(format!("{} ({kind}, {} pins)", d.meta.name, d.pins.len())).font(theme::mono(12.0)));
                                    if kind == "subckt" {
                                        let label = if choice.symbol.is_empty() { "Generated box".to_string() } else { choice.symbol.clone() };
                                        egui::ComboBox::from_id_salt(("sym", &d.id)).width(140.0).selected_text(label).show_ui(ui, |ui| {
                                            ui.selectable_value(&mut choice.symbol, String::new(), "Generated box");
                                            for id in &symbol_ids {
                                                if symbols::get(id).pins.len() == d.pins.len() {
                                                    ui.selectable_value(&mut choice.symbol, id.clone(), id);
                                                }
                                            }
                                        });
                                    }
                                });
                                if !choice.symbol.is_empty() && matches!(d.kind, DefKind::Subcircuit(_)) {
                                    let sym_pins: Vec<String> = symbols::get(&choice.symbol).pins.iter().map(|p| p.name.clone()).collect();
                                    if choice.pins.len() != d.pins.len() || choice.pins.iter().any(|p| !sym_pins.contains(p)) {
                                        choice.pins = sym_pins.clone();
                                    }
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new("Ports →").color(c::TEXT_DIM).small());
                                        for (k, port) in d.pins.iter().enumerate() {
                                            ui.label(RichText::new(&port.name).font(theme::mono(11.0)));
                                            egui::ComboBox::from_id_salt(("pin", &d.id, k)).width(60.0).selected_text(choice.pins[k].clone()).show_ui(ui, |ui| {
                                                for sp in &sym_pins {
                                                    ui.selectable_value(&mut choice.pins[k], sp.clone(), sp);
                                                }
                                            });
                                        }
                                    });
                                }
                            }
                        });
                        if !imp.report.is_empty() {
                            ui.collapsing(format!("Import report ({})", imp.report.len()), |ui| {
                                for r in &imp.report {
                                    ui.label(RichText::new(r).font(theme::mono(11.0)).color(c::WARN));
                                }
                            });
                        }
                        ui.horizontal(|ui| {
                            ui.label("Package");
                            ui.text_edit_singleline(&mut self.package);
                            let dupes = self.choices.iter().any(|ch| ch.pins.len() > 1 && ch.pins.iter().enumerate().any(|(i, p)| ch.pins[..i].contains(p)));
                            if ui.add_enabled(!imp.defs.is_empty() && !dupes, egui::Button::new(RichText::new("Add to library").color(egui::Color32::WHITE)).fill(c::ACCENT)).clicked() {
                                match packages::save_package(&self.package, if self.file_name.is_empty() { "import.lib" } else { &self.file_name }, &self.text, self.choices.clone()) {
                                    Ok(dir) => {
                                        for d in packages::finish_defs(&imp.defs, &self.choices, &self.package) {
                                            cat.lib.insert(d);
                                        }
                                        changed = true;
                                        self.status = format!("Saved package to {}; the parts are in the Components panel under Imported", dir.display());
                                    }
                                    Err(e) => self.status = format!("Could not save the package: {e}"),
                                }
                            }
                            if dupes {
                                ui.colored_label(c::DANGER, "Each port needs its own symbol pin");
                            }
                        });
                    }
                }
                if !self.status.is_empty() {
                    ui.label(RichText::new(&self.status).color(c::TEXT_DIM));
                }
            });
        if !open {
            self.open = false;
        }
        changed
    }
}
