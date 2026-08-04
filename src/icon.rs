use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// 由build.rs在编译期从assets/icon.svg栅格化而来，预乘alpha
const ICON_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon.rgba"));
const ICON_SIZE: u32 = 64;

/// 窗口与托盘共用的图标
pub fn image() -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(ICON_SIZE, ICON_SIZE);
    buffer.make_mut_bytes().copy_from_slice(ICON_RGBA);
    Image::from_rgba8_premultiplied(buffer)
}
