// Release builds are a GUI app, with no console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions_panel;
mod app;
mod arrange_panels;
mod audio;
mod bitmaps;
mod buttons;
mod color;
mod convert_symbol;
mod cursors;
mod eraser;
mod filters;
mod free_transform;
mod gradient_transform;
mod home;
mod icons;
mod layer_tree;
mod library;
mod menu;
mod paint_tools;
mod panels;
mod pen;
mod publish;
mod scene_panel;
mod sound;
mod spritesheet;
mod stage;
mod symbol_dialogs;
mod test_movie;
mod text_tool;
mod theme;
mod timeline;
mod tools;
mod tweens;
mod video;
mod widgets;

use eframe::egui;

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Minnamate")
        .with_inner_size([1600.0, 940.0])
        // The app bar is the title bar, as in Animate on Windows.
        .with_decorations(false)
        .with_min_inner_size([960.0, 600.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/logo.png")) {
        viewport = viewport.with_icon(icon);
    }
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    // winit's Wayland backend takes no file drops; XWayland does.
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_some() {
        use winit::platform::x11::EventLoopBuilderExtX11;
        options.event_loop_builder = Some(Box::new(|b| {
            b.with_x11();
        }));
    }
    eframe::run_native("Minnamate", options, Box::new(|cc| Ok(Box::new(app::Editor::new(&cc.egui_ctx, path)))))
}
