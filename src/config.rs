use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Error, Result, anyhow};
use serde::{Deserialize, Serialize};

/// 按键映射中可注入的单个目标
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Keyboard(u16),
    MouseLeft,
    MouseMiddle,
    MouseRight,
}

/// 配置中的名称与虚拟键码的对照表；解析与保存共用，保证两个方向一致
const KEYBOARD_KEYS: &[(&str, u16)] = &[
    ("a", 0x41), ("b", 0x42), ("c", 0x43), ("d", 0x44), ("e", 0x45), ("f", 0x46),
    ("g", 0x47), ("h", 0x48), ("i", 0x49), ("j", 0x4a), ("k", 0x4b), ("l", 0x4c),
    ("m", 0x4d), ("n", 0x4e), ("o", 0x4f), ("p", 0x50), ("q", 0x51), ("r", 0x52),
    ("s", 0x53), ("t", 0x54), ("u", 0x55), ("v", 0x56), ("w", 0x57), ("x", 0x58),
    ("y", 0x59), ("z", 0x5a),

    ("0", 0x30), ("1", 0x31), ("2", 0x32), ("3", 0x33), ("4", 0x34),
    ("5", 0x35), ("6", 0x36), ("7", 0x37), ("8", 0x38), ("9", 0x39),

    ("-", 0xbd), ("=", 0xbb), ("\\", 0xdc), ("`", 0xc0), ("[", 0xdb), ("]", 0xdd),
    (";", 0xba), ("'", 0xde), (",", 0xbc), (".", 0xbe), ("/", 0xbf),

    ("esc", 0x1b), ("tab", 0x09), ("backspace", 0x08), ("enter", 0x0d), ("space", 0x20),
    ("home", 0x24), ("end", 0x23), ("pageup", 0x21), ("pagedown", 0x22),
    ("insert", 0x2d), ("delete", 0x2e),

    ("f1", 0x70), ("f2", 0x71), ("f3", 0x72), ("f4", 0x73), ("f5", 0x74), ("f6", 0x75),
    ("f7", 0x76), ("f8", 0x77), ("f9", 0x78), ("f10", 0x79), ("f11", 0x7a), ("f12", 0x7b),

    ("up", 0x26), ("down", 0x28), ("left", 0x25), ("right", 0x27),

    ("num0", 0x60), ("num1", 0x61), ("num2", 0x62), ("num3", 0x63), ("num4", 0x64),
    ("num5", 0x65), ("num6", 0x66), ("num7", 0x67), ("num8", 0x68), ("num9", 0x69),
    ("numplus", 0x6b), ("numminus", 0x6d), ("nummultiply", 0x6a), ("numdivide", 0x6f),
    ("numdot", 0x6e),

    ("ctrl", 0xa2), ("shift", 0xa0), ("alt", 0xa4), ("meta", 0x5b),
];

const MOUSE_KEYS: &[(&str, Key)] = &[
    ("mouseLeft", Key::MouseLeft),
    ("mouseMiddle", Key::MouseMiddle),
    ("mouseRight", Key::MouseRight),
];

impl Key {
    pub fn name(&self) -> &'static str {
        match self {
            Key::Keyboard(code) => KEYBOARD_KEYS
                .iter()
                .find(|(_, value)| value == code)
                .map(|(name, _)| *name)
                .unwrap_or("none"),
            _ => MOUSE_KEYS
                .iter()
                .find(|(_, value)| value == self)
                .map(|(name, _)| *name)
                .unwrap_or("none"),
        }
    }

    fn parse(name: &str) -> Result<Self> {
        if let Some((_, code)) = KEYBOARD_KEYS.iter().find(|(key, _)| *key == name) {
            return Ok(Key::Keyboard(*code));
        }
        if let Some((_, key)) = MOUSE_KEYS.iter().find(|(key, _)| *key == name) {
            return Ok(*key);
        }
        Err(anyhow!("'{}'不是有效的按键映射配置", name))
    }
}

/// 所有可映射的按键；下标顺序同时决定了界面上的排列顺序
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Control {
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
    StylusButton0,
    StylusButton1,
}
impl Control {
    pub const ALL: [Control; 13] = [
        Control::Button0,
        Control::Button1,
        Control::Button2,
        Control::Button3,
        Control::Button4,
        Control::Button5,
        Control::Button6,
        Control::Button7,
        Control::Ring0,
        Control::Ring1,
        Control::RingButton,
        Control::StylusButton0,
        Control::StylusButton1,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Control::Button0 => "按键0",
            Control::Button1 => "按键1",
            Control::Button2 => "按键2",
            Control::Button3 => "按键3",
            Control::Button4 => "按键4",
            Control::Button5 => "按键5",
            Control::Button6 => "按键6",
            Control::Button7 => "按键7",
            Control::Ring0 => "转环逆时针",
            Control::Ring1 => "转环顺时针",
            Control::RingButton => "转环中键",
            Control::StylusButton0 => "笔下侧键",
            Control::StylusButton1 => "笔上侧键",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Keymap {
    None,
    Press(Arc<Vec<Key>>),
    SwitchSchema,
    /// 沿用前一个方案的映射；第一个方案中等同于`None`
    #[default]
    Fallback,
}
impl Keymap {
    pub fn to_config_value(&self) -> String {
        match self {
            Keymap::None => "none".to_string(),
            Keymap::SwitchSchema => "switchSchema".to_string(),
            Keymap::Fallback => "fallback".to_string(),
            Keymap::Press(keys) => keys
                .iter()
                .map(|key| key.name())
                .collect::<Vec<_>>()
                .join("+"),
        }
    }
}
impl TryFrom<&str> for Keymap {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self> {
        let mut parts: Vec<&str> = Vec::new();
        for part in value.split('+').map(|part| part.trim()) {
            if !parts.contains(&part) {
                parts.push(part);
            }
        }

        macro_rules! match_exclusive {
            ($($name:literal => $variant:expr),+ $(,)?) => {
                $(
                    if parts.contains(&$name) {
                        if parts.len() > 1 {
                            return Err(anyhow!(concat!("不能把'", $name, "'和其他键组合")));
                        }
                        return Ok($variant);
                    }
                )+
            };
        }
        match_exclusive! {
            "switchSchema" => Keymap::SwitchSchema,
            "fallback" => Keymap::Fallback,
            "none" => Keymap::None,
        }

        let mut keys = Vec::with_capacity(parts.len());
        for part in parts {
            keys.push(Key::parse(part)?);
        }
        Ok(Keymap::Press(Arc::new(keys)))
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct KeymapConfig {
    keymaps: [Keymap; Control::ALL.len()],
}
impl KeymapConfig {
    /// 全部禁用；用于凭空造出第一套方案
    pub fn none() -> Self {
        Self {
            keymaps: std::array::from_fn(|_| Keymap::None),
        }
    }

    pub fn get(&self, control: Control) -> &Keymap {
        &self.keymaps[control as usize]
    }

    pub fn set(&mut self, control: Control, keymap: Keymap) {
        self.keymaps[control as usize] = keymap;
    }
}

/// 笔压的响应曲线；两端的取值都归一化到`[0, 1]`。
/// 输入端的裁剪由控制点本身表达，例如首点取`[0.2, 0.0]`即忽略两成以下的压力
#[derive(Clone, Debug, PartialEq)]
pub struct PressureCurve {
    pub points: Vec<[f32; 2]>,
}
impl Default for PressureCurve {
    fn default() -> Self {
        Self {
            points: vec![[0.0, 0.0], [1.0, 1.0]],
        }
    }
}
impl PressureCurve {
    pub fn evaluate(&self, x: f32) -> f32 {
        let Some(first) = self.points.first() else {
            return x;
        };
        if x <= first[0] {
            return first[1].clamp(0.0, 1.0);
        }
        for pair in self.points.windows(2) {
            let (left, right) = (pair[0], pair[1]);
            if x > right[0] {
                continue;
            }
            let span = right[0] - left[0];
            let value = if span <= f32::EPSILON {
                right[1]
            } else {
                left[1] + (right[1] - left[1]) * (x - left[0]) / span
            };
            return value.clamp(0.0, 1.0);
        }
        self.points[self.points.len() - 1][1].clamp(0.0, 1.0)
    }

    /// 控制点必须按横坐标有序，拖动后需要重新整理
    pub fn sort_points(&mut self) {
        self.points
            .sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    }
}

/// 按显示器比例修正时，保留区域的哪个角作为基准
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AreaAnchor {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}
impl AreaAnchor {
    pub const ALL: [AreaAnchor; 4] = [
        AreaAnchor::TopLeft,
        AreaAnchor::TopRight,
        AreaAnchor::BottomLeft,
        AreaAnchor::BottomRight,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            AreaAnchor::TopLeft => "对齐左上角",
            AreaAnchor::TopRight => "对齐右上角",
            AreaAnchor::BottomLeft => "对齐左下角",
            AreaAnchor::BottomRight => "对齐右下角",
        }
    }

    pub fn is_right(&self) -> bool {
        matches!(self, AreaAnchor::TopRight | AreaAnchor::BottomRight)
    }

    pub fn is_bottom(&self) -> bool {
        matches!(self, AreaAnchor::BottomLeft | AreaAnchor::BottomRight)
    }

    fn to_config_value(self) -> &'static str {
        match self {
            AreaAnchor::TopLeft => "topLeft",
            AreaAnchor::TopRight => "topRight",
            AreaAnchor::BottomLeft => "bottomLeft",
            AreaAnchor::BottomRight => "bottomRight",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "topRight" => AreaAnchor::TopRight,
            "bottomLeft" => AreaAnchor::BottomLeft,
            "bottomRight" => AreaAnchor::BottomRight,
            _ => AreaAnchor::TopLeft,
        }
    }
}

/// 绘图板上实际参与映射的区域，归一化到设备上报的量程。
/// 设备声明的量程通常大于真实可感应范围，需要靠实测得出
#[derive(Clone, Debug, PartialEq)]
pub struct TabletArea {
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
    pub anchor: AreaAnchor,
}
impl Default for TabletArea {
    fn default() -> Self {
        Self {
            x_min: 0.0,
            x_max: 1.0,
            y_min: 0.0,
            y_max: 1.0,
            anchor: AreaAnchor::default(),
        }
    }
}
impl TabletArea {
    pub fn width(&self) -> f32 {
        (self.x_max - self.x_min).max(f32::EPSILON)
    }

    pub fn height(&self) -> f32 {
        (self.y_max - self.y_min).max(f32::EPSILON)
    }

    fn sanitized(mut self) -> Self {
        self.x_min = self.x_min.clamp(0.0, 1.0);
        self.y_min = self.y_min.clamp(0.0, 1.0);
        self.x_max = self.x_max.clamp(0.0, 1.0);
        self.y_max = self.y_max.clamp(0.0, 1.0);
        if self.x_max <= self.x_min || self.y_max <= self.y_min {
            return Self::default();
        }
        self
    }
}

#[derive(Clone, PartialEq)]
pub struct Config {
    pub keymaps: Vec<KeymapConfig>,
    /// 上次使用的方案下标
    pub keymap_index: usize,
    pub pressure: PressureCurve,
    pub area: TabletArea,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            keymaps: vec![KeymapConfig::none()],
            keymap_index: 0,
            pressure: PressureCurve::default(),
            area: TabletArea::default(),
        }
    }
}
impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path).context("无法读取配置文件")?;
        let raw: RawConfig = toml::from_str(&content).context("TOML解析失败")?;
        let mut keymaps = Vec::with_capacity(raw.keymaps.len());
        for (index, raw_keymap) in raw.keymaps.iter().enumerate() {
            keymaps.push(
                KeymapConfig::try_from(raw_keymap)
                    .with_context(|| format!("第{}套按键映射方案配置有误", index))?,
            );
        }
        if keymaps.is_empty() {
            keymaps.push(KeymapConfig::none());
        }
        let keymap_index = if raw.keymap_index < keymaps.len() {
            raw.keymap_index
        } else {
            0
        };
        let pressure = match raw.pressure {
            Some(raw) => PressureCurve::from(&raw),
            None => PressureCurve::default(),
        };
        let area = match raw.area {
            Some(raw) => TabletArea::from(&raw),
            None => TabletArea::default(),
        };
        Ok(Self {
            keymaps,
            keymap_index,
            pressure,
            area,
        })
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let raw = RawConfig::from(self);
        let content = toml::to_string(&raw).context("无法序列化配置")?;
        std::fs::write(path, content).context("无法写入配置文件")?;
        Ok(())
    }

    /// 逐级向前查找，直到遇到非`Fallback`的映射
    pub fn resolve(&self, index: usize, control: Control) -> Keymap {
        let mut index = index.min(self.keymaps.len().saturating_sub(1));
        loop {
            match self.keymaps[index].get(control) {
                Keymap::Fallback => {
                    if index == 0 {
                        return Keymap::None;
                    }
                    index -= 1;
                }
                keymap => return keymap.clone(),
            }
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawConfig {
    /// 必须声明在表与表数组之前，否则序列化出的TOML无法再被解析
    #[serde(default)]
    keymap_index: usize,

    #[serde(default)]
    area: Option<RawTabletArea>,

    #[serde(default)]
    pressure: Option<RawPressureCurve>,

    #[serde(default, rename = "keymap")]
    keymaps: Vec<RawKeymapConfig>,
}
impl From<&Config> for RawConfig {
    fn from(value: &Config) -> Self {
        Self {
            keymap_index: value.keymap_index,
            area: Some(RawTabletArea::from(&value.area)),
            pressure: Some(RawPressureCurve::from(&value.pressure)),
            keymaps: value.keymaps.iter().map(RawKeymapConfig::from).collect(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct RawTabletArea {
    x_min: f32,
    x_max: f32,
    y_min: f32,
    y_max: f32,
    anchor: String,
}
impl Default for RawTabletArea {
    fn default() -> Self {
        Self::from(&TabletArea::default())
    }
}
impl From<&TabletArea> for RawTabletArea {
    fn from(value: &TabletArea) -> Self {
        Self {
            x_min: value.x_min,
            x_max: value.x_max,
            y_min: value.y_min,
            y_max: value.y_max,
            anchor: value.anchor.to_config_value().to_string(),
        }
    }
}
impl From<&RawTabletArea> for TabletArea {
    fn from(value: &RawTabletArea) -> Self {
        TabletArea {
            x_min: value.x_min,
            x_max: value.x_max,
            y_min: value.y_min,
            y_max: value.y_max,
            anchor: AreaAnchor::parse(&value.anchor),
        }
        .sanitized()
    }
}

#[derive(Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct RawPressureCurve {
    points: Vec<[f32; 2]>,
}
impl Default for RawPressureCurve {
    fn default() -> Self {
        Self {
            points: PressureCurve::default().points,
        }
    }
}
impl From<&PressureCurve> for RawPressureCurve {
    fn from(value: &PressureCurve) -> Self {
        Self {
            points: value.points.clone(),
        }
    }
}
impl From<&RawPressureCurve> for PressureCurve {
    fn from(value: &RawPressureCurve) -> Self {
        let mut curve = Self {
            points: value
                .points
                .iter()
                .map(|point| [point[0].clamp(0.0, 1.0), point[1].clamp(0.0, 1.0)])
                .collect(),
        };
        if curve.points.len() < 2 {
            curve.points = PressureCurve::default().points;
        }
        curve.sort_points();
        curve
    }
}

macro_rules! raw_keymap_config {
    ($($field:ident => $control:ident),+ $(,)?) => {
        #[derive(Deserialize, Serialize)]
        #[serde(default, rename_all = "camelCase")]
        struct RawKeymapConfig {
            $($field: String),+
        }
        impl Default for RawKeymapConfig {
            fn default() -> Self {
                Self {
                    $($field: "fallback".to_string()),+
                }
            }
        }
        impl TryFrom<&RawKeymapConfig> for KeymapConfig {
            type Error = Error;
            fn try_from(value: &RawKeymapConfig) -> Result<Self> {
                let mut config = KeymapConfig::default();
                $(
                    config.set(
                        Control::$control,
                        Keymap::try_from(value.$field.as_str())
                            .context(concat!("字段'", stringify!($field), "'配置有误"))?,
                    );
                )+
                Ok(config)
            }
        }
        impl From<&KeymapConfig> for RawKeymapConfig {
            fn from(value: &KeymapConfig) -> Self {
                Self {
                    $($field: value.get(Control::$control).to_config_value()),+
                }
            }
        }
    };
}
raw_keymap_config! {
    button0 => Button0,
    button1 => Button1,
    button2 => Button2,
    button3 => Button3,
    button4 => Button4,
    button5 => Button5,
    button6 => Button6,
    button7 => Button7,
    ring0 => Ring0,
    ring1 => Ring1,
    ring_button => RingButton,
    stylus_button0 => StylusButton0,
    stylus_button1 => StylusButton1,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_roundtrip() {
        for value in ["none", "switchSchema", "fallback", "ctrl+shift+z", "mouseRight"] {
            let keymap = Keymap::try_from(value).unwrap();
            assert_eq!(keymap.to_config_value(), value);
        }
    }

    #[test]
    fn reject_invalid_combination() {
        assert!(Keymap::try_from("ctrl+switchSchema").is_err());
        assert!(Keymap::try_from("nosuchkey").is_err());
    }

    #[test]
    fn fallback_resolves_to_previous_schema() {
        let mut first = KeymapConfig::default();
        first.set(Control::Button0, Keymap::try_from("a").unwrap());
        let second = KeymapConfig::default();
        let config = Config {
            keymaps: vec![first, second],
            ..Default::default()
        };
        assert_eq!(
            config.resolve(1, Control::Button0).to_config_value(),
            "a"
        );
        assert_eq!(config.resolve(1, Control::Button1), Keymap::None);
    }
}
