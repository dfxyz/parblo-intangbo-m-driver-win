use std::sync::mpsc::{Receiver, channel};

use anyhow::{Context, Result};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};
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

        let (rgba, width, height) = icon_image()?;
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
const ICON_SVG: &str = include_str!("../../assets/icon.svg");

/// 把图标的矢量图栅格化成托盘与窗口都能用的RGBA位图
pub fn icon_image() -> Result<(Vec<u8>, u32, u32)> {
    let tree = Tree::from_str(ICON_SVG, &Options::default()).context("无法解析图标SVG")?;
    let size = tree.size();
    let scale = ICON_SIZE as f32 / size.width().max(size.height());
    let mut pixmap = Pixmap::new(ICON_SIZE, ICON_SIZE).context("无法分配图标位图")?;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // 栅格化结果是预乘alpha的，图标接口要的是非预乘
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    Ok((rgba, ICON_SIZE, ICON_SIZE))
}
