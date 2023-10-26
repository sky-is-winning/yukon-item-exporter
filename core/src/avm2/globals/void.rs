// note: this should be a ClassObject-less class,
// with only the instance side.

use crate::avm2::activation::Activation;
use crate::avm2::class::Class;
use crate::avm2::method::Method;
use crate::avm2::object::Object;
use crate::avm2::value::Value;
use crate::avm2::Error;
use crate::avm2::QName;
use gc_arena::GcCell;

fn void_init<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    // note: after class refactor, this method should be unreachable!()
    // or not exist at all.
    Ok(Value::Undefined)
}

pub fn create_class<'gc>(activation: &mut Activation<'_, 'gc>) -> GcCell<'gc, Class<'gc>> {
    let mc = activation.context.gc_context;
    let class = Class::new(
        QName::new(activation.avm2().public_namespace, "void"),
        None,
        Method::from_builtin(void_init, "", mc),
        Method::from_builtin(void_init, "", mc),
        mc,
    );

    class
}
