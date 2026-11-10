//! Electrocraft: draw electronic schematics and simulate them live.
//!
//! ```text
//! electrocraft [file.ecs] [--new] [--sample <id>] [--software-canvas] [--version]
//! ```

mod analysis_ui;
mod app;
mod catalog;
mod chrome;
mod code_editor;
mod commands;
mod component_ui;
mod components;
mod icons;
mod import_ui;
mod packages;
mod samples;
mod scene;
mod scope;
mod theme;

use app::{App, Cmd};
use std::path::PathBuf;
use std::time::Instant;
use theme::c;

/// Application id: Wayland app id / X11 WM_CLASS, matched by the desktop
/// file's `StartupWMClass` (AppGlider reads it for window grouping).
pub const APP_ID: &str = "electrocraft";

struct Shell {
    app: App,
    status_seen: String,
    status_since: Instant,
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let app = &mut self.app;
        // Hot reload of user JavaScript components (spec 6.3).
        if app.user_lib.poll(&mut app.cat) {
            app.rev += 1;
            app.status = "Reloaded user components".into();
        }
        if app.analysis.poll() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        app.poll_sim();
        app.sync_sim(false);
        app.shortcuts(&ctx);
        if app.status != self.status_seen {
            self.status_seen = app.status.clone();
            self.status_since = Instant::now();
        } else if !app.status.is_empty() && self.status_since.elapsed().as_secs() > 6 {
            app.status.clear();
            self.status_seen.clear();
        }
        // Unsaved changes guard.
        if ctx.input(|i| i.viewport().close_requested()) && app.any_modified() && !app.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            app.confirm_close = true;
        }
        let mut cmds: Vec<Cmd> = Vec::new();
        cmds.extend(app.titlebar(ui));
        if app.home || !app.has_doc {
            cmds.extend(app.home_screen(ui));
        } else {
            cmds.extend(app.ribbon(ui));
            cmds.extend(app.sim_bar(ui));
            app.scopes(ui);
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(c::CANVAS))
                .show(ui, |ui| app.canvas(ui));
            if app.show_components {
                cmds.extend(app.components_panel(&ctx));
            }
            app.scope_window(&ctx);
            app.breadcrumb(&ctx);
            app.component_editor(&ctx);
            if app.show_inspector {
                app.inspector(&ctx);
            }
            cmds.extend(app.nav_toolbar(&ctx));
        }
        cmds.extend(app.palette(&ctx));
        app.dialogs(&ctx);
        if let Some(which) = app.analysis.dialog_ui(&ctx, &app.built) {
            cmds.push(Cmd::RunAnalysis(which));
        }
        app.analysis.windows(&ctx);
        if app.import.ui(&ctx, &mut app.cat) {
            app.rev += 1;
        }
        if app.confirm_close {
            let names: Vec<String> = (0..app.tabs.len())
                .filter_map(|k| {
                    let (name, modified) = app.tab_info(k);
                    modified.then_some(name)
                })
                .collect();
            egui::Window::new("Save changes?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(&ctx, |ui| {
                    ui.label(format!("Unsaved changes in {}.", names.join(", ")));
                    ui.horizontal(|ui| {
                        if ui.button("Save All").clicked() {
                            // Quits once everything is saved; a cancelled Save As keeps the app open.
                            app.confirm_close = false;
                            cmds.push(Cmd::SaveAll);
                            cmds.push(Cmd::Quit);
                        }
                        if ui.button("Discard").clicked() {
                            app.allow_close = true;
                            cmds.push(Cmd::Quit);
                        }
                        if ui.button("Cancel").clicked() {
                            app.confirm_close = false;
                        }
                    });
                });
        }
        if let Some(k) = app.confirm_tab {
            let name = if k < app.tabs.len() {
                app.tab_info(k).0
            } else {
                String::new()
            };
            egui::Window::new("Close tab?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(&ctx, |ui| {
                    ui.label(format!("{name} has unsaved changes."));
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            app.confirm_tab = None;
                            app.switch_tab(k);
                            app.run(Cmd::Save, &ctx);
                            if !app.modified() {
                                app.close_tab(app.active, true);
                            }
                        }
                        if ui.button("Discard").clicked() {
                            app.confirm_tab = None;
                            app.close_tab(k, true);
                        }
                        if ui.button("Cancel").clicked() {
                            app.confirm_tab = None;
                        }
                    });
                });
        }
        for cmd in cmds {
            app.run(cmd, &ctx);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        c::TITLEBAR.to_normalized_gamma_f32()
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("electrocraft {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--datasheet") {
        // Desktop action: hand over to the companion tool beside this binary.
        if let Some(exe) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("electrocraft-datasheet")))
        {
            let _ = std::process::Command::new(exe).spawn();
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--command-catalog") {
        println!("{}", commands::catalog_json());
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "usage: electrocraft [file.ecs] [--new] [--sample <id>] [--software-canvas] [--size WxH] [--commands file.json] [--command-catalog] [--version]"
        );
        println!(
            "samples: {}",
            samples::all()
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(());
    }
    let software = args.iter().any(|a| a == "--software-canvas")
        || std::env::var_os("ELECTROCRAFT_SOFTWARE_CANVAS").is_some();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let size = value("--size")
        .and_then(|s| {
            s.split_once('x')
                .and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?)))
        })
        .unwrap_or((1440.0, 900.0));
    let sample = value("--sample");
    let script = value("--commands");
    let file = args
        .iter()
        .enumerate()
        .find(|(i, a)| {
            !a.starts_with("--")
                && (*i == 0
                    || !matches!(args[i - 1].as_str(), "--sample" | "--size" | "--commands"))
        })
        .map(|(_, a)| PathBuf::from(a));
    let new = args.iter().any(|a| a == "--new");

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Electrocraft")
            .with_app_id(APP_ID)
            .with_decorations(false)
            .with_inner_size([size.0, size.1])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Electrocraft",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = App::new(software);
            if let Some(p) = file {
                app.run(Cmd::OpenPath(p), &cc.egui_ctx);
            } else if let Some(s) = sample {
                if let Some(found) = samples::all().into_iter().find(|x| x.id == s) {
                    app.run(Cmd::OpenSample(found.id), &cc.egui_ctx);
                }
            } else if new {
                app.run(Cmd::New, &cc.egui_ctx);
            }
            // --commands <file.json>: [{"id": "...", "args": {...}}, ...]
            if let Some(path) = &script {
                let list: Vec<serde_json::Value> = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_default();
                for item in list {
                    let id = item["id"].as_str().unwrap_or_default();
                    match commands::from_id(id, &item["args"]) {
                        Ok(c2) => app.run(c2, &cc.egui_ctx),
                        Err(e) => eprintln!("electrocraft: {e}"),
                    }
                }
            }
            Ok(Box::new(Shell {
                app,
                status_seen: String::new(),
                status_since: Instant::now(),
            }))
        }),
    )
}
