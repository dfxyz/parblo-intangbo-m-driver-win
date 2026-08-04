use std::path::PathBuf;

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

/// 运行时窗口与托盘图标的边长
const RUNTIME_ICON_SIZE: u32 = 64;
/// 嵌进exe的ICO里包含的尺寸。32是任务栏与Alt+Tab实际取用的那一档，
/// 缺了它系统会拿48缩下来，边缘会糊
const ICO_SIZES: [u32; 3] = [16, 32, 48];

fn main() {
    slint_build::compile("ui/appWindow.slint").expect("Slint build failed");
    println!("cargo:rerun-if-changed=assets/icon.svg");
    build_icons();
}

/// 图标只在编译期栅格化一次：一份多尺寸ICO嵌进exe，一份位图留给运行时
fn build_icons() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("缺少OUT_DIR"));
    let svg = std::fs::read_to_string("assets/icon.svg").expect("无法读取图标SVG");
    let tree = Tree::from_str(&svg, &Options::default()).expect("无法解析图标SVG");

    // 运行时那份保持预乘，正好对上slint的Image::from_rgba8_premultiplied
    let runtime = render(&tree, RUNTIME_ICON_SIZE);
    std::fs::write(out_dir.join("icon.rgba"), runtime.data()).expect("无法写出运行时图标");

    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in ICO_SIZES {
        let image = ico::IconImage::from_rgba_data(size, size, demultiply(&render(&tree, size)));
        dir.add_entry(ico::IconDirEntry::encode(&image).expect("无法编码ICO条目"));
    }
    let ico_path = out_dir.join("icon.ico");
    let file = std::fs::File::create(&ico_path).expect("无法创建ICO文件");
    dir.write(file).expect("无法写出ICO文件");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(ico_path.to_str().expect("ICO路径不是合法的UTF-8"));
    resource.compile().expect("无法把图标嵌入可执行文件");
}

fn render(tree: &Tree, size: u32) -> Pixmap {
    let bounds = tree.size();
    let scale = size as f32 / bounds.width().max(bounds.height());
    let mut pixmap = Pixmap::new(size, size).expect("无法分配图标位图");
    resvg::render(tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap
}

/// ICO存的是非预乘的RGBA，而栅格化的结果是预乘的
fn demultiply(pixmap: &Pixmap) -> Vec<u8> {
    pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect()
}
