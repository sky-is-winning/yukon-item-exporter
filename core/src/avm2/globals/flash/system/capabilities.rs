//! `flash.display.Capabilities` native methods

use crate::avm2::{Activation, AvmString, Error, Object, Value};

/// Implements `flash.system.Capabilities.version`
pub fn get_version<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    // TODO: Report the correct OS instead of always reporting Linux
    Ok(AvmString::new_utf8(
        activation.context.gc_context,
        format!("LNX {},0,0,0", activation.avm2().player_version),
    )
    .into())
}

/// Implements `flash.system.Capabilities.playerType`
pub fn get_player_type<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    // TODO: When should "External" be returned?
    let player_type = if cfg!(target_family = "wasm") {
        "PlugIn"
    } else {
        "StandAlone"
    };

    Ok(AvmString::new_utf8(activation.context.gc_context, player_type).into())
}

/// Implements `flash.system.Capabilities.screenResolutionX`
pub fn get_screen_resolution_x<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let viewport_dimensions = activation.context.renderer.viewport_dimensions();
    // Viewport size is adjusted for HiDPI.
    let adjusted_width = f64::from(viewport_dimensions.width) / viewport_dimensions.scale_factor;
    Ok(adjusted_width.round().into())
}

/// Implements `flash.system.Capabilities.screenResolutionY`
pub fn get_screen_resolution_y<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let viewport_dimensions = activation.context.renderer.viewport_dimensions();
    // Viewport size is adjusted for HiDPI.
    let adjusted_height = f64::from(viewport_dimensions.height) / viewport_dimensions.scale_factor;
    Ok(adjusted_height.round().into())
}

/// Implements `flash.system.Capabilities.pixelAspectRatio`
pub fn get_pixel_aspect_ratio<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    // source: https://web.archive.org/web/20230611050355/https://flylib.com/books/en/4.13.1.272/1/
    Ok(1.into())
}

/// Implements `flash.system.Capabilities.screenDPI`
pub fn get_screen_dpi<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    // source: https://tracker.adobe.com/#/view/FP-3949775
    Ok(72.into())
}
