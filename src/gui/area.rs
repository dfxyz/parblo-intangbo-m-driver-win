use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
};

use crate::config::{AreaAnchor, TabletArea};
use crate::device::hid::PenRanges;
use crate::gui::App;

/// 本次会话中笔实际到达过的坐标范围
pub struct Observed {
    pub min_x: u16,
    pub max_x: u16,
    pub min_y: u16,
    pub max_y: u16,
    pub samples: u64,
}
impl Default for Observed {
    fn default() -> Self {
        Self {
            min_x: u16::MAX,
            max_x: 0,
            min_y: u16::MAX,
            max_y: 0,
            samples: 0,
        }
    }
}
impl Observed {
    fn record(&mut self, x: u16, y: u16) {
        self.min_x = self.min_x.min(x);
        self.max_x = self.max_x.max(x);
        self.min_y = self.min_y.min(y);
        self.max_y = self.max_y.max(y);
        self.samples += 1;
    }

    fn is_empty(&self) -> bool {
        self.samples == 0
    }
}

impl App {
    pub(super) fn area_page(&mut self, ui: &mut egui::Ui) {
        ui.ctx().request_repaint();
        let ranges = self.shared.status().ranges.unwrap_or_default();
        if let Some(pen) = self.shared.monitor().pen {
            if pen.in_area {
                self.observed.record(pen.x, pen.y);
            }
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(
                RichText::new(
                    "设备声明的坐标范围通常大于真实可感应范围。把笔贴着板子边缘划一圈测出真实范围后，\
                     再按显示器比例取一块矩形映射到全屏，笔迹就不会变形。",
                )
                .weak(),
            );
            ui.add_space(8.0);

            self.live_readout(ui, ranges);
            ui.add_space(10.0);
            self.area_preview(ui, ranges);
        });
    }

    /// 实时原始坐标与观测极值，用来判断笔是否已经顶到感应边缘
    fn live_readout(&mut self, ui: &mut egui::Ui, ranges: PenRanges) {
        egui::Grid::new("area_readout")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label("当前坐标");
                match self.shared.monitor().pen.filter(|pen| pen.in_area) {
                    Some(pen) => ui.label(format!(
                        "X = {:>5} ({:>5.1}%)      Y = {:>5} ({:>5.1}%)",
                        pen.x,
                        pen.x as f32 * 100.0 / ranges.x_max.max(1) as f32,
                        pen.y,
                        pen.y as f32 * 100.0 / ranges.y_max.max(1) as f32,
                    )),
                    None => ui.label("笔不在感应区内"),
                };
                ui.end_row();

                ui.label("观测极值");
                if self.observed.is_empty() {
                    ui.label("把笔贴着板子边缘划一圈即可测出真实范围");
                } else {
                    ui.label(format!(
                        "X = {}..{}      Y = {}..{}      （{} 个样本）",
                        self.observed.min_x,
                        self.observed.max_x,
                        self.observed.min_y,
                        self.observed.max_y,
                        self.observed.samples
                    ));
                }
                ui.end_row();

                ui.label("声明量程");
                ui.label(format!(
                    "X = 0..{}      Y = 0..{}",
                    ranges.x_max, ranges.y_max
                ));
                ui.end_row();

                ui.label("当前区域");
                ui.label(format!(
                    "X = {}..{}      Y = {}..{}",
                    (self.area.x_min * ranges.x_max as f32).round() as u32,
                    (self.area.x_max * ranges.x_max as f32).round() as u32,
                    (self.area.y_min * ranges.y_max as f32).round() as u32,
                    (self.area.y_max * ranges.y_max as f32).round() as u32,
                ));
                ui.end_row();
            });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let usable = !self.observed.is_empty();
            if ui
                .add_enabled(usable, egui::Button::new("用观测极值填充区域"))
                .clicked()
            {
                self.area.x_min = self.observed.min_x as f32 / ranges.x_max.max(1) as f32;
                self.area.x_max = self.observed.max_x as f32 / ranges.x_max.max(1) as f32;
                self.area.y_min = self.observed.min_y as f32 / ranges.y_max.max(1) as f32;
                self.area.y_max = self.observed.max_y as f32 / ranges.y_max.max(1) as f32;
                self.set_message("已按观测极值填充，记得保存".to_string(), false);
            }
            if ui.button("清除观测记录").clicked() {
                self.observed = Observed::default();
            }
            if ui.button("重置为全区域").clicked() {
                let anchor = self.area.anchor;
                self.area = TabletArea {
                    anchor,
                    ..Default::default()
                };
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for anchor in AreaAnchor::ALL {
                if ui
                    .selectable_label(self.area.anchor == anchor, anchor.label())
                    .clicked()
                {
                    self.area.anchor = anchor;
                }
            }
        });
    }

    fn area_preview(&self, ui: &mut egui::Ui, ranges: PenRanges) {
        let aspect = ranges.x_max.max(1) as f32 / ranges.y_max.max(1) as f32;
        let width = ui.available_width().min(460.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, width / aspect), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_gray(32));

        let to_rect = |x_min: f32, x_max: f32, y_min: f32, y_max: f32| {
            Rect::from_min_max(
                Pos2::new(
                    rect.left() + rect.width() * x_min,
                    rect.top() + rect.height() * y_min,
                ),
                Pos2::new(
                    rect.left() + rect.width() * x_max,
                    rect.top() + rect.height() * y_max,
                ),
            )
        };

        let area_rect = to_rect(
            self.area.x_min,
            self.area.x_max,
            self.area.y_min,
            self.area.y_max,
        );
        painter.rect_filled(area_rect, 0.0, Color32::from_rgb(0x22, 0x3a, 0x4a));

        // 实际映射到屏幕的那块，靠向配置指定的角
        let (screen_width, screen_height) = screen_size();
        let screen_aspect = screen_width.max(1) as f32 / screen_height.max(1) as f32;
        let area_width = self.area.width() * ranges.x_max as f32;
        let area_height = self.area.height() * ranges.y_max as f32;
        let (used_width, used_height) = if area_width / area_height > screen_aspect {
            (area_height * screen_aspect, area_height)
        } else {
            (area_width, area_width / screen_aspect)
        };
        let used_size = Vec2::new(
            rect.width() * used_width / ranges.x_max.max(1) as f32,
            rect.height() * used_height / ranges.y_max.max(1) as f32,
        );
        let used_min = Pos2::new(
            if self.area.anchor.is_right() {
                area_rect.right() - used_size.x
            } else {
                area_rect.left()
            },
            if self.area.anchor.is_bottom() {
                area_rect.bottom() - used_size.y
            } else {
                area_rect.top()
            },
        );
        let used_rect = Rect::from_min_size(used_min, used_size);
        painter.rect_filled(used_rect, 0.0, Color32::from_rgb(0x2f, 0x5d, 0x7a));
        painter.rect_stroke(
            used_rect,
            0.0,
            Stroke::new(1.5, Color32::from_rgb(0x4a, 0x9e, 0xdd)),
            egui::StrokeKind::Inside,
        );

        if !self.observed.is_empty() {
            let observed_rect = to_rect(
                self.observed.min_x as f32 / ranges.x_max.max(1) as f32,
                self.observed.max_x as f32 / ranges.x_max.max(1) as f32,
                self.observed.min_y as f32 / ranges.y_max.max(1) as f32,
                self.observed.max_y as f32 / ranges.y_max.max(1) as f32,
            );
            painter.rect_stroke(
                observed_rect,
                0.0,
                Stroke::new(1.0, Color32::from_rgb(0xc8, 0x96, 0x30)),
                egui::StrokeKind::Inside,
            );
        }

        if let Some(pen) = self.shared.monitor().pen {
            if pen.in_area {
                let x = rect.left() + rect.width() * pen.x as f32 / ranges.x_max.max(1) as f32;
                let y = rect.top() + rect.height() * pen.y as f32 / ranges.y_max.max(1) as f32;
                painter.circle_filled(Pos2::new(x, y), 4.0, Color32::from_rgb(0x5c, 0xc8, 0x7a));
            }
        }

        painter.text(
            rect.left_bottom() + Vec2::new(6.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            "深蓝=映射区域　亮蓝=按屏幕比例修正后　黄框=观测极值",
            egui::FontId::proportional(12.0),
            Color32::from_gray(150),
        );
    }
}

fn screen_size() -> (i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}
