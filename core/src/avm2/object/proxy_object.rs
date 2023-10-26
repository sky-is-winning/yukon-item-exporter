//! Object representation for `Proxy`.

use crate::avm2::activation::Activation;
use crate::avm2::object::script_object::ScriptObjectData;
use crate::avm2::object::{ClassObject, Object, ObjectPtr, QNameObject, TObject};
use crate::avm2::value::Value;
use crate::avm2::Error;
use crate::avm2::Multiname;
use core::fmt;
use gc_arena::{Collect, GcCell, GcWeakCell, Mutation};
use std::cell::{Ref, RefMut};

/// A class instance allocator that allocates Proxy objects.
pub fn proxy_allocator<'gc>(
    class: ClassObject<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Object<'gc>, Error<'gc>> {
    let base = ScriptObjectData::new(class);

    Ok(ProxyObject(GcCell::new(
        activation.context.gc_context,
        ProxyObjectData { base },
    ))
    .into())
}

#[derive(Clone, Collect, Copy)]
#[collect(no_drop)]
pub struct ProxyObject<'gc>(pub GcCell<'gc, ProxyObjectData<'gc>>);

#[derive(Clone, Collect, Copy, Debug)]
#[collect(no_drop)]
pub struct ProxyObjectWeak<'gc>(pub GcWeakCell<'gc, ProxyObjectData<'gc>>);

impl fmt::Debug for ProxyObject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyObject")
            .field("ptr", &self.0.as_ptr())
            .finish()
    }
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct ProxyObjectData<'gc> {
    /// Base script object
    base: ScriptObjectData<'gc>,
}

impl<'gc> TObject<'gc> for ProxyObject<'gc> {
    fn base(&self) -> Ref<ScriptObjectData<'gc>> {
        Ref::map(self.0.read(), |read| &read.base)
    }

    fn base_mut(&self, mc: &Mutation<'gc>) -> RefMut<ScriptObjectData<'gc>> {
        RefMut::map(self.0.write(mc), |write| &mut write.base)
    }

    fn as_ptr(&self) -> *const ObjectPtr {
        self.0.as_ptr() as *const ObjectPtr
    }

    fn value_of(&self, _mc: &Mutation<'gc>) -> Result<Value<'gc>, Error<'gc>> {
        Ok(Object::from(*self).into())
    }

    fn get_property_local(
        self,
        multiname: &Multiname<'gc>,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        let qname = QNameObject::from_name(activation, multiname.clone())?;

        self.call_property(
            &Multiname::new(activation.avm2().proxy_namespace, "getProperty"),
            &[qname.into()],
            activation,
        )
    }

    fn set_property_local(
        self,
        multiname: &Multiname<'gc>,
        value: Value<'gc>,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<(), Error<'gc>> {
        let qname = QNameObject::from_name(activation, multiname.clone())?;

        self.call_property(
            &Multiname::new(activation.avm2().proxy_namespace, "setProperty"),
            &[qname.into(), value],
            activation,
        )?;

        Ok(())
    }

    fn call_property_local(
        self,
        multiname: &Multiname<'gc>,
        arguments: &[Value<'gc>],
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        let qname = QNameObject::from_name(activation, multiname.clone())?;
        let mut args = vec![qname.into()];
        args.extend_from_slice(arguments);

        self.call_property(
            &Multiname::new(activation.avm2().proxy_namespace, "callProperty"),
            &args,
            activation,
        )
    }

    fn delete_property_local(
        self,
        activation: &mut Activation<'_, 'gc>,
        multiname: &Multiname<'gc>,
    ) -> Result<bool, Error<'gc>> {
        let qname = QNameObject::from_name(activation, multiname.clone())?;

        Ok(self
            .call_property(
                &Multiname::new(activation.avm2().proxy_namespace, "deleteProperty"),
                &[qname.into()],
                activation,
            )?
            .coerce_to_boolean())
    }

    fn has_property_via_in(
        self,
        activation: &mut Activation<'_, 'gc>,
        name: &Multiname<'gc>,
    ) -> Result<bool, Error<'gc>> {
        Ok(self
            .call_property(
                &Multiname::new(activation.avm2().proxy_namespace, "hasProperty"),
                &[name
                    .local_name()
                    .map(Value::from)
                    .unwrap_or_else(|| "*".into())],
                activation,
            )?
            .coerce_to_boolean())
    }

    fn get_next_enumerant(
        self,
        last_index: u32,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Option<u32>, Error<'gc>> {
        Ok(Some(
            self.call_property(
                &Multiname::new(activation.avm2().proxy_namespace, "nextNameIndex"),
                &[last_index.into()],
                activation,
            )?
            .coerce_to_u32(activation)?,
        ))
    }

    fn get_enumerant_name(
        self,
        index: u32,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        self.call_property(
            &Multiname::new(activation.avm2().proxy_namespace, "nextName"),
            &[index.into()],
            activation,
        )
    }

    fn get_enumerant_value(
        self,
        index: u32,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        self.call_property(
            &Multiname::new(activation.avm2().proxy_namespace, "nextValue"),
            &[index.into()],
            activation,
        )
    }
}
