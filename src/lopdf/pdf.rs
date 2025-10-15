use cosmic::{
    iced::{
        Color, Font, Pixels, Point, Rectangle, Size, Vector,
        advanced::graphics::text::{
            self,
            cosmic_text::{self, Attrs, AttrsOwned, FamilyOwned, Stretch, Style, Weight, fontdb},
        },
        alignment::{Horizontal, Vertical},
        keyboard,
        widget::{
            canvas::{
                self,
                path::lyon_path::geom::euclid::{Point2D, Transform2D, UnknownUnit, Vector2D},
            },
            image,
            text::{LineHeight, Shaping},
        },
    },
    iced_renderer::geometry::Frame,
};
use lopdf::{Dictionary, Document, Encoding, Object, ObjectId};
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    mem, str,
    sync::{Arc, Mutex},
};

use super::text::Text;

type Transform = Transform2D<f32, UnknownUnit, UnknownUnit>;

#[derive(Clone, Debug)]
struct GraphicsState<'a> {
    line_join_style: i64,
    line_width: f32,
    text_attrs: AttrsOwned,
    text_encoding: Option<Arc<Encoding<'a>>>,
    text_leading: f32,
    text_mode: i64,
    text_rise: f32,
    text_size: f32,
    transform: Transform,
}

impl<'a> Default for GraphicsState<'a> {
    fn default() -> Self {
        Self {
            line_join_style: 0,
            line_width: 1.0,
            text_attrs: AttrsOwned::new(&Attrs::new()),
            text_encoding: None,
            text_leading: 0.0,
            text_mode: 0,
            text_rise: 0.0,
            text_size: 0.0,
            transform: Transform::identity(),
        }
    }
}

pub struct Image {
    pub name: String,
    pub rect: Rectangle,
    pub handle: image::Handle,
}

#[derive(Clone, Debug)]
struct TextState {
    cursor_tf: Transform,
    line_tf: Transform,
}

impl TextState {
    fn set_tf(&mut self, tf: Transform) {
        self.cursor_tf = tf;
        self.line_tf = tf;
    }
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            cursor_tf: Transform::identity(),
            line_tf: Transform::identity(),
        }
    }
}

pub struct CanvasState {
    pub scale: f32,
    pub translate: Vector,
    pub modifiers: keyboard::Modifiers,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            // Default PDF DPI is 72, default screen DPI is 96
            scale: 96.0 / 72.0,
            translate: Vector::new(0.0, 0.0),
            modifiers: keyboard::Modifiers::empty(),
        }
    }
}

// lopdf dropped Object::as_name_str in newer versions
fn as_name_str(object: &Object) -> lopdf::Result<&str> {
    str::from_utf8(object.as_name()?).map_err(|_| lopdf::Error::CharacterEncoding)
}

//TODO: errors
fn convert_color(color_space: &str, color: &[Object]) -> Color {
    use color_space::ToRgb;
    log::info!("convert {:?} {:?}", color_space, color);
    match color_space {
        "DeviceGray" => {
            let v = color[0].as_float().unwrap();
            Color::from_rgb(v, v, v)
        }
        "DeviceRGB" => {
            let r = color[0].as_float().unwrap();
            let g = color[1].as_float().unwrap();
            let b = color[2].as_float().unwrap();
            Color::from_rgb(r, g, b)
        }
        "DeviceCMYK" => {
            let c = color[0].as_float().unwrap();
            let m = color[1].as_float().unwrap();
            let y = color[2].as_float().unwrap();
            //TODO: why does this sometimes only have 3 components?
            let rgb = if color.len() > 3 {
                let k = color[3].as_float().unwrap();
                color_space::Cmyk::new(c.into(), m.into(), y.into(), k.into()).to_rgb()
            } else {
                color_space::Cmy::new(c.into(), m.into(), y.into()).to_rgb()
            };
            Color::from_rgb(rgb.r as f32, rgb.g as f32, rgb.b as f32)
        }
        _ => {
            log::warn!(
                "unsupported color space {:?} with color {:?}",
                color_space,
                color
            );
            Color::BLACK
        }
    }
}

fn finish_path(original: &mut canvas::path::Builder, transform: &Transform) -> canvas::Path {
    let mut builder = canvas::path::Builder::default();
    mem::swap(original, &mut builder);
    builder.build().transform(transform)
}

pub struct PageOp {
    pub path: Option<canvas::Path>,
    pub fill: Option<canvas::Fill>,
    pub stroke: Option<canvas::Stroke<'static>>,
    pub image: Option<Image>,
}

fn load_fonts(doc: &Document, fonts: &BTreeMap<Vec<u8>, &Dictionary>) {
    let mut font_system = text::font_system().write().expect("Write font system");

    for (name_bytes, font) in fonts.iter() {
        let name = match str::from_utf8(name_bytes) {
            Ok(ok) => ok,
            Err(err) => {
                log::warn!("failed to parse font name {name_bytes:?}: {err}");
                continue;
            }
        };
        log::info!("font {name:?} {font:?}");

        let desc = match font
            .get_deref(b"FontDescriptor", doc)
            .and_then(|x| x.as_dict())
        {
            Ok(ok) => ok,
            Err(err) => {
                log::warn!("failed to find font descriptor for font {name:?}: {err}");
                continue;
            }
        };
        log::info!("desc {desc:?}");

        match desc
            .get_deref(b"FontFile2", doc)
            .and_then(|x| x.as_stream())
        {
            Ok(stream_raw) => {
                let mut stream = stream_raw.clone();
                stream.decompress();

                let data = Arc::new(stream.content);
                let n = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
                for index in 0..n {
                    match super::ttf::parse_face_info(
                        fontdb::Source::Binary(data.clone()),
                        &data,
                        index,
                        || match font.get(b"BaseFont").and_then(as_name_str) {
                            Ok(base_font) => Some((
                                vec![(
                                    base_font.to_string(),
                                    ttf_parser::Language::English_UnitedStates,
                                )],
                                base_font.to_string(),
                            )),
                            Err(err) => {
                                log::error!("failed to get BaseFont for font {name:?}: {err}");
                                None
                            }
                        },
                    ) {
                        Ok(info) => {
                            log::info!(
                                "loaded font face {:?} for font {name:?}: {:?} {:?} {:?} {:?}",
                                info.post_script_name,
                                info.families,
                                info.stretch,
                                info.style,
                                info.weight,
                            );
                            font_system.raw().db_mut().push_face_info(info);
                        }
                        Err(e) => {
                            log::warn!("failed to load a font face {index} for font {name:?}: {e}.")
                        }
                    }
                }
                log::info!("loaded font {name:?} with {n} faces");
            }
            Err(err) => {
                log::warn!("failed to find FontFile2 for font {name:?}: {err}");
            }
        }
    }

    for face in font_system.raw().db().faces() {
        if let fontdb::Source::Binary(_) = face.source {
            log::info!("added font: {:?}", face.post_script_name);
        }
    }
}

fn load_image(
    doc: &Document,
    page_id: ObjectId,
    name: &str,
) -> Result<(image::Handle, i64, i64), lopdf::Error> {
    let page = doc.get_dictionary(page_id)?;
    let resources = doc.get_dict_in_dict(page, b"Resources")?;
    let xobject = doc.get_dict_in_dict(resources, b"XObject")?;
    let xvalue = xobject.get(name.as_bytes())?;
    let id = xvalue.as_reference()?;
    let xvalue = doc.get_object(id)?;
    let xvalue = xvalue.as_stream()?;
    let dict = &xvalue.dict;
    let sub_type = dict.get(b"Subtype")?.as_name()?;
    if sub_type != b"Image" {
        return Err(lopdf::Error::DictType {
            expected: "Image",
            found: String::from_utf8_lossy(sub_type).to_string(),
        });
    }
    let width = dict.get(b"Width")?.as_i64()?;
    let height = dict.get(b"Height")?.as_i64()?;
    let color_space = match dict.get(b"ColorSpace") {
        Ok(cs) => match cs {
            Object::Array(array) => Some(String::from_utf8_lossy(array[0].as_name()?).to_string()),
            Object::Name(name) => Some(String::from_utf8_lossy(name).to_string()),
            _ => None,
        },
        Err(_) => None,
    };
    let bits_per_component = match dict.get(b"BitsPerComponent") {
        Ok(bpc) => Some(bpc.as_i64()?),
        Err(_) => None,
    };
    let mut filters = vec![];
    if let Ok(filter) = dict.get(b"Filter") {
        match filter {
            Object::Array(array) => {
                for obj in array.iter() {
                    let name = obj.as_name()?;
                    filters.push(String::from_utf8_lossy(name).to_string());
                }
            }
            Object::Name(name) => {
                filters.push(String::from_utf8_lossy(name).to_string());
            }
            _ => {}
        }
    };

    Ok((
        image::Handle::from_bytes(xvalue.content.clone()),
        width,
        height,
    ))
}

pub fn page_ops(doc: &Document, page_id: ObjectId) -> Vec<PageOp> {
    let mut page_ops = Vec::new();
    let content = match doc.get_and_decode_page_content(page_id) {
        Ok(ok) => ok,
        Err(err) => {
            log::warn!("failed to get page contents for page {page_id:?}: {err}");
            return page_ops;
        }
    };

    let fonts = match doc.get_page_fonts(page_id) {
        Ok(ok) => ok,
        Err(err) => {
            log::warn!("failed to load fonts for page {page_id:?}: {err}");
            BTreeMap::new()
        }
    };
    load_fonts(doc, &fonts);

    /*TODO
    let (res_dict, res_vec) = doc.get_page_resources(page_id);
    println!("{:#?}", res_dict);
    println!("{:#?}", res_vec);
    */

    let mut color_space_fill = "DeviceGray".to_string();
    let mut color_fill = vec![Object::Real(0.0)];
    let mut color_space_stroke = "DeviceGray".to_string();
    let mut color_stroke = vec![Object::Real(0.0)];
    let mut graphics_states = vec![GraphicsState::default()];
    let mut text_states = vec![];
    let mut p = canvas::path::Builder::new();
    for op in content.operations.iter() {
        //TODO: better handle errors with object conversions
        // https://pdfa.org/wp-content/uploads/2023/08/PDF-Operators-CheatSheet.pdf
        match op.operator.as_str() {
            // Path construction
            "c" => {
                let x1 = op.operands[0].as_float().unwrap();
                let y1 = op.operands[1].as_float().unwrap();
                let x2 = op.operands[2].as_float().unwrap();
                let y2 = op.operands[3].as_float().unwrap();
                let x3 = op.operands[4].as_float().unwrap();
                let y3 = op.operands[5].as_float().unwrap();
                log::info!("bezier_curve_to {x1}, {y1}; {x2}, {y2}; {x3}, {y3}");
                p.bezier_curve_to(Point::new(x1, y1), Point::new(x2, y2), Point::new(x3, y3));
            }
            "h" => {
                log::info!("close");
                p.close();
            }
            "l" => {
                let x = op.operands[0].as_float().unwrap();
                let y = op.operands[1].as_float().unwrap();
                log::info!("line_to {x}, {y}");
                p.line_to(Point::new(x, y));
            }
            "m" => {
                let x = op.operands[0].as_float().unwrap();
                let y = op.operands[1].as_float().unwrap();
                log::info!("move_to {x}, {y}");
                p.move_to(Point::new(x, y));
            }
            "re" => {
                let x = op.operands[0].as_float().unwrap();
                let y = op.operands[1].as_float().unwrap();
                let w = op.operands[2].as_float().unwrap();
                let h = op.operands[3].as_float().unwrap();
                log::info!("rectangle {x}, {y}, {w}, {y}");
                p.rectangle(Point::new(x, y), Size::new(w, h));
            }

            // Path painting
            "b" | "B" | "b*" | "B*" | "f" | "f*" | "n" | "s" | "S" => {
                let (close, fill, stroke, rule) = match op.operator.as_str() {
                    "b" => (true, true, true, canvas::fill::Rule::NonZero),
                    "B" => (false, true, true, canvas::fill::Rule::NonZero),
                    "b*" => (true, true, true, canvas::fill::Rule::EvenOdd),
                    "B*" => (false, true, true, canvas::fill::Rule::EvenOdd),
                    "f" => (true, true, false, canvas::fill::Rule::NonZero),
                    "f*" => (false, true, false, canvas::fill::Rule::EvenOdd),
                    "F" => (false, true, false, canvas::fill::Rule::NonZero),
                    "n" => (false, false, false, canvas::fill::Rule::NonZero),
                    "s" => (true, false, true, canvas::fill::Rule::NonZero),
                    "S" => (false, false, true, canvas::fill::Rule::NonZero),
                    _ => panic!("unexpected path painting operator {}", op.operator),
                };
                log::info!(
                    "{}{}{}end path using {:?} winding rule",
                    if close { "close, " } else { "" },
                    if fill { "fill, " } else { "" },
                    if stroke { "stroke, " } else { "" },
                    rule
                );
                if close {
                    p.close();
                }
                let gs = graphics_states.last().unwrap();
                page_ops.push(PageOp {
                    path: Some(finish_path(&mut p, &gs.transform)),
                    fill: if fill {
                        let mut f =
                            canvas::Fill::from(convert_color(&color_space_fill, &color_fill));
                        f.rule = rule;
                        Some(f)
                    } else {
                        None
                    },
                    stroke: if stroke {
                        Some(
                            canvas::Stroke::default()
                                .with_color(convert_color(&color_space_stroke, &color_stroke))
                                .with_line_join(match gs.line_join_style {
                                    0 => canvas::LineJoin::Miter,
                                    1 => canvas::LineJoin::Round,
                                    2 => canvas::LineJoin::Bevel,
                                    _ => canvas::LineJoin::default(),
                                }),
                        )
                    } else {
                        None
                    },
                    image: None,
                });
            }

            // Text object
            "BT" => {
                text_states.push(TextState::default());
            }
            "ET" => {
                text_states.pop();
            }

            // Text state
            "Tf" => {
                //TODO: use font name
                let name = as_name_str(&op.operands[0]).unwrap();
                let size = op.operands[1].as_float().unwrap();
                log::info!("set font {name:?} size {size}");

                let mut encoding = None;
                let mut attrs = AttrsOwned::new(&Attrs::new());
                match fonts
                    .iter()
                    .find(|(font_name, _font_dict)| name.as_bytes() == *font_name)
                {
                    Some((_font_name, font_dict)) => {
                        log::info!("{:?}", font_dict);

                        encoding = match font_dict.get_font_encoding(doc) {
                            Ok(ok) => Some(ok),
                            Err(err) => {
                                log::warn!("failed to get encoding: {:?}", err);
                                None
                            }
                        };

                        match font_dict
                            .get_deref(b"FontDescriptor", doc)
                            .and_then(|x| x.as_dict())
                        {
                            Ok(desc) => {
                                log::info!("{desc:?}");

                                match desc.get(b"FontStretch").and_then(as_name_str) {
                                    Ok(font_stretch) => match font_stretch {
                                        "UltraCondensed" => attrs.stretch = Stretch::UltraCondensed,
                                        "ExtraCondensed" => attrs.stretch = Stretch::ExtraCondensed,
                                        "Condensed" => attrs.stretch = Stretch::Condensed,
                                        "SemiCondensed" => attrs.stretch = Stretch::SemiCondensed,
                                        "Normal" => attrs.stretch = Stretch::Normal,
                                        "SemiExpanded" => attrs.stretch = Stretch::SemiExpanded,
                                        "Expanded" => attrs.stretch = Stretch::Expanded,
                                        "ExtraExpanded" => attrs.stretch = Stretch::ExtraExpanded,
                                        "UltraExpanded" => attrs.stretch = Stretch::UltraExpanded,
                                        _ => {
                                            log::warn!("unknown stretch {:?}", font_stretch);
                                        }
                                    },
                                    Err(_err) => {}
                                }

                                match desc.get(b"FontWeight").and_then(|x| x.as_i64()) {
                                    Ok(font_weight) => match u16::try_from(font_weight) {
                                        Ok(ok) => attrs.weight = Weight(ok),
                                        Err(_) => {
                                            log::warn!("unknown weight {:?}", font_weight);
                                        }
                                    },
                                    Err(_err) => {}
                                }

                                match desc.get(b"Flags").and_then(|x| x.as_i64()) {
                                    Ok(flags) => {
                                        if flags & (1 << 0) != 0 {
                                            // FixedPitch
                                            //TODO: needs to use courier compatible font: attrs.family_owned = FamilyOwned::Monospace;
                                            attrs.family_owned =
                                                FamilyOwned::Name("Liberation Mono".into());
                                        } else if flags & (1 << 1) != 0 {
                                            // Serif
                                            //TODO: serif fallback is wrong, needs to use times new roman compatible font: attrs.family_owned = FamilyOwned::Serif;
                                            attrs.family_owned =
                                                FamilyOwned::Name("Liberation Serif".into());
                                        } else if flags & (1 << 3) != 0 {
                                            // Script
                                            attrs.family_owned = FamilyOwned::Cursive;
                                        } else {
                                            // Standard is sans-serif
                                            //TODO: needs to use helvetica compatible font: attrs.family_owned = FamilyOwned::SansSerif;
                                            attrs.family_owned =
                                                FamilyOwned::Name("Liberation Sans".into());
                                        }
                                        if flags & (1 << 6) != 0 {
                                            // Italic
                                            attrs.style = Style::Italic;
                                        }
                                    }
                                    Err(_err) => {}
                                }

                                match desc.get(b"FontFamily").and_then(as_name_str) {
                                    Ok(font_family) => {
                                        attrs.family_owned = FamilyOwned::Name(font_family.into());
                                    }
                                    Err(_err) => {}
                                }
                            }
                            Err(err) => {
                                log::error!(
                                    "failed to find font descriptor for font {name:?}: {err}"
                                );
                            }
                        }

                        match font_dict.get(b"BaseFont").and_then(as_name_str) {
                            Ok(base_font) => {
                                log::info!("BaseFont {:?}", base_font);

                                //TODO: get ID after inserting fonts?
                                let mut font_system =
                                    text::font_system().write().expect("Write font system");
                                let mut found = false;
                                for face in font_system.raw().db().faces() {
                                    if face.post_script_name == base_font {
                                        log::info!(
                                            "found font {name:?} by postscript name {base_font:?}"
                                        );

                                        attrs.family_owned =
                                            FamilyOwned::Name(face.families[0].0.clone().into());
                                        attrs.stretch = face.stretch;
                                        attrs.style = face.style;
                                        attrs.weight = face.weight;

                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    log::warn!(
                                        "failed to find font {name:?} by postscript name {base_font:?}"
                                    );
                                }
                            }
                            Err(err) => {
                                log::error!("failed to get BaseFont for font {name:?}: {err}");
                            }
                        }
                    }
                    None => {
                        log::error!("failed to find font {name:?}");
                    }
                }

                let gs = graphics_states.last_mut().unwrap();
                gs.text_encoding = encoding.map(Arc::new);
                gs.text_attrs = attrs;
                gs.text_size = size;
                log::info!(
                    "encoding {:?} attrs {:?} size {:?}",
                    gs.text_encoding,
                    gs.text_attrs,
                    gs.text_size
                );
            }
            "TL" => {
                let leading = op.operands[0].as_float().unwrap();
                log::info!("set text leading {leading}");
                let gs = graphics_states.last_mut().unwrap();
                gs.text_leading = leading;
            }
            "Ts" => {
                let rise = op.operands[0].as_float().unwrap();
                log::info!("set text rise {rise}");
                let gs = graphics_states.last_mut().unwrap();
                gs.text_rise = rise;
            }

            // Text positioning
            "T*" => {
                log::info!("move to start of next line");
                let gs = graphics_states.last_mut().unwrap();
                let ts = text_states.last_mut().unwrap();
                ts.set_tf(
                    ts.line_tf
                        .pre_translate(Vector2D::new(0.0, -gs.text_leading)),
                );
            }
            "Td" => {
                let x = op.operands[0].as_float().unwrap();
                let y = op.operands[1].as_float().unwrap();
                log::info!("move to start of next line {x}, {y}");
                let ts = text_states.last_mut().unwrap();
                ts.set_tf(ts.line_tf.pre_translate(Vector2D::new(x, y)));
            }
            "TD" => {
                let x = op.operands[0].as_float().unwrap();
                let y = op.operands[1].as_float().unwrap();
                log::info!("move to start of next line {x}, {y} and set leading");
                let gs = graphics_states.last_mut().unwrap();
                let ts = text_states.last_mut().unwrap();
                ts.set_tf(ts.line_tf.pre_translate(Vector2D::new(x, y)));
            }
            "Tm" => {
                let a = op.operands[0].as_float().unwrap();
                let b = op.operands[1].as_float().unwrap();
                let c = op.operands[2].as_float().unwrap();
                let d = op.operands[3].as_float().unwrap();
                let e = op.operands[4].as_float().unwrap();
                let f = op.operands[5].as_float().unwrap();
                let ts = text_states.last_mut().unwrap();
                ts.set_tf(Transform::new(a, b, c, d, e, f));
                log::info!("set text transform {:?}", ts.line_tf);
            }

            // Text showing
            "Tj" | "TJ" => {
                let has_adjustment = match op.operator.as_str() {
                    "Tj" => false,
                    "TJ" => true,
                    _ => panic!("uexpected text showing operator {}", op.operator),
                };
                log::info!(
                    "show text{} {:?}",
                    if has_adjustment {
                        " with adjustment"
                    } else {
                        ""
                    },
                    op.operands
                );
                //TODO: clean this up
                let elements = if has_adjustment {
                    op.operands[0].as_array().unwrap()
                } else {
                    &op.operands
                };
                let mut i = 0;
                while i < elements.len() {
                    let gs = graphics_states.last_mut().unwrap();
                    let ts = text_states.last_mut().unwrap();
                    let content = match gs.text_encoding.as_deref() {
                        Some(encoding) => {
                            Document::decode_text(encoding, elements[i].as_str().unwrap()).unwrap()
                        }
                        None => String::from_utf8_lossy(elements[i].as_str().unwrap()).to_string(),
                    };
                    i += 1;
                    let adjustment = if has_adjustment && i < elements.len() {
                        if let Ok(adjustment) = elements[i].as_float() {
                            i += 1;
                            adjustment
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    };
                    //TODO: fill or stroke?
                    let stroke = false;
                    let text = Text {
                        content: content.to_string(),
                        //TODO: is this y coordinate correct?
                        position: Point::new(0.0, -gs.text_rise - gs.text_size),
                        color: if stroke {
                            convert_color(&color_space_stroke, &color_stroke)
                        } else {
                            convert_color(&color_space_fill, &color_fill)
                        },
                        size: Pixels(gs.text_size),
                        line_height: LineHeight::Absolute(Pixels(gs.text_leading)),
                        attrs: gs.text_attrs.clone(),
                        horizontal_alignment: Horizontal::Left,
                        vertical_alignment: Vertical::Top,
                        shaping: Shaping::Advanced,
                    };
                    log::debug!("{:?}", text);
                    let max_w = text.draw_with(|mut path, color| {
                        path = path
                            .transform(&Transform::scale(1.0, -1.0))
                            .transform(&ts.cursor_tf);
                        page_ops.push(PageOp {
                            path: Some(path),
                            //TODO: more fill options
                            fill: if !stroke {
                                Some(canvas::Fill::from(color))
                            } else {
                                None
                            },
                            //TODO: more stroke options
                            stroke: if stroke {
                                Some(canvas::Stroke::default().with_color(color))
                            } else {
                                None
                            },
                            image: None,
                        });
                    });
                    ts.cursor_tf = ts
                        .cursor_tf
                        .pre_translate(Vector2D::new(max_w - adjustment / 1000.0, 0.0));
                }
            }

            // Graphics state
            "cm" => {
                let a = op.operands[0].as_float().unwrap();
                let b = op.operands[1].as_float().unwrap();
                let c = op.operands[2].as_float().unwrap();
                let d = op.operands[3].as_float().unwrap();
                let e = op.operands[4].as_float().unwrap();
                let f = op.operands[5].as_float().unwrap();
                let gs = graphics_states.last_mut().unwrap();
                gs.transform = Transform::new(a, b, c, d, e, f);
                log::info!("set graphics transform {:?}", gs.transform);
            }
            "j" => {
                let gs = graphics_states.last_mut().unwrap();
                gs.line_join_style = op.operands[0].as_i64().unwrap();
                log::info!("set line join style {}", gs.line_join_style);
            }
            "q" => {
                log::info!("save graphics state");
                let gs = graphics_states.last().cloned().unwrap_or_default();
                graphics_states.push(gs);
            }
            "Q" => {
                log::info!("restore graphics state");
                graphics_states.pop();
            }
            "w" => {
                let gs = graphics_states.last_mut().unwrap();
                gs.line_width = op.operands[0].as_float().unwrap();
                log::info!("set line width {}", gs.line_width);
            }

            // Color
            "cs" => {
                color_space_fill = as_name_str(&op.operands[0]).unwrap().to_string();
                log::info!("color space (fill) {color_space_fill}");
            }
            "CS" => {
                color_space_stroke = as_name_str(&op.operands[0]).unwrap().to_string();
                log::info!("color space (stroke) {color_space_stroke}");
            }
            "g" => {
                color_space_fill = "DeviceGray".to_string();
                color_fill = op.operands.clone();
                log::info!("color (fill) {color_fill:?}");
            }
            "G" => {
                color_space_stroke = "DeviceGray".to_string();
                color_stroke = op.operands.clone();
                log::info!("color (stroke) {color_stroke:?}");
            }
            "k" => {
                color_space_fill = "DeviceCMYK".to_string();
                color_fill = op.operands.clone();
                log::info!("color (fill) {color_fill:?}");
            }
            "K" => {
                color_space_stroke = "DeviceCMYK".to_string();
                color_stroke = op.operands.clone();
                log::info!("color (stroke) {color_stroke:?}");
            }
            "rg" => {
                color_space_fill = "DeviceRGB".to_string();
                color_fill = op.operands.clone();
                log::info!("color (fill) {color_fill:?}");
            }
            "RG" => {
                color_space_stroke = "DeviceRGB".to_string();
                color_stroke = op.operands.clone();
                log::info!("color (stroke) {color_stroke:?}");
            }
            "scn" => {
                color_fill = op.operands.clone();
                log::info!("color (fill) {color_fill:?}");
            }
            "SCN" => {
                color_stroke = op.operands.clone();
                log::info!("color (stroke) {color_stroke:?}");
            }

            // Object painting
            "Do" => {
                let name = as_name_str(&op.operands[0]).unwrap();
                log::info!("image {name:?}");

                match load_image(doc, page_id, name) {
                    Ok((handle, width, height)) => {
                        let gs = graphics_states.last().unwrap();
                        let a = gs.transform.transform_point(Point2D::new(0.0, 0.0));
                        let b = gs.transform.transform_point(Point2D::new(1.0, 1.0));
                        page_ops.push(PageOp {
                            path: None,
                            fill: None,
                            stroke: None,
                            image: Some(Image { name: name.to_string(), handle, rect:
                                //TODO: figure out corrrect rectangle
                                Rectangle::new(
                                    Point::new(a.x.min(b.x), a.y.max(b.y)),
                                    Size::new((a.x - b.x).abs(), (a.y - b.y).abs())
                                )
                             }),
                        });
                    }
                    Err(err) => {
                        log::warn!("failed to load image {:?}: {}", name, err);
                    }
                }
            }

            _ => {
                log::warn!("unknown op {:?}", op);
            }
        }
    }

    page_ops
}
