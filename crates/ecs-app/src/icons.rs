//! Vector icons on a 24-unit design grid, painted with egui so they stay
//! sharp at any scale. Two-tone like SolidCraft's ribbon: a line colour and
//! an accent fill.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};
use std::f32::consts::PI;

/// Every icon the shell uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Logo,
    Home,
    New,
    Open,
    Save,
    Undo,
    Redo,
    Plus,
    Close,
    Settings,
    Help,
    Search,
    Select,
    Wire,
    Label,
    Text,
    Probe,
    Rotate,
    Mirror,
    Delete,
    Play,
    Pause,
    Step,
    Reset,
    Fit,
    ZoomIn,
    ZoomOut,
    Grid,
    Voltage,
    Current,
    DcOp,
    Sweep,
    Transient,
    Ac,
    Scope,
    Import,
    Minimize,
    Maximize,
    Duplicate,
    Warning,
    Pan,
    Chevron,
    Info,
}

struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    s: f32,
    line: Color32,
    accent: Color32,
}

impl Pen<'_> {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.r.min.x + x * self.s, self.r.min.y + y * self.s)
    }
    fn stroke(&self) -> Stroke {
        Stroke::new((1.6 * self.s).max(1.0), self.line)
    }
    fn l(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(Shape::line(v, self.stroke()));
    }
    fn closed(&self, pts: &[(f32, f32)], fill: Option<Color32>) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        match fill {
            Some(f) => {
                self.p.add(Shape::convex_polygon(v, f, self.stroke()));
            }
            None => {
                self.p.add(Shape::closed_line(v, self.stroke()));
            }
        }
    }
    fn fill(&self, pts: &[(f32, f32)], color: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(Shape::convex_polygon(v, color, Stroke::NONE));
    }
    fn circle(&self, x: f32, y: f32, r: f32, fill: Option<Color32>) {
        match fill {
            Some(f) => {
                self.p.circle(self.pt(x, y), r * self.s, f, self.stroke());
            }
            None => {
                self.p
                    .circle_stroke(self.pt(x, y), r * self.s, self.stroke());
            }
        }
    }
    fn dot(&self, x: f32, y: f32, r: f32, color: Color32) {
        self.p.circle_filled(self.pt(x, y), r * self.s, color);
    }
    fn arc(&self, x: f32, y: f32, r: f32, a0: f32, a1: f32) {
        let n = 20;
        let pts: Vec<(f32, f32)> = (0..=n)
            .map(|k| {
                let a = a0 + (a1 - a0) * k as f32 / n as f32;
                (x + r * a.cos(), y + r * a.sin())
            })
            .collect();
        self.l(&pts);
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, fill: Option<Color32>) {
        self.closed(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], fill);
    }
}

/// Paint `icon` inside `rect` (square) with a line colour; the accent is the
/// SolidCraft icon blue unless `mono` is set.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, line: Color32, mono: bool) {
    let side = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), egui::vec2(side, side));
    let accent = if mono { line } else { crate::theme::c::ICON };
    let pen = Pen {
        p,
        r,
        s: side / 24.0,
        line,
        accent,
    };
    let a = pen.accent;
    let faint = a.gamma_multiply(0.35);
    match icon {
        Icon::Logo => {
            pen.fill(
                &[
                    (13.0, 2.0),
                    (5.0, 13.5),
                    (11.0, 13.5),
                    (9.0, 22.0),
                    (19.0, 9.5),
                    (13.0, 9.5),
                    (15.0, 2.0),
                ],
                a,
            );
        }
        Icon::Home => {
            pen.closed(
                &[
                    (4.0, 11.0),
                    (12.0, 4.0),
                    (20.0, 11.0),
                    (20.0, 20.0),
                    (4.0, 20.0),
                ],
                Some(a),
            );
            pen.fill(
                &[(10.0, 20.0), (10.0, 14.0), (14.0, 14.0), (14.0, 20.0)],
                crate::theme::c::TITLEBAR,
            );
        }
        Icon::New => {
            pen.closed(
                &[
                    (5.0, 3.0),
                    (14.0, 3.0),
                    (19.0, 8.0),
                    (19.0, 21.0),
                    (5.0, 21.0),
                ],
                Some(faint),
            );
            pen.l(&[(14.0, 3.0), (14.0, 8.0), (19.0, 8.0)]);
            pen.circle(17.5, 18.0, 4.5, Some(a));
            pen.l(&[(17.5, 15.8), (17.5, 20.2)]);
            pen.l(&[(15.3, 18.0), (19.7, 18.0)]);
        }
        Icon::Open => {
            pen.closed(
                &[
                    (3.0, 6.0),
                    (9.0, 6.0),
                    (11.0, 8.0),
                    (20.0, 8.0),
                    (20.0, 19.0),
                    (3.0, 19.0),
                ],
                Some(faint),
            );
            pen.closed(
                &[(3.0, 19.0), (6.0, 11.0), (22.0, 11.0), (20.0, 19.0)],
                Some(a),
            );
        }
        Icon::Save => {
            pen.closed(
                &[
                    (4.0, 4.0),
                    (17.0, 4.0),
                    (20.0, 7.0),
                    (20.0, 20.0),
                    (4.0, 20.0),
                ],
                Some(faint),
            );
            pen.rect(8.0, 4.0, 15.0, 9.0, None);
            pen.rect(7.0, 13.0, 17.0, 20.0, Some(a));
        }
        Icon::Undo => {
            pen.arc(13.0, 13.0, 6.5, -PI / 2.0, PI * 0.75);
            pen.fill(&[(4.0, 6.5), (11.0, 2.5), (11.0, 10.5)], a);
        }
        Icon::Redo => {
            pen.arc(11.0, 13.0, 6.5, PI * 0.25, PI * 1.5);
            pen.fill(&[(20.0, 6.5), (13.0, 2.5), (13.0, 10.5)], a);
        }
        Icon::Plus => {
            pen.l(&[(12.0, 5.0), (12.0, 19.0)]);
            pen.l(&[(5.0, 12.0), (19.0, 12.0)]);
        }
        Icon::Close => {
            pen.l(&[(6.0, 6.0), (18.0, 18.0)]);
            pen.l(&[(18.0, 6.0), (6.0, 18.0)]);
        }
        Icon::Minimize => pen.l(&[(6.0, 12.0), (18.0, 12.0)]),
        Icon::Maximize => pen.rect(6.5, 6.5, 17.5, 17.5, None),
        Icon::Settings => {
            for k in 0..8 {
                let ang = k as f32 * PI / 4.0;
                pen.l(&[
                    (12.0 + 6.0 * ang.cos(), 12.0 + 6.0 * ang.sin()),
                    (12.0 + 9.0 * ang.cos(), 12.0 + 9.0 * ang.sin()),
                ]);
            }
            pen.circle(12.0, 12.0, 6.0, Some(a));
            pen.dot(12.0, 12.0, 2.3, crate::theme::c::TITLEBAR);
        }
        Icon::Help => {
            pen.circle(12.0, 12.0, 9.0, Some(a));
            let dark = crate::theme::c::TITLEBAR;
            let s = Stroke::new(1.8 * pen.s, dark);
            let pts: Vec<Pos2> = (0..=12)
                .map(|k| {
                    let ang = PI * 1.1 + (PI * 1.4) * k as f32 / 12.0;
                    pen.pt(12.0 + 3.0 * ang.cos(), 10.0 + 3.0 * ang.sin())
                })
                .chain([pen.pt(12.0, 14.0)])
                .collect();
            p.add(Shape::line(pts, s));
            pen.dot(12.0, 17.0, 1.2, dark);
        }
        Icon::Search => {
            pen.circle(10.0, 10.0, 6.0, None);
            pen.l(&[(14.5, 14.5), (20.0, 20.0)]);
        }
        Icon::Select => {
            pen.closed(
                &[
                    (6.0, 3.0),
                    (6.0, 19.0),
                    (10.0, 15.0),
                    (13.0, 21.0),
                    (15.5, 20.0),
                    (12.5, 14.0),
                    (18.0, 14.0),
                ],
                Some(a),
            );
        }
        Icon::Wire => {
            pen.l(&[(4.0, 18.0), (12.0, 18.0), (12.0, 6.0), (20.0, 6.0)]);
            pen.dot(4.0, 18.0, 2.5, a);
            pen.dot(20.0, 6.0, 2.5, a);
            pen.dot(12.0, 12.0, 2.0, a);
        }
        Icon::Label => {
            pen.closed(
                &[
                    (3.0, 8.0),
                    (16.0, 8.0),
                    (21.0, 12.0),
                    (16.0, 16.0),
                    (3.0, 16.0),
                ],
                Some(faint),
            );
            pen.dot(16.0, 12.0, 1.6, a);
        }
        Icon::Text => {
            pen.l(&[(5.0, 6.0), (19.0, 6.0)]);
            pen.l(&[(12.0, 6.0), (12.0, 19.0)]);
            pen.l(&[(9.0, 19.0), (15.0, 19.0)]);
        }
        Icon::Probe => {
            pen.closed(
                &[(14.0, 3.0), (21.0, 10.0), (12.0, 19.0), (5.0, 12.0)],
                Some(faint),
            );
            pen.l(&[(5.0, 19.0), (8.5, 15.5)]);
            pen.dot(4.0, 20.0, 2.0, a);
            pen.l(&[(10.0, 11.0), (13.0, 14.0)]);
        }
        Icon::Rotate => {
            pen.arc(12.0, 12.0, 7.0, -PI * 0.9, PI * 0.6);
            pen.fill(&[(3.0, 7.0), (9.0, 4.0), (8.5, 10.0)], a);
        }
        Icon::Mirror => {
            pen.l(&[(12.0, 3.0), (12.0, 21.0)]);
            pen.closed(&[(9.0, 6.0), (9.0, 18.0), (3.0, 18.0)], Some(faint));
            pen.closed(&[(15.0, 6.0), (15.0, 18.0), (21.0, 18.0)], Some(a));
        }
        Icon::Delete => {
            pen.l(&[(4.0, 7.0), (20.0, 7.0)]);
            pen.l(&[(9.0, 7.0), (10.0, 4.0), (14.0, 4.0), (15.0, 7.0)]);
            pen.closed(
                &[(6.0, 7.0), (18.0, 7.0), (17.0, 20.0), (7.0, 20.0)],
                Some(faint),
            );
            pen.l(&[(10.0, 10.0), (10.0, 17.0)]);
            pen.l(&[(14.0, 10.0), (14.0, 17.0)]);
        }
        Icon::Play => pen.fill(&[(7.0, 4.0), (19.0, 12.0), (7.0, 20.0)], a),
        Icon::Pause => {
            pen.fill(&[(6.0, 5.0), (10.0, 5.0), (10.0, 19.0), (6.0, 19.0)], a);
            pen.fill(&[(14.0, 5.0), (18.0, 5.0), (18.0, 19.0), (14.0, 19.0)], a);
        }
        Icon::Step => {
            pen.fill(&[(5.0, 5.0), (15.0, 12.0), (5.0, 19.0)], a);
            pen.fill(&[(16.0, 5.0), (19.0, 5.0), (19.0, 19.0), (16.0, 19.0)], a);
        }
        Icon::Reset => {
            pen.fill(&[(5.0, 5.0), (8.0, 5.0), (8.0, 19.0), (5.0, 19.0)], a);
            pen.fill(&[(19.0, 5.0), (9.0, 12.0), (19.0, 19.0)], a);
        }
        Icon::Fit => {
            for (x, y, dx, dy) in [
                (4.0, 4.0, 1.0, 1.0),
                (20.0, 4.0, -1.0, 1.0),
                (4.0, 20.0, 1.0, -1.0),
                (20.0, 20.0, -1.0, -1.0),
            ] {
                pen.l(&[(x + 5.0 * dx, y), (x, y), (x, y + 5.0 * dy)]);
            }
            pen.rect(9.0, 9.0, 15.0, 15.0, Some(a));
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            pen.circle(10.0, 10.0, 6.5, None);
            pen.l(&[(15.0, 15.0), (20.5, 20.5)]);
            pen.l(&[(7.0, 10.0), (13.0, 10.0)]);
            if icon == Icon::ZoomIn {
                pen.l(&[(10.0, 7.0), (10.0, 13.0)]);
            }
        }
        Icon::Grid => {
            for k in [4.0, 10.0, 16.0] {
                for j in [4.0, 10.0, 16.0] {
                    pen.dot(k + 2.0, j + 2.0, 1.4, a);
                }
            }
        }
        Icon::Voltage => {
            pen.l(&[(3.0, 17.0), (10.0, 17.0)]);
            pen.l(&[(14.0, 7.0), (21.0, 7.0)]);
            let p0 = pen.pt(10.0, 17.0);
            let p1 = pen.pt(14.0, 7.0);
            p.line_segment([p0, p1], Stroke::new(2.4 * pen.s, a));
        }
        Icon::Current => {
            pen.l(&[(3.0, 12.0), (21.0, 12.0)]);
            for x in [6.0, 12.0, 18.0] {
                pen.dot(x, 12.0, 2.4, a);
            }
        }
        Icon::DcOp => {
            pen.circle(12.0, 10.0, 6.5, Some(a));
            pen.rect(9.0, 16.0, 15.0, 20.0, None);
        }
        Icon::Sweep => {
            pen.l(&[(4.0, 20.0), (4.0, 4.0)]);
            pen.l(&[(4.0, 20.0), (20.0, 20.0)]);
            pen.l(&[
                (5.0, 18.0),
                (9.0, 17.0),
                (12.0, 13.0),
                (15.0, 8.0),
                (20.0, 6.0),
            ]);
        }
        Icon::Transient => {
            pen.l(&[(3.0, 12.0), (21.0, 12.0)]);
            let pts: Vec<(f32, f32)> = (0..=24)
                .map(|k| {
                    let x = 3.0 + 18.0 * k as f32 / 24.0;
                    (x, 12.0 - 6.0 * (2.0 * PI * k as f32 / 12.0).sin())
                })
                .collect();
            let v: Vec<Pos2> = pts.iter().map(|&(x, y)| pen.pt(x, y)).collect();
            p.add(Shape::line(v, Stroke::new(1.8 * pen.s, a)));
        }
        Icon::Ac => {
            pen.l(&[(4.0, 20.0), (4.0, 4.0)]);
            pen.l(&[(4.0, 20.0), (20.0, 20.0)]);
            let v: Vec<Pos2> = [
                (5.0, 8.0),
                (11.0, 8.0),
                (14.0, 10.0),
                (17.0, 15.0),
                (20.0, 19.0),
            ]
            .iter()
            .map(|&(x, y)| pen.pt(x, y))
            .collect();
            p.add(Shape::line(v, Stroke::new(1.8 * pen.s, a)));
        }
        Icon::Scope => {
            pen.rect(3.0, 4.0, 21.0, 18.0, Some(faint));
            let v: Vec<Pos2> = [
                (5.0, 13.0),
                (8.0, 13.0),
                (10.0, 7.0),
                (13.0, 15.0),
                (15.0, 10.0),
                (19.0, 10.0),
            ]
            .iter()
            .map(|&(x, y)| pen.pt(x, y))
            .collect();
            p.add(Shape::line(v, Stroke::new(1.8 * pen.s, a)));
            pen.l(&[(9.0, 21.0), (15.0, 21.0)]);
        }
        Icon::Import => {
            pen.l(&[(4.0, 14.0), (4.0, 20.0), (20.0, 20.0), (20.0, 14.0)]);
            pen.l(&[(12.0, 3.0), (12.0, 14.0)]);
            pen.fill(&[(8.0, 11.0), (16.0, 11.0), (12.0, 16.0)], a);
        }
        Icon::Duplicate => {
            pen.rect(8.0, 8.0, 20.0, 20.0, Some(faint));
            pen.l(&[(4.0, 15.0), (4.0, 4.0), (15.0, 4.0)]);
            pen.l(&[(14.0, 11.0), (14.0, 17.0)]);
            pen.l(&[(11.0, 14.0), (17.0, 14.0)]);
        }
        Icon::Warning => {
            pen.closed(
                &[(12.0, 3.0), (22.0, 20.0), (2.0, 20.0)],
                Some(crate::theme::c::WARN),
            );
            pen.dot(12.0, 17.0, 1.2, crate::theme::c::TITLEBAR);
            p.line_segment(
                [pen.pt(12.0, 8.5), pen.pt(12.0, 14.0)],
                Stroke::new(2.0 * pen.s, crate::theme::c::TITLEBAR),
            );
        }
        Icon::Pan => {
            pen.l(&[(12.0, 3.0), (12.0, 21.0)]);
            pen.l(&[(3.0, 12.0), (21.0, 12.0)]);
            for pts in [
                [(12.0, 2.0), (9.0, 6.0), (15.0, 6.0)],
                [(12.0, 22.0), (9.0, 18.0), (15.0, 18.0)],
                [(2.0, 12.0), (6.0, 9.0), (6.0, 15.0)],
                [(22.0, 12.0), (18.0, 9.0), (18.0, 15.0)],
            ] {
                pen.fill(&pts, a);
            }
        }
        Icon::Chevron => pen.l(&[(7.0, 10.0), (12.0, 15.0), (17.0, 10.0)]),
        Icon::Info => {
            pen.circle(12.0, 12.0, 9.0, Some(a));
            pen.dot(12.0, 7.5, 1.3, crate::theme::c::TITLEBAR);
            p.line_segment(
                [pen.pt(12.0, 10.5), pen.pt(12.0, 17.0)],
                Stroke::new(2.0 * pen.s, crate::theme::c::TITLEBAR),
            );
        }
    }
}

/// An icon on its own: allocate `size` and paint.
pub fn show(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    paint(ui.painter(), rect, icon, color, false);
    resp
}
