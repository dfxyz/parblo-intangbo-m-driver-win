//! 设备私有协议的常量与解析。
//!
//! 设备暴露了多个HID顶层集合，其中两个是厂商私有的：用途页`0xff0a`用来上报输入事件，
//! 用途页`0xff0c`用来握手。标准的鼠标、键盘集合在Windows上打不开，数位板集合被系统的
//! 笔栈独占，因此驱动只走这两个私有集合。
//!
//! # 握手
//!
//! 开始接收事件前要依次发出[`HANDSHAKE_MESSAGES`]里的四条报文，前三条发往`0xff0c`，
//! 最后一条发往`0xff0a`。每条发完都要读一次输入报告，握手本身不关心响应内容，
//! 读取只是为了排空缓冲并确认设备有回应。不握手的话设备只会走系统自带的笔驱动，
//! 私有集合上收不到任何东西。
//!
//! # 输入报文
//!
//! 事件集合上报的报文至少10字节，首字节固定是[`REPORT_ID_EVENT`]：
//!
//! ```text
//! 偏移  长度  含义
//!  0     1    报告ID，恒为0x02
//!  1     1    状态字节，见下
//!  2     2    X坐标，小端u16（按键事件里另作他用）
//!  4     2    Y坐标，小端u16
//!  6     2    压力，小端u16
//!  8     1    X方向倾斜角，i8，单位已经是度
//!  9     1    Y方向倾斜角，i8，单位已经是度
//! ```
//!
//! 状态字节的高4位决定报文类型：
//!
//! - `0xf0`：按键事件。此时偏移2、3两字节按**大端**拼成键码，对照表见`parse_button`；
//!   松开时键码为0。设备一次只上报一个按键，没有组合键。
//! - `0xa0`：笔在感应区内，低3位分别是笔尖(bit0)、下侧键(bit1)、上侧键(bit2)。
//! - `0xc0`：笔离开感应区，坐标与压力不再有意义。
//!
//! 笔的量程不在这个报文里声明，得从标准数位板集合（用途页`0x0d`、用途`0x02`）的
//! 报告描述符里读，见`super::hid::read_pen_ranges`。

pub const VENDOR_ID: u16 = 0x0483;
pub const PRODUCT_ID: u16 = 0xa013;

pub const USAGE_PAGE_EVENT: u16 = 0xff0a;
pub const USAGE_PAGE_HANDSHAKE: u16 = 0xff0c;

pub const REPORT_ID_EVENT: u8 = 0x02;

/// 握手消息；前三条发往0xff0c用途，最后一条发往0xff0a用途
pub const HANDSHAKE_MESSAGES: &[&[u8]] = &[
    &[
        0xfd, 0x89, 0xff, 0xff, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x03, 0x01, 0x01, 0x01, 0x91,
        0x20,
    ],
    &[
        0xfd, 0x89, 0xff, 0xff, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00, 0x01, 0x01, 0x02, 0x02, 0xfd,
        0x58,
    ],
    &[
        0xfd, 0x89, 0xff, 0xff, 0x00, 0x02, 0x00, 0x06, 0x00, 0x00, 0x01, 0x01, 0x02, 0x04, 0x4e,
        0x69,
    ],
    &[0x02, 0xb0, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00],
];

#[derive(Debug, PartialEq, Eq)]
pub enum InputEvent {
    Button(Button),
    Pen(PenEvent),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Button {
    Release,
    Button0,
    Button1,
    Button2,
    Button3,
    Button4,
    Button5,
    Button6,
    Button7,
    Ring0,
    Ring1,
    RingButton,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct PenEvent {
    pub in_area: bool,
    pub tip_pressed: bool,
    pub button0_pressed: bool,
    pub button1_pressed: bool,
    pub x: u16,
    pub y: u16,
    pub pressure: u16,
    pub tilt_x: i8,
    pub tilt_y: i8,
}

/// 解析私有接口上报的输入事件；返回`None`表示报文无法识别
pub fn parse_input(buf: &[u8]) -> Option<InputEvent> {
    if buf.len() < 10 || buf[0] != REPORT_ID_EVENT {
        return None;
    }
    let status = buf[1];
    if status & 0xf0 == 0xf0 {
        let code = ((buf[2] as u16) << 8) | (buf[3] as u16);
        return parse_button(code).map(InputEvent::Button);
    }

    let in_area = match status & 0xf0 {
        0xa0 => true,
        0xc0 => false,
        _ => return None,
    };
    Some(InputEvent::Pen(PenEvent {
        in_area,
        tip_pressed: status & (1 << 0) != 0,
        button0_pressed: status & (1 << 1) != 0,
        button1_pressed: status & (1 << 2) != 0,
        x: u16::from_le_bytes([buf[2], buf[3]]),
        y: u16::from_le_bytes([buf[4], buf[5]]),
        pressure: u16::from_le_bytes([buf[6], buf[7]]),
        tilt_x: buf[8] as i8,
        tilt_y: buf[9] as i8,
    }))
}

fn parse_button(code: u16) -> Option<Button> {
    match code {
        0x0000 => Some(Button::Release),
        0x0100 => Some(Button::Button0),
        0x0200 => Some(Button::Button1),
        0x0400 => Some(Button::Button2),
        0x0800 => Some(Button::Button3),
        0x0801 => Some(Button::Ring1),
        0x0802 => Some(Button::Ring0),
        0x0803 => Some(Button::RingButton),
        0x1000 => Some(Button::Button4),
        0x2000 => Some(Button::Button5),
        0x4000 => Some(Button::Button6),
        0x8000 => Some(Button::Button7),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_buttons() {
        let cases = [
            ([0x02, 0xf0, 0x00, 0x00], Button::Release),
            ([0x02, 0xf0, 0x01, 0x00], Button::Button0),
            ([0x02, 0xf0, 0x02, 0x00], Button::Button1),
            ([0x02, 0xf0, 0x04, 0x00], Button::Button2),
            ([0x02, 0xf0, 0x08, 0x00], Button::Button3),
            ([0x02, 0xf0, 0x10, 0x00], Button::Button4),
            ([0x02, 0xf0, 0x20, 0x00], Button::Button5),
            ([0x02, 0xf0, 0x40, 0x00], Button::Button6),
            ([0x02, 0xf0, 0x80, 0x00], Button::Button7),
            ([0x02, 0xf0, 0x08, 0x01], Button::Ring1),
            ([0x02, 0xf0, 0x08, 0x02], Button::Ring0),
            ([0x02, 0xf0, 0x08, 0x03], Button::RingButton),
        ];
        for (head, expected) in cases {
            let mut buf = [0u8; 10];
            buf[..4].copy_from_slice(&head);
            assert_eq!(parse_input(&buf), Some(InputEvent::Button(expected)));
        }
    }

    /// 样本取自实机采集
    #[test]
    fn parse_pen() {
        let buf = [0x02, 0xa0, 0x5b, 0x03, 0x30, 0x03, 0x00, 0x00, 0x03, 0xfc];
        let event = parse_input(&buf).unwrap();
        assert_eq!(
            event,
            InputEvent::Pen(PenEvent {
                in_area: true,
                tip_pressed: false,
                button0_pressed: false,
                button1_pressed: false,
                x: 859,
                y: 816,
                pressure: 0,
                tilt_x: 3,
                tilt_y: -4,
            })
        );
    }

    #[test]
    fn parse_pen_status_bits() {
        let mut buf = [0x02, 0xa1, 0, 0, 0, 0, 0, 0, 0, 0];
        let InputEvent::Pen(pen) = parse_input(&buf).unwrap() else {
            panic!("应当解析成笔事件");
        };
        assert!(pen.tip_pressed && !pen.button0_pressed && !pen.button1_pressed);

        buf[1] = 0xa2;
        let InputEvent::Pen(pen) = parse_input(&buf).unwrap() else {
            panic!("应当解析成笔事件");
        };
        assert!(!pen.tip_pressed && pen.button0_pressed && !pen.button1_pressed);

        buf[1] = 0xa4;
        let InputEvent::Pen(pen) = parse_input(&buf).unwrap() else {
            panic!("应当解析成笔事件");
        };
        assert!(!pen.tip_pressed && !pen.button0_pressed && pen.button1_pressed);

        buf[1] = 0xc0;
        let InputEvent::Pen(pen) = parse_input(&buf).unwrap() else {
            panic!("应当解析成笔事件");
        };
        assert!(!pen.in_area);
    }

    #[test]
    fn reject_invalid() {
        assert_eq!(parse_input(&[]), None);
        assert_eq!(parse_input(&[0x02, 0xa0]), None);
        assert_eq!(parse_input(&[0xfc, 0xa0, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(parse_input(&[0x02, 0x10, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(parse_input(&[0x02, 0xf0, 0x00, 0x09, 0, 0, 0, 0, 0, 0]), None);
    }
}
