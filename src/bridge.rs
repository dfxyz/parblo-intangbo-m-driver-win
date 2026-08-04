//! 把驱动线程写进[`Shared`]的数据搬到界面上。
//!
//! Slint是数据驱动的，而笔事件有两百多赫兹，逐事件唤醒事件循环会把它打爆，
//! 因此这里反过来做：驱动只管写`Shared`，界面按固定频率来取。
//! 每帧只刷新当前页要用的东西，配置类数据则靠版本号判断有没有变。

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::config::{Config, Control, PressureCurve};
use crate::device::hid::PenRanges;
use crate::device::protocol::{Button, PenEvent};
use crate::inject::pen::{screen_aspect, tablet_aspect};
use crate::shared::Shared;
use crate::{
    AppLogic, AppWindow, AreaLogic, CurvePoint, KeymapEntry, KeymapLogic, LogEntry, MonitorLogic,
    NormRect, PressureLogic, Status,
};

const REFRESH_INTERVAL: Duration = Duration::from_micros(16_667);

/// 页面下标，与`AppLogic.page`一致
const PAGE_KEYMAP: i32 = 0;
const PAGE_AREA: i32 = 1;
const PAGE_PRESSURE: i32 = 2;
const PAGE_MONITOR: i32 = 3;

pub struct Bridge {
    shared: Arc<Shared>,
    /// 配置类数据只在版本号变化时重建，不必每帧刷
    config_version: Cell<u64>,
    /// 绘图板上的切换键也会改方案，界面得跟着走
    keymap_index: Cell<usize>,
    log_revision: Cell<u64>,
    /// 换页时要立刻补一次配置类数据，因为它们是按页写进去的
    page: Cell<i32>,
    /// 正在录制快捷键的行下标，-1表示没有在录制
    recording: Cell<i32>,
}

pub fn create(shared: Arc<Shared>) -> Rc<Bridge> {
    Rc::new(Bridge {
        shared,
        // 这几个初值都取不可能出现的值，保证首帧一定会把数据推上去
        config_version: Cell::new(0),
        keymap_index: Cell::new(usize::MAX),
        log_revision: Cell::new(u64::MAX),
        page: Cell::new(-1),
        recording: Cell::new(-1),
    })
}

/// 启动定时刷新；返回的`Timer`必须一直持有，丢掉就停了
pub fn start(bridge: Rc<Bridge>, ui: &AppWindow) -> Timer {
    let timer = Timer::default();
    let weak = ui.as_weak();
    timer.start(TimerMode::Repeated, REFRESH_INTERVAL, move || {
        if let Some(ui) = weak.upgrade() {
            bridge.refresh(&ui);
        }
    });
    timer
}

impl Bridge {
    /// 界面自己已经显示着新值时调用，避免下一帧又把它覆盖回去
    pub fn mark_config_seen(&self) {
        self.config_version.set(self.shared.config_version());
    }

    pub fn recording(&self) -> i32 {
        self.recording.get()
    }

    pub fn set_recording(&self, index: i32) {
        self.recording.set(index);
    }

    fn refresh(&self, ui: &AppWindow) {
        let status = self.shared.status();
        ui.global::<Status>().set_connected(status.connected);

        let page = ui.global::<AppLogic>().get_page();
        let page_changed = self.page.replace(page) != page;

        // 方案下标不走配置版本号：绘图板上的切换键改的是它，配置本身没变
        let index = self.shared.keymap_index();
        let version = self.shared.config_version();
        let stale = self.config_version.replace(version) != version
            || self.keymap_index.replace(index) != index;
        if stale || page_changed {
            self.refresh_config(ui, page);
        }

        let ranges = status.ranges.unwrap_or_default();
        match page {
            PAGE_AREA => self.refresh_area(ui, ranges),
            PAGE_PRESSURE => self.refresh_pressure(ui, ranges),
            PAGE_MONITOR => self.refresh_monitor(ui, ranges),
            _ => {}
        }
    }

    /// 配置本身的投影：按键映射、压感控制点、映射区域的基准角
    fn refresh_config(&self, ui: &AppWindow, page: i32) {
        let config = self.shared.config();
        match page {
            PAGE_KEYMAP => {
                let index = self.shared.keymap_index();
                let keymap = ui.global::<KeymapLogic>();
                keymap.set_schema_count(config.keymaps.len() as i32);
                keymap.set_schema_index(index as i32);
                keymap.set_entries(keymap_entries(&config, index));
            }
            PAGE_AREA => {
                ui.global::<AreaLogic>()
                    .set_anchor(config.area.anchor as i32);
            }
            PAGE_PRESSURE => {
                let pressure = ui.global::<PressureLogic>();
                pressure.set_points(curve_points(&config.pressure));
                pressure.set_commands(curve_commands(&config.pressure).into());
            }
            _ => {}
        }
    }

    fn refresh_area(&self, ui: &AppWindow, ranges: PenRanges) {
        let config = self.shared.config();
        let monitor = self.shared.monitor();
        let area = ui.global::<AreaLogic>();

        area.set_declared_x(axis_range('X', 0, ranges.x_max as u32));
        area.set_declared_y(axis_range('Y', 0, ranges.y_max as u32));
        area.set_configured_x(axis_range(
            'X',
            to_device(config.area.x_min, ranges.x_max),
            to_device(config.area.x_max, ranges.x_max),
        ));
        area.set_configured_y(axis_range(
            'Y',
            to_device(config.area.y_min, ranges.y_max),
            to_device(config.area.y_max, ranges.y_max),
        ));

        let observed = monitor.observed;
        let has_measured = !observed.is_empty();
        area.set_has_measured(has_measured);
        if has_measured {
            area.set_measured_x(axis_range('X', observed.min_x as u32, observed.max_x as u32));
            area.set_measured_y(axis_range('Y', observed.min_y as u32, observed.max_y as u32));
            area.set_area_measured(NormRect {
                x1: observed.min_x as f32 / ranges.x_max.max(1) as f32,
                y1: observed.min_y as f32 / ranges.y_max.max(1) as f32,
                x2: observed.max_x as f32 / ranges.x_max.max(1) as f32,
                y2: observed.max_y as f32 / ranges.y_max.max(1) as f32,
            });
        } else {
            area.set_measured_x("把笔贴着板子边缘划一圈".into());
            area.set_measured_y(SharedString::new());
        }

        let pen = monitor.pen.filter(|pen| pen.in_area);
        area.set_pen_in_area(pen.is_some());
        if let Some(pen) = pen {
            area.set_current_x(axis_percent('X', pen.x, ranges.x_max));
            area.set_current_y(axis_percent('Y', pen.y, ranges.y_max));
            area.set_pen_x(pen.x as f32 / ranges.x_max.max(1) as f32);
            area.set_pen_y(pen.y as f32 / ranges.y_max.max(1) as f32);
        }

        area.set_tablet_aspect(tablet_aspect(ranges));
        area.set_area(NormRect {
            x1: config.area.x_min,
            y1: config.area.y_min,
            x2: config.area.x_max,
            y2: config.area.y_max,
        });
        let used = config.area.effective(tablet_aspect(ranges), screen_aspect());
        area.set_area_used(NormRect {
            x1: used.x_min,
            y1: used.y_min,
            x2: used.x_max,
            y2: used.y_max,
        });
    }

    fn refresh_pressure(&self, ui: &AppWindow, ranges: PenRanges) {
        let text = match self.shared.monitor().pen {
            Some(pen) => {
                let raw = pen.pressure as f32 / ranges.pressure_max.max(1) as f32;
                let mapped = self.shared.config().pressure.evaluate(raw);
                format!("原始 {:.1}%  →  输出 {:.1}%", raw * 100.0, mapped * 100.0)
            }
            None => "把笔压在绘图板上即可看到实时换算".to_string(),
        };
        ui.global::<PressureLogic>().set_current(text.into());
    }

    fn refresh_monitor(&self, ui: &AppWindow, ranges: PenRanges) {
        let monitor = self.shared.monitor();
        let logic = ui.global::<MonitorLogic>();

        match monitor.pen {
            Some(pen) => {
                logic.set_pen_state(pen_state(&pen).into());
                logic.set_pen_coord(
                    format!(
                        "X = {} / {}  Y = {} / {}",
                        pen.x, ranges.x_max, pen.y, ranges.y_max
                    )
                    .into(),
                );
                let ratio = pen.pressure as f32 / ranges.pressure_max.max(1) as f32;
                logic.set_pen_pressure(
                    format!(
                        "{} / {}  ({:.1}%)",
                        pen.pressure,
                        ranges.pressure_max,
                        ratio * 100.0
                    )
                    .into(),
                );
                logic.set_pen_tilt(format!("X = {}°  Y = {}°", pen.tilt_x, pen.tilt_y).into());
            }
            None => {
                logic.set_pen_state("尚未收到笔事件".into());
                logic.set_pen_coord(SharedString::new());
                logic.set_pen_pressure(SharedString::new());
                logic.set_pen_tilt(SharedString::new());
            }
        }

        let buttons: Vec<&str> = monitor
            .recent_buttons
            .iter()
            .rev()
            .filter_map(|button| button_label(*button))
            .collect();
        logic.set_recent_buttons(if buttons.is_empty() {
            "尚未收到按键事件".into()
        } else {
            buttons.join("、").into()
        });

        let revision = crate::macros::revision();
        if self.log_revision.replace(revision) != revision {
            let logs: Vec<LogEntry> = crate::macros::entries()
                .into_iter()
                .map(|entry| LogEntry {
                    level: entry.level.into(),
                    text: entry.text.into(),
                })
                .collect();
            logic.set_logs(ModelRc::new(VecModel::from(logs)));
        }
    }
}

fn keymap_entries(config: &Config, index: usize) -> ModelRc<KeymapEntry> {
    let entries: Vec<KeymapEntry> = Control::ALL
        .into_iter()
        .map(|control| {
            let value = match config.keymaps.get(index) {
                Some(keymap) => keymap.get(control).to_config_value(),
                None => "none".to_string(),
            };
            KeymapEntry {
                label: control.label().into(),
                value: value.into(),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(entries))
}

fn curve_points(curve: &PressureCurve) -> ModelRc<CurvePoint> {
    let points: Vec<CurvePoint> = curve
        .points
        .iter()
        .map(|point| CurvePoint {
            x: point[0],
            y: point[1],
        })
        .collect();
    ModelRc::new(VecModel::from(points))
}

/// 折线的Path命令，y轴向下。两端各补一段水平线，对应求值时的常数外推
pub fn curve_commands(curve: &PressureCurve) -> String {
    let (Some(first), Some(last)) = (curve.points.first(), curve.points.last()) else {
        return "M 0 1 L 1 0".to_string();
    };
    let mut commands = format!("M 0 {:.4}", 1.0 - first[1]);
    for point in &curve.points {
        commands.push_str(&format!(" L {:.4} {:.4}", point[0], 1.0 - point[1]));
    }
    commands.push_str(&format!(" L 1 {:.4}", 1.0 - last[1]));
    commands
}

fn to_device(normalized: f32, max: u16) -> u32 {
    (normalized * max as f32).round() as u32
}

fn axis_range(axis: char, min: u32, max: u32) -> SharedString {
    format!("{} = {} .. {}", axis, min, max).into()
}

fn axis_percent(axis: char, value: u16, max: u16) -> SharedString {
    format!(
        "{} = {} ({:.1}%)",
        axis,
        value,
        value as f32 * 100.0 / max.max(1) as f32
    )
    .into()
}

fn pen_state(pen: &PenEvent) -> String {
    if !pen.in_area {
        return "已离开感应区".to_string();
    }
    let mut parts = vec!["在感应区内"];
    if pen.tip_pressed {
        parts.push("笔尖");
    }
    if pen.button0_pressed {
        parts.push("下侧键");
    }
    if pen.button1_pressed {
        parts.push("上侧键");
    }
    parts.join("  ")
}

fn button_label(button: Button) -> Option<&'static str> {
    let control = match button {
        Button::Release => return None,
        Button::Button0 => Control::Button0,
        Button::Button1 => Control::Button1,
        Button::Button2 => Control::Button2,
        Button::Button3 => Control::Button3,
        Button::Button4 => Control::Button4,
        Button::Button5 => Control::Button5,
        Button::Button6 => Control::Button6,
        Button::Button7 => Control::Button7,
        Button::Ring0 => Control::Ring0,
        Button::Ring1 => Control::Ring1,
        Button::RingButton => Control::RingButton,
    };
    Some(control.label())
}
