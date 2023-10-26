//! `Class` builtin/prototype

use crate::avm2::activation::Activation;
use crate::avm2::class::Class;
use crate::avm2::method::{Method, NativeMethodImpl};
use crate::avm2::object::{Object, TObject};
use crate::avm2::value::Value;
use crate::avm2::Error;
use crate::avm2::Multiname;
use crate::avm2::QName;
use gc_arena::GcCell;

/// Implements `Class`'s instance initializer.
///
/// Notably, you cannot construct new classes this way, so this returns an
/// error.
pub fn instance_init<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    Err("Classes cannot be constructed.".into())
}

/// Implement's `Class`'s class initializer.
pub fn class_init<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(Value::Undefined)
}

fn prototype<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    if let Some(class) = this.as_class_object() {
        return Ok(class.prototype().into());
    }

    Ok(Value::Undefined)
}

/// Construct `Class`'s class.
pub fn create_class<'gc>(activation: &mut Activation<'_, 'gc>) -> GcCell<'gc, Class<'gc>> {
    let gc_context = activation.context.gc_context;
    let class_class = Class::new(
        QName::new(activation.avm2().public_namespace, "Class"),
        Some(Multiname::new(activation.avm2().public_namespace, "Object")),
        Method::from_builtin(instance_init, "<Class instance initializer>", gc_context),
        Method::from_builtin(class_init, "<Class class initializer>", gc_context),
        gc_context,
    );

    let mut write = class_class.write(gc_context);

    // 'length' is a weird undocumented constant in Class.
    // We need to define it, since it shows up in 'describeType'
    const CLASS_CONSTANTS: &[(&str, i32)] = &[("length", 1)];
    write.define_constant_int_class_traits(
        activation.avm2().public_namespace,
        CLASS_CONSTANTS,
        activation,
    );

    const PUBLIC_INSTANCE_PROPERTIES: &[(
        &str,
        Option<NativeMethodImpl>,
        Option<NativeMethodImpl>,
    )] = &[("prototype", Some(prototype), None)];
    write.define_builtin_instance_properties(
        gc_context,
        activation.avm2().public_namespace,
        PUBLIC_INSTANCE_PROPERTIES,
    );

    class_class
}
