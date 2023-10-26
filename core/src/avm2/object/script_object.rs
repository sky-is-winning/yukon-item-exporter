//! Default AVM2 object impl

use crate::avm2::activation::Activation;
use crate::avm2::error;
use crate::avm2::object::{ClassObject, FunctionObject, Object, ObjectPtr, TObject};
use crate::avm2::value::Value;
use crate::avm2::vtable::VTable;
use crate::avm2::Multiname;
use crate::avm2::{Error, QName};
use crate::string::AvmString;
use fnv::FnvHashMap;
use gc_arena::{Collect, GcCell, GcWeakCell, Mutation};
use std::cell::{Ref, RefMut};
use std::collections::hash_map::Entry;
use std::fmt::Debug;

/// A class instance allocator that allocates `ScriptObject`s.
pub fn scriptobject_allocator<'gc>(
    class: ClassObject<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Object<'gc>, Error<'gc>> {
    let base = ScriptObjectData::new(class);

    Ok(ScriptObject(GcCell::new(activation.context.gc_context, base)).into())
}

/// Default implementation of `avm2::Object`.
#[derive(Clone, Collect, Copy)]
#[collect(no_drop)]
pub struct ScriptObject<'gc>(pub GcCell<'gc, ScriptObjectData<'gc>>);

#[derive(Clone, Collect, Copy, Debug)]
#[collect(no_drop)]
pub struct ScriptObjectWeak<'gc>(pub GcWeakCell<'gc, ScriptObjectData<'gc>>);

/// Base data common to all `TObject` implementations.
///
/// Host implementations of `TObject` should embed `ScriptObjectData` and
/// forward any trait method implementations it does not overwrite to this
/// struct.
#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct ScriptObjectData<'gc> {
    /// Values stored on this object.
    values: FnvHashMap<AvmString<'gc>, Value<'gc>>,

    /// Slots stored on this object.
    slots: Vec<Value<'gc>>,

    /// Methods stored on this object.
    bound_methods: Vec<Option<FunctionObject<'gc>>>,

    /// Implicit prototype of this script object.
    proto: Option<Object<'gc>>,

    /// The class object that this is an instance of.
    /// If `none`, this is not an ES4 object at all.
    instance_of: Option<ClassObject<'gc>>,

    /// The table used for non-dynamic property lookups.
    vtable: Option<VTable<'gc>>,

    /// Enumeratable property names.
    enumerants: Vec<AvmString<'gc>>,
}

impl<'gc> TObject<'gc> for ScriptObject<'gc> {
    fn base(&self) -> Ref<ScriptObjectData<'gc>> {
        self.0.read()
    }

    fn base_mut(&self, mc: &Mutation<'gc>) -> RefMut<ScriptObjectData<'gc>> {
        self.0.write(mc)
    }

    fn as_ptr(&self) -> *const ObjectPtr {
        self.0.as_ptr() as *const ObjectPtr
    }

    fn value_of(&self, _mc: &Mutation<'gc>) -> Result<Value<'gc>, Error<'gc>> {
        Ok(Value::Object(Object::from(*self)))
    }
}

impl<'gc> ScriptObject<'gc> {
    /// Construct an instance with a possibly-none class and proto chain.
    /// NOTE: this is a low-level function.
    /// This should *not* be used unless you really need
    /// to do something low-level, weird or lazily initialize the object.
    /// You shouldn't let scripts observe this weirdness.
    /// Another exception is ES3 class-less objects, which we don't really understand well :)
    ///
    /// The "everyday" way to create a normal empty ScriptObject (AS "Object") is to call
    /// `avm2.classes().object.construct(self, &[])`.
    /// This is equivalent to AS3 `new Object()`.
    ///
    /// (calling `custom_object(mc, object_class, object_class.prototype()`)
    /// is technically also equivalent and faster, but not recommended outside lower-level Core code)
    pub fn custom_object(
        mc: &Mutation<'gc>,
        class: Option<ClassObject<'gc>>,
        proto: Option<Object<'gc>>,
    ) -> Object<'gc> {
        ScriptObject(GcCell::new(mc, ScriptObjectData::custom_new(proto, class))).into()
    }

    /// A special case for `newcatch` implementation. Basically a variable (q)name
    /// which maps to slot 1.
    pub fn catch_scope(mc: &Mutation<'gc>, qname: &QName<'gc>) -> Object<'gc> {
        // TODO: use a proper ClassObject here; purposefully crafted bytecode
        // can observe (the lack of) it.
        let mut base = ScriptObjectData::custom_new(None, None);
        let vt = VTable::newcatch(mc, qname);
        base.set_vtable(vt);
        base.install_instance_slots();

        ScriptObject(GcCell::new(mc, base)).into()
    }
}

impl<'gc> ScriptObjectData<'gc> {
    /// Create new object data of a given class.
    /// This is a low-level function used to implement things like object allocators.
    pub fn new(instance_of: ClassObject<'gc>) -> Self {
        Self::custom_new(Some(instance_of.prototype()), Some(instance_of))
    }

    /// Create new custom object data of a given possibly-none class and prototype.
    /// This is a low-level function used to implement things like object allocators.
    /// This should *not* be used, unless you really need
    /// to do something weird or lazily initialize the object.
    /// You shouldn't let scripts observe this weirdness.
    pub fn custom_new(proto: Option<Object<'gc>>, instance_of: Option<ClassObject<'gc>>) -> Self {
        ScriptObjectData {
            values: Default::default(),
            slots: Vec::new(),
            bound_methods: Vec::new(),
            proto,
            instance_of,
            vtable: instance_of.map(|cls| cls.instance_vtable()),
            enumerants: Vec::new(),
        }
    }

    pub fn get_property_local(
        &self,
        multiname: &Multiname<'gc>,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        if !multiname.contains_public_namespace() {
            return Err(error::make_reference_error(
                activation,
                error::ReferenceErrorCode::InvalidRead,
                multiname,
                self.instance_of(),
            ));
        }

        let Some(local_name) = multiname.local_name() else {
            // when can this happen?
            return Err(error::make_reference_error(
                activation,
                error::ReferenceErrorCode::InvalidRead,
                multiname,
                self.instance_of(),
            ));
        };

        let value = self.values.get(&local_name);
        if let Some(value) = value {
            return Ok(*value);
        }

        // follow the prototype chain
        let mut proto = self.proto();
        while let Some(obj) = proto {
            let obj = obj.base();
            let value = obj.values.get(&local_name);
            if let Some(value) = value {
                return Ok(*value);
            }
            proto = obj.proto();
        }

        // Special case: Unresolvable properties on dynamic classes are treated
        // as dynamic properties that have not yet been set, and yield
        // `undefined`
        if self.is_sealed() {
            return Err(error::make_reference_error(
                activation,
                error::ReferenceErrorCode::InvalidRead,
                multiname,
                self.instance_of(),
            ));
        } else {
            Ok(Value::Undefined)
        }
    }

    pub fn set_property_local(
        &mut self,
        multiname: &Multiname<'gc>,
        value: Value<'gc>,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<(), Error<'gc>> {
        if self.is_sealed() || !multiname.contains_public_namespace() {
            return Err(error::make_reference_error(
                activation,
                error::ReferenceErrorCode::InvalidWrite,
                multiname,
                self.instance_of(),
            ));
        }

        let Some(local_name) = multiname.local_name() else {
            return Err(error::make_reference_error(
                activation,
                error::ReferenceErrorCode::InvalidWrite,
                multiname,
                self.instance_of(),
            ));
        };

        match self.values.entry(local_name) {
            Entry::Occupied(mut o) => {
                o.insert(value);
            }
            Entry::Vacant(v) => {
                //TODO: Not all classes are dynamic like this
                self.enumerants.push(local_name);
                v.insert(value);
            }
        };
        Ok(())
    }

    pub fn init_property_local(
        &mut self,
        multiname: &Multiname<'gc>,
        value: Value<'gc>,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<(), Error<'gc>> {
        self.set_property_local(multiname, value, activation)
    }

    pub fn delete_property_local(&mut self, multiname: &Multiname<'gc>) -> bool {
        if !multiname.contains_public_namespace() {
            return false;
        }
        if let Some(name) = multiname.local_name() {
            self.set_local_property_is_enumerable(name, false);
            self.values.remove(&name);
            true
        } else {
            false
        }
    }

    pub fn get_slot(&self, id: u32) -> Result<Value<'gc>, Error<'gc>> {
        self.slots
            .get(id as usize)
            .cloned()
            .ok_or_else(|| format!("Slot index {id} out of bounds!").into())
    }

    /// Set a slot by its index.
    pub fn set_slot(
        &mut self,
        id: u32,
        value: Value<'gc>,
        _mc: &Mutation<'gc>,
    ) -> Result<(), Error<'gc>> {
        if let Some(slot) = self.slots.get_mut(id as usize) {
            *slot = value;
            Ok(())
        } else {
            Err(format!("Slot index {id} out of bounds!").into())
        }
    }

    /// Initialize a slot by its index.
    pub fn init_slot(
        &mut self,
        id: u32,
        value: Value<'gc>,
        _mc: &Mutation<'gc>,
    ) -> Result<(), Error<'gc>> {
        if let Some(slot) = self.slots.get_mut(id as usize) {
            *slot = value;
            Ok(())
        } else {
            Err(format!("Slot index {id} out of bounds!").into())
        }
    }

    pub fn install_instance_slots(&mut self) {
        use std::ops::Deref;
        let vtable = self.vtable.unwrap();
        let default_slots = vtable.default_slots();
        for value in default_slots.deref() {
            if let Some(value) = value {
                self.slots.push(*value);
            } else {
                self.slots.push(Value::Undefined)
            }
        }
    }

    /// Set a slot by its index. This does extend the array if needed.
    /// This should only be used during AVM initialization, not at runtime.
    pub fn install_const_slot_late(&mut self, id: u32, value: Value<'gc>) {
        if self.slots.len() < id as usize + 1 {
            self.slots.resize(id as usize + 1, Value::Undefined);
        }
        if let Some(slot) = self.slots.get_mut(id as usize) {
            *slot = value;
        }
    }

    /// Retrieve a bound method from the method table.
    pub fn get_bound_method(&self, id: u32) -> Option<FunctionObject<'gc>> {
        self.bound_methods.get(id as usize).and_then(|v| *v)
    }

    pub fn has_trait(&self, name: &Multiname<'gc>) -> bool {
        match self.vtable {
            //Class instances have instance traits from any class in the base
            //class chain.
            Some(vtable) => vtable.has_trait(name),

            // bare objects do not have traits.
            // TODO: should we have bare objects at all?
            // Shouldn't every object have a vtable?
            None => false,
        }
    }

    pub fn has_own_dynamic_property(&self, name: &Multiname<'gc>) -> bool {
        if name.contains_public_namespace() {
            if let Some(name) = name.local_name() {
                return self.values.get(&name).is_some();
            }
        }
        false
    }

    pub fn has_own_property(&self, name: &Multiname<'gc>) -> bool {
        self.has_trait(name) || self.has_own_dynamic_property(name)
    }

    pub fn proto(&self) -> Option<Object<'gc>> {
        self.proto
    }

    pub fn set_proto(&mut self, proto: Object<'gc>) {
        self.proto = Some(proto)
    }

    pub fn get_next_enumerant(&self, last_index: u32) -> Option<u32> {
        if last_index < self.enumerants.len() as u32 {
            Some(last_index.saturating_add(1))
        } else {
            None
        }
    }

    pub fn get_enumerant_name(&self, index: u32) -> Option<Value<'gc>> {
        // NOTE: AVM2 object enumeration is one of the weakest parts of an
        // otherwise well-designed VM. Notably, because of the way they
        // implemented `hasnext` and `hasnext2`, all enumerants start from ONE.
        // Hence why we have to `checked_sub` here in case some miscompiled
        // code doesn't check for the zero index, which is actually a failure
        // sentinel.
        let true_index = (index as usize).checked_sub(1)?;

        self.enumerants.get(true_index).cloned().map(|q| q.into())
    }

    pub fn property_is_enumerable(&self, name: AvmString<'gc>) -> bool {
        self.enumerants.contains(&name)
    }

    pub fn set_local_property_is_enumerable(&mut self, name: AvmString<'gc>, is_enumerable: bool) {
        if is_enumerable && self.values.contains_key(&name) && !self.enumerants.contains(&name) {
            self.enumerants.push(name);
        } else if !is_enumerable && self.enumerants.contains(&name) {
            let mut index = None;
            for (i, other_name) in self.enumerants.iter().enumerate() {
                if *other_name == name {
                    index = Some(i);
                }
            }

            if let Some(index) = index {
                self.enumerants.remove(index);
            }
        }
    }

    /// Gets the number of (standard) enumerants.
    pub fn num_enumerants(&self) -> u32 {
        self.enumerants.len() as u32
    }

    /// Install a method into the object.
    pub fn install_bound_method(&mut self, disp_id: u32, function: FunctionObject<'gc>) {
        if self.bound_methods.len() <= disp_id as usize {
            self.bound_methods
                .resize_with(disp_id as usize + 1, Default::default);
        }

        *self.bound_methods.get_mut(disp_id as usize).unwrap() = Some(function);
    }

    /// Get the class object for this object, if it has one.
    pub fn instance_of(&self) -> Option<ClassObject<'gc>> {
        self.instance_of
    }

    /// Get the vtable for this object, if it has one.
    pub fn vtable(&self) -> Option<VTable<'gc>> {
        self.vtable
    }

    pub fn is_sealed(&self) -> bool {
        self.instance_of()
            .map(|cls| cls.inner_class_definition().read().is_sealed())
            .unwrap_or(false)
    }

    /// Set the class object for this object.
    pub fn set_instance_of(&mut self, instance_of: ClassObject<'gc>, vtable: VTable<'gc>) {
        self.instance_of = Some(instance_of);
        self.vtable = Some(vtable);
    }

    pub fn set_vtable(&mut self, vtable: VTable<'gc>) {
        self.vtable = Some(vtable);
    }

    pub fn debug_class_name(&self) -> Box<dyn std::fmt::Debug + 'gc> {
        let class_name = self
            .instance_of()
            .map(|class_obj| class_obj.debug_class_name());

        match class_name {
            Some(class_name) => Box::new(class_name),
            None => Box::new("<None>"),
        }
    }
}

impl<'gc> Debug for ScriptObject<'gc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        let mut f = f.debug_struct("ScriptObject");

        match self.0.try_read() {
            Ok(obj) => f.field("name", &obj.debug_class_name()),
            Err(err) => f.field("name", &err),
        };

        f.field("ptr", &self.0.as_ptr()).finish()
    }
}
