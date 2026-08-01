use egui::{Color32, RichText};

use crate::config::{Control, Keymap};
use crate::gui::App;

const FIELD_WIDTH: f32 = 450.0;

impl App {
    pub(super) fn keymap_page(&mut self, ui: &mut egui::Ui) {
        self.schema_tabs(ui);
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.keymap_editor(ui);
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "可填写：单个按键或组合（如ctrl+shift+z）、mouseLeft/mouseMiddle/mouseRight、\
                     switchSchema（切换方案）、fallback（沿用前一个方案）、none（禁用）。",
                )
                .weak(),
            );
        });
    }

    fn keymap_editor(&mut self, ui: &mut egui::Ui) {
        let captured = self.poll_recording(ui);
        let index = self.schema_index();
        if let Some((control_index, value)) = captured {
            self.editing[index][control_index] = value;
            self.recording = None;
        }

        let recording = self.recording;
        let schema = &mut self.editing[index];
        let mut toggle = None;
        let field_size = egui::vec2(FIELD_WIDTH, ui.spacing().interact_size.y);
        egui::Grid::new("keymap_grid")
            .num_columns(3)
            .spacing([10.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                for (control_index, control) in Control::ALL.into_iter().enumerate() {
                    ui.label(control.label());
                    if recording == Some(control_index) {
                        ui.add_sized(
                            field_size,
                            egui::Label::new(
                                RichText::new("请按下快捷键…")
                                    .color(Color32::from_rgb(0x3d, 0xa5, 0x5d))
                                    .strong(),
                            ),
                        );
                    } else {
                        let text = &mut schema[control_index];
                        let valid = Keymap::try_from(text.as_str()).is_ok();
                        // Grid会按内容重算列宽，desired_width会被压回去，这里必须强制尺寸
                        ui.add_sized(
                            field_size,
                            egui::TextEdit::singleline(text).text_color_opt(if valid {
                                None
                            } else {
                                Some(Color32::from_rgb(0xd0, 0x50, 0x50))
                            }),
                        );
                    }
                    let label = if recording == Some(control_index) {
                        "取消"
                    } else {
                        "录制"
                    };
                    if ui.button(label).clicked() {
                        toggle = Some(control_index);
                    }
                    ui.end_row();
                }
            });

        if let Some(control_index) = toggle {
            self.recording = if self.recording == Some(control_index) {
                None
            } else {
                Some(control_index)
            };
        }
    }

    /// 录制期间吞掉键盘事件，返回捕获到的组合
    fn poll_recording(&mut self, ui: &mut egui::Ui) -> Option<(usize, String)> {
        let control_index = self.recording?;
        let captured = ui.input(|input| {
            for event in &input.events {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                else {
                    continue;
                };
                let Some(name) = key_name(*key) else {
                    continue;
                };
                let mut parts = Vec::with_capacity(4);
                if modifiers.ctrl {
                    parts.push("ctrl");
                }
                if modifiers.shift {
                    parts.push("shift");
                }
                if modifiers.alt {
                    parts.push("alt");
                }
                if modifiers.command && !modifiers.ctrl {
                    parts.push("meta");
                }
                parts.push(name);
                return Some(parts.join("+"));
            }
            None
        });
        captured.map(|value| (control_index, value))
    }
}

fn key_name(key: egui::Key) -> Option<&'static str> {
    use egui::Key;
    let name = match key {
        Key::A => "a",
        Key::B => "b",
        Key::C => "c",
        Key::D => "d",
        Key::E => "e",
        Key::F => "f",
        Key::G => "g",
        Key::H => "h",
        Key::I => "i",
        Key::J => "j",
        Key::K => "k",
        Key::L => "l",
        Key::M => "m",
        Key::N => "n",
        Key::O => "o",
        Key::P => "p",
        Key::Q => "q",
        Key::R => "r",
        Key::S => "s",
        Key::T => "t",
        Key::U => "u",
        Key::V => "v",
        Key::W => "w",
        Key::X => "x",
        Key::Y => "y",
        Key::Z => "z",

        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",

        Key::Minus => "-",
        Key::Equals => "=",
        Key::Backslash => "\\",
        Key::Backtick => "`",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Semicolon => ";",
        Key::Quote => "'",
        Key::Comma => ",",
        Key::Period => ".",
        Key::Slash => "/",

        Key::Escape => "esc",
        Key::Tab => "tab",
        Key::Backspace => "backspace",
        Key::Enter => "enter",
        Key::Space => "space",
        Key::Home => "home",
        Key::End => "end",
        Key::PageUp => "pageup",
        Key::PageDown => "pagedown",
        Key::Insert => "insert",
        Key::Delete => "delete",

        Key::F1 => "f1",
        Key::F2 => "f2",
        Key::F3 => "f3",
        Key::F4 => "f4",
        Key::F5 => "f5",
        Key::F6 => "f6",
        Key::F7 => "f7",
        Key::F8 => "f8",
        Key::F9 => "f9",
        Key::F10 => "f10",
        Key::F11 => "f11",
        Key::F12 => "f12",

        Key::ArrowUp => "up",
        Key::ArrowDown => "down",
        Key::ArrowLeft => "left",
        Key::ArrowRight => "right",

        _ => return None,
    };
    Some(name)
}
