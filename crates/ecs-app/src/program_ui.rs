//! Microcontroller program editor (spec 7.2) and part numbers in the inspector.

use crate::app::App;
use crate::code_editor::{self, Lang, Opts};
use crate::theme::{self, c};
use ecs_schematic::{Command, Id};
use egui::{RichText, vec2};

/// The program editor window's state.
pub struct ProgramEditor {
    pub part: Id,
    pub text: String,
    pub check: Option<Result<String, Vec<String>>>,
}

impl App {
    /// Open the program editor for a microcontroller part.
    pub fn open_program(&mut self, part: Id) {
        let Some(p) = self.doc.part(part) else { return };
        if !ecs_mcu::is_mcu(&p.def) {
            self.status = format!("{} is not a microcontroller", p.refdes);
            return;
        }
        let text = if p.code.trim().is_empty() {
            ecs_mcu::template(&p.def)
        } else {
            p.code.clone()
        };
        self.program_editor = Some(ProgramEditor {
            part,
            text,
            check: None,
        });
    }

    /// Set a part number; a known one also sets its model parameters.
    pub fn set_part_number(&mut self, part: Id, number: &str) {
        let Some(p) = self.doc.part(part).cloned() else {
            return;
        };
        let number = number.trim().to_string();
        let mut params = Vec::new();
        if let Some(k) = ecs_lib::parts::find(&number) {
            if k.def == p.def {
                params = k
                    .params
                    .iter()
                    .map(|(n, v)| (n.clone(), ecs_units::format_digits(*v, "", 4)))
                    .collect();
            } else {
                let kind = self
                    .cat
                    .def(&k.def)
                    .map_or(k.def.clone(), |d| d.meta.name.clone());
                self.status = format!("{} is a {kind}: only the number is set", k.number);
            }
        }
        self.edit(Command::SetPartNumber {
            part,
            number,
            params,
        });
    }

    /// Part number row for the inspector grid.
    pub fn part_number_row(&mut self, ui: &mut egui::Ui, part: Id, def_id: &str) {
        let Some(p) = self.doc.part(part).cloned() else {
            return;
        };
        ui.label(RichText::new("Part no.").color(c::TEXT_DIM))
            .on_hover_text("Manufacturer part number; known ones set the model parameters");
        ui.horizontal(|ui| {
            let key = format!("pn-{part}");
            let mut text = self
                .inspector_edit
                .get(&key)
                .cloned()
                .unwrap_or(p.part_number.clone());
            let resp = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(112.0)
                    .hint_text("e.g. 2N2222A"),
            );
            if resp.changed() {
                self.inspector_edit.insert(key.clone(), text.clone());
            }
            if resp.lost_focus() {
                self.inspector_edit.remove(&key);
                if text.trim() != p.part_number {
                    self.set_part_number(part, &text);
                }
            }
            let known = ecs_lib::parts::for_def(def_id);
            if !known.is_empty() {
                let mut pick = None;
                egui::ComboBox::from_id_salt(("pn", part))
                    .width(24.0)
                    .selected_text("")
                    .show_ui(ui, |ui| {
                        ui.set_min_width(300.0);
                        for k in known {
                            let label = RichText::new(format!("{:<12} {}", k.number, k.note))
                                .font(theme::mono(11.5));
                            if ui
                                .selectable_label(k.number == p.part_number, label)
                                .clicked()
                            {
                                pick = Some(k.number.clone());
                            }
                        }
                    });
                if let Some(n) = pick {
                    self.set_part_number(part, &n);
                }
            }
        });
        ui.end_row();
    }

    /// The program editor window.
    pub fn program_window(&mut self, ctx: &egui::Context) {
        let Some(mut ed) = self.program_editor.take() else {
            return;
        };
        let Some(part) = self.doc.part(ed.part).cloned() else {
            return;
        };
        let Some(dev) = ecs_mcu::device(&part.def) else {
            return;
        };
        let lang = if dev.arch == ecs_mcu::Arch::Mips {
            Lang::AsmMips
        } else {
            Lang::AsmPic
        };
        let mut open = true;
        let mut apply = false;
        egui::Window::new(RichText::new(format!("Program — {} ({})", part.refdes, dev.part)).font(theme::bold(14.0)))
            .id(egui::Id::unique(("program", ed.part)))
            .open(&mut open)
            .default_size(vec2(860.0, 680.0))
            .min_size(vec2(560.0, 420.0))
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                let syntax = match dev.arch {
                    ecs_mcu::Arch::Pic14 => "MPASM syntax (default radix hex: write D'10' or .10 for decimal). Starts at ORG 0; interrupts at 4.",
                    ecs_mcu::Arch::Pic24 => "XC16 syntax: #literal, Wn, [Wn++]. Starts at __reset.",
                    ecs_mcu::Arch::Mips => "GNU as syntax, $registers, NOP-filled delay slots (.set reorder). Starts at main.",
                };
                ui.label(RichText::new(format!("{} {syntax}", dev.desc)).color(c::TEXT_DIM).small());
                ui.horizontal(|ui| {
                    if ui.button("Template").on_hover_text("Replace with the blink example").clicked() {
                        ed.text = ecs_mcu::template(&part.def);
                        ed.check = None;
                    }
                    if ui.button("Assemble").on_hover_text("Check the program without applying it").clicked() {
                        ed.check = Some(check(&self.cat, &part.def, &ed.text));
                    }
                    ui.label(RichText::new("Apply restarts the simulation with the new program.").color(c::TEXT_FAINT).small());
                });
                let footer = 70.0;
                let h = (ui.available_height() - footer).max(200.0);
                let vocab = code_editor::mcu_vocabulary(&part.def);
                egui::ScrollArea::vertical().id_salt(("prog-src", ed.part)).max_height(h).show(ui, |ui| {
                    code_editor::code_editor(ui, ("prog", ed.part), &mut ed.text, lang, &vocab, Opts { rows: 30, ..Default::default() });
                });
                ui.separator();
                match &ed.check {
                    Some(Ok(m)) => {
                        ui.label(RichText::new(m).color(c::OK).small());
                    }
                    Some(Err(errs)) => {
                        egui::ScrollArea::vertical().id_salt(("prog-errs", ed.part)).max_height(48.0).show(ui, |ui| {
                            for e in errs {
                                ui.label(RichText::new(e).color(c::DANGER).small());
                            }
                        });
                    }
                    None => {}
                }
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("Apply").strong()).clicked() {
                            apply = true;
                        }
                    });
                });
            });
        if apply {
            ed.check = Some(check(&self.cat, &part.def, &ed.text));
            if part.code != ed.text {
                self.edit(Command::SetPartCode {
                    part: ed.part,
                    code: ed.text.clone(),
                });
                self.status = format!("{} reprogrammed", part.refdes);
            }
        }
        if open {
            self.program_editor = Some(ed);
        }
    }
}

fn check(cat: &crate::catalog::Catalog, def_id: &str, text: &str) -> Result<String, Vec<String>> {
    let Some(def) = cat.def(def_id) else {
        return Err(vec![format!("unknown {def_id}")]);
    };
    let (_, errors) = ecs_mcu::with_program(def, text);
    if errors.is_empty() {
        let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
        Ok(format!("Assembled without errors ({lines} lines)."))
    } else {
        Err(errors.iter().map(|e| e.to_string()).collect())
    }
}
