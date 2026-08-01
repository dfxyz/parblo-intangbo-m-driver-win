use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::device::hid::PenRanges;
use crate::gui::App;
use crate::shared::Monitor;

impl App {
    pub(super) fn monitor_page(&mut self, ui: &mut egui::Ui) {
        // 事件率两百多赫兹，这里按帧率主动刷新，驱动侧不必逐事件唤醒界面
        ui.ctx().request_repaint();

        let status = self.shared.status();
        let monitor = self.shared.monitor();
        let ranges = status.ranges.unwrap_or_default();

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.tablet_view(ui, &monitor, ranges);
            ui.add_space(8.0);
            pen_values(ui, &monitor, ranges);
            ui.add_space(8.0);
            button_values(ui, &monitor);
        });
    }

    fn tablet_view(&self, ui: &mut egui::Ui, monitor: &Monitor, ranges: PenRanges) {
        let aspect = ranges.x_max.max(1) as f32 / ranges.y_max.max(1) as f32;
        let width = ui.available_width().min(420.0);
        let size = Vec2::new(width, width / aspect);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 4.0, Color32::from_gray(32));
        for index in 1..4 {
            let ratio = index as f32 / 4.0;
            let x = rect.left() + rect.width() * ratio;
            let y = rect.top() + rect.height() * ratio;
            let grid = Stroke::new(1.0, Color32::from_gray(52));
            painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], grid);
            painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], grid);
        }

        let Some(pen) = monitor.pen else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "把笔靠近绘图板",
                egui::FontId::proportional(14.0),
                Color32::from_gray(120),
            );
            return;
        };
        if !pen.in_area {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "笔已离开感应区",
                egui::FontId::proportional(14.0),
                Color32::from_gray(120),
            );
            return;
        }

        let x = rect.left() + rect.width() * pen.x as f32 / ranges.x_max.max(1) as f32;
        let y = rect.top() + rect.height() * pen.y as f32 / ranges.y_max.max(1) as f32;
        let center = Pos2::new(x, y);
        let color = if pen.tip_pressed {
            Color32::from_rgb(0x5c, 0xc8, 0x7a)
        } else {
            Color32::from_rgb(0x4a, 0x9e, 0xdd)
        };
        let radius = if pen.tip_pressed {
            4.0 + 10.0 * pen.pressure as f32 / ranges.pressure_max.max(1) as f32
        } else {
            4.0
        };
        painter.circle_filled(center, radius, color);
        let cross = Stroke::new(1.0, color.gamma_multiply(0.5));
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            cross,
        );
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            cross,
        );
    }
}

fn pen_values(ui: &mut egui::Ui, monitor: &Monitor, ranges: PenRanges) {
    ui.label(RichText::new("笔").strong());
    egui::Grid::new("pen_values")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            let Some(pen) = monitor.pen else {
                ui.label("状态");
                ui.label("尚未收到笔事件");
                ui.end_row();
                return;
            };
            ui.label("坐标");
            ui.label(format!(
                "X = {} / {}      Y = {} / {}",
                pen.x, ranges.x_max, pen.y, ranges.y_max
            ));
            ui.end_row();

            ui.label("压力");
            let ratio = pen.pressure as f32 / ranges.pressure_max.max(1) as f32;
            ui.label(format!(
                "{} / {}   ({:.1}%)",
                pen.pressure,
                ranges.pressure_max,
                ratio * 100.0
            ));
            ui.end_row();

            ui.label("倾斜");
            ui.label(format!("X = {}°      Y = {}°", pen.tilt_x, pen.tilt_y));
            ui.end_row();

            ui.label("状态");
            ui.label(format!(
                "{}{}{}{}",
                if pen.in_area { "感应区内 " } else { "感应区外 " },
                if pen.tip_pressed { "笔尖 " } else { "" },
                if pen.button0_pressed { "下侧键 " } else { "" },
                if pen.button1_pressed { "上侧键" } else { "" },
            ));
            ui.end_row();

            ui.label("累计事件");
            ui.label(format!("{}", monitor.pen_count));
            ui.end_row();
        });
}

fn button_values(ui: &mut egui::Ui, monitor: &Monitor) {
    ui.label(RichText::new("按键").strong());
    egui::Grid::new("button_values")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            ui.label("最近一次");
            match monitor.button {
                Some(button) => ui.label(format!("{:?}", button)),
                None => ui.label("尚未收到按键事件"),
            };
            ui.end_row();

            ui.label("累计事件");
            ui.label(format!("{}", monitor.button_count));
            ui.end_row();
        });
}

/// 供画布使用的辅助：把归一化坐标映射到矩形内
pub fn lerp_in(rect: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(
        rect.left() + rect.width() * x,
        rect.bottom() - rect.height() * y,
    )
}
