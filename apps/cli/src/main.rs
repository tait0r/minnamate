use std::path::PathBuf;
use std::process::ExitCode;

use minnamate_core::export::{self, Format};
use minnamate_core::{Result, html5, publish, render::render, spritesheet, swf_export, xfl};

const USAGE: &str = "usage: minnamate-cli render <file.fla | xfl-folder> [--frame N] [--scale S] [-o out.png]
       minnamate-cli swf <file.fla | xfl-folder> [-o out.swf]
       minnamate-cli html5 <file.fla | xfl-folder> [-o out-folder]   (HTML5 Canvas: index.html, index.js, runtime.js, images/)
       minnamate-cli image <file.fla | xfl-folder> [--frame N] [--scale S] [--quality Q] [--transparent] [-o out.png | out.jpg]
       minnamate-cli gif <file.fla | xfl-folder> [--scale S] [--repeat N] [--transparent] [-o out.gif]
       minnamate-cli png-sequence <file.fla | xfl-folder> [--scale S] [--transparent] [-o out.png]
       minnamate-cli video <file.fla | xfl-folder> [--scale S] [-o out.mp4]
       minnamate-cli publish <file.fla | xfl-folder>
       minnamate-cli spritesheet <file.fla | xfl-folder> --symbol NAME... [--format F] [--algorithm basic | maxrects] [--scale S]
                     [--no-trim] [--no-stack] [--border N] [--padding N] [--max-size N] [-o out]
  --frame N     1-based, as in Animate's timeline (default 1)
  --symbol NAME a library symbol, or a folder for every symbol in it (\"EXPORT\"); repeat for more
  --format F    sparrow-v2 (default), starling, json, json-array, cocos2d-v2, cocos2d-v3, easeljs
  spritesheet writes out.png and its data file (out.xml, .json or .plist), named after the first symbol by default
  --quality Q   JPEG quality, 1 to 100 (default 80)
  --repeat N    the GIF plays N more times (default: loops for ever)
  png-sequence numbers its files from the output's name: out0001.png, out0002.png...
  publish writes what the document's Publish Settings turn on, beside it";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let mut args = args.into_iter();
    let command = args.next().unwrap_or_default();
    let (mut input, mut frame, mut scale, mut out) = (None, 1u32, 1.0f32, None);
    let (mut quality, mut repeat, mut transparent) = (80u8, None, false);
    let (mut symbols, mut sheet) = (Vec::new(), spritesheet::Options::default());
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "-f" | "--frame" => frame = value()?.parse()?,
            "-s" | "--scale" => scale = value()?.parse()?,
            "-o" | "--out" => out = Some(PathBuf::from(value()?)),
            "--quality" => quality = value()?.parse()?,
            "--repeat" => repeat = Some(value()?.parse()?),
            "--transparent" => transparent = true,
            "--symbol" => symbols.push(value()?),
            "--format" => {
                let name = value()?;
                sheet.format = spritesheet::DataFormat::parse(&name).ok_or(format!("unknown sprite sheet format \"{name}\"\n{USAGE}"))?;
            }
            "--algorithm" => {
                sheet.algorithm = match value()?.to_ascii_lowercase().as_str() {
                    "basic" => spritesheet::Algorithm::Basic,
                    "maxrects" => spritesheet::Algorithm::MaxRects,
                    other => return Err(format!("unknown packing algorithm \"{other}\": basic or maxrects").into()),
                }
            }
            "--no-trim" => sheet.trim = false,
            "--no-stack" => sheet.stack_duplicates = false,
            "--border" => sheet.border_padding = value()?.parse()?,
            "--padding" => sheet.shape_padding = value()?.parse()?,
            "--max-size" => sheet.max_size = value()?.parse()?,
            _ if input.is_none() && !arg.starts_with('-') => input = Some(PathBuf::from(&arg)),
            _ => return Err(USAGE.into()),
        }
    }
    let input = input.ok_or(USAGE)?;
    if !["render", "swf", "html5", "image", "gif", "png-sequence", "video", "publish", "spritesheet"].contains(&command.as_str()) {
        return Err(USAGE.into());
    }
    let frame = frame.checked_sub(1).ok_or("frames start at 1")?;
    let doc = xfl::load(&input)?;
    let out_or = |ext: &str| out.clone().unwrap_or_else(|| input.with_extension(ext));
    let (written, skipped) = match command.as_str() {
        "render" => {
            let (pixmap, skipped) = render(&doc, frame, scale)?;
            let out = out.clone().unwrap_or_else(|| "frame.png".into());
            pixmap.save_png(&out)?;
            (vec![out], skipped)
        }
        "swf" => {
            let (bytes, skipped) = swf_export::export(&doc)?;
            let out = out_or("swf");
            std::fs::write(&out, bytes)?;
            (vec![out], skipped)
        }
        "image" => {
            let out = out_or("png");
            let jpeg = out.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
            let format = if jpeg { Format::Jpeg(quality) } else { Format::Png };
            let (bytes, skipped) = export::image(&doc, frame, scale, transparent, format)?;
            std::fs::write(&out, bytes)?;
            (vec![out], skipped)
        }
        "gif" => {
            let (bytes, skipped) = export::gif(&doc, 0..export::frame_count(&doc), scale, transparent, repeat)?;
            let out = out_or("gif");
            std::fs::write(&out, bytes)?;
            (vec![out], skipped)
        }
        "png-sequence" => export::png_sequence(&doc, &out_or("png"), scale, transparent)?,
        "video" => {
            let out = out_or("mp4");
            let skipped = export::video(&doc, &out, scale)?;
            (vec![out], skipped)
        }
        "publish" => {
            let settings = publish::Settings::read(&input)?;
            let files = publish::publish(&doc, settings.current(), &input)?;
            let skipped = files.iter().flat_map(|(_, s)| s.iter().cloned()).collect();
            (files.into_iter().map(|(f, _)| f).collect(), skipped)
        }
        "spritesheet" => {
            if symbols.is_empty() {
                return Err(format!("spritesheet needs --symbol\n{USAGE}").into());
            }
            let mut all = Vec::new();
            for s in &symbols {
                let found = spritesheet::symbols_in(&doc, s);
                if found.is_empty() {
                    return Err(format!("there's no symbol or folder \"{s}\" in the library").into());
                }
                for f in found {
                    if !all.contains(&f) {
                        all.push(f);
                    }
                }
            }
            sheet.scale = scale;
            let out = out.clone().unwrap_or_else(|| input.with_file_name(spritesheet::animation_name(&symbols[0])));
            spritesheet::export(&doc, &all, &sheet, &out)?
        }
        "html5" => {
            let stem = input.file_stem().map_or("movie".into(), |s| s.to_string_lossy());
            let out = out.clone().unwrap_or_else(|| input.with_file_name(format!("{stem}-html5")));
            let skipped = html5::publish(&doc, &out, "index")?;
            (vec![out], skipped)
        }
        _ => return Err(USAGE.into()),
    };
    for s in skipped {
        eprintln!("warning: skipped {s}");
    }
    for out in written {
        println!("wrote {}", out.display());
    }
    Ok(())
}
