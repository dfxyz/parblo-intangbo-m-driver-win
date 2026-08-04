pub mod hid;
pub mod notify;
pub mod protocol;

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::System::Threading::{INFINITE, WaitForMultipleObjects};

use crate::config::{Config, Control, Keymap};
use crate::device::hid::{Event, HidDevice, HidReader};
use crate::device::notify::DeviceNotifier;
use crate::device::protocol::{
    Button, HANDSHAKE_MESSAGES, InputEvent, PRODUCT_ID, PenEvent, USAGE_PAGE_EVENT,
    USAGE_PAGE_HANDSHAKE, VENDOR_ID, parse_input,
};
use crate::inject::keyboard::KeyInjector;
use crate::inject::pen::PenInjector;
use crate::shared::Shared;
use crate::{debug, info, warn};

const HANDSHAKE_TIMEOUT_MS: u32 = 2000;
const RECONNECT_INTERVAL_MS: u32 = 1000;

pub struct Driver {
    shared: Arc<Shared>,
    config: Config,
    config_version: u64,
    pen: PenInjector,
    button_keys: KeyInjector,
    /// 两个笔侧键可以同时按下，因此各自独立注入
    stylus_keys: [KeyInjector; 2],
    stylus_pressed: [bool; 2],
    session: Option<Session>,
    /// 重连每秒重试一次，这里记住上次的失败原因，只在变化时写日志
    last_connect_error: Option<String>,
    /// 笔事件有两百多赫兹，调试日志按这个计数抽样，否则瞬间冲掉整个日志环
    pen_events: u64,
}

/// 字段顺序决定析构顺序：读取器持有设备句柄，必须先于设备析构
struct Session {
    reader: HidReader,
    _event_device: HidDevice,
    _handshake_device: HidDevice,
}

impl Driver {
    pub fn new(shared: Arc<Shared>) -> Result<Self> {
        let config = shared.config();
        let config_version = shared.config_version();
        let mut pen = PenInjector::new(Default::default())?;
        pen.set_curve(config.pressure.clone());
        pen.set_area(config.area.clone());
        Ok(Self {
            shared,
            config,
            config_version,
            pen,
            button_keys: KeyInjector::default(),
            stylus_keys: Default::default(),
            stylus_pressed: [false; 2],
            session: None,
            last_connect_error: None,
            pen_events: 0,
        })
    }

    pub fn run(&mut self, quit: &Event) -> Result<()> {
        let notifier = DeviceNotifier::new().context("无法注册设备到达通知")?;
        info!("驱动任务开始运行");
        loop {
            self.refresh_config();

            if self.session.is_none() {
                if !self.try_connect(quit, &notifier)? {
                    return Ok(());
                }
                continue;
            }

            let session = self.session.as_mut().unwrap();
            if let Err(e) = session.reader.arm() {
                warn!("发起读取失败: {:#}", e);
                self.disconnect();
                continue;
            }

            let handles = [session.reader.event(), quit.handle(), notifier.event()];
            let waited = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
            match waited.0.wrapping_sub(WAIT_OBJECT_0.0) {
                0 => {
                    let outcome = {
                        let session = self.session.as_mut().unwrap();
                        session.reader.complete().map(parse_input)
                    };
                    match outcome {
                        Ok(Some(event)) => self.handle_event(event)?,
                        Ok(None) => {}
                        Err(e) => {
                            warn!("读取设备输入失败: {:#}", e);
                            self.disconnect();
                        }
                    }
                }
                1 => return Ok(()),
                2 => notifier.reset(),
                _ => return Err(anyhow!("WaitForMultipleObjects返回了意外的结果: {:?}", waited)),
            }
        }
    }

    fn refresh_config(&mut self) {
        let version = self.shared.config_version();
        if version == self.config_version {
            return;
        }
        self.config_version = version;
        self.config = self.shared.config();
        self.pen.set_curve(self.config.pressure.clone());
        self.pen.set_area(self.config.area.clone());
        if self.shared.keymap_index() >= self.config.keymaps.len() {
            self.shared.set_keymap_index(0);
        }
        debug!(
            "已重新加载配置；当前使用按键映射方案{}",
            self.shared.keymap_index()
        );
    }

    /// 返回`false`表示收到了退出信号
    fn try_connect(&mut self, quit: &Event, notifier: &DeviceNotifier) -> Result<bool> {
        notifier.reset();
        match open_session() {
            Ok(session) => {
                self.session = Some(session);
                self.last_connect_error = None;
                let ranges = match hid::read_pen_ranges(VENDOR_ID, PRODUCT_ID) {
                    Ok(ranges) => {
                        info!(
                            "设备已连接；笔量程 X=0..{} Y=0..{} 压力=0..{}",
                            ranges.x_max, ranges.y_max, ranges.pressure_max
                        );
                        self.pen.set_ranges(ranges);
                        Some(ranges)
                    }
                    Err(e) => {
                        warn!("无法读取笔的量程，沿用默认值: {:#}", e);
                        None
                    }
                };
                self.pen.refresh_screen();
                self.shared.update_status(|status| {
                    status.connected = true;
                    status.ranges = ranges;
                });
                Ok(true)
            }
            Err(e) => {
                let message = format!("{:#}", e);
                if self.last_connect_error.as_deref() != Some(message.as_str()) {
                    warn!("连接设备失败，将持续重试: {}", message);
                    self.last_connect_error = Some(message);
                }
                let handles = [quit.handle(), notifier.event()];
                let waited =
                    unsafe { WaitForMultipleObjects(&handles, false, RECONNECT_INTERVAL_MS) };
                Ok(waited != WAIT_OBJECT_0)
            }
        }
    }

    fn disconnect(&mut self) {
        self.session = None;
        self.cleanup();
        warn!("设备已断开，等待重新连接");
    }

    /// 系统不会替我们收拾注入出去的状态：设备断开时若某个键正按着，
    /// 它会一直卡在按下状态，因此每条退出路径都必须补上释放
    fn cleanup(&mut self) {
        self.stylus_pressed = [false; 2];
        let _ = self.pen.leave();
        let _ = self.button_keys.release();
        for keys in &mut self.stylus_keys {
            let _ = keys.release();
        }
        self.shared.update_status(|status| status.connected = false);
    }

    fn handle_event(&mut self, event: InputEvent) -> Result<()> {
        match event {
            InputEvent::Button(button) => self.handle_button(button),
            InputEvent::Pen(pen) => self.handle_pen(pen),
        }
    }

    fn handle_button(&mut self, button: Button) -> Result<()> {
        debug!("收到按键事件: {:?}", button);
        self.shared.record_button(button);
        let Some(control) = control_of(button) else {
            return self.button_keys.release();
        };
        match self.config.resolve(self.shared.keymap_index(), control) {
            Keymap::None | Keymap::Fallback => Ok(()),
            Keymap::Press(keys) => self.button_keys.press(&keys),
            Keymap::SwitchSchema => {
                self.switch_schema();
                Ok(())
            }
        }
    }

    fn handle_pen(&mut self, pen: PenEvent) -> Result<()> {
        if self.pen_events % 60 == 0 {
            debug!("笔事件: {:?}", pen);
        }
        self.pen_events += 1;
        self.shared.record_pen(pen);
        let keymaps = [
            self.config
                .resolve(self.shared.keymap_index(), Control::StylusButton0),
            self.config
                .resolve(self.shared.keymap_index(), Control::StylusButton1),
        ];
        let pressed = [pen.button0_pressed, pen.button1_pressed];
        for index in 0..2 {
            self.apply_stylus_button(index, pressed[index], &keymaps[index])?;
        }
        self.pen.handle(&pen)
    }

    fn apply_stylus_button(&mut self, index: usize, pressed: bool, keymap: &Keymap) -> Result<()> {
        if pressed == self.stylus_pressed[index] {
            return Ok(());
        }
        self.stylus_pressed[index] = pressed;
        if !pressed {
            return self.stylus_keys[index].release();
        }
        match keymap {
            Keymap::None | Keymap::Fallback => Ok(()),
            Keymap::Press(keys) => self.stylus_keys[index].press(keys),
            Keymap::SwitchSchema => {
                self.switch_schema();
                Ok(())
            }
        }
    }

    fn switch_schema(&mut self) {
        let count = self.config.keymaps.len();
        if count <= 1 {
            return;
        }
        let index = (self.shared.keymap_index() + 1) % count;
        self.shared.set_keymap_index(index);
        info!("已切换到按键映射方案{}", index);
    }
}

/// 覆盖驱动循环正常返回、返回错误、以及线程展开这三条路径
impl Drop for Driver {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn control_of(button: Button) -> Option<Control> {
    match button {
        Button::Release => None,
        Button::Button0 => Some(Control::Button0),
        Button::Button1 => Some(Control::Button1),
        Button::Button2 => Some(Control::Button2),
        Button::Button3 => Some(Control::Button3),
        Button::Button4 => Some(Control::Button4),
        Button::Button5 => Some(Control::Button5),
        Button::Button6 => Some(Control::Button6),
        Button::Button7 => Some(Control::Button7),
        Button::Ring0 => Some(Control::Ring0),
        Button::Ring1 => Some(Control::Ring1),
        Button::RingButton => Some(Control::RingButton),
    }
}

fn open_session() -> Result<Session> {
    let interfaces = hid::enumerate(VENDOR_ID, PRODUCT_ID).context("枚举HID接口失败")?;
    let event_interface = interfaces
        .iter()
        .find(|interface| interface.usage_page == USAGE_PAGE_EVENT)
        .context("找不到事件用途的HID集合")?;
    let handshake_interface = interfaces
        .iter()
        .find(|interface| interface.usage_page == USAGE_PAGE_HANDSHAKE)
        .context("找不到握手用途的HID集合")?;

    let event_device = HidDevice::open(event_interface)?;
    let handshake_device = HidDevice::open(handshake_interface)?;

    for (index, message) in HANDSHAKE_MESSAGES.iter().enumerate() {
        let (device, input_len) = if message[0] == 0xfd {
            (&handshake_device, handshake_interface.input_len)
        } else {
            (&event_device, event_interface.input_len)
        };
        device
            .write_report(message)
            .with_context(|| format!("发送第{}条握手消息失败", index))?;
        // 设备已在上报输入时，这里读到的可能是笔事件而非握手响应；
        // 握手本身不依赖响应内容，读取只是为了排空并确认设备有回应
        let response = device
            .read_report(input_len, HANDSHAKE_TIMEOUT_MS)
            .with_context(|| format!("读取第{}条握手响应失败", index))?;
        debug!(
            "握手[{}]响应: {:02x?}",
            index,
            &response[..response.len().min(20)]
        );
    }

    let reader = HidReader::new(&event_device, event_interface.input_len)?;
    Ok(Session {
        reader,
        _event_device: event_device,
        _handshake_device: handshake_device,
    })
}
