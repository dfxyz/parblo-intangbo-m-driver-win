use anyhow::{Context, Result};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, DestroySyntheticPointerDevice, HSYNTHETICPOINTERDEVICE,
    POINTER_FEEDBACK_NONE, POINTER_TYPE_INFO, POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::Input::Pointer::{
    InjectSyntheticPointerInput, POINTER_FLAG_DOWN, POINTER_FLAG_FIRSTBUTTON,
    POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_PRIMARY, POINTER_FLAG_UP,
    POINTER_FLAG_UPDATE, POINTER_FLAGS, POINTER_INFO, POINTER_PEN_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, PEN_MASK_PRESSURE, PEN_MASK_TILT_X, PEN_MASK_TILT_Y, PT_PEN,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use crate::config::{PressureCurve, TabletArea};
use crate::device::hid::PenRanges;
use crate::device::protocol::PenEvent;

/// 合成笔的压力上限；由Windows Ink规定，设备的8192级需要压缩到这个范围
const INJECT_PRESSURE_MAX: u32 = 1024;

pub struct PenInjector {
    device: HSYNTHETICPOINTERDEVICE,
    ranges: PenRanges,
    curve: PressureCurve,
    area: TabletArea,
    screen: ScreenRect,
    in_area: bool,
    tip_pressed: bool,
}

#[derive(Clone, Copy)]
struct ScreenRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl PenInjector {
    /// 注入坐标用的是物理像素，要求进程已声明Per-Monitor-V2的DPI感知，
    /// 由`crate::declare_dpi_awareness`在启动最早期负责
    pub fn new(ranges: PenRanges) -> Result<Self> {
        let device = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_NONE) }
            .context("CreateSyntheticPointerDevice")?;
        Ok(Self {
            device,
            ranges,
            curve: PressureCurve::default(),
            area: TabletArea::default(),
            screen: read_screen_rect(),
            in_area: false,
            tip_pressed: false,
        })
    }

    /// 显示器分辨率或布局变化后需要重新读取
    pub fn refresh_screen(&mut self) {
        self.screen = read_screen_rect();
    }

    pub fn set_ranges(&mut self, ranges: PenRanges) {
        self.ranges = ranges;
    }

    pub fn set_curve(&mut self, curve: PressureCurve) {
        self.curve = curve;
    }

    pub fn set_area(&mut self, area: TabletArea) {
        self.area = area;
    }

    pub fn handle(&mut self, event: &PenEvent) -> Result<()> {
        if !event.in_area {
            return self.leave();
        }

        let entering = !self.in_area;
        self.in_area = true;

        let mut flags = POINTER_FLAG_INRANGE | POINTER_FLAG_PRIMARY;
        if event.tip_pressed {
            flags |= POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON;
            if self.tip_pressed {
                flags |= POINTER_FLAG_UPDATE;
            } else {
                flags |= POINTER_FLAG_DOWN;
            }
        } else if self.tip_pressed {
            flags |= POINTER_FLAG_UP;
        } else {
            flags |= POINTER_FLAG_UPDATE;
        }
        self.tip_pressed = event.tip_pressed;

        // 刚进入感应区时先补一帧悬停，避免落笔位置被上一次离开时的坐标带偏
        if entering && event.tip_pressed {
            self.inject(
                event,
                POINTER_FLAG_INRANGE | POINTER_FLAG_PRIMARY | POINTER_FLAG_UPDATE,
                false,
            )?;
        }
        self.inject(event, flags, event.tip_pressed)
    }

    /// 笔离开感应区
    pub fn leave(&mut self) -> Result<()> {
        if !self.in_area {
            return Ok(());
        }
        let event = PenEvent {
            in_area: false,
            tip_pressed: false,
            button0_pressed: false,
            button1_pressed: false,
            x: 0,
            y: 0,
            pressure: 0,
            tilt_x: 0,
            tilt_y: 0,
        };
        let mut flags = POINTER_FLAG_UPDATE;
        if self.tip_pressed {
            flags = POINTER_FLAG_UP;
        }
        self.tip_pressed = false;
        self.in_area = false;
        self.inject(&event, flags, false)
    }

    fn inject(&self, event: &PenEvent, flags: POINTER_FLAGS, in_contact: bool) -> Result<()> {
        let point = self.to_screen(event.x, event.y);
        let pressure = if in_contact {
            scale_pressure(event.pressure, self.ranges.pressure_max, &self.curve)
        } else {
            0
        };

        let info = POINTER_PEN_INFO {
            pointerInfo: POINTER_INFO {
                pointerType: PT_PEN,
                pointerId: 0,
                ptPixelLocation: point,
                pointerFlags: flags,
                ..Default::default()
            },
            penFlags: 0,
            penMask: PEN_MASK_PRESSURE | PEN_MASK_TILT_X | PEN_MASK_TILT_Y,
            pressure,
            rotation: 0,
            tiltX: event.tilt_x as i32,
            tiltY: event.tilt_y as i32,
        };
        let type_info = POINTER_TYPE_INFO {
            r#type: PT_PEN,
            Anonymous: POINTER_TYPE_INFO_0 { penInfo: info },
        };
        unsafe { InjectSyntheticPointerInput(self.device, &[type_info]) }
            .context("InjectSyntheticPointerInput")
    }

    /// 把设备坐标映射到屏幕；参与映射的子区域由[`TabletArea::effective`]算出
    fn to_screen(&self, x: u16, y: u16) -> POINT {
        let used = self.area.effective(
            tablet_aspect(self.ranges),
            self.screen.width.max(1) as f32 / self.screen.height.max(1) as f32,
        );
        let left = used.x_min * self.ranges.x_max as f32;
        let top = used.y_min * self.ranges.y_max as f32;
        let width = ((used.x_max - used.x_min) * self.ranges.x_max as f32).max(f32::EPSILON);
        let height = ((used.y_max - used.y_min) * self.ranges.y_max as f32).max(f32::EPSILON);

        let ratio_x = ((x as f32 - left) / width).clamp(0.0, 1.0);
        let ratio_y = ((y as f32 - top) / height).clamp(0.0, 1.0);
        POINT {
            x: self.screen.x + (ratio_x * (self.screen.width - 1) as f32).round() as i32,
            y: self.screen.y + (ratio_y * (self.screen.height - 1) as f32).round() as i32,
        }
    }
}

/// 量程的宽高比；两个轴的分辨率相同，因此它也是绘图板的物理宽高比
pub fn tablet_aspect(ranges: PenRanges) -> f32 {
    ranges.x_max.max(1) as f32 / ranges.y_max.max(1) as f32
}

/// 虚拟屏幕的宽高比
pub fn screen_aspect() -> f32 {
    let screen = read_screen_rect();
    screen.width.max(1) as f32 / screen.height.max(1) as f32
}

impl Drop for PenInjector {
    fn drop(&mut self) {
        let _ = self.leave();
        unsafe {
            let _ = DestroySyntheticPointerDevice(self.device);
        }
    }
}

fn read_screen_rect() -> ScreenRect {
    unsafe {
        ScreenRect {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

fn scale_pressure(pressure: u16, max: u16, curve: &PressureCurve) -> u32 {
    if max == 0 {
        return 1;
    }
    let normalized = pressure as f32 / max as f32;
    let mapped = curve.evaluate(normalized).clamp(0.0, 1.0);
    let scaled = (mapped * INJECT_PRESSURE_MAX as f32).round() as u32;
    scaled.clamp(1, INJECT_PRESSURE_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_scaling_without_curve() {
        let curve = PressureCurve::default();
        assert_eq!(scale_pressure(0, 8191, &curve), 1);
        assert_eq!(scale_pressure(8191, 8191, &curve), 1024);
        assert_eq!(scale_pressure(4095, 8191, &curve), 512);
    }

    /// 首个控制点右移即等价于裁掉过轻的压力
    #[test]
    fn pressure_clipped_by_first_point() {
        let curve = PressureCurve {
            points: vec![[0.5, 0.0], [1.0, 1.0]],
        };
        assert_eq!(scale_pressure(4095, 8191, &curve), 1);
        assert_eq!(scale_pressure(8191, 8191, &curve), 1024);
    }
}
