use egui::{Color32, Pos2, RichText, Sense, Stroke, Vec2};

use crate::config::PressureCurve;
use crate::gui::App;
use crate::gui::monitor::lerp_in;

const HANDLE_RADIUS: f32 = 5.0;

impl App {
    pub(super) fn pressure_page(&mut self, ui: &mut egui::Ui) {
        ui.ctx().request_repaint();
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(
                RichText::new(
                    "横轴为绘图板上报的原始压力，纵轴为注入系统的压力。\
                     拖动控制点调整曲线，双击空白处添加控制点，右键控制点删除。\
                     末点左移即可提前到达满压（如拖到 50% 处，用一半力就是满压），\
                     首点右移则忽略过轻的压力。",
                )
                .weak(),
            );
            ui.add_space(8.0);
            self.curve_editor(ui);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("重置为直线").clicked() {
                    self.pressure = PressureCurve::default();
                }
                if ui.button("柔和起笔").clicked() {
                    self.pressure.points = vec![[0.0, 0.0], [0.5, 0.3], [1.0, 1.0]];
                }
                if ui.button("硬朗起笔").clicked() {
                    self.pressure.points = vec![[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]];
                }
            });

            ui.add_space(6.0);
            let status = self.shared.status();
            match (self.shared.monitor().pen, status.ranges) {
                (Some(pen), Some(ranges)) => {
                    let raw = pen.pressure as f32 / ranges.pressure_max.max(1) as f32;
                    ui.label(format!(
                        "当前压力：原始 {:.1}% → 输出 {:.1}%",
                        raw * 100.0,
                        self.pressure.evaluate(raw) * 100.0
                    ));
                }
                _ => {
                    ui.label(RichText::new("当前压力：把笔压在绘图板上即可看到实时换算").weak());
                }
            }
        });
    }

    fn curve_editor(&mut self, ui: &mut egui::Ui) {
        let size = ui.available_width().min(375.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 4.0, Color32::from_gray(32));
        for index in 1..4 {
            let ratio = index as f32 / 4.0;
            let grid = Stroke::new(1.0, Color32::from_gray(52));
            let x = rect.left() + rect.width() * ratio;
            let y = rect.top() + rect.height() * ratio;
            painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], grid);
            painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], grid);
        }
        painter.line_segment(
            [
                Pos2::new(rect.left(), rect.bottom()),
                Pos2::new(rect.right(), rect.top()),
            ],
            Stroke::new(1.0, Color32::from_gray(60)),
        );

        self.handle_curve_input(&response, rect);

        let curve_color = Color32::from_rgb(0x4a, 0x9e, 0xdd);
        let samples: Vec<Pos2> = (0..=64)
            .map(|step| {
                let x = step as f32 / 64.0;
                lerp_in(rect, x, self.pressure.evaluate(x))
            })
            .collect();
        for pair in samples.windows(2) {
            painter.line_segment([pair[0], pair[1]], Stroke::new(2.0, curve_color));
        }

        for point in &self.pressure.points {
            let center = lerp_in(rect, point[0], point[1]);
            painter.circle_filled(center, HANDLE_RADIUS, Color32::WHITE);
            painter.circle_filled(center, HANDLE_RADIUS - 2.0, curve_color);
        }
    }

    fn handle_curve_input(&mut self, response: &egui::Response, rect: egui::Rect) {
        let to_normalized = |pos: Pos2| {
            [
                ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0),
            ]
        };

        if response.double_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let point = to_normalized(pos);
                self.pressure.points.push(point);
                self.pressure.sort_points();
            }
            return;
        }

        if response.secondary_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(index) = self.nearest_point(rect, pos) {
                    let last = self.pressure.points.len() - 1;
                    if self.pressure.points.len() > 2 && index != 0 && index != last {
                        self.pressure.points.remove(index);
                    }
                }
            }
            return;
        }

        if response.drag_started() {
            self.dragging_point = response
                .interact_pointer_pos()
                .and_then(|pos| self.nearest_point(rect, pos));
        }
        if response.dragged() {
            if let (Some(index), Some(pos)) =
                (self.dragging_point, response.interact_pointer_pos())
            {
                // 控制点可以自由移动：末点左移即提前满压，首点右移即忽略轻压，
                // 两端之外的输入由求值逻辑按常数外推
                self.pressure.points[index] = to_normalized(pos);
            }
        }
        if response.drag_stopped() {
            self.pressure.sort_points();
            self.dragging_point = None;
        }
    }

    fn nearest_point(&self, rect: egui::Rect, pos: Pos2) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (index, point) in self.pressure.points.iter().enumerate() {
            let center = lerp_in(rect, point[0], point[1]);
            let distance = center.distance(pos);
            if distance > HANDLE_RADIUS * 3.0 {
                continue;
            }
            if best.is_none_or(|(_, best)| distance < best) {
                best = Some((index, distance));
            }
        }
        best.map(|(index, _)| index)
    }
}
