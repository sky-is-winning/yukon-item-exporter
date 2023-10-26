//! Object representation for `ShaderData`

use crate::avm2::activation::Activation;
use crate::avm2::object::script_object::ScriptObjectData;
use crate::avm2::object::{ClassObject, Object, ObjectPtr, TObject};
use crate::avm2::value::Value;
use crate::avm2::Error;
use core::fmt;
use gc_arena::barrier::unlock;
use gc_arena::lock::RefLock;
use gc_arena::{Collect, Gc, GcWeak, Mutation};
use ruffle_render::pixel_bender::PixelBenderShaderHandle;
use std::cell::{Cell, Ref, RefMut};

/// A class instance allocator that allocates ShaderData objects.
pub fn shader_data_allocator<'gc>(
    class: ClassObject<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Object<'gc>, Error<'gc>> {
    Ok(ShaderDataObject(Gc::new(
        activation.gc(),
        ShaderDataObjectData {
            base: RefLock::new(ScriptObjectData::new(class)),
            shader: Cell::new(None),
        },
    ))
    .into())
}

#[derive(Clone, Collect, Copy)]
#[collect(no_drop)]
pub struct ShaderDataObject<'gc>(pub Gc<'gc, ShaderDataObjectData<'gc>>);

#[derive(Clone, Collect, Copy, Debug)]
#[collect(no_drop)]
pub struct ShaderDataObjectWeak<'gc>(pub GcWeak<'gc, ShaderDataObjectData<'gc>>);

impl fmt::Debug for ShaderDataObject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShaderDataObject")
            .field("ptr", &Gc::as_ptr(self.0))
            .finish()
    }
}

impl<'gc> ShaderDataObject<'gc> {
    pub fn pixel_bender_shader(&self) -> Option<PixelBenderShaderHandle> {
        let shader = &self.0.shader;
        let guard = scopeguard::guard(shader.take(), |stolen| shader.set(stolen));
        guard.clone()
    }

    pub fn set_pixel_bender_shader(&self, shader: PixelBenderShaderHandle) {
        self.0.shader.set(Some(shader));
    }
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct ShaderDataObjectData<'gc> {
    /// Base script object
    base: RefLock<ScriptObjectData<'gc>>,

    shader: Cell<Option<PixelBenderShaderHandle>>,
}

impl<'gc> TObject<'gc> for ShaderDataObject<'gc> {
    fn base(&self) -> Ref<ScriptObjectData<'gc>> {
        self.0.base.borrow()
    }

    fn base_mut(&self, mc: &Mutation<'gc>) -> RefMut<ScriptObjectData<'gc>> {
        unlock!(Gc::write(mc, self.0), ShaderDataObjectData, base).borrow_mut()
    }

    fn as_ptr(&self) -> *const ObjectPtr {
        Gc::as_ptr(self.0) as *const ObjectPtr
    }

    fn value_of(&self, _mc: &Mutation<'gc>) -> Result<Value<'gc>, Error<'gc>> {
        Ok(Value::Object(Object::from(*self)))
    }

    fn as_shader_data(&self) -> Option<ShaderDataObject<'gc>> {
        Some(*self)
    }
}
