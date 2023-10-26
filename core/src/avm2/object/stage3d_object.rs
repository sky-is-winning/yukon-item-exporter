//! Object representation for Stage3D objects

use crate::avm2::activation::Activation;
use crate::avm2::object::script_object::ScriptObjectData;
use crate::avm2::object::{ClassObject, Object, ObjectPtr, TObject};
use crate::avm2::value::Value;
use crate::avm2::Error;
use core::fmt;
use gc_arena::barrier::unlock;
use gc_arena::lock::{Lock, RefLock};
use gc_arena::{Collect, Gc, GcWeak, Mutation};
use std::cell::{Cell, Ref, RefMut};

/// A class instance allocator that allocates Stage3D objects.
pub fn stage_3d_allocator<'gc>(
    class: ClassObject<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Object<'gc>, Error<'gc>> {
    Ok(Stage3DObject(Gc::new(
        activation.gc(),
        Stage3DObjectData {
            base: RefLock::new(ScriptObjectData::new(class)),
            context3d: Lock::new(None),
            visible: Cell::new(true),
        },
    ))
    .into())
}

#[derive(Clone, Collect, Copy)]
#[collect(no_drop)]
pub struct Stage3DObject<'gc>(pub Gc<'gc, Stage3DObjectData<'gc>>);

#[derive(Clone, Collect, Copy, Debug)]
#[collect(no_drop)]
pub struct Stage3DObjectWeak<'gc>(pub GcWeak<'gc, Stage3DObjectData<'gc>>);

impl fmt::Debug for Stage3DObject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stage3DObject")
            .field("ptr", &Gc::as_ptr(self.0))
            .finish()
    }
}

impl<'gc> Stage3DObject<'gc> {
    pub fn context3d(self) -> Option<Object<'gc>> {
        self.0.context3d.get()
    }

    pub fn set_context3d(self, context3d: Object<'gc>, mc: &Mutation<'gc>) {
        unlock!(Gc::write(mc, self.0), Stage3DObjectData, context3d).set(Some(context3d))
    }

    pub fn visible(self) -> bool {
        self.0.visible.get()
    }

    pub fn set_visible(self, visible: bool) {
        self.0.visible.set(visible);
    }
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct Stage3DObjectData<'gc> {
    /// Base script object
    base: RefLock<ScriptObjectData<'gc>>,

    /// The context3D object associated with this Stage3D object,
    /// if it's been created with `requestContext3D`
    context3d: Lock<Option<Object<'gc>>>,
    visible: Cell<bool>,
}

impl<'gc> TObject<'gc> for Stage3DObject<'gc> {
    fn base(&self) -> Ref<ScriptObjectData<'gc>> {
        self.0.base.borrow()
    }

    fn base_mut(&self, mc: &Mutation<'gc>) -> RefMut<ScriptObjectData<'gc>> {
        unlock!(Gc::write(mc, self.0), Stage3DObjectData, base).borrow_mut()
    }

    fn as_ptr(&self) -> *const ObjectPtr {
        Gc::as_ptr(self.0) as *const ObjectPtr
    }

    fn value_of(&self, _mc: &Mutation<'gc>) -> Result<Value<'gc>, Error<'gc>> {
        Ok(Value::Object(Object::from(*self)))
    }

    fn as_stage_3d(&self) -> Option<Stage3DObject<'gc>> {
        Some(*self)
    }
}
