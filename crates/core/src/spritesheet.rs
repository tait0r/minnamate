//! Generate Sprite Sheet: every frame of one or more library symbols drawn as published,
//! trimmed, packed into one PNG and described in a data file for a game engine or framework:
//! Sparrow v2 and Starling (XML TextureAtlas), JSON (hash) and JSON-Array, cocos2D v2 and v3
//! (plists), and EaselJS. A frame is named after its symbol and numbered from 0000, as
//! "Idle0000"; frames that draw the same pixels share one place on the sheet.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use tiny_skia::{IntRect, Pixmap, PixmapPaint, Transform};

use crate::Result;
use crate::doc::{Document, Matrix};
use crate::hit::{self, Rect};
use crate::render::render_published;

/// The data file a sheet comes with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataFormat {
    SparrowV2,
    Starling,
    Json,
    JsonArray,
    Cocos2dV2,
    Cocos2dV3,
    EaselJs,
}

impl DataFormat {
    pub const ALL: [DataFormat; 7] =
        [DataFormat::Cocos2dV2, DataFormat::Cocos2dV3, DataFormat::EaselJs, DataFormat::Json, DataFormat::JsonArray, DataFormat::SparrowV2, DataFormat::Starling];

    /// As the Data format list names it.
    pub fn name(self) -> &'static str {
        match self {
            DataFormat::SparrowV2 => "Sparrow v2",
            DataFormat::Starling => "Starling",
            DataFormat::Json => "JSON",
            DataFormat::JsonArray => "JSON-Array",
            DataFormat::Cocos2dV2 => "cocos2D v2",
            DataFormat::Cocos2dV3 => "cocos2D v3",
            DataFormat::EaselJs => "EaselJS",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            DataFormat::SparrowV2 | DataFormat::Starling => "xml",
            DataFormat::Json | DataFormat::JsonArray | DataFormat::EaselJs => "json",
            DataFormat::Cocos2dV2 | DataFormat::Cocos2dV3 => "plist",
        }
    }

    /// A format by its name, any case, spaces and dashes ignored: "sparrow-v2", "json-array", "cocos2d".
    pub fn parse(name: &str) -> Option<DataFormat> {
        let key: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        Some(match key.as_str() {
            "sparrow" | "sparrowv2" => DataFormat::SparrowV2,
            "starling" => DataFormat::Starling,
            "json" | "jsonhash" => DataFormat::Json,
            "jsonarray" => DataFormat::JsonArray,
            "cocos2d" | "cocos2dv2" => DataFormat::Cocos2dV2,
            "cocos2dv3" => DataFormat::Cocos2dV3,
            "easeljs" | "createjs" => DataFormat::EaselJs,
            _ => return None,
        })
    }
}

/// How frames are laid out on the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    /// In order, row by row.
    Basic,
    /// Tightly, biggest first, each into the free space it fits best.
    MaxRects,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub format: DataFormat,
    pub algorithm: Algorithm,
    /// Frames drawn at this times the symbols' size.
    pub scale: f32,
    /// Each frame cut down to the box around its pixels; its place in the full frame is kept in the data.
    pub trim: bool,
    /// Pixels between the sheet's edges and the frames.
    pub border_padding: u32,
    /// Pixels between frames.
    pub shape_padding: u32,
    /// Frames that draw the same pixels share one place on the sheet.
    pub stack_duplicates: bool,
    /// The sheet's longest side may be at most this; it's sized to fit the frames under that.
    pub max_size: u32,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            format: DataFormat::SparrowV2,
            algorithm: Algorithm::MaxRects,
            scale: 1.0,
            trim: true,
            border_padding: 0,
            shape_padding: 2,
            stack_duplicates: true,
            max_size: 8192,
        }
    }
}

/// One frame of one symbol, as the data file describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetFrame {
    /// The symbol's name and the frame's number: "Idle0003".
    pub name: String,
    /// The symbol's name, without its folders.
    pub animation: String,
    /// Where its pixels are on the sheet: x, y, width, height.
    pub rect: (u32, u32, u32, u32),
    /// The full frame's size, the same for every frame of a symbol.
    pub source: (u32, u32),
    /// Where `rect`'s pixels sit in the full frame.
    pub offset: (u32, u32),
    /// The symbol's registration point in the full frame.
    pub registration: (f32, f32),
}

impl SheetFrame {
    fn trimmed(&self) -> bool {
        (self.rect.2, self.rect.3) != self.source
    }
}

pub struct Sheet {
    pub image: Pixmap,
    pub frames: Vec<SheetFrame>,
    pub frame_rate: f32,
    pub skipped: BTreeSet<String>,
    trim: bool,
}

/// The symbols `item` stands for: a symbol itself, or every symbol in a library folder and the
/// folders in it, by name.
pub fn symbols_in(doc: &Document, item: &str) -> Vec<String> {
    if doc.library.contains_key(item) {
        return vec![item.to_string()];
    }
    let folder = format!("{}/", item.trim_end_matches('/'));
    let mut all: Vec<String> = doc.library.keys().filter(|k| item.is_empty() || k.starts_with(&folder)).cloned().collect();
    all.sort();
    all
}

/// A symbol's name without its folders: what its frames are named after.
pub fn animation_name(symbol: &str) -> &str {
    symbol.rsplit('/').next().unwrap_or(symbol)
}

/// Pixels drawn round a frame's measured box, for antialiasing and filters reaching past it.
// ponytail: a glow or blur wider than this is cut off; grow it from the filters when that bites.
const MARGIN: f32 = 16.0;

/// Every frame of `symbols`, in order, drawn and packed as `opts` asks.
pub fn generate(doc: &Document, symbols: &[String], opts: &Options) -> Result<Sheet> {
    if symbols.is_empty() {
        return Err("no symbols to put on the sheet".into());
    }
    let scale = opts.scale;
    if !(scale > 0.0 && scale.is_finite()) {
        return Err(format!("scale {scale} isn't above 0").into());
    }
    let mut skipped = BTreeSet::new();
    let mut frames = Vec::new();
    // What each frame shows on the sheet, by index into `images`.
    let (mut images, mut shows): (Vec<Pixmap>, Vec<usize>) = (Vec::new(), Vec::new());
    let mut seen: HashMap<(u32, u32, Vec<u8>), usize> = HashMap::new();
    for name in symbols {
        let symbol = doc.library.get(name).ok_or_else(|| format!("there's no symbol \"{name}\" in the library"))?;
        let tl = &symbol.timeline;
        let count = tl.frame_count().max(1);
        let measured = (0..count).filter_map(|f| hit::published_bounds(doc, tl, f)).reduce(Rect::union);
        let Rect { min, max } = measured.unwrap_or(Rect { min: (0.0, 0.0), max: (0.0, 0.0) });
        let (x0, y0) = ((min.0 * scale).floor() - MARGIN, (min.1 * scale).floor() - MARGIN);
        let (x1, y1) = ((max.0 * scale).ceil() + MARGIN, (max.1 * scale).ceil() + MARGIN);
        let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
        if w as u64 * h as u64 > 1 << 28 {
            return Err(format!("\"{name}\" drawn at {scale}× is {w} × {h} px, too large for a sprite sheet").into());
        }
        let matrix = Matrix { a: scale, b: 0.0, c: 0.0, d: scale, tx: -x0, ty: -y0 };
        let mut drawn = Vec::new();
        for f in 0..count {
            let (pixmap, s) = render_published(doc, tl, f, &matrix, (w, h))?;
            skipped.extend(s);
            let opaque = opaque_box(&pixmap);
            drawn.push((pixmap, opaque));
        }
        // The full frame: the box round every frame's pixels.
        let full = drawn.iter().filter_map(|(_, b)| *b).reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))).unwrap_or((0, 0, 1, 1));
        let source = (full.2 - full.0, full.3 - full.1);
        let registration = (-x0 - full.0 as f32, -y0 - full.1 as f32);
        let animation = animation_name(name).to_string();
        for (f, (pixmap, opaque)) in drawn.into_iter().enumerate() {
            let cut = if opts.trim { opaque.unwrap_or((full.0, full.1, full.0 + 1, full.1 + 1)) } else { full };
            let rect = IntRect::from_ltrb(cut.0 as i32, cut.1 as i32, cut.2 as i32, cut.3 as i32).ok_or("empty frame")?;
            let crop = pixmap.clone_rect(rect).ok_or("frame outside its canvas")?;
            let key = (crop.width(), crop.height(), crop.data().to_vec());
            let at = match seen.get(&key).filter(|_| opts.stack_duplicates) {
                Some(&i) => i,
                None => {
                    images.push(crop);
                    seen.insert(key, images.len() - 1);
                    images.len() - 1
                }
            };
            shows.push(at);
            frames.push(SheetFrame {
                name: format!("{animation}{f:04}"),
                animation: animation.clone(),
                rect: (0, 0, cut.2 - cut.0, cut.3 - cut.1),
                source,
                offset: (cut.0 - full.0, cut.1 - full.1),
                registration,
            });
        }
    }
    let sizes: Vec<(u32, u32)> = images.iter().map(|p| (p.width(), p.height())).collect();
    let ((width, height), places) = pack(&sizes, opts)?;
    let mut image = Pixmap::new(width, height).ok_or("bad sheet size")?;
    for (pixmap, &(x, y)) in images.iter().zip(&places) {
        image.draw_pixmap(x as i32, y as i32, pixmap.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
    for (frame, &i) in frames.iter_mut().zip(&shows) {
        frame.rect.0 = places[i].0;
        frame.rect.1 = places[i].1;
    }
    Ok(Sheet { image, frames, frame_rate: doc.frame_rate, skipped, trim: opts.trim })
}

/// Writes the sheet's PNG and data file at `path`, its extension replaced by each one's.
/// Returns the files written.
pub fn export(doc: &Document, symbols: &[String], opts: &Options, path: &Path) -> Result<(Vec<PathBuf>, BTreeSet<String>)> {
    let sheet = generate(doc, symbols, opts)?;
    Ok((sheet.write(opts.format, path)?, sheet.skipped))
}

/// The box round a pixmap's pixels that aren't fully transparent: left, top, right, bottom.
fn opaque_box(p: &Pixmap) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (p.width(), p.height());
    let px = p.pixels();
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for y in 0..h {
        let row = &px[(y * w) as usize..((y + 1) * w) as usize];
        let Some(first) = row.iter().position(|c| c.alpha() > 0) else { continue };
        let last = row.iter().rposition(|c| c.alpha() > 0).unwrap_or(first);
        let (l, r) = (first as u32, last as u32 + 1);
        b = Some(b.map_or((l, y, r, y + 1), |(bl, bt, br, _)| (bl.min(l), bt, br.max(r), y + 1)));
    }
    b
}

/// Places boxes of `sizes` on a sheet as small as the algorithm manages within `opts.max_size`:
/// the sheet's size, and each box's top left.
fn pack(sizes: &[(u32, u32)], opts: &Options) -> Result<((u32, u32), Vec<(u32, u32)>)> {
    let (border, pad) = (opts.border_padding, opts.shape_padding);
    let padded: Vec<(u32, u32)> = sizes.iter().map(|&(w, h)| (w + pad, h + pad)).collect();
    let inner_max = opts.max_size.saturating_sub(2 * border).saturating_add(pad);
    let widest = padded.iter().map(|s| s.0).max().unwrap_or(1);
    let tallest = padded.iter().map(|s| s.1).max().unwrap_or(1);
    let too_big = || format!("the frames don't fit on a sheet {0} × {0} px: lower the scale, or raise the maximum size", opts.max_size);
    if widest > inner_max || tallest > inner_max {
        return Err(too_big().into());
    }
    let area: f64 = padded.iter().map(|&(w, h)| w as f64 * h as f64).sum();
    // Widths to try, from the widest box up past a square's.
    let side = area.sqrt();
    let mut widths: Vec<u32> = (0..=16).map(|i| (side * (0.6 + 0.1 * i as f64)).ceil() as u32).chain([widest]).map(|w| w.clamp(widest, inner_max)).collect();
    widths.sort_unstable();
    widths.dedup();
    let mut best: Option<((u32, u32), Vec<(u32, u32)>)> = None;
    for w in widths {
        let placed = match opts.algorithm {
            Algorithm::Basic => rows(&padded, w),
            Algorithm::MaxRects => max_rects(&padded, w, inner_max),
        };
        let Some(places) = placed else { continue };
        let used_w = places.iter().zip(&padded).map(|(p, s)| p.0 + s.0).max().unwrap_or(1);
        let used_h = places.iter().zip(&padded).map(|(p, s)| p.1 + s.1).max().unwrap_or(1);
        if used_h > inner_max {
            continue;
        }
        let size = (used_w, used_h);
        let better = |b: &(u32, u32)| {
            let (a0, a1) = (size.0 as u64 * size.1 as u64, b.0 as u64 * b.1 as u64);
            a0 < a1 || (a0 == a1 && size.0.abs_diff(size.1) < b.0.abs_diff(b.1))
        };
        if best.as_ref().is_none_or(|(b, _)| better(b)) {
            best = Some((size, places));
        }
    }
    let ((w, h), places) = best.ok_or_else(too_big)?;
    let outer = |v: u32| (v + 2 * border).saturating_sub(pad).max(1);
    Ok(((outer(w), outer(h)), places.into_iter().map(|(x, y)| (x + border, y + border)).collect()))
}

/// Basic: in order, left to right, a new row when the next box would pass `width`.
fn rows(sizes: &[(u32, u32)], width: u32) -> Option<Vec<(u32, u32)>> {
    let (mut x, mut y, mut row_h) = (0, 0, 0);
    let mut out = Vec::with_capacity(sizes.len());
    for &(w, h) in sizes {
        if x > 0 && x + w > width {
            (x, y, row_h) = (0, y + row_h, 0);
        }
        out.push((x, y));
        x += w;
        row_h = row_h.max(h);
    }
    Some(out)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Free {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl Free {
    fn contains(&self, o: &Free) -> bool {
        o.x >= self.x && o.y >= self.y && o.x + o.w <= self.x + self.w && o.y + o.h <= self.y + self.h
    }
}

/// MaxRects, best short side fit, biggest boxes first, in a `width` × `height` bin. None if
/// they don't all fit.
fn max_rects(sizes: &[(u32, u32)], width: u32, height: u32) -> Option<Vec<(u32, u32)>> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse((sizes[i].0.max(sizes[i].1), sizes[i].0 * sizes[i].1)));
    let mut free = vec![Free { x: 0, y: 0, w: width, h: height }];
    let mut out = vec![(0, 0); sizes.len()];
    for i in order {
        let (w, h) = sizes[i];
        let fit = free
            .iter()
            .filter(|f| f.w >= w && f.h >= h)
            .min_by_key(|f| {
                let (dw, dh) = (f.w - w, f.h - h);
                (dw.min(dh), dw.max(dh), f.y, f.x)
            })
            .copied()?;
        let placed = Free { x: fit.x, y: fit.y, w, h };
        out[i] = (placed.x, placed.y);
        let mut next = Vec::with_capacity(free.len() + 4);
        for f in free {
            let overlaps = placed.x < f.x + f.w && placed.x + placed.w > f.x && placed.y < f.y + f.h && placed.y + placed.h > f.y;
            if !overlaps {
                next.push(f);
                continue;
            }
            if placed.x > f.x {
                next.push(Free { w: placed.x - f.x, ..f });
            }
            if placed.x + placed.w < f.x + f.w {
                next.push(Free { x: placed.x + placed.w, w: f.x + f.w - placed.x - placed.w, ..f });
            }
            if placed.y > f.y {
                next.push(Free { h: placed.y - f.y, ..f });
            }
            if placed.y + placed.h < f.y + f.h {
                next.push(Free { y: placed.y + placed.h, h: f.y + f.h - placed.y - placed.h, ..f });
            }
        }
        // Free spaces inside others are dropped.
        let mut kept: Vec<Free> = Vec::with_capacity(next.len());
        for (j, f) in next.iter().enumerate() {
            let inside = next.iter().enumerate().any(|(k, o)| k != j && o.contains(f) && (o != f || k < j));
            if !inside {
                kept.push(*f);
            }
        }
        free = kept;
    }
    Some(out)
}

impl Sheet {
    /// Writes the PNG and its data file in `format` at `path`, its extension replaced by each
    /// one's. Returns the files written.
    pub fn write(&self, format: DataFormat, path: &Path) -> Result<Vec<PathBuf>> {
        let png = path.with_extension("png");
        let data = path.with_extension(format.extension());
        let image_name = png.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(dir) = png.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&png, self.image.encode_png()?)?;
        std::fs::write(&data, self.data(format, &image_name))?;
        Ok(vec![png, data])
    }

    /// The data file in `format`, naming the sheet's image `image_name`.
    pub fn data(&self, format: DataFormat, image_name: &str) -> String {
        match format {
            DataFormat::SparrowV2 => self.texture_atlas(image_name, false),
            DataFormat::Starling => self.texture_atlas(image_name, true),
            DataFormat::Json => self.json(image_name, false),
            DataFormat::JsonArray => self.json(image_name, true),
            DataFormat::Cocos2dV2 => self.plist(image_name, 2),
            DataFormat::Cocos2dV3 => self.plist(image_name, 3),
            DataFormat::EaselJs => self.easeljs(image_name),
        }
    }

    /// Sparrow v2 and Starling: a TextureAtlas of SubTextures, the trimmed ones with where they
    /// sit in the full frame (frameX and frameY, zero or less). Starling's also carry the
    /// symbol's registration point as their pivot.
    fn texture_atlas(&self, image_name: &str, pivot: bool) -> String {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        let _ = writeln!(s, "<TextureAtlas imagePath=\"{}\">", xml(image_name));
        s.push_str("\t<!-- Created with minnamate -->\n");
        for f in &self.frames {
            let (x, y, w, h) = f.rect;
            let _ = write!(s, "\t<SubTexture name=\"{}\" x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\"", xml(&f.name));
            if self.trim {
                let _ = write!(s, " frameX=\"{}\" frameY=\"{}\" frameWidth=\"{}\" frameHeight=\"{}\"", -(f.offset.0 as i64), -(f.offset.1 as i64), f.source.0, f.source.1);
            }
            if pivot {
                let _ = write!(s, " pivotX=\"{}\" pivotY=\"{}\"", num(f.registration.0), num(f.registration.1));
            }
            s.push_str("/>\n");
        }
        s.push_str("</TextureAtlas>\n");
        s
    }

    /// JSON, as a hash of frames by name or (JSON-Array) a list of them, each with its place on
    /// the sheet and in the full frame, and a "meta" block about the sheet.
    fn json(&self, image_name: &str, array: bool) -> String {
        let mut s = String::from(if array { "{\"frames\": [\n" } else { "{\"frames\": {\n" });
        for (i, f) in self.frames.iter().enumerate() {
            let (x, y, w, h) = f.rect;
            let head = if array { format!("{{\n\t\"filename\": {},", json(&f.name)) } else { format!("{}: {{", json(&f.name)) };
            let _ = write!(
                s,
                "{head}\n\t\"frame\": {{\"x\":{x},\"y\":{y},\"w\":{w},\"h\":{h}}},\n\t\"rotated\": false,\n\t\"trimmed\": {},\n\t\"spriteSourceSize\": {{\"x\":{},\"y\":{},\"w\":{w},\"h\":{h}}},\n\t\"sourceSize\": {{\"w\":{},\"h\":{}}}\n}}",
                f.trimmed(),
                f.offset.0,
                f.offset.1,
                f.source.0,
                f.source.1
            );
            s.push_str(if i + 1 < self.frames.len() { ",\n" } else { "\n" });
        }
        s.push_str(if array { "],\n" } else { "},\n" });
        let _ = write!(
            s,
            "\"meta\": {{\n\t\"app\": \"minnamate\",\n\t\"version\": \"{}\",\n\t\"image\": {},\n\t\"format\": \"RGBA8888\",\n\t\"size\": {{\"w\":{},\"h\":{}}},\n\t\"scale\": \"1\"\n}}\n}}\n",
            env!("CARGO_PKG_VERSION"),
            json(image_name),
            self.image.width(),
            self.image.height()
        );
        s
    }

    /// cocos2D's property lists: format 2 (frame, offset, rotated, sourceColorRect, sourceSize)
    /// or format 3 (spriteOffset, spriteSize, spriteSourceSize, textureRect, textureRotated).
    /// Offsets run from the full frame's centre to the trimmed one's, y up.
    fn plist(&self, image_name: &str, version: u8) -> String {
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>frames</key>\n\t<dict>\n",
        );
        let string = |key: &str, value: String| format!("\t\t\t<key>{key}</key>\n\t\t\t<string>{value}</string>\n");
        for f in &self.frames {
            let (x, y, w, h) = f.rect;
            let ox = f.offset.0 as f32 + w as f32 / 2.0 - f.source.0 as f32 / 2.0;
            let oy = f.source.1 as f32 / 2.0 - (f.offset.1 as f32 + h as f32 / 2.0);
            let _ = write!(s, "\t\t<key>{}</key>\n\t\t<dict>\n", xml(&f.name));
            if version == 2 {
                s += &string("frame", format!("{{{{{x},{y}}},{{{w},{h}}}}}"));
                s += &string("offset", format!("{{{},{}}}", num(ox), num(oy)));
                s += "\t\t\t<key>rotated</key>\n\t\t\t<false/>\n";
                s += &string("sourceColorRect", format!("{{{{{},{}}},{{{w},{h}}}}}", f.offset.0, f.offset.1));
                s += &string("sourceSize", format!("{{{},{}}}", f.source.0, f.source.1));
            } else {
                s += "\t\t\t<key>aliases</key>\n\t\t\t<array/>\n";
                s += &string("spriteOffset", format!("{{{},{}}}", num(ox), num(oy)));
                s += &string("spriteSize", format!("{{{w},{h}}}"));
                s += &string("spriteSourceSize", format!("{{{},{}}}", f.source.0, f.source.1));
                s += &string("textureRect", format!("{{{{{x},{y}}},{{{w},{h}}}}}"));
                s += "\t\t\t<key>textureRotated</key>\n\t\t\t<false/>\n";
            }
            s += "\t\t</dict>\n";
        }
        let (iw, ih) = (self.image.width(), self.image.height());
        let name = xml(image_name);
        let _ = write!(s, "\t</dict>\n\t<key>metadata</key>\n\t<dict>\n\t\t<key>format</key>\n\t\t<integer>{version}</integer>\n");
        if version == 3 {
            s += "\t\t<key>pixelFormat</key>\n\t\t<string>RGBA8888</string>\n\t\t<key>premultiplyAlpha</key>\n\t\t<false/>\n";
        }
        let _ = write!(
            s,
            "\t\t<key>realTextureFileName</key>\n\t\t<string>{name}</string>\n\t\t<key>size</key>\n\t\t<string>{{{iw},{ih}}}</string>\n\t\t<key>textureFileName</key>\n\t\t<string>{name}</string>\n\t</dict>\n</dict>\n</plist>\n"
        );
        s
    }

    /// EaselJS's SpriteSheet data: the image, each frame as [x, y, width, height, image,
    /// registration x, registration y], and an animation per symbol listing its frames.
    fn easeljs(&self, image_name: &str) -> String {
        let mut s = format!("{{\"images\": [{}],\n\"framerate\": {},\n\"frames\": [\n", json(image_name), num(self.frame_rate));
        for (i, f) in self.frames.iter().enumerate() {
            let (x, y, w, h) = f.rect;
            let (rx, ry) = (f.registration.0 - f.offset.0 as f32, f.registration.1 - f.offset.1 as f32);
            let _ = write!(s, "\t[{x},{y},{w},{h},0,{},{}]", num(rx), num(ry));
            s.push_str(if i + 1 < self.frames.len() { ",\n" } else { "\n" });
        }
        s.push_str("],\n\"animations\": {\n");
        let mut animations: Vec<(&str, Vec<usize>)> = Vec::new();
        for (i, f) in self.frames.iter().enumerate() {
            match animations.iter_mut().find(|(a, _)| *a == f.animation) {
                Some((_, list)) => list.push(i),
                None => animations.push((&f.animation, vec![i])),
            }
        }
        for (n, (name, list)) in animations.iter().enumerate() {
            let list: Vec<String> = list.iter().map(|i| i.to_string()).collect();
            let _ = write!(s, "\t{}: {{\"frames\": [{}]}}", json(name), list.join(","));
            s.push_str(if n + 1 < animations.len() { ",\n" } else { "\n" });
        }
        s.push_str("}\n}\n");
        s
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A number with at most two decimals, and none when it's whole.
fn num(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::*;
    use crate::draw;

    /// A movie-clip symbol "Anims/Walk": a 10 × 10 red square at x = 0, 20, 0 over three
    /// frames (the third drawing what the first does), and a one-frame "Anims/Idle", a 6 × 4
    /// blue box, each registered at its top left.
    fn doc() -> Document {
        let mut doc = Document::new(100.0, 100.0, 24.0);
        let square = |x: f32, w: f32, h: f32, c: Rgba| draw::rectangle([(x, 0.0), (x + w, h)], 0.0, None, Some(Paint::Solid(c)));
        let mut walk = Timeline::new("Walk");
        let layer = &mut walk.layers[0];
        for (f, x) in [0.0, 20.0, 0.0].into_iter().enumerate() {
            if f > 0 {
                layer.insert_blank_keyframe(f as u32);
            }
            draw::add(&mut layer.frames[f], square(x, 10.0, 10.0, [1.0, 0.0, 0.0, 1.0]));
        }
        let mut idle = Timeline::new("Idle");
        draw::add(&mut idle.layers[0].frames[0], square(0.0, 6.0, 4.0, [0.0, 0.0, 1.0, 1.0]));
        for (name, tl) in [("Anims/Walk", walk), ("Anims/Idle", idle)] {
            doc.library.insert(name.into(), Symbol { kind: SymbolKind::MovieClip, timeline: tl, extra: Extra::default() });
        }
        doc
    }

    fn walk_and_idle(doc: &Document) -> Vec<String> {
        let mut all = symbols_in(doc, "Anims");
        all.reverse();
        assert_eq!(all, ["Anims/Walk", "Anims/Idle"]);
        all
    }

    #[test]
    fn frames_are_trimmed_stacked_and_named() {
        let doc = doc();
        let opts = Options { shape_padding: 0, ..Options::default() };
        let sheet = generate(&doc, &walk_and_idle(&doc), &opts).unwrap();
        let names: Vec<&str> = sheet.frames.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Walk0000", "Walk0001", "Walk0002", "Idle0000"]);
        let walk = &sheet.frames[..3];
        // The full frame spans both squares: 30 × 10; each frame is cut to its square.
        assert!(walk.iter().all(|f| f.source == (30, 10) && (f.rect.2, f.rect.3) == (10, 10)), "{walk:?}");
        assert_eq!((walk[0].offset, walk[1].offset), ((0, 0), (20, 0)));
        assert_eq!(walk[0].registration, (0.0, 0.0));
        // Trimmed, each Walk frame is the same red square, only placed differently in the frame.
        assert!(walk[1].rect == walk[0].rect && walk[2].rect == walk[0].rect, "{walk:?}");
        // Two distinct images: the square and the 6 × 4 box.
        let area = sheet.image.width() * sheet.image.height();
        assert!(area >= 10 * 10 + 6 * 4 && area <= 16 * 10, "{} × {}", sheet.image.width(), sheet.image.height());
        let red = sheet.image.pixel(walk[1].rect.0 + 5, walk[1].rect.1 + 5).unwrap().demultiply();
        assert_eq!((red.red(), red.alpha()), (255, 255));
        let idle = &sheet.frames[3];
        let blue = sheet.image.pixel(idle.rect.0 + 3, idle.rect.1 + 2).unwrap().demultiply();
        assert_eq!((blue.blue(), blue.alpha()), (255, 255));

        // Untrimmed, every Walk frame is the full 30 × 10.
        let sheet = generate(&doc, &walk_and_idle(&doc), &Options { trim: false, stack_duplicates: false, ..opts }).unwrap();
        assert!(sheet.frames[..3].iter().all(|f| (f.rect.2, f.rect.3) == (30, 10) && f.offset == (0, 0)));
        let rects: BTreeSet<_> = sheet.frames.iter().map(|f| (f.rect.0, f.rect.1)).collect();
        assert_eq!(rects.len(), 4, "unstacked, each frame has its own place");
    }

    #[test]
    fn packing_never_overlaps_and_respects_padding() {
        let sizes: Vec<(u32, u32)> = (0..40).map(|i| (5 + (i * 7) % 23, 3 + (i * 11) % 17)).collect();
        for algorithm in [Algorithm::Basic, Algorithm::MaxRects] {
            let opts = Options { algorithm, border_padding: 3, shape_padding: 2, ..Options::default() };
            let ((w, h), places) = pack(&sizes, &opts).unwrap();
            for (i, (&(x, y), &(sw, sh))) in places.iter().zip(&sizes).enumerate() {
                assert!(x >= 3 && y >= 3 && x + sw + 3 <= w && y + sh + 3 <= h, "{algorithm:?}: box {i} at ({x}, {y}) in {w} × {h}");
                for (j, (&(ox, oy), &(ow, oh))) in places.iter().zip(&sizes).enumerate().skip(i + 1) {
                    let apart = x + sw + 2 <= ox || ox + ow + 2 <= x || y + sh + 2 <= oy || oy + oh + 2 <= y;
                    assert!(apart, "{algorithm:?}: boxes {i} and {j} are closer than the padding");
                }
            }
        }
        let tiny = Options { max_size: 20, ..Options::default() };
        assert!(pack(&sizes, &tiny).is_err());
    }

    #[test]
    fn data_formats_describe_the_frames() {
        let doc = doc();
        let sheet = generate(&doc, &walk_and_idle(&doc), &Options::default()).unwrap();
        let second = &sheet.frames[1];
        let (x, y, w, h) = second.rect;

        let sparrow = sheet.data(DataFormat::SparrowV2, "anims.png");
        let xmldoc = roxmltree::Document::parse(&sparrow).unwrap();
        let atlas = xmldoc.root_element();
        assert_eq!((atlas.tag_name().name(), atlas.attribute("imagePath")), ("TextureAtlas", Some("anims.png")));
        let subs: Vec<_> = atlas.children().filter(|n| n.has_tag_name("SubTexture")).collect();
        assert_eq!(subs.len(), 4);
        let attrs: Vec<_> = ["name", "x", "y", "width", "height", "frameX", "frameY", "frameWidth", "frameHeight"].iter().map(|a| subs[1].attribute(*a).unwrap().to_string()).collect();
        assert_eq!(attrs, ["Walk0001", &x.to_string(), &y.to_string(), &w.to_string(), &h.to_string(), "-20", "0", "30", "10"]);
        assert!(subs[1].attribute("pivotX").is_none());
        let starling = sheet.data(DataFormat::Starling, "anims.png");
        assert!(starling.contains("pivotX=\"0\" pivotY=\"0\""), "{starling}");

        let hash = sheet.data(DataFormat::Json, "anims.png");
        assert!(hash.starts_with("{\"frames\": {\n\"Walk0000\": {"), "{hash}");
        assert!(hash.contains(&format!("\"frame\": {{\"x\":{x},\"y\":{y},\"w\":{w},\"h\":{h}}}")));
        assert!(hash.contains("\"spriteSourceSize\": {\"x\":20,\"y\":0,\"w\":10,\"h\":10}") && hash.contains("\"sourceSize\": {\"w\":30,\"h\":10}"));
        assert!(hash.contains("\"image\": \"anims.png\""));
        let array = sheet.data(DataFormat::JsonArray, "anims.png");
        assert!(array.starts_with("{\"frames\": [\n{\n\t\"filename\": \"Walk0000\","), "{array}");
        for text in [&hash, &array] {
            assert_eq!(text.matches('{').count(), text.matches('}').count());
            assert_eq!(text.matches('[').count(), text.matches(']').count());
        }

        for version in [DataFormat::Cocos2dV2, DataFormat::Cocos2dV3] {
            let plist = sheet.data(version, "anims.png");
            let parsed = roxmltree::Document::parse_with_options(&plist, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).unwrap();
            let keys: Vec<_> = parsed.descendants().filter(|n| n.has_tag_name("key")).filter_map(|n| n.text()).collect();
            assert!(keys.contains(&"Walk0001") && keys.contains(&"textureFileName"), "{plist}");
        }
        let v2 = sheet.data(DataFormat::Cocos2dV2, "anims.png");
        // The second frame's square is the right third of the full frame: 10 px right of its centre.
        assert!(v2.contains(&format!("<string>{{{{{x},{y}}},{{{w},{h}}}}}</string>")) && v2.contains("<string>{10,0}</string>"), "{v2}");
        assert!(v2.contains("<integer>2</integer>") && sheet.data(DataFormat::Cocos2dV3, "a.png").contains("<integer>3</integer>"));

        let easel = sheet.data(DataFormat::EaselJs, "anims.png");
        assert!(easel.contains(&format!("[{x},{y},{w},{h},0,-20,0]")), "{easel}");
        assert!(easel.contains("\"Walk\": {\"frames\": [0,1,2]}") && easel.contains("\"Idle\": {\"frames\": [3]}"));
        assert!(easel.contains("\"framerate\": 24"));
    }

    #[test]
    fn export_writes_the_png_and_its_data() {
        let dir = std::env::temp_dir().join(format!("minnamate-sheet-{}", std::process::id()));
        let doc = doc();
        let opts = Options { format: DataFormat::Cocos2dV3, ..Options::default() };
        let (files, _) = export(&doc, &symbols_in(&doc, "Anims/Walk"), &opts, &dir.join("walk.whatever")).unwrap();
        let names: Vec<_> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["walk.png", "walk.plist"]);
        assert!(Pixmap::load_png(&files[0]).is_ok());
        assert!(std::fs::read_to_string(&files[1]).unwrap().contains("<string>walk.png</string>"));
        let _ = std::fs::remove_dir_all(dir);
        assert!(generate(&doc, &["Nope".into()], &opts).is_err());
    }

    #[test]
    fn formats_parse_by_name() {
        for f in DataFormat::ALL {
            assert_eq!(DataFormat::parse(f.name()), Some(f));
        }
        assert_eq!(DataFormat::parse("sparrow"), Some(DataFormat::SparrowV2));
        assert_eq!(DataFormat::parse("unknown"), None);
    }
}
