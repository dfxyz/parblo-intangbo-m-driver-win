use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::config::Config;
use crate::device::hid::PenRanges;
use crate::device::protocol::{Button, PenEvent};

/// 监视面板上「最近的按键事件」保留几条
const MAX_RECENT_BUTTONS: usize = 5;

#[derive(Clone, Default)]
pub struct Status {
    pub connected: bool,
    pub ranges: Option<PenRanges>,
}

/// 笔在本次测量中实际到达过的坐标范围。
/// 设备声明的量程通常大于真实可感应范围，靠这个来修正
#[derive(Clone, Copy)]
pub struct Observed {
    pub min_x: u16,
    pub max_x: u16,
    pub min_y: u16,
    pub max_y: u16,
}
impl Default for Observed {
    fn default() -> Self {
        Self {
            min_x: u16::MAX,
            max_x: 0,
            min_y: u16::MAX,
            max_y: 0,
        }
    }
}
impl Observed {
    pub fn is_empty(&self) -> bool {
        self.min_x > self.max_x
    }

    fn record(&mut self, x: u16, y: u16) {
        self.min_x = self.min_x.min(x);
        self.max_x = self.max_x.max(x);
        self.min_y = self.min_y.min(y);
        self.max_y = self.max_y.max(y);
    }
}

/// 界面用的实时数据
#[derive(Clone, Default)]
pub struct Monitor {
    pub pen: Option<PenEvent>,
    pub recent_buttons: VecDeque<Button>,
    pub observed: Observed,
}

/// 驱动线程与界面线程之间的共享状态
pub struct Shared {
    config: Mutex<Config>,
    config_version: AtomicU64,
    status: Mutex<Status>,
    /// 当前生效的按键映射方案；界面与驱动共用同一个下标，两边都能改
    keymap_index: AtomicUsize,
    monitor: Mutex<Monitor>,
}

impl Shared {
    pub fn new(config: Config) -> Self {
        let keymap_index = config.keymap_index;
        Self {
            config: Mutex::new(config),
            config_version: AtomicU64::new(1),
            status: Mutex::new(Status::default()),
            keymap_index: AtomicUsize::new(keymap_index),
            monitor: Mutex::new(Monitor::default()),
        }
    }

    pub fn monitor(&self) -> Monitor {
        self.monitor.lock().unwrap().clone()
    }

    /// 每个笔事件都会调到这里，包括测量坐标极值；
    /// 事件率两百多赫兹，但无争用的互斥量开销可以忽略
    pub fn record_pen(&self, event: PenEvent) {
        let mut monitor = self.monitor.lock().unwrap();
        if event.in_area {
            monitor.observed.record(event.x, event.y);
        }
        monitor.pen = Some(event);
    }

    /// 松开事件不进列表，那不是「按了哪个键」
    pub fn record_button(&self, button: Button) {
        if button == Button::Release {
            return;
        }
        let mut monitor = self.monitor.lock().unwrap();
        while monitor.recent_buttons.len() >= MAX_RECENT_BUTTONS {
            monitor.recent_buttons.pop_front();
        }
        monitor.recent_buttons.push_back(button);
    }

    /// 重新开始测量坐标极值
    pub fn clear_observed(&self) {
        self.monitor.lock().unwrap().observed = Observed::default();
    }

    pub fn keymap_index(&self) -> usize {
        self.keymap_index.load(Ordering::Acquire)
    }

    pub fn set_keymap_index(&self, index: usize) {
        self.keymap_index.store(index, Ordering::Release);
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
    }
}
