use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

const MAX_ENTRIES: usize = 512;

#[derive(Clone)]
pub struct LogEntry {
    pub level: &'static str,
    pub text: String,
}

static ENTRIES: Mutex<VecDeque<LogEntry>> = Mutex::new(VecDeque::new());
static REVISION: AtomicU64 = AtomicU64::new(0);
static DEBUG_ENABLED: OnceLock<bool> = OnceLock::new();

pub fn debug_enabled() -> bool {
    *DEBUG_ENABLED.get_or_init(|| std::env::var("PARBLO_DEBUG").is_ok())
}

pub fn push(level: &'static str, text: String) {
    println!("[{}] {}", level, text);
    let mut entries = ENTRIES.lock().unwrap();
    while entries.len() >= MAX_ENTRIES {
        entries.pop_front();
    }
    entries.push_back(LogEntry { level, text });
    REVISION.fetch_add(1, Ordering::Release);
}

pub fn entries() -> Vec<LogEntry> {
    ENTRIES.lock().unwrap().iter().cloned().collect()
}

/// 每次增删都会变；界面靠它判断要不要重建列表。
/// 环满之后条数不再变化，只看长度会漏掉内容的变化
pub fn revision() -> u64 {
    REVISION.load(Ordering::Acquire)
}

pub fn clear() {
    ENTRIES.lock().unwrap().clear();
    REVISION.fetch_add(1, Ordering::Release);
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        if $crate::macros::debug_enabled() {
            $crate::macros::push("DEBUG", format!($($arg)*))
        }
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::macros::push("INFO", format!($($arg)*))
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::macros::push("WARN", format!($($arg)*))
    };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::macros::push("ERROR", format!($($arg)*))
    };
}
