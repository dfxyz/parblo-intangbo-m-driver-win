use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::config::Config;
use crate::device::hid::PenRanges;
use crate::device::protocol::{Button, PenEvent};

#[derive(Clone, Default)]
pub struct Status {
    pub connected: bool,
    pub ranges: Option<PenRanges>,
}

/// 监视面板用的原始数据；仅在面板打开时才由驱动写入
#[derive(Clone, Default)]
pub struct Monitor {
    pub pen: Option<PenEvent>,
    pub button: Option<Button>,
    pub pen_count: u64,
    pub button_count: u64,
}

/// 界面线程与驱动线程之间的共享状态
pub struct Shared {
    config: Mutex<Config>,
    config_version: AtomicU64,
    status: Mutex<Status>,
    /// 当前生效的按键映射方案；界面与驱动共用同一份，两边都能改
    keymap_index: AtomicUsize,
    show_requested: AtomicBool,
    monitor_enabled: AtomicBool,
    monitor: Mutex<Monitor>,
    on_change: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

impl Shared {
    pub fn new(config: Config) -> Self {
        let keymap_index = config.keymap_index;
        Self {
            config: Mutex::new(config),
            config_version: AtomicU64::new(1),
            status: Mutex::new(Status::default()),
            keymap_index: AtomicUsize::new(keymap_index),
            show_requested: AtomicBool::new(false),
            monitor_enabled: AtomicBool::new(false),
            monitor: Mutex::new(Monitor::default()),
            on_change: OnceLock::new(),
        }
    }

    pub fn set_monitor_enabled(&self, enabled: bool) {
        self.monitor_enabled.store(enabled, Ordering::Release);
    }

    pub fn monitor_enabled(&self) -> bool {
        self.monitor_enabled.load(Ordering::Acquire)
    }

    pub fn monitor(&self) -> Monitor {
        self.monitor.lock().unwrap().clone()
    }

    /// 事件率高达两百多赫兹，这里只更新数据、不触发重绘，由界面自行按帧率刷新
    pub fn record_pen(&self, event: PenEvent) {
        let mut monitor = self.monitor.lock().unwrap();
        monitor.pen = Some(event);
        monitor.pen_count += 1;
    }

    pub fn record_button(&self, button: Button) {
        let mut monitor = self.monitor.lock().unwrap();
        monitor.button = Some(button);
        monitor.button_count += 1;
    }

    pub fn keymap_index(&self) -> usize {
        self.keymap_index.load(Ordering::Acquire)
    }

    pub fn set_keymap_index(&self, index: usize) {
        self.keymap_index.store(index, Ordering::Release);
        if let Some(on_change) = self.on_change.get() {
            on_change();
        }
    }

    /// 由另一个实例触发，请求界面线程显示窗口
    pub fn request_show(&self) {
        self.show_requested.store(true, Ordering::Release);
        if let Some(on_change) = self.on_change.get() {
            on_change();
        }
    }

    pub fn take_show_request(&self) -> bool {
        self.show_requested.swap(false, Ordering::AcqRel)
    }

    /// 注册状态变化的通知回调；由界面线程设置为请求重绘
    pub fn set_on_change<F>(&self, f: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let _ = self.on_change.set(Box::new(f));
    }

    pub fn config_version(&self) -> u64 {
        self.config_version.load(Ordering::Acquire)
    }

    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn set_config(&self, config: Config) {
        *self.config.lock().unwrap() = config;
        self.config_version.fetch_add(1, Ordering::Release);
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn update_status<F>(&self, f: F)
    where
        F: FnOnce(&mut Status),
    {
        f(&mut self.status.lock().unwrap());
        if let Some(on_change) = self.on_change.get() {
            on_change();
        }
    }
}
