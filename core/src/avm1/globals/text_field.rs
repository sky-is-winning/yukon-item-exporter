use crate::avm1::activation::Activation;
use crate::avm1::error::Error;
use crate::avm1::globals::bitmap_filter;
use crate::avm1::object::NativeObject;
use crate::avm1::property_decl::{define_properties_on, Declaration};
use crate::avm1::{globals, ArrayObject, Object, ScriptObject, TObject, Value};
use crate::context::GcContext;
use crate::display_object::{AutoSizeMode, EditText, TDisplayObject, TextSelection};
use crate::font::round_down_to_pixel;
use crate::html::TextFormat;
use crate::string::{AvmString, WStr};
use gc_arena::Gc;
use swf::Color;

macro_rules! tf_method {
    ($fn:expr) => {
        |activation, this, args| {
            if let Some(display_object) = this.as_display_object() {
                if let Some(text_field) = display_object.as_edit_text() {
                    return $fn(text_field, activation, args);
                }
            }
            Ok(Value::Undefined)
        }
    };
}

macro_rules! tf_getter {
    ($get:expr) => {
        |activation, this, _args| {
            if let Some(display_object) = this.as_display_object() {
                if let Some(edit_text) = display_object.as_edit_text() {
                    return $get(edit_text, activation);
                }
            }
            Ok(Value::Undefined)
        }
    };
}

macro_rules! tf_setter {
    ($set:expr) => {
        |activation, this, args| {
            if let Some(display_object) = this.as_display_object() {
                if let Some(edit_text) = display_object.as_edit_text() {
                    let value = args.get(0).unwrap_or(&Value::Undefined).clone();
                    $set(edit_text, activation, value)?;
                }
            }
            Ok(Value::Undefined)
        }
    };
}

const PROTO_DECLS: &[Declaration] = declare_properties! {
    "getNewTextFormat" => method(tf_method!(get_new_text_format); DONT_ENUM | DONT_DELETE);
    "setNewTextFormat" => method(tf_method!(set_new_text_format); DONT_ENUM | DONT_DELETE);
    "getTextFormat" => method(tf_method!(get_text_format); DONT_ENUM | DONT_DELETE);
    "setTextFormat" => method(tf_method!(set_text_format); DONT_ENUM | DONT_DELETE);
    "replaceSel" => method(tf_method!(replace_sel); DONT_ENUM | DONT_DELETE);
    "replaceText" => method(tf_method!(replace_text); DONT_ENUM | DONT_DELETE);
    "removeTextField" => method(tf_method!(remove_text_field); DONT_ENUM | DONT_DELETE);
    "autoSize" => property(tf_getter!(auto_size), tf_setter!(set_auto_size));
    "background" => property(tf_getter!(background), tf_setter!(set_background));
    "backgroundColor" => property(tf_getter!(background_color), tf_setter!(set_background_color));
    "border" => property(tf_getter!(border), tf_setter!(set_border));
    "borderColor" => property(tf_getter!(border_color), tf_setter!(set_border_color));
    "bottomScroll" => property(tf_getter!(bottom_scroll));
    "embedFonts" => property(tf_getter!(embed_fonts), tf_setter!(set_embed_fonts));
    "filters" => property(tf_getter!(filters), tf_setter!(set_filters); DONT_DELETE | DONT_ENUM | VERSION_8);
    "getDepth" => method(globals::get_depth; DONT_ENUM | DONT_DELETE | READ_ONLY | VERSION_6);
    "hscroll" => property(tf_getter!(hscroll), tf_setter!(set_hscroll));
    "html" => property(tf_getter!(html), tf_setter!(set_html));
    "htmlText" => property(tf_getter!(html_text), tf_setter!(set_html_text));
    "length" => property(tf_getter!(length));
    "maxhscroll" => property(tf_getter!(maxhscroll));
    "maxscroll" => property(tf_getter!(maxscroll));
    "multiline" => property(tf_getter!(multiline), tf_setter!(set_multiline));
    "password" => property(tf_getter!(password), tf_setter!(set_password));
    "scroll" => property(tf_getter!(scroll), tf_setter!(set_scroll));
    "selectable" => property(tf_getter!(selectable), tf_setter!(set_selectable));
    "text" => property(tf_getter!(text), tf_setter!(set_text));
    "textColor" => property(tf_getter!(text_color), tf_setter!(set_text_color));
    "textHeight" => property(tf_getter!(text_height));
    "textWidth" => property(tf_getter!(text_width));
    "type" => property(tf_getter!(get_type), tf_setter!(set_type));
    "variable" => property(tf_getter!(variable), tf_setter!(set_variable));
    "wordWrap" => property(tf_getter!(word_wrap), tf_setter!(set_word_wrap));
    "antiAliasType" => property(tf_getter!(anti_alias_type), tf_setter!(set_anti_alias_type));
    "gridFitType" => property(tf_getter!(grid_fit_type), tf_setter!(set_grid_fit_type));
    "sharpness" => property(tf_getter!(sharpness), tf_setter!(set_sharpness));
    "thickness" => property(tf_getter!(thickness), tf_setter!(set_thickness));
};

/// Implements `TextField`
pub fn constructor<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.into())
}

pub fn create_proto<'gc>(
    context: &mut GcContext<'_, 'gc>,
    proto: Object<'gc>,
    fn_proto: Object<'gc>,
) -> Object<'gc> {
    let object = ScriptObject::new(context.gc_context, Some(proto));
    define_properties_on(PROTO_DECLS, context, object, fn_proto);
    object.into()
}
pub fn password<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.is_password().into())
}

pub fn set_password<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    this.set_password(
        value.as_bool(activation.swf_version()),
        &mut activation.context,
    );
    Ok(())
}

fn new_text_format<'gc>(
    activation: &mut Activation<'_, 'gc>,
    text_format: TextFormat,
) -> ScriptObject<'gc> {
    let proto = activation.context.avm1.prototypes().text_format;
    let object = ScriptObject::new(activation.context.gc_context, Some(proto));
    object.set_native(
        activation.context.gc_context,
        NativeObject::TextFormat(Gc::new(activation.context.gc_context, text_format.into())),
    );
    object
}

fn get_new_text_format<'gc>(
    text_field: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let text_format = text_field.new_text_format();
    Ok(new_text_format(activation, text_format).into())
}

fn set_new_text_format<'gc>(
    text_field: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    if let [Value::Object(text_format), ..] = args {
        if let NativeObject::TextFormat(text_format) = text_format.native() {
            text_field.set_new_text_format(text_format.borrow().clone(), &mut activation.context);
        }
    }

    Ok(Value::Undefined)
}

fn get_text_format<'gc>(
    text_field: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let (begin_index, end_index) = match args {
        [begin_index, end_index, ..] => {
            let begin_index = begin_index.coerce_to_u32(activation)? as usize;
            let end_index = end_index.coerce_to_u32(activation)? as usize;
            (begin_index, end_index)
        }
        [begin_index] => {
            let begin_index = begin_index.coerce_to_u32(activation)? as usize;
            let end_index = begin_index + 1;
            (begin_index, end_index)
        }
        [] => {
            let begin_index = 0;
            let end_index = text_field.text_length();
            (begin_index, end_index)
        }
    };

    let text_format = text_field.text_format(begin_index, end_index);
    Ok(new_text_format(activation, text_format).into())
}

fn set_text_format<'gc>(
    text_field: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let (begin_index, end_index, text_format) = match args {
        [begin_index, end_index, text_format, ..] => {
            let begin_index = begin_index.coerce_to_u32(activation)? as usize;
            let end_index = end_index.coerce_to_u32(activation)? as usize;
            (begin_index, end_index, text_format)
        }
        [begin_index, text_format, ..] => {
            let begin_index = begin_index.coerce_to_u32(activation)? as usize;
            let end_index = begin_index + 1;
            (begin_index, end_index, text_format)
        }
        [text_format] => {
            let begin_index = 0;
            let end_index = text_field.text_length();
            (begin_index, end_index, text_format)
        }
        _ => return Ok(Value::Undefined),
    };

    if let Value::Object(text_format) = text_format {
        if let NativeObject::TextFormat(text_format) = text_format.native() {
            text_field.set_text_format(
                begin_index,
                end_index,
                text_format.borrow().clone(),
                &mut activation.context,
            );
        }
    }

    Ok(Value::Undefined)
}

fn replace_sel<'gc>(
    text_field: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let text = args
        .get(0)
        .unwrap_or(&Value::Undefined)
        .coerce_to_string(activation)?;

    // TODO: The AS2 reference says that this only works if you first call TextField.focus(),
    // but that doesn't seem to be the case; seems to default to inserting at the start of the text.
    // Verify the exact behavior.
    let selection = text_field
        .selection()
        .unwrap_or_else(|| TextSelection::for_position(0));
    text_field.replace_text(
        selection.start(),
        selection.end(),
        &text,
        &mut activation.context,
    );
    text_field.set_selection(
        Some(TextSelection::for_position(selection.start() + text.len())),
        activation.context.gc_context,
    );

    text_field.propagate_text_binding(activation);

    Ok(Value::Undefined)
}

fn replace_text<'gc>(
    text_field: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let from = args
        .get(0)
        .cloned()
        .unwrap_or(Value::Undefined)
        .coerce_to_f64(activation)?;
    let to = args
        .get(1)
        .cloned()
        .unwrap_or(Value::Undefined)
        .coerce_to_f64(activation)?;
    let text = args
        .get(2)
        .cloned()
        .unwrap_or(Value::Undefined)
        .coerce_to_string(activation)?;

    text_field.replace_text(from as usize, to as usize, &text, &mut activation.context);

    Ok(Value::Undefined)
}

pub fn remove_text_field<'gc>(
    text_field: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    globals::remove_display_object(text_field.into(), activation);
    Ok(Value::Undefined)
}

pub fn text<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(AvmString::new(activation.context.gc_context, this.text()).into())
}

pub fn set_text<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    this.set_text(
        &value.coerce_to_string(activation)?,
        &mut activation.context,
    );
    this.propagate_text_binding(activation);

    Ok(())
}

pub fn html<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.is_html().into())
}

pub fn set_html<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let value = value.as_bool(activation.swf_version());
    this.set_is_html(&mut activation.context, value);
    Ok(())
}

pub fn text_color<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    if let Some(color) = this.new_text_format().color {
        return Ok(color.to_rgb().into());
    }
    Ok(Value::Undefined)
}

pub fn set_text_color<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let rgb = value.coerce_to_u32(activation)?;
    let text_format = TextFormat {
        color: Some(swf::Color::from_rgb(rgb, 0)),
        ..Default::default()
    };
    this.set_text_format(
        0,
        this.text_length(),
        text_format.clone(),
        &mut activation.context,
    );
    this.set_new_text_format(text_format, &mut activation.context);
    Ok(())
}

pub fn html_text<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(AvmString::new(activation.context.gc_context, this.html_text()).into())
}

pub fn set_html_text<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let text = value.coerce_to_string(activation)?;
    this.set_html_text(&text, &mut activation.context);
    // Changing the htmlText does NOT update variable bindings (does not call EditText::propagate_text_binding).
    Ok(())
}

pub fn background<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.has_background().into())
}

pub fn set_background<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let has_background = value.as_bool(activation.swf_version());
    this.set_has_background(activation.context.gc_context, has_background);
    Ok(())
}

pub fn background_color<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.background_color().to_rgb().into())
}

pub fn set_background_color<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let rgb = value.coerce_to_u32(activation)?;
    let color = Color::from_rgb(rgb, 255);
    this.set_background_color(activation.context.gc_context, color);
    Ok(())
}

pub fn border<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.has_border().into())
}

pub fn set_border<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let has_border = value.as_bool(activation.swf_version());
    this.set_has_border(activation.context.gc_context, has_border);
    Ok(())
}

pub fn border_color<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.border_color().to_rgb().into())
}

pub fn set_border_color<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let rgb = value.coerce_to_u32(activation)?;
    let color = Color::from_rgb(rgb, 255);
    this.set_border_color(activation.context.gc_context, color);
    Ok(())
}

pub fn embed_fonts<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok((!this.is_device_font()).into())
}

pub fn set_embed_fonts<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let embed_fonts = value.as_bool(activation.swf_version());
    this.set_is_device_font(&mut activation.context, !embed_fonts);
    Ok(())
}

pub fn length<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok((this.text_length() as f64).into())
}

pub fn text_width<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    let metrics = this.measure_text(&mut activation.context);
    Ok(round_down_to_pixel(metrics.0).to_pixels().into())
}

pub fn text_height<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    let metrics = this.measure_text(&mut activation.context);
    Ok(round_down_to_pixel(metrics.1).to_pixels().into())
}

pub fn multiline<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.is_multiline().into())
}

pub fn set_multiline<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let is_multiline = value.as_bool(activation.swf_version());
    this.set_multiline(is_multiline, &mut activation.context);
    Ok(())
}

pub fn selectable<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.is_selectable().into())
}

pub fn set_selectable<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let set_selectable = value.as_bool(activation.swf_version());
    this.set_selectable(set_selectable, &mut activation.context);
    Ok(())
}

fn variable<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    if let Some(variable) = this.variable() {
        return Ok(AvmString::new_utf8(activation.context.gc_context, &variable[..]).into());
    }

    // Unset `variable` returns null, not undefined
    Ok(Value::Null)
}

fn set_variable<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let variable = match value {
        Value::Undefined | Value::Null => None,
        v => Some(v.coerce_to_string(activation)?),
    };
    this.set_variable(variable.map(|v| v.to_string()), activation);
    Ok(())
}

pub fn word_wrap<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.is_word_wrap().into())
}

pub fn set_word_wrap<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let is_word_wrap = value.as_bool(activation.swf_version());
    this.set_word_wrap(is_word_wrap, &mut activation.context);
    Ok(())
}

pub fn auto_size<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(match this.autosize() {
        AutoSizeMode::None => "none".into(),
        AutoSizeMode::Left => "left".into(),
        AutoSizeMode::Center => "center".into(),
        AutoSizeMode::Right => "right".into(),
    })
}

pub fn set_auto_size<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let mode = match value {
        Value::String(s) if s.eq_ignore_case(WStr::from_units(b"left")) => AutoSizeMode::Left,
        Value::String(s) if s.eq_ignore_case(WStr::from_units(b"center")) => AutoSizeMode::Center,
        Value::String(s) if s.eq_ignore_case(WStr::from_units(b"right")) => AutoSizeMode::Right,
        Value::Bool(true) => AutoSizeMode::Left,
        _ => AutoSizeMode::None,
    };
    this.set_autosize(mode, &mut activation.context);

    Ok(())
}

pub fn get_type<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    let tf_type = match this.is_editable() {
        true => "input",
        false => "dynamic",
    };
    Ok(tf_type.into())
}

pub fn set_type<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let value = value.coerce_to_string(activation)?;

    if value.eq_ignore_case(WStr::from_units(b"input")) {
        this.set_editable(true, &mut activation.context);
    } else if value.eq_ignore_case(WStr::from_units(b"dynamic")) {
        this.set_editable(false, &mut activation.context)
    } else {
        tracing::warn!("Invalid TextField.type: {}", value);
    }

    Ok(())
}

pub fn hscroll<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.hscroll().into())
}

pub fn set_hscroll<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    // SWF v8 and earlier has the simple clamping behaviour below. SWF v9+ is much more complicated. See #4634.
    let hscroll_pixels = value.coerce_to_i32(activation)? as f64;
    let clamped = hscroll_pixels.clamp(0.0, this.maxhscroll());
    this.set_hscroll(clamped, &mut activation.context);
    Ok(())
}

pub fn maxhscroll<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.maxhscroll().into())
}

pub fn scroll<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.scroll().into())
}

pub fn set_scroll<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let input = value.coerce_to_f64(activation)?;
    this.set_scroll(input, &mut activation.context);
    Ok(())
}

pub fn maxscroll<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.maxscroll().into())
}

pub fn bottom_scroll<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.bottom_scroll().into())
}

pub fn anti_alias_type<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    if this.render_settings().is_advanced() {
        Ok("advanced".into())
    } else {
        Ok("normal".into())
    }
}

pub fn set_anti_alias_type<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let old_settings = this.render_settings();
    let new_type = value.coerce_to_string(activation)?;

    if &new_type == b"advanced" {
        this.set_render_settings(
            activation.context.gc_context,
            old_settings.with_advanced_rendering(),
        );
    } else if &new_type == b"normal" {
        this.set_render_settings(
            activation.context.gc_context,
            old_settings.with_normal_rendering(),
        );
    }

    Ok(())
}

pub fn grid_fit_type<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    match this.render_settings().grid_fit() {
        swf::TextGridFit::None => Ok("none".into()),
        swf::TextGridFit::Pixel => Ok("pixel".into()),
        swf::TextGridFit::SubPixel => Ok("subpixel".into()),
    }
}

pub fn set_grid_fit_type<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let old_settings = this.render_settings();
    let new_type = value.coerce_to_string(activation)?;

    if &new_type == b"pixel" {
        this.set_render_settings(
            activation.context.gc_context,
            old_settings.with_grid_fit(swf::TextGridFit::Pixel),
        );
    } else if &new_type == b"subpixel" {
        this.set_render_settings(
            activation.context.gc_context,
            old_settings.with_grid_fit(swf::TextGridFit::SubPixel),
        );
    } else if &new_type == b"none" {
        this.set_render_settings(
            activation.context.gc_context,
            old_settings.with_grid_fit(swf::TextGridFit::None),
        );
    } // NOTE: In AS2 invalid values do nothing.

    Ok(())
}

pub fn thickness<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.render_settings().thickness().into())
}

pub fn set_thickness<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let old_settings = this.render_settings();
    let new_thickness = value.coerce_to_f64(activation)?;

    this.set_render_settings(
        activation.context.gc_context,
        old_settings.with_thickness(new_thickness as f32),
    );

    Ok(())
}

pub fn sharpness<'gc>(
    this: EditText<'gc>,
    _activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(this.render_settings().sharpness().into())
}

pub fn set_sharpness<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let old_settings = this.render_settings();
    let new_sharpness = value.coerce_to_f64(activation)?;

    this.set_render_settings(
        activation.context.gc_context,
        old_settings.with_sharpness(new_sharpness as f32),
    );

    Ok(())
}

fn filters<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(ArrayObject::new(
        activation.context.gc_context,
        activation.context.avm1.prototypes().array,
        this.filters()
            .into_iter()
            .map(|filter| bitmap_filter::filter_to_avm1(activation, filter)),
    )
    .into())
}

fn set_filters<'gc>(
    this: EditText<'gc>,
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<(), Error<'gc>> {
    let mut filters = vec![];
    if let Value::Object(value) = value {
        for index in value.get_keys(activation, false).into_iter().rev() {
            let filter_object = value.get(index, activation)?.coerce_to_object(activation);
            if let Some(filter) =
                bitmap_filter::avm1_to_filter(filter_object, &mut activation.context)
            {
                filters.push(filter);
            }
        }
    }
    this.set_filters(activation.context.gc_context, filters);
    Ok(())
}
