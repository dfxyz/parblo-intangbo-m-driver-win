#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows::core::{HSTRING, PCWSTR};

use crate::config::Config;
use crate::device::hid::Event;
use crate::gui::App;
use crate::shared::Shared;

mod config;
mod device;
mod gui;
mod inject;
mod macros;
mod shared;

const WINDOW_TITLE: &str = "Parblo Intangbo M 驱动";
const INSTANCE_MUTEX_NAME: &str = "Global\\ParbloIntangboMDriver";
const SHOW_EVENT_NAME: &str = "Global\\ParbloIntangboMDriverShow";

fn main() -> Result<()> {
    let Some(_instance) = SingleInstance::acquire()? else {
        activate_existing_window();
        return Ok(());
    };

    let config_path = match std::env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => default_config_path()?,
    };
    let config = match Config::load(&config_path) {
        Ok(config) => {
            info!(
                "已加载配置文件{}，共{}套按键映射方案",
                config_path.display(),
                config.keymaps.len()
            );
            config
        }
        Err(e) => {
            warn!("加载配置文件失败，将使用空配置: {:#}", e);
            Config::default()
        }
    };

    let shared = Arc::new(Shared::new(config));
    let quit = Arc::new(Event::new()?);
    let show_signal = match Event::named(SHOW_EVENT_NAME) {
        Ok(event) => Some(Arc::new(event)),
        Err(e) => {
            warn!("无法创建唤起窗口用的事件，再次启动时将无法唤回窗口: {:#}", e);
            None
        }
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_inner_size([600.0, 600.0])
        .with_resizable(false)
        .with_maximize_button(false)
        // 常驻后台，启动时只留托盘图标，需要时再从托盘唤起
        .with_visible(false);
    match gui::icon_image() {
        Ok((rgba, width, height)) => {
            viewport = viewport.with_icon(egui::IconData {
                rgba,
                width,
                height,
            })
        }
        Err(e) => warn!("无法生成窗口图标: {:#}", e),
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            Ok(Box::new(App::new(
                &cc.egui_ctx,
                shared,
                quit,
                show_signal,
                config_path,
            )))
        }),
    )
    .map_err(|e| anyhow!("界面异常退出: {}", e))
}

/// 与可执行文件同目录、同主文件名，例如`parblo.exe`对应`parblo.toml`
fn default_config_path() -> Result<PathBuf> {
    let mut path = std::env::current_exe().context("无法获取可执行文件路径")?;
    path.set_extension("toml");
    Ok(path)
}

/// 同时运行多个实例会互相抢夺设备上报的报文
struct SingleInstance(HANDLE);
impl SingleInstance {
    fn acquire() -> Result<Option<Self>> {
        let name = HSTRING::from(INSTANCE_MUTEX_NAME);
        let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) }
            .context("CreateMutexW")?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Ok(None);
        }
        Ok(Some(Self(handle)))
    }
}
impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// 只发信号，由已有实例的界面线程自己显示窗口
fn activate_existing_window() {
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }
    match Event::open(SHOW_EVENT_NAME) {
        Ok(event) => event.set(),
        Err(e) => error!("无法通知已有实例显示窗口: {:#}", e),
    }
}
