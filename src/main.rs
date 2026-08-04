#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::{Context, Result};
use slint::{CloseRequestResponse, ComponentHandle};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows::core::{HSTRING, PCWSTR};

use crate::config::Config;
use crate::device::Driver;
use crate::device::hid::Event;
use crate::shared::Shared;

mod bridge;
mod callbacks;
mod config;
mod device;
mod icon;
mod inject;
mod macros;
mod shared;

slint::include_modules!();

/// 同时运行多个实例会互相抢夺设备上报的报文
const INSTANCE_MUTEX_NAME: &str = "Global\\ParbloIntangboMDriverAlt";
const SHOW_EVENT_NAME: &str = "Global\\ParbloIntangboMDriverAltShow";

fn main() -> Result<()> {
    declare_dpi_awareness();

    let Some(_instance) = SingleInstance::acquire()? else {
        activate_existing_window();
        return Ok(());
    };

    // 唯一支持的参数：启动时只显示托盘图标，不弹主界面
    let start_hidden = std::env::args().skip(1).any(|arg| arg == "-d");

    let config_path = default_config_path()?;
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
    let driver_thread = spawn_driver(shared.clone(), quit.clone());

    let ui = AppWindow::new()?;
    let bridge = bridge::create(shared.clone());
    callbacks::install(&ui, shared.clone(), bridge.clone(), config_path);
    // 定时器要一直持有，丢掉就停了
    let _refresh = bridge::start(bridge, &ui);

    // 托盘实例同样要持有到最后，drop掉图标就从通知区消失了
    let tray = install_tray(&ui);
    let _show_listener = spawn_show_listener(&ui, tray.as_ref(), quit.clone());

    if !start_hidden {
        show_window(&ui, tray.as_ref());
    }
    // 关掉窗口只是把它藏起来，事件循环要靠托盘菜单里的「退出」才结束
    slint::run_event_loop_until_quit().context("运行界面时发生错误")?;

    quit.set();
    let _ = driver_thread.join();
    Ok(())
}

fn install_tray(ui: &AppWindow) -> Option<TrayIcon> {
    let tray = match TrayIcon::new() {
        Ok(tray) => tray,
        Err(e) => {
            error!("无法创建托盘图标: {}", e);
            return None;
        }
    };
    tray.set_tray_icon(icon::image());

    // 点图标和点菜单第一项是同一个动作：在显示与隐藏之间切换
    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    tray.on_toggle_window(move || {
        let (Some(ui), tray) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };
        if ui.window().is_visible() {
            hide_window(&ui, tray.as_ref());
        } else {
            show_window(&ui, tray.as_ref());
        }
    });

    tray.on_quit(|| {
        if let Err(e) = slint::quit_event_loop() {
            error!("无法退出事件循环: {}", e);
        }
    });

    // 点窗口的关闭按钮走的是隐藏，菜单文字得跟着回到「显示窗口」
    let tray_weak = tray.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(tray) = tray_weak.upgrade() {
            tray.set_window_visible(false);
        }
        CloseRequestResponse::HideWindow
    });

    Some(tray)
}

/// 等另一个实例的唤起信号。窗口必须由界面线程来显示，
/// 所以这里只把请求投递回事件循环
fn spawn_show_listener(
    ui: &AppWindow,
    tray: Option<&TrayIcon>,
    quit: Arc<Event>,
) -> Option<JoinHandle<()>> {
    let signal = match Event::named(SHOW_EVENT_NAME) {
        Ok(event) => event,
        Err(e) => {
            warn!("无法创建唤起窗口用的事件，再次启动时将无法唤回窗口: {:#}", e);
            return None;
        }
    };
    let ui_weak = ui.as_weak();
    let tray_weak = tray.map(|tray| tray.as_weak());
    Some(std::thread::spawn(move || {
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        use windows::Win32::System::Threading::{INFINITE, WaitForMultipleObjects};

        let handles = [signal.handle(), quit.handle()];
        loop {
            let waited = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
            if waited.0.wrapping_sub(WAIT_OBJECT_0.0) != 0 {
                return;
            }
            let ui_weak = ui_weak.clone();
            let tray_weak = tray_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    let tray = tray_weak.and_then(|weak| weak.upgrade());
                    show_window(&ui, tray.as_ref());
                }
            });
        }
    }))
}

fn show_window(ui: &AppWindow, tray: Option<&TrayIcon>) {
    if let Err(e) = ui.show() {
        error!("无法显示主界面: {}", e);
        return;
    }
    // 上次可能是最小化后从托盘唤起的
    ui.window().set_minimized(false);
    if let Some(tray) = tray {
        tray.set_window_visible(true);
    }
}

fn hide_window(ui: &AppWindow, tray: Option<&TrayIcon>) {
    if let Err(e) = ui.hide() {
        error!("无法隐藏主界面: {}", e);
        return;
    }
    if let Some(tray) = tray {
        tray.set_window_visible(false);
    }
}

fn spawn_driver(shared: Arc<Shared>, quit: Arc<Event>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut driver = match Driver::new(shared) {
            Ok(driver) => driver,
            Err(e) => {
                error!("初始化驱动任务时发生错误: {:#}", e);
                return;
            }
        };
        if let Err(e) = driver.run(&quit) {
            error!("驱动任务发生错误并退出: {:#}", e);
        }
    })
}

/// 注入笔坐标用的是物理像素，进程必须声明Per-Monitor-V2的DPI感知。
/// 每个进程只能设一次，因此要抢在界面框架初始化之前
fn declare_dpi_awareness() {
    unsafe {
        if let Err(e) = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) {
            warn!("无法声明DPI感知，笔的落点可能不准: {}", e);
        }
    }
}

/// 与可执行文件同目录、同主文件名，例如`parblo.exe`对应`parblo.toml`
fn default_config_path() -> Result<PathBuf> {
    let mut path = std::env::current_exe().context("无法获取可执行文件路径")?;
    path.set_extension("toml");
    Ok(path)
}

struct SingleInstance(HANDLE);
impl SingleInstance {
    fn acquire() -> Result<Option<Self>> {
        let name = HSTRING::from(INSTANCE_MUTEX_NAME);
        let handle =
            unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) }.context("CreateMutexW")?;
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
