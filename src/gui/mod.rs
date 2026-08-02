mod area;
mod monitor;
mod pressure;
mod recorder;
mod tray;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::Result;
use egui::{Color32, RichText, ViewportCommand};
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::System::Threading::{INFINITE, WaitForMultipleObjects};

use crate::config::{Config, Control, Keymap, KeymapConfig, PressureCurve, TabletArea};
use crate::device::Driver;
use crate::device::hid::Event;
use crate::error;
use crate::gui::area::Observed;
use crate::gui::tray::{Tray, TrayCommand};
use crate::shared::Shared;

pub use crate::gui::tray::icon_image;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Keymap,
    Area,
    Pressure,
    Monitor,
    Log,
}
impl Page {
    const ALL: [Page; 5] = [
        Page::Keymap,
        Page::Area,
        Page::Pressure,
        Page::Monitor,
        Page::Log,
    ];

    fn label(&self) -> &'static str {
        match self {
            Page::Keymap => "按键映射",
            Page::Area => "映射区域",
            Page::Pressure => "压感曲线",
            Page::Monitor => "原始数据",
            Page::Log => "日志",
        }
    }
}

pub struct App {
    shared: Arc<Shared>,
    quit: Arc<Event>,
    driver_thread: Option<JoinHandle<()>>,
    tray: Option<Tray>,
    config_path: PathBuf,
    /// 界面上编辑中的文本；保存时才解析成配置
    editing: Vec<[String; Control::ALL.len()]>,
    pressure: PressureCurve,
    area: TabletArea,
    page: Page,
    /// 正在录制快捷键的控件下标
    recording: Option<usize>,
    dragging_point: Option<usize>,
    observed: Observed,
    message: Option<Message>,
    /// eframe为了避免首帧白屏，会在画完第一帧后强制显示窗口，
    /// 覆盖掉`with_visible(false)`；这里数着帧，等它显示之后再隐藏
    frames_until_hide: u32,
    exiting: bool,
}

struct Message {
    text: String,
    error: bool,
}

impl App {
    pub fn new(
        ctx: &egui::Context,
        shared: Arc<Shared>,
        quit: Arc<Event>,
        show_signal: Option<Arc<Event>>,
        config_path: PathBuf,
    ) -> Self {
        shared.set_on_change({
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        });

        if let Some(show_signal) = show_signal {
            spawn_show_listener(shared.clone(), quit.clone(), show_signal);
        }

        let tray = match Tray::new(ctx.clone()) {
            Ok(tray) => Some(tray),
            Err(e) => {
                error!("无法创建托盘图标: {:#}", e);
                None
            }
        };

        let driver_thread = {
            let shared = shared.clone();
            let quit = quit.clone();
            std::thread::spawn(move || {
                let mut driver = match Driver::new(shared.clone()) {
                    Ok(driver) => driver,
                    Err(e) => {
                        error!("初始化驱动任务时发生错误: {:#}", e);
                        return;
                    }
                };
                if let Err(e) = driver.run(&quit) {
                    error!("驱动任务发生错误并退出: {:#}", e);
                    shared.update_status(|status| status.connected = false);
                }
            })
        };

        let config = shared.config();
        Self {
            shared,
            quit,
            driver_thread: Some(driver_thread),
            tray,
            config_path,
            editing: editing_from_config(&config),
            pressure: config.pressure.clone(),
            area: config.area.clone(),
            page: Page::Keymap,
            recording: None,
            dragging_point: None,
            observed: Observed::default(),
            message: None,
            frames_until_hide: 2,
            exiting: false,
        }
    }

    fn apply(&mut self, save: bool) {
        let config = match config_from_editing(
            &self.editing,
            self.schema_index(),
            self.pressure.clone(),
            self.area.clone(),
        ) {
            Ok(config) => config,
            Err(e) => {
                self.set_message(format!("{:#}", e), true);
                return;
            }
        };
        if save {
            if let Err(e) = config.save(&self.config_path) {
                self.set_message(format!("保存失败: {:#}", e), true);
                return;
            }
        }
        self.shared.set_config(config);
        let text = if save {
            "已保存并应用"
        } else {
            "已应用（未写入文件）"
        };
        self.set_message(text.to_string(), false);
    }

    fn reload(&mut self) {
        match Config::load(&self.config_path) {
            Ok(config) => {
                self.editing = editing_from_config(&config);
                self.pressure = config.pressure.clone();
                self.area = config.area.clone();
                self.shared.set_keymap_index(config.keymap_index);
                self.shared.set_config(config);
                self.set_message("已从文件重新加载".to_string(), false);
            }
            Err(e) => self.set_message(format!("重新加载失败: {:#}", e), true),
        }
    }

    fn set_message(&mut self, text: String, error: bool) {
        self.message = Some(Message { text, error });
    }

    fn show_window(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn handle_tray(&mut self, ctx: &egui::Context) {
        let mut commands = Vec::new();
        if let Some(tray) = &self.tray {
            while let Some(command) = tray.poll() {
                commands.push(command);
            }
        }
        for command in commands {
            match command {
                TrayCommand::Show => self.show_window(ctx),
                TrayCommand::Quit => {
                    self.exiting = true;
                    self.shutdown();
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
    }

    fn shutdown(&mut self) {
        self.quit.set();
        if let Some(thread) = self.driver_thread.take() {
            let _ = thread.join();
        }
    }

    /// 下标由驱动与界面共享，配置方案数变化时可能暂时越界
    fn schema_index(&self) -> usize {
        self.shared
            .keymap_index()
            .min(self.editing.len().saturating_sub(1))
    }

    fn connection_indicator(&self, ui: &mut egui::Ui) {
        let (text, color) = if self.shared.status().connected {
            ("● 已连接", Color32::from_rgb(0x3d, 0xa5, 0x5d))
        } else {
            ("● 未连接", Color32::from_rgb(0xc0, 0x50, 0x50))
        };
        ui.label(RichText::new(text).color(color).strong());
    }

    /// 方案标签同时也是方案切换器，与绘图板上的切换键共用同一个下标
    fn schema_tabs(&mut self, ui: &mut egui::Ui) {
        let current = self.schema_index();
        ui.horizontal_wrapped(|ui| {
            for index in 0..self.editing.len() {
                if ui
                    .selectable_label(index == current, format!("方案 {}", index))
                    .clicked()
                {
                    self.shared.set_keymap_index(index);
                }
            }
            if ui.button("＋").on_hover_text("添加一套方案").clicked() {
                self.editing
                    .push(std::array::from_fn(|_| "fallback".to_string()));
                self.shared.set_keymap_index(self.editing.len() - 1);
            }
            let removable = self.editing.len() > 1;
            if ui
                .add_enabled(removable, egui::Button::new("－"))
                .on_hover_text("删除当前方案")
                .clicked()
            {
                self.editing.remove(current);
                self.shared
                    .set_keymap_index(current.min(self.editing.len() - 1));
            }
        });
    }
}

impl eframe::App for App {
    /// 界面隐藏时仍会被调用，托盘事件因此不会积压
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.frames_until_hide > 0 {
            self.frames_until_hide -= 1;
            if self.frames_until_hide == 0 {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            } else {
                ctx.request_repaint();
            }
        }
        self.handle_tray(ctx);
        if self.shared.take_show_request() {
            self.show_window(ctx);
        }
        if ctx.input(|input| input.viewport().close_requested()) && !self.exiting {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        // 压感页要显示实时换算、映射区域页要取点，同样依赖监视数据
        let needs_monitor = matches!(self.page, Page::Monitor | Page::Pressure | Page::Area);
        self.shared
            .set_monitor_enabled(needs_monitor && !self.exiting);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("actions").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("保存并应用").clicked() {
                    self.apply(true);
                }
                if ui.button("仅应用").clicked() {
                    self.apply(false);
                }
                if ui.button("从文件重新加载").clicked() {
                    self.reload();
                }
                if let Some(message) = &self.message {
                    ui.separator();
                    let color = if message.error {
                        Color32::from_rgb(0xd0, 0x50, 0x50)
                    } else {
                        Color32::from_rgb(0x3d, 0xa5, 0x5d)
                    };
                    ui.label(RichText::new(&message.text).color(color));
                }
            });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                for page in Page::ALL {
                    if ui
                        .selectable_label(self.page == page, page.label())
                        .clicked()
                    {
                        self.page = page;
                        self.recording = None;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.connection_indicator(ui)
                });
            });
            ui.separator();
            match self.page {
                Page::Keymap => self.keymap_page(ui),
                Page::Area => self.area_page(ui),
                Page::Pressure => self.pressure_page(ui),
                Page::Monitor => self.monitor_page(ui),
                Page::Log => self.log_page(ui),
            }
        });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl App {
    fn log_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("清空").clicked() {
                crate::macros::clear();
            }
            ui.label(
                RichText::new("设置环境变量 PARBLO_DEBUG 可输出更详细的日志")
                    .weak(),
            );
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for entry in crate::macros::entries() {
                    let color = match entry.level {
                        "ERROR" => Some(Color32::from_rgb(0xd0, 0x50, 0x50)),
                        "WARN" => Some(Color32::from_rgb(0xc8, 0x96, 0x30)),
                        "DEBUG" => Some(Color32::GRAY),
                        _ => None,
                    };
                    let text = RichText::new(format!("[{}] {}", entry.level, entry.text)).monospace();
                    ui.label(match color {
                        Some(color) => text.color(color),
                        None => text,
                    });
                }
            });
    }
}

/// 等待另一个实例的唤起信号；窗口必须由界面线程通过egui显示，
/// 否则winit不知道视口重新可见，之后关闭按钮就会失效
fn spawn_show_listener(shared: Arc<Shared>, quit: Arc<Event>, signal: Arc<Event>) {
    std::thread::spawn(move || {
        let handles = [signal.handle(), quit.handle()];
        loop {
            let waited = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
            match waited.0.wrapping_sub(WAIT_OBJECT_0.0) {
                0 => shared.request_show(),
                _ => return,
            }
        }
    });
}

fn editing_from_config(config: &Config) -> Vec<[String; Control::ALL.len()]> {
    if config.keymaps.is_empty() {
        return vec![std::array::from_fn(|_| "none".to_string())];
    }
    config
        .keymaps
        .iter()
        .map(|keymap| {
            std::array::from_fn(|index| keymap.get(Control::ALL[index]).to_config_value())
        })
        .collect()
}

fn config_from_editing(
    editing: &[[String; Control::ALL.len()]],
    keymap_index: usize,
    pressure: PressureCurve,
    area: TabletArea,
) -> Result<Config> {
    let mut keymaps = Vec::with_capacity(editing.len());
    for (schema_index, schema) in editing.iter().enumerate() {
        let mut keymap = KeymapConfig::default();
        for (index, control) in Control::ALL.into_iter().enumerate() {
            let parsed = Keymap::try_from(schema[index].as_str()).map_err(|e| {
                anyhow::anyhow!("方案{}的「{}」配置有误: {}", schema_index, control.label(), e)
            })?;
            keymap.set(control, parsed);
        }
        keymaps.push(keymap);
    }
    Ok(Config {
        keymaps,
        keymap_index,
        pressure,
        area,
    })
}
