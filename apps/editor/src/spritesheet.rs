//! The Library's Generate Sprite Sheet: every frame of a symbol, or of every symbol in a
//! folder, packed into a PNG with a data file for a game engine (minnamate_core::spritesheet),
//! its options beside a preview of the sheet. The sheet is made in the background as the
//! options change, and Export writes the one shown.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use eframe::egui::{self, Align, Color32, ColorImage, Frame, Key, Layout, Margin, Rect, RichText, Sense, TextureHandle, TextureOptions, pos2, vec2};
use minnamate_core::spritesheet::{self, Algorithm, DataFormat, Options, Sheet};
use minnamate_core::tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

use crate::app::Editor;
use crate::widgets::{self, Number};
use crate::{home, theme};

/// The preview's box.
const PREVIEW: (f32, f32) = (420.0, 340.0);

type Made = Result<(Sheet, ColorImage), String>;

pub struct Dialog {
    /// What the files are named after: the one item it was opened on, else their folder.
    name: String,
    symbols: Vec<String>,
    pub(crate) opts: Options,
    /// The sheet the options last made (or why they couldn't), and what made it.
    made: Option<(u64, Result<Sheet, String>)>,
    preview: Option<TextureHandle>,
    /// The sheet being made in the background, and what's making it.
    making: Option<(u64, Receiver<Made>)>,
}

impl Dialog {
    /// The symbols going on the sheet, in order.
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }
}

/// Opens the dialog on the Library's `items`: symbols, and folders for the symbols in them.
pub fn open(ed: &mut Editor, items: &[String]) {
    let mut symbols: Vec<String> = Vec::new();
    for item in items {
        for s in spritesheet::symbols_in(&ed.doc, item) {
            if !symbols.contains(&s) {
                symbols.push(s);
            }
        }
    }
    if symbols.is_empty() {
        return;
    }
    let parent = |p: &str| p.rsplit_once('/').map_or(String::new(), |(f, _)| f.to_string());
    let name = match items {
        [one] => spritesheet::animation_name(one).to_string(),
        [first, rest @ ..] if rest.iter().all(|i| parent(i) == parent(first)) && !parent(first).is_empty() => spritesheet::animation_name(&parent(first)).to_string(),
        _ => "spritesheet".to_string(),
    };
    ed.spritesheet = Some(Dialog { name, symbols, opts: Options::default(), made: None, preview: None, making: None });
}

/// What the sheet's pixels depend on: the document, the symbols and every option but the data format.
fn key(rev: u32, d: &Dialog) -> u64 {
    let o = &d.opts;
    crate::app::hash((rev, &d.symbols, o.algorithm == Algorithm::Basic, o.scale.to_bits(), o.trim, o.border_padding, o.shape_padding, o.stack_duplicates, o.max_size))
}

/// Starts making the sheet for the options as they are, unless that's made or being made.
fn refresh(ed: &mut Editor, ctx: &egui::Context) {
    let rev = ed.rev;
    let Some(d) = &mut ed.spritesheet else { return };
    let key = key(rev, d);
    let current = d.making.as_ref().map(|(k, _)| *k).or(d.made.as_ref().map(|(k, _)| *k));
    if current == Some(key) {
        return;
    }
    let (tx, rx) = channel();
    let (doc, symbols, opts) = (ed.doc.clone(), d.symbols.clone(), d.opts.clone());
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let made = spritesheet::generate(&doc, &symbols, &opts).map_err(|e| e.to_string()).map(|sheet| {
            let image = thumbnail(&sheet.image);
            (sheet, image)
        });
        let _ = tx.send(made);
        ctx.request_repaint();
    });
    d.making = Some((key, rx));
}

/// The sheet fitted into the preview's box.
fn thumbnail(sheet: &Pixmap) -> ColorImage {
    let (w, h) = (sheet.width() as f32, sheet.height() as f32);
    let fit = (PREVIEW.0 / w).min(PREVIEW.1 / h).min(1.0);
    let (tw, th) = ((w * fit).ceil().max(1.0) as u32, (h * fit).ceil().max(1.0) as u32);
    let mut small = Pixmap::new(tw, th).expect("a non-empty thumbnail");
    let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
    small.draw_pixmap(0, 0, sheet.as_ref(), &paint, Transform::from_scale(fit, fit), None);
    ColorImage::from_rgba_premultiplied([tw as usize, th as usize], small.data())
}

/// Takes in the sheet made in the background, once it's done.
fn collect(d: &mut Dialog, ctx: &egui::Context) {
    let Some((key, rx)) = &d.making else { return };
    let made = match rx.try_recv() {
        Ok(made) => made,
        Err(TryRecvError::Empty) => {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        Err(TryRecvError::Disconnected) => Err("making the sheet stopped".into()),
    };
    let key = *key;
    d.making = None;
    match made {
        Ok((sheet, image)) => {
            d.preview = Some(ctx.load_texture("spritesheet-preview", image, TextureOptions::LINEAR));
            d.made = Some((key, Ok(sheet)));
        }
        Err(e) => {
            d.preview = None;
            d.made = Some((key, Err(e)));
        }
    }
}

/// The dialog, while open.
pub fn dialog(ctx: &egui::Context, ed: &mut Editor) {
    if ed.spritesheet.is_none() {
        return;
    }
    refresh(ed, ctx);
    let rev = ed.rev;
    let Some(d) = &mut ed.spritesheet else { return };
    collect(d, ctx);
    let fresh = d.made.as_ref().is_some_and(|(k, _)| *k == key(rev, d));
    let popup_open = egui::Popup::is_any_open(ctx) || ed.error.is_some();
    let (mut export, mut cancel) = (false, false);
    home::dialog("generate-sprite-sheet", ctx, theme::PANEL_BG).show(ctx, |ui| {
        cancel = home::title_bar(ui, "Generate Sprite Sheet", 720.0);
        Frame::NONE.inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 14 }).show(ui, |ui| {
            ui.set_width(688.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    preview(ui, d);
                    let status = match (&d.made, d.making.is_some()) {
                        (_, true) => "Generating...".to_string(),
                        (Some((_, Ok(sheet))), _) => {
                            let images: std::collections::BTreeSet<_> = sheet.frames.iter().map(|f| (f.rect.0, f.rect.1)).collect();
                            format!("{} frames, {} images, {} × {} px", sheet.frames.len(), images.len(), sheet.image.width(), sheet.image.height())
                        }
                        (Some((_, Err(e))), _) => e.clone(),
                        (None, false) => String::new(),
                    };
                    ui.add_space(4.0);
                    ui.label(RichText::new(status).color(theme::TEXT_DIM));
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_width(240.0);
                    options(ui, d);
                });
            });
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                cancel |= ui.add(egui::Button::new("Cancel").min_size(vec2(72.0, 28.0))).clicked();
                let (_, rect) = ui.allocate_space(vec2(72.0, 28.0));
                export = home::push(ui, rect, "Export", true, 4.0).clicked();
            });
        });
    });
    if !popup_open {
        export |= ctx.input(|i| i.key_pressed(Key::Enter));
        cancel |= ctx.input(|i| i.key_pressed(Key::Escape));
    }
    if cancel {
        ed.spritesheet = None;
    } else if export {
        save(ed, fresh);
    }
}

/// The sheet on a checkerboard, fitted into its box.
fn preview(ui: &mut egui::Ui, d: &Dialog) {
    let (area, _) = ui.allocate_exact_size(vec2(PREVIEW.0, PREVIEW.1), Sense::hover());
    let p = ui.painter_at(area);
    p.rect_filled(area, 0.0, theme::INSET);
    for row in 0..(area.height() / 10.0) as i32 {
        for col in 0..(area.width() / 10.0) as i32 {
            if (row + col) % 2 == 0 {
                p.rect_filled(Rect::from_min_size(area.min + vec2(col as f32 * 10.0, row as f32 * 10.0), vec2(10.0, 10.0)), 0.0, Color32::from_gray(0x3A));
            }
        }
    }
    if let Some(tex) = &d.preview {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        p.image(tex.id(), Rect::from_center_size(area.center(), tex.size_vec2()), uv, Color32::WHITE);
    }
}

fn options(ui: &mut egui::Ui, d: &mut Dialog) {
    let title = |ui: &mut egui::Ui, text: &str| {
        ui.label(RichText::new(text).font(theme::semibold(12.0)).color(theme::SECTION_TITLE));
    };
    title(ui, "Symbols");
    let leaves: Vec<&str> = d.symbols.iter().map(|s| spritesheet::animation_name(s)).collect();
    let what = if leaves.len() == 1 { d.symbols[0].clone() } else { format!("{} symbols: {}", leaves.len(), leaves.join(", ")) };
    widgets::row(ui, |ui| ui.add(egui::Label::new(RichText::new(what).color(theme::TEXT)).truncate()));
    ui.add_space(6.0);
    title(ui, "Output");
    let o = &mut d.opts;
    widgets::row(ui, |ui| {
        widgets::label(ui, "Data format", 90.0);
        let formats: Vec<(DataFormat, &str, bool)> = DataFormat::ALL.iter().map(|&f| (f, f.name(), true)).collect();
        if let Some(f) = widgets::pick(ui, "Data format", o.format, &formats, 130.0) {
            o.format = f;
        }
    });
    widgets::row(ui, |ui| {
        widgets::label(ui, "Image format", 90.0);
        ui.add_enabled_ui(false, |ui| widgets::pick(ui, "Image format", 0, &[(0, "PNG 32 bit", true)], 130.0));
    });
    widgets::row(ui, |ui| {
        widgets::label(ui, "Algorithm", 90.0);
        if let Some(a) = widgets::pick(ui, "Algorithm", o.algorithm, &[(Algorithm::Basic, "Basic", true), (Algorithm::MaxRects, "MaxRects", true)], 130.0) {
            o.algorithm = a;
        }
    });
    let whole = |ui: &mut egui::Ui, label: &str, v: &mut u32, max: f32, suffix: &str| {
        widgets::row(ui, |ui| {
            widgets::label(ui, label, 90.0);
            let mut f = *v as f32;
            if ui.add(Number::new(label, &mut f, 0.0..=max).suffix(suffix).width(64.0)).changed() {
                *v = f.round() as u32;
            }
        });
    };
    whole(ui, "Max size", &mut o.max_size, 16384.0, " px");
    whole(ui, "Border padding", &mut o.border_padding, 100.0, " px");
    whole(ui, "Shape padding", &mut o.shape_padding, 100.0, " px");
    widgets::row(ui, |ui| {
        widgets::label(ui, "Scale", 90.0);
        let mut percent = o.scale * 100.0;
        if ui.add(Number::new("Scale", &mut percent, 1.0..=1000.0).suffix(" %").width(64.0)).changed() {
            o.scale = percent / 100.0;
        }
    });
    o.max_size = o.max_size.max(1);
    widgets::row(ui, |ui| widgets::checkbox(ui, &mut o.trim, "Trim"));
    widgets::row(ui, |ui| widgets::checkbox(ui, &mut o.stack_duplicates, "Stack frames"));
}

/// Export: asks where, then writes the PNG and its data file there, using the sheet shown if
/// it's `fresh`.
fn save(ed: &mut Editor, fresh: bool) {
    let Some(d) = &ed.spritesheet else { return };
    let name = d.name.clone();
    let format = d.opts.format;
    let picked = rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(format!("{name}.png")).save_file();
    let Some(path) = picked else { return };
    let written = match d.made.as_ref().filter(|_| fresh) {
        Some((_, Ok(sheet))) => sheet.write(format, &path).map(|files| (files, sheet.skipped.clone())),
        _ => spritesheet::export(&ed.doc, &d.symbols, &d.opts, &path),
    };
    match written {
        Ok((files, skipped)) => {
            ed.spritesheet = None;
            for f in files {
                ed.log(format!("Exported {}", f.display()));
            }
            for s in skipped {
                ed.log(format!("Warning: export skipped {s}"));
            }
        }
        Err(e) => ed.error = Some(format!("Couldn't export the sprite sheet.\n\n{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testkit::*;

    /// The tween fixture's symbols, as the Library's context menu would open the dialog on them.
    fn opened(h: &mut egui_kittest::Harness<'_, Editor>) -> String {
        let ed = h.state_mut();
        let symbol = ed.doc.library.keys().min().expect("the fixture has a symbol").clone();
        open(ed, std::slice::from_ref(&symbol));
        symbol
    }

    /// Runs the harness until the sheet for the options as they are is made.
    fn made(h: &mut egui_kittest::Harness<'_, Editor>) -> (u32, u32) {
        for _ in 0..500 {
            h.run();
            let ed = h.state();
            let d = ed.spritesheet.as_ref().expect("still open");
            if let Some((k, made)) = &d.made
                && *k == key(ed.rev, d)
            {
                let sheet = made.as_ref().unwrap_or_else(|e| panic!("no sheet: {e}"));
                return sheet.frames[0].source;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the sheet wasn't made");
    }

    #[test]
    fn the_sheet_is_made_in_the_background_and_written_on_export() {
        let mut h = harness(false);
        let symbol = opened(&mut h);
        let full = made(&mut h);
        let d = h.state().spritesheet.as_ref().unwrap();
        let Some((_, Ok(sheet))) = &d.made else { unreachable!() };
        assert_eq!(sheet.frames[0].name, format!("{}0000", spritesheet::animation_name(&symbol)));
        assert!(d.preview.is_some());

        // A changed option makes the sheet again; Escape closes the dialog.
        h.state_mut().spritesheet.as_mut().unwrap().opts.scale = 0.5;
        let half = made(&mut h);
        assert!(half.0.abs_diff(full.0 / 2) <= 1 && half.1.abs_diff(full.1 / 2) <= 1, "{full:?} at half scale: {half:?}");
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().spritesheet.is_none());
    }

    /// Writes target/screenshots/generate_sprite_sheet.png: the dialog over the tween fixture.
    #[test]
    #[ignore = "needs a GPU adapter; run with --ignored"]
    fn generate_sprite_sheet_screenshot() {
        let mut h = harness(true);
        opened(&mut h);
        for _ in 0..200 {
            h.run();
            if h.state().spritesheet.as_ref().is_some_and(|d| d.made.is_some()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        h.run();
        screenshot(&mut h, "generate_sprite_sheet");
    }
}
