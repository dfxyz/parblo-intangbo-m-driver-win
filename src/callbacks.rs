//! 界面回调的实现，即UI到Rust这个方向。
//!
//! 所有改动都先落到[`Shared`]里的配置上，驱动线程下一轮循环就会取到；
//! 写盘则是另一回事，只有「保存」按钮和坐标范围页的几个动作才会做。

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, Model, SharedString};

use crate::config::{
    AreaAnchor, Config, Control, Key, Keymap, KeymapConfig, PressureCurve, TabletArea,
};
use crate::bridge::{Bridge, curve_commands};
use crate::shared::Shared;
use crate::{
    AppWindow, AreaLogic, CurvePoint, KeymapLogic, MonitorLogic, PressureLogic, error, info, warn,
};

pub fn install(ui: &AppWindow, shared: Arc<Shared>, bridge: Rc<Bridge>, config_path: PathBuf) {
    let ctx = Rc::new(Context {
        shared,
        bridge,
        config_path,
    });
    install_monitor(ui);
    install_area(ui, ctx.clone());
    install_pressure(ui, ctx.clone());
    install_keymap(ui, ctx);
}

struct Context {
    shared: Arc<Shared>,
    bridge: Rc<Bridge>,
    config_path: PathBuf,
}

impl Context {
    fn config(&self) -> Config {
        self.shared.config()
    }

    /// 改动立即对驱动生效，并让界面在下一帧重新取一遍
    fn apply(&self, config: Config) {
        self.shared.set_config(config);
    }

    /// 界面自己已经显示着新值的改动走这条路，免得下一帧把用户正在输入的内容冲掉
    fn apply_quietly(&self, config: Config) {
        self.shared.set_config(config);
        self.bridge.mark_config_seen();
    }

    fn save(&self) {
        let mut config = self.config();
        // 顺手记住当前方案，下次启动仍用它；运行时用的是共享状态里的下标，
        // 所以只写进要落盘的这份副本就够了
        config.keymap_index = self.shared.keymap_index();
        match config.save(&self.config_path) {
            Ok(()) => info!("已保存配置到{}", self.config_path.display()),
            Err(e) => error!("保存配置失败: {:#}", e),
        }
    }

    fn reload(&self) {
        match Config::load(&self.config_path) {
            Ok(config) => {
                let index = config.keymap_index.min(config.keymaps.len().saturating_sub(1));
                self.shared.set_keymap_index(index);
                self.apply(config);
                info!("已从{}重新加载配置", self.config_path.display());
            }
            Err(e) => warn!("重新加载配置失败: {:#}", e),
        }
    }
}

fn install_monitor(ui: &AppWindow) {
    ui.global::<MonitorLogic>().on_clear_logs(crate::macros::clear);
}

fn install_area(ui: &AppWindow, ctx: Rc<Context>) {
    let area = ui.global::<AreaLogic>();

    {
        let ctx = ctx.clone();
        area.on_apply_measured(move || {
            let observed = ctx.shared.monitor().observed;
            if observed.is_empty() {
                return;
            }
            let ranges = ctx.shared.status().ranges.unwrap_or_default();
            let mut config = ctx.config();
            config.area.x_min = observed.min_x as f32 / ranges.x_max.max(1) as f32;
            config.area.x_max = observed.max_x as f32 / ranges.x_max.max(1) as f32;
            config.area.y_min = observed.min_y as f32 / ranges.y_max.max(1) as f32;
            config.area.y_max = observed.max_y as f32 / ranges.y_max.max(1) as f32;
            info!(
                "坐标范围已按实测值更新为 X={}..{} Y={}..{}",
                observed.min_x, observed.max_x, observed.min_y, observed.max_y
            );
            ctx.apply(config);
        });
    }
    {
        let ctx = ctx.clone();
        area.on_remeasure(move || {
            ctx.shared.clear_observed();
            info!("已清空实测记录，重新开始测量");
        });
    }
    {
        let ctx = ctx.clone();
        area.on_reset_range(move || {
            let mut config = ctx.config();
            // 只重置范围，基准角是另一项设置
            config.area = TabletArea {
                anchor: config.area.anchor,
                ..Default::default()
            };
            info!("坐标范围已重置为全范围");
            ctx.apply(config);
        });
    }
    {
        let ctx = ctx.clone();
        area.on_select_anchor(move |index| {
            let anchor = AreaAnchor::ALL
                .get(index.max(0) as usize)
                .copied()
                .unwrap_or_default();
            let mut config = ctx.config();
            config.area.anchor = anchor;
            ctx.apply(config);
        });
    }
    {
        let ctx = ctx.clone();
        area.on_save(move || ctx.save());
    }
    area.on_reload(move || ctx.reload());
}

fn install_pressure(ui: &AppWindow, ctx: Rc<Context>) {
    let pressure = ui.global::<PressureLogic>();

    {
        let ctx = ctx.clone();
        let weak = ui.as_weak();
        pressure.on_move_point(move |index, x, y| {
            let index = index.max(0) as usize;
            let mut config = ctx.config();
            let Some(point) = config.pressure.points.get_mut(index) else {
                return;
            };
            let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
            *point = [x, y];
            // 拖动过程中不排序，抬手时才整理，见commit-points
            let commands = curve_commands(&config.pressure);
            ctx.apply_quietly(config);

            // 整体换掉model会让Slint销毁重建所有控制点，正在拖的那个
            // TouchArea一并没了，鼠标grab随之丢失；这里只改动那一行
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let logic = ui.global::<PressureLogic>();
            logic.get_points().set_row_data(index, CurvePoint { x, y });
            logic.set_commands(commands.into());
        });
    }
    {
        let ctx = ctx.clone();
        pressure.on_commit_points(move || {
            let mut config = ctx.config();
            config.pressure.sort_points();
            ctx.apply(config);
        });
    }
    {
        let ctx = ctx.clone();
        pressure.on_add_point(move |x, y| {
            let mut config = ctx.config();
            config
                .pressure
                .points
                .push([x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)]);
            config.pressure.sort_points();
            ctx.apply(config);
        });
    }
    {
        let ctx = ctx.clone();
        pressure.on_remove_point(move |index| {
            let mut config = ctx.config();
            // 两个点才能定义一条曲线，再删就没了
            if config.pressure.points.len() <= 2 {
                return;
            }
            let index = index.max(0) as usize;
            if index < config.pressure.points.len() {
                config.pressure.points.remove(index);
                ctx.apply(config);
            }
        });
    }
    {
        let ctx = ctx.clone();
        pressure.on_reset(move || {
            let mut config = ctx.config();
            config.pressure = PressureCurve::default();
            ctx.apply(config);
        });
    }
    {
        let ctx = ctx.clone();
        pressure.on_soft(move || {
            let mut config = ctx.config();
            config.pressure = PressureCurve {
                points: vec![[0.0, 0.0], [0.5, 0.3], [1.0, 1.0]],
            };
            ctx.apply(config);
        });
    }
    {
        let ctx = ctx.clone();
        pressure.on_hard(move || {
            let mut config = ctx.config();
            config.pressure = PressureCurve {
                points: vec![[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]],
            };
            ctx.apply(config);
        });
    }
    {
        let ctx = ctx.clone();
        pressure.on_save(move || ctx.save());
    }
    pressure.on_reload(move || ctx.reload());
}

fn install_keymap(ui: &AppWindow, ctx: Rc<Context>) {
    let keymap = ui.global::<KeymapLogic>();

    {
        let ctx = ctx.clone();
        keymap.on_select_schema(move |index| {
            let count = ctx.config().keymaps.len();
            if index >= 0 && (index as usize) < count {
                ctx.shared.set_keymap_index(index as usize);
            }
        });
    }
    {
        let ctx = ctx.clone();
        keymap.on_add_schema(move || {
            let mut config = ctx.config();
            // 插在当前方案之后，而不是追加到末尾
            let at = (ctx.shared.keymap_index() + 1).min(config.keymaps.len());
            config.keymaps.insert(at, KeymapConfig::default());
            ctx.shared.set_keymap_index(at);
            info!("已新增按键映射方案{}", at);
            ctx.apply(config);
        });
    }
    {
        let ctx = ctx.clone();
        keymap.on_remove_schema(move || {
            let mut config = ctx.config();
            if config.keymaps.len() <= 1 {
                return;
            }
            let index = ctx.shared.keymap_index().min(config.keymaps.len() - 1);
            config.keymaps.remove(index);
            ctx.shared
                .set_keymap_index(index.min(config.keymaps.len() - 1));
            info!("已删除按键映射方案{}", index);
            ctx.apply(config);
        });
    }
    {
        let ctx = ctx.clone();
        keymap.on_set_entry(move |index, value| {
            let Some(control) = Control::ALL.get(index.max(0) as usize).copied() else {
                return;
            };
            let mut config = ctx.config();
            let schema = ctx.shared.keymap_index();
            let Some(keymap) = config.keymaps.get_mut(schema) else {
                return;
            };
            // 填了无效值就当作没映射，不打断用户继续输入
            keymap.set(control, Keymap::try_from(value.as_str()).unwrap_or(Keymap::None));
            ctx.apply_quietly(config);
        });
    }
    {
        let ctx = ctx.clone();
        keymap.on_save(move || ctx.save());
    }
    {
        let ctx = ctx.clone();
        keymap.on_reload(move || ctx.reload());
    }
    keymap.on_validate(|value| Keymap::try_from(value.as_str()).is_ok());
    {
        let ctx = ctx.clone();
        keymap.on_start_record(move |index| ctx.bridge.set_recording(index));
    }
    {
        let ctx = ctx.clone();
        keymap.on_cancel_record(move || ctx.bridge.set_recording(-1));
    }

    let weak = ui.as_weak();
    ui.global::<KeymapLogic>()
        .on_record_key(move |text, ctrl, shift, alt, meta| {
            let Some(name) = key_name(&text) else {
                // 单独按下修饰键不算数，等真正的键
                return false;
            };
            let mut parts: Vec<&str> = Vec::with_capacity(4);
            if ctrl {
                parts.push("ctrl");
            }
            if shift {
                parts.push("shift");
            }
            if alt {
                parts.push("alt");
            }
            if meta {
                parts.push("meta");
            }
            parts.push(name);
            let value = parts.join("+");

            let index = ctx.bridge.recording();
            ctx.bridge.set_recording(-1);
            let Some(ui) = weak.upgrade() else {
                return true;
            };
            let entries = ui.global::<KeymapLogic>().get_entries();
            if let Some(mut entry) = entries.row_data(index.max(0) as usize) {
                entry.value = SharedString::from(value.as_str());
                entries.set_row_data(index.max(0) as usize, entry);
            }
            keymap_set(&ctx, index, &value);
            true
        });
}

fn keymap_set(ctx: &Context, index: i32, value: &str) {
    let Some(control) = Control::ALL.get(index.max(0) as usize).copied() else {
        return;
    };
    let mut config = ctx.config();
    let schema = ctx.shared.keymap_index();
    let Some(keymap) = config.keymaps.get_mut(schema) else {
        return;
    };
    keymap.set(control, Keymap::try_from(value).unwrap_or(Keymap::None));
    ctx.apply_quietly(config);
}

/// 把Slint的按键文本翻成配置里用的键名。
/// 普通键给的就是字符本身，特殊键给的是私有区里的固定码位
fn key_name(text: &SharedString) -> Option<&'static str> {
    let mut chars = text.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        return None;
    };
    // 字母数字和符号直接按小写查表
    if let Some(name) = Key::config_name(ch.to_ascii_lowercase()) {
        return Some(name);
    }
    let name = match ch {
        '\u{F700}' => "up",
        '\u{F701}' => "down",
        '\u{F702}' => "left",
        '\u{F703}' => "right",
        '\u{F704}' => "f1",
        '\u{F705}' => "f2",
        '\u{F706}' => "f3",
        '\u{F707}' => "f4",
        '\u{F708}' => "f5",
        '\u{F709}' => "f6",
        '\u{F70A}' => "f7",
        '\u{F70B}' => "f8",
        '\u{F70C}' => "f9",
        '\u{F70D}' => "f10",
        '\u{F70E}' => "f11",
        '\u{F70F}' => "f12",
        '\u{F727}' => "insert",
        '\u{F729}' => "home",
        '\u{F72B}' => "end",
        '\u{F72C}' => "pageup",
        '\u{F72D}' => "pagedown",
        '\u{0008}' => "backspace",
        '\u{0009}' => "tab",
        '\u{000A}' | '\u{000D}' => "enter",
        '\u{001B}' => "esc",
        '\u{0020}' => "space",
        '\u{007F}' => "delete",
        _ => return None,
    };
    Some(name)
}
