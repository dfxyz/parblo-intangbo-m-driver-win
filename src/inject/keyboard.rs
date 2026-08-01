use anyhow::{Result, anyhow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC_EX, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEINPUT, MapVirtualKeyW, SendInput, VIRTUAL_KEY,
};

use crate::config::Key;

/// 设备本身不支持同时按下多个组合，因此只需记录当前按下的这一组
#[derive(Default)]
pub struct KeyInjector {
    pressed: Vec<Key>,
}

impl KeyInjector {
    pub fn press(&mut self, keys: &[Key]) -> Result<()> {
        self.release()?;
        for key in keys {
            send(&[make_input(*key, false)])?;
            self.pressed.push(*key);
        }
        Ok(())
    }

    pub fn release(&mut self) -> Result<()> {
        if self.pressed.is_empty() {
            return Ok(());
        }
        let keys = std::mem::take(&mut self.pressed);
        for key in keys.iter().rev() {
            send(&[make_input(*key, true)])?;
        }
        Ok(())
    }
}

impl Drop for KeyInjector {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

fn make_input(key: Key, up: bool) -> INPUT {
    match key {
        Key::Keyboard(vk) => {
            let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) };
            let extended = matches!(scan & 0xff00, 0xe000 | 0xe100);
            let mut flags = KEYEVENTF_SCANCODE;
            if extended {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            if up {
                flags |= KEYEVENTF_KEYUP;
            }
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: (scan & 0xff) as u16,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }
        }
        Key::MouseLeft | Key::MouseMiddle | Key::MouseRight => {
            let flags = match (key, up) {
                (Key::MouseLeft, false) => MOUSEEVENTF_LEFTDOWN,
                (Key::MouseLeft, true) => MOUSEEVENTF_LEFTUP,
                (Key::MouseMiddle, false) => MOUSEEVENTF_MIDDLEDOWN,
                (Key::MouseMiddle, true) => MOUSEEVENTF_MIDDLEUP,
                (Key::MouseRight, false) => MOUSEEVENTF_RIGHTDOWN,
                (Key::MouseRight, true) => MOUSEEVENTF_RIGHTUP,
                _ => unreachable!(),
            };
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }
        }
    }
}

fn send(inputs: &[INPUT]) -> Result<()> {
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err(anyhow!("SendInput只发出了{}项，期望{}项", sent, inputs.len()));
    }
    Ok(())
}
