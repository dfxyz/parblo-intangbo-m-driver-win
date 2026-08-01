use std::sync::mpsc::{Receiver, channel};

use anyhow::{Context, Result};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum TrayCommand {
    Show,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    receiver: Receiver<TrayCommand>,
}

impl Tray {
    pub fn new(ctx: egui::Context) -> Result<Self> {
        let (sender, receiver) = channel();

        let menu = Menu::new();
        let show_item = MenuItem::new("显示窗口", true, None);
        let quit_item = MenuItem::new("退出", true, None);
        menu.append(&show_item).context("无法添加托盘菜单项")?;
        menu.append(&quit_item).context("无法添加托盘菜单项")?;
        let show_id = show_item.id().clone();
        let quit_id = quit_item.id().clone();

        {
            let sender = sender.clone();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let command = if event.id == show_id {
                    TrayCommand::Show
                } else if event.id == quit_id {
                    TrayCommand::Quit
                } else {
                    return;
                };
                let _ = sender.send(command);
                ctx.request_repaint();
            }));
        }
        {
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::DoubleClick { .. } = event {
                    let _ = sender.send(TrayCommand::Show);
                    ctx.request_repaint();
                }
            }));
        }

        let (rgba, width, height) = icon_image();
        let icon = Icon::from_rgba(rgba, width, height).context("无法构造托盘图标")?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Parblo Intangbo M")
            .with_icon(icon)
            .build()
            .context("无法创建托盘图标")?;

        Ok(Self {
            _icon: tray,
            receiver,
        })
    }

    pub fn poll(&self) -> Option<TrayCommand> {
        self.receiver.try_recv().ok()
    }
}

const ICON_SIZE: u32 = 64;

/// 大写字母P的覆盖率蒙版，逐像素一个字节。
/// 由Fira Noto SC Bold的字形轮廓离线渲染而来，免去运行时的字体依赖
const LETTER_MASK: &[u8] = include_bytes!("letter_p.mask");

/// 画一个紫底白字P的圆形图标，省去随程序分发图片文件。
/// 圆做四倍超采样，否则这个尺寸下的圆弧会有明显锯齿
pub fn icon_image() -> (Vec<u8>, u32, u32) {
    const SAMPLES: u32 = 4;
    const PURPLE: [f32; 3] = [0x7c as f32, 0x3a as f32, 0xed as f32];
    let center = ICON_SIZE as f32 / 2.0;
    let radius = center - 0.5;

    let circle_coverage = |x: u32, y: u32| {
        let mut hits = 0;
        for sub_y in 0..SAMPLES {
            for sub_x in 0..SAMPLES {
                let px = x as f32 + (sub_x as f32 + 0.5) / SAMPLES as f32 - center;
                let py = y as f32 + (sub_y as f32 + 0.5) / SAMPLES as f32 - center;
                if (px * px + py * py).sqrt() <= radius {
                    hits += 1;
                }
            }
        }
        hits as f32 / (SAMPLES * SAMPLES) as f32
    };

    let mut rgba = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let circle = circle_coverage(x, y);
            if circle <= 0.0 {
                continue;
            }
            let index = (y * ICON_SIZE + x) as usize;
            let letter = (LETTER_MASK[index] as f32 / 255.0).min(circle);
            let offset = index * 4;
            for channel in 0..3 {
                let value = PURPLE[channel] + (255.0 - PURPLE[channel]) * letter;
                rgba[offset + channel] = value.round() as u8;
            }
            rgba[offset + 3] = (circle * 255.0).round() as u8;
        }
    }
    (rgba, ICON_SIZE, ICON_SIZE)
}
