//! Class object impl

use crate::avm2::activation::Activation;
use crate::avm2::class::{Allocator, AllocatorFn, Class, ClassHashWrapper};
use crate::avm2::error::{argument_error, make_error_1127, reference_error, type_error};
use crate::avm2::function::Executable;
use crate::avm2::method::Method;
use crate::avm2::object::function_object::FunctionObject;
use crate::avm2::object::script_object::{scriptobject_allocator, ScriptObjectData};
use crate::avm2::object::{Object, ObjectPtr, TObject};
use crate::avm2::property::Property;
use crate::avm2::scope::{Scope, ScopeChain};
use crate::avm2::value::Value;
use crate::avm2::vtable::{ClassBoundMethod, VTable};
use crate::avm2::Multiname;
use crate::avm2::QName;
use crate::avm2::TranslationUnit;
use crate::avm2::{Domain, Error};
use crate::string::AvmString;
use fnv::FnvHashMap;
use gc_arena::{Collect, GcCell, GcWeakCell, Mutation};
use std::cell::{BorrowError, Ref, RefMut};
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};

/// An Object which can be called to execute its function code.
#[derive(Collect, Clone, Copy)]
#[collect(no_drop)]
pub struct ClassObject<'gc>(pub GcCell<'gc, ClassObjectData<'gc>>);

#[derive(Collect, Clone, Copy, Debug)]
#[collect(no_drop)]
pub struct ClassObjectWeak<'gc>(pub GcWeakCell<'gc, ClassObjectData<'gc>>);

#[derive(Collect, Clone)]
#[collect(no_drop)]
pub struct ClassObjectData<'gc> {
    /// Base script object
    base: ScriptObjectData<'gc>,

    /// The class associated with this class object.
    class: GcCell<'gc, Class<'gc>>,

    /// The associated prototype.
    /// Should always be non-None after initialization.
    prototype: Option<Object<'gc>>,

    /// The captured scope that all class traits will use.
    class_scope: ScopeChain<'gc>,

    /// The captured scope that all instance traits will use.
    instance_scope: ScopeChain<'gc>,

    /// The base class of this one.
    ///
    /// If `None`, this class has no parent. In practice, this is only used for
    /// interfaces (at least by the AS3 compiler in Animate CC 2020.)
    superclass_object: Option<ClassObject<'gc>>,

    /// The instance allocator for this class.
    #[collect(require_static)]
    instance_allocator: Allocator,

    /// The instance constructor function
    constructor: Method<'gc>,

    /// The native instance constructor function
    native_constructor: Method<'gc>,

    /// The customization point for `Class(args...)` without `new`
    /// If None, a simple coercion is done.
    call_handler: Option<Method<'gc>>,

    /// The parameters of this specialized class.
    ///
    /// None flags that this class has not been specialized.
    ///
    /// An individual parameter of `None` signifies the parameter `*`, which is
    /// represented in AVM2 as `null` with regards to type application.
    params: Option<Option<ClassObject<'gc>>>,

    /// List of all applications of this class.
    ///
    /// Only applicable if this class is generic.
    ///
    /// It is legal to apply a type with the value `null`, which is represented
    /// as `None` here. AVM2 considers both applications to be separate
    /// classes, though we consider the parameter to be the class `Object` when
    /// we get a param of `null`.
    applications: FnvHashMap<Option<ClassObject<'gc>>, ClassObject<'gc>>,

    /// Interfaces implemented by this class, including interfaces
    /// from parent classes and superinterfaces (recursively).
    /// TODO - avoid cloning this when a subclass implements the
    /// same interface as its superclass.
    interfaces: Vec<GcCell<'gc, Class<'gc>>>,

    /// VTable used for instances of this class.
    instance_vtable: VTable<'gc>,

    /// VTable used for a ScriptObject of this class object.
    class_vtable: VTable<'gc>,
}

impl<'gc> ClassObject<'gc> {
    /// Allocate the prototype for this class.
    ///
    /// This function is not used during the initialization of "early classes",
    /// i.e. `Object`, `Function`, and `Class`. Those classes and their
    /// prototypes are weaved together separately.
    fn allocate_prototype(
        self,
        activation: &mut Activation<'_, 'gc>,
        superclass_object: Option<ClassObject<'gc>>,
    ) -> Result<Object<'gc>, Error<'gc>> {
        let proto = activation
            .avm2()
            .classes()
            .object
            .construct(activation, &[])?;

        if let Some(superclass_object) = superclass_object {
            let base_proto = superclass_object.prototype();
            proto.set_proto(activation.context.gc_context, base_proto);
        }
        Ok(proto)
    }

    /// Construct a class.
    ///
    /// This function returns the class constructor object, which should be
    /// used in all cases where the type needs to be referred to. It's class
    /// initializer will be executed during this function call.
    ///
    /// `base_class` is allowed to be `None`, corresponding to a `null` value
    /// in the VM. This corresponds to no base class, and in practice appears
    /// to be limited to interfaces.
    pub fn from_class(
        activation: &mut Activation<'_, 'gc>,
        class: GcCell<'gc, Class<'gc>>,
        superclass_object: Option<ClassObject<'gc>>,
    ) -> Result<Self, Error<'gc>> {
        let class_object = Self::from_class_partial(activation, class, superclass_object)?;
        let class_proto = class_object.allocate_prototype(activation, superclass_object)?;

        class_object.link_prototype(activation, class_proto)?;

        let class_class = activation.avm2().classes().class;
        let class_class_proto = class_class.prototype();

        class_object.link_type(
            activation.context.gc_context,
            class_class_proto,
            class_class,
        );
        class_object.init_instance_vtable(activation)?;
        class_object.into_finished_class(activation)
    }

    /// Allocate a class but do not properly construct it.
    ///
    /// This function does the bare minimum to allocate classes, without taking
    /// any action that would require the existence of any other objects in the
    /// object graph. The resulting class will be a bare object and should not
    /// be used or presented to user code until you finish initializing it. You
    /// do that by calling `link_prototype`, `link_type`, and then
    /// `into_finished_class` in that order.
    ///
    /// This returns the class object directly (*not* an `Object`), to allow
    /// further manipulation of the class once it's dependent types have been
    /// allocated.
    pub fn from_class_partial(
        activation: &mut Activation<'_, 'gc>,
        class: GcCell<'gc, Class<'gc>>,
        superclass_object: Option<ClassObject<'gc>>,
    ) -> Result<Self, Error<'gc>> {
        let scope = activation.create_scopechain();
        if let Some(base_class) = superclass_object.map(|b| b.inner_class_definition()) {
            if base_class.read().is_final() {
                return Err(format!(
                    "Base class {:?} is final and cannot be extended",
                    base_class.read().name().local_name()
                )
                .into());
            }

            if base_class.read().is_interface() {
                return Err(format!(
                    "Base class {:?} is an interface and cannot be extended",
                    base_class.read().name().local_name()
                )
                .into());
            }
        }

        let instance_allocator = class
            .read()
            .instance_allocator()
            .or_else(|| superclass_object.and_then(|c| c.instance_allocator()))
            .unwrap_or(scriptobject_allocator);

        let class_object = ClassObject(GcCell::new(
            activation.context.gc_context,
            ClassObjectData {
                base: ScriptObjectData::custom_new(None, None),
                class,
                prototype: None,
                class_scope: scope,
                instance_scope: scope,
                superclass_object,
                instance_allocator: Allocator(instance_allocator),
                constructor: class.read().instance_init(),
                native_constructor: class.read().native_instance_init(),
                call_handler: class.read().call_handler(),
                params: None,
                applications: Default::default(),
                interfaces: Vec::new(),
                instance_vtable: VTable::empty(activation.context.gc_context),
                class_vtable: VTable::empty(activation.context.gc_context),
            },
        ));

        // instance scope = [..., class object]
        let instance_scope = scope.chain(
            activation.context.gc_context,
            &[Scope::new(class_object.into())],
        );

        class_object
            .0
            .write(activation.context.gc_context)
            .instance_scope = instance_scope;

        Ok(class_object)
    }

    pub fn init_instance_vtable(
        self,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<(), Error<'gc>> {
        let class = self.inner_class_definition();
        self.instance_of().ok_or(
            "Cannot finish initialization of core class without it being linked to a type!",
        )?;

        class.read().validate_class(self.superclass_object())?;

        self.instance_vtable().init_vtable(
            self,
            class.read().instance_traits(),
            self.instance_scope(),
            self.superclass_object().map(|cls| cls.instance_vtable()),
            activation,
        )?;
        Ok(())
    }

    /// Finish initialization of the class.
    ///
    /// This is intended for classes that were pre-allocated with
    /// `from_class_partial`. It skips several critical initialization steps
    /// that are necessary to obtain a functioning class object:
    ///
    ///  - The `link_type` step, which makes the class an instance of another
    ///    type
    ///  - The `link_prototype` step, which installs a prototype for instances
    ///    of this type to inherit
    ///  - The `init_instance_vtable` steps, which initializes the instance vtable
    ///    using the superclass vtable.
    ///
    /// Make sure to call them before calling this function, or it may yield an
    /// error.
    ///
    /// This function is also when class trait validation happens. Verify
    /// errors will be raised at this time.
    pub fn into_finished_class(
        mut self,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Self, Error<'gc>> {
        let class = self.inner_class_definition();

        // class vtable == class traits + Class instance traits
        self.class_vtable().init_vtable(
            self,
            class.read().class_traits(),
            self.class_scope(),
            Some(self.instance_of().unwrap().instance_vtable()),
            activation,
        )?;

        self.link_interfaces(activation)?;
        self.install_class_vtable_and_slots(activation.context.gc_context);
        self.run_class_initializer(activation)?;

        Ok(self)
    }

    fn install_class_vtable_and_slots(&mut self, mc: &Mutation<'gc>) {
        self.set_vtable(mc, self.class_vtable());
        self.base_mut(mc).install_instance_slots();
    }

    /// Link this class to a prototype.
    pub fn link_prototype(
        self,
        activation: &mut Activation<'_, 'gc>,
        class_proto: Object<'gc>,
    ) -> Result<(), Error<'gc>> {
        self.0.write(activation.context.gc_context).prototype = Some(class_proto);
        class_proto.set_string_property_local("constructor", self.into(), activation)?;
        class_proto.set_local_property_is_enumerable(
            activation.context.gc_context,
            "constructor".into(),
            false,
        );

        Ok(())
    }

    /// Link this class to it's interfaces.
    ///
    /// This should be done after all instance traits has been resolved, as
    /// instance traits will be resolved to their corresponding methods at this
    /// time.
    pub fn link_interfaces(self, activation: &mut Activation<'_, 'gc>) -> Result<(), Error<'gc>> {
        let mut write = self.0.write(activation.context.gc_context);
        let class = write.class;
        let scope = write.class_scope;

        let interface_names = class.read().direct_interfaces().to_vec();
        let mut interfaces = Vec::with_capacity(interface_names.len());

        let mut dedup = HashSet::new();
        let mut queue = vec![class];
        while let Some(cls) = queue.pop() {
            for interface_name in cls.read().direct_interfaces() {
                let interface = self.early_resolve_class(
                    scope.domain(),
                    interface_name,
                    activation.context.gc_context,
                )?;

                if !interface.read().is_interface() {
                    return Err(format!(
                        "Class {:?} is not an interface and cannot be implemented by classes",
                        interface.read().name().local_name()
                    )
                    .into());
                }

                if dedup.insert(ClassHashWrapper(interface)) {
                    queue.push(interface);
                    interfaces.push(interface);
                }
            }

            if let Some(superclass_name) = cls.read().super_class_name() {
                queue.push(self.early_resolve_class(
                    scope.domain(),
                    superclass_name,
                    activation.context.gc_context,
                )?);
            }
        }
        write.interfaces = interfaces;
        drop(write);

        let read = self.0.read();

        // FIXME - we should only be copying properties for newly-implemented
        // interfaces (i.e. those that were not already implemented by the superclass)
        // Otherwise, our behavior diverges from Flash Player in certain cases.
        // See the ignored test 'tests/tests/swfs/avm2/weird_superinterface_properties/'
        for interface in &read.interfaces {
            let iface_read = interface.read();
            for interface_trait in iface_read.instance_traits() {
                if !interface_trait.name().namespace().is_public() {
                    let public_name = QName::new(
                        activation.context.avm2.public_namespace,
                        interface_trait.name().local_name(),
                    );
                    self.instance_vtable().copy_property_for_interface(
                        activation.context.gc_context,
                        public_name,
                        interface_trait.name(),
                    );
                }
            }
        }

        Ok(())
    }

    // Looks up a class by name, without using `ScopeChain.resolve`
    // This lets us look up an class before its `ClassObject` has been constructed,
    // which is needed to resolve classes when constructing a (different) `ClassObject`.
    fn early_resolve_class(
        &self,
        domain: Domain<'gc>,
        class_name: &Multiname<'gc>,
        mc: &Mutation<'gc>,
    ) -> Result<GcCell<'gc, Class<'gc>>, Error<'gc>> {
        domain
            .get_class(class_name, mc)?
            .ok_or_else(|| format!("Could not resolve class {class_name:?}").into())
    }

    /// Manually set the type of this `Class`.
    ///
    /// This is intended to support initialization of early types such as
    /// `Class` and `Object`. All other types should pull `Class`'s prototype
    /// and type object from the `Avm2` instance.
    pub fn link_type(
        self,
        gc_context: &Mutation<'gc>,
        proto: Object<'gc>,
        instance_of: ClassObject<'gc>,
    ) {
        let instance_vtable = instance_of.instance_vtable();

        let mut write = self.0.write(gc_context);

        write.base.set_instance_of(instance_of, instance_vtable);
        write.base.set_proto(proto);
    }

    /// Run the class's initializer method.
    pub fn run_class_initializer(
        self,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<(), Error<'gc>> {
        let object: Object<'gc> = self.into();

        let scope = self.0.read().class_scope;
        let class = self.0.read().class;
        let class_read = class.read();

        if !class_read.is_class_initialized() {
            let class_initializer = class_read.class_init();
            let class_init_fn = FunctionObject::from_method(
                activation,
                class_initializer,
                scope,
                Some(object),
                Some(self),
            );

            drop(class_read);
            class
                .write(activation.context.gc_context)
                .mark_class_initialized();

            class_init_fn.call(object.into(), &[], activation)?;
        }

        Ok(())
    }

    /// Determine if this class has a given type in its superclass chain.
    ///
    /// The given object `test_class` should be either a superclass or
    /// interface we are checking against this class.
    ///
    /// To test if a class *instance* is of a given type, see is_of_type.
    pub fn has_class_in_chain(self, test_class: GcCell<'gc, Class<'gc>>) -> bool {
        let mut my_class = Some(self);

        while let Some(class) = my_class {
            if GcCell::ptr_eq(class.inner_class_definition(), test_class) {
                return true;
            }

            my_class = class.superclass_object()
        }

        // A `ClassObject` stores all of the interfaces it implements,
        // including those from superinterfaces and superclasses (recursively).
        // Therefore, we only need to check interfaces once, and we can skip
        // checking them when we processing superclasses in the `while`
        // further down in this method.
        if test_class.read().is_interface() {
            for interface in self.interfaces() {
                if GcCell::ptr_eq(interface, test_class) {
                    return true;
                }
            }
        }

        false
    }

    /// Call the instance initializer.
    pub fn call_init(
        self,
        receiver: Value<'gc>,
        arguments: &[Value<'gc>],
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        let scope = self.0.read().instance_scope;
        let constructor =
            Executable::from_method(self.0.read().constructor, scope, None, Some(self));

        constructor.exec(receiver, arguments, activation, self.into())
    }

    /// Call the instance's native initializer.
    ///
    /// The native initializer is called when native code needs to construct an
    /// object, or when supercalling into a parent constructor (as there are
    /// classes that cannot be constructed but can be supercalled).
    pub fn call_native_init(
        self,
        receiver: Value<'gc>,
        arguments: &[Value<'gc>],
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        let scope = self.0.read().instance_scope;
        let constructor =
            Executable::from_method(self.0.read().native_constructor, scope, None, Some(self));

        constructor.exec(receiver, arguments, activation, self.into())
    }

    /// Supercall a method defined in this class.
    ///
    /// This is intended to be called on the class object that is the
    /// superclass of the one that defined the currently called property. If no
    /// such superclass exists, you should use the class object for the
    /// receiver's actual type (i.e. the lowest in the chain). This ensures
    /// that repeated supercalls to the same method will call parent and
    /// grandparent methods, and so on.
    ///
    /// If no method exists with the given name, this falls back to calling a
    /// property of the `receiver`. This fallback only triggers if the property
    /// is associated with a trait. Dynamic properties will still error out.
    ///
    /// This function will search through the class object tree starting from
    /// this class up to `Object` for a method trait with the given name. If it
    /// is found, it will be called with the receiver and arguments you
    /// provided, as if it were defined on the target instance object.
    ///
    /// The class that defined the method being called will also be provided to
    /// the `Activation` that the method runs on so that further supercalls
    /// will work as expected.
    ///
    /// This method corresponds directly to the AVM2 operation `callsuper`,
    /// with the caveat listed above about what object to call it on.
    pub fn call_super(
        self,
        multiname: &Multiname<'gc>,
        receiver: Object<'gc>,
        arguments: &[Value<'gc>],
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        let property = self.instance_vtable().get_trait(multiname);
        if property.is_none() {
            let qualified_multiname_name = multiname.as_uri(activation.context.gc_context);
            let qualified_class_name = self
                .inner_class_definition()
                .read()
                .name()
                .to_qualified_name_err_message(activation.context.gc_context);

            return Err(Error::AvmError(reference_error(
                activation,
                &format!(
                    "Error #1070: Method {} not found on {}",
                    qualified_multiname_name, qualified_class_name
                ),
                1070,
            )?));
        }

        if let Some(Property::Method { disp_id, .. }) = property {
            // todo: handle errors
            let ClassBoundMethod {
                class,
                scope,
                method,
            } = self.instance_vtable().get_full_method(disp_id).unwrap();
            let callee =
                FunctionObject::from_method(activation, method, scope, Some(receiver), Some(class));

            callee.call(receiver.into(), arguments, activation)
        } else {
            receiver.call_property(multiname, arguments, activation)
        }
    }

    /// Supercall a getter defined in this class.
    ///
    /// This is intended to be called on the class object that is the
    /// superclass of the one that defined the currently called property. If no
    /// such superclass exists, you should use the class object for the
    /// receiver's actual type (i.e. the lowest in the chain). This ensures
    /// that repeated supercalls to the same getter will call parent and
    /// grandparent getters, and so on.
    ///
    /// If no getter exists with the given name, this falls back to getting a
    /// property of the `receiver`. This fallback only triggers if the property
    /// is associated with a trait. Dynamic properties will still error out.
    ///
    /// This function will search through the class object tree starting from
    /// this class up to `Object` for a getter trait with the given name. If it
    /// is found, it will be called with the receiver you provided, as if it
    /// were defined on the target instance object.
    ///
    /// The class that defined the getter being called will also be provided to
    /// the `Activation` that the getter runs on so that further supercalls
    /// will work as expected.
    ///
    /// This method corresponds directly to the AVM2 operation `getsuper`,
    /// with the caveat listed above about what object to call it on.
    pub fn get_super(
        self,
        multiname: &Multiname<'gc>,
        receiver: Object<'gc>,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        let property = self.instance_vtable().get_trait(multiname);

        match property {
            Some(
                Property::Virtual {
                    get: Some(disp_id), ..
                }
                | Property::Method { disp_id },
            ) => {
                // todo: handle errors
                let ClassBoundMethod {
                    class,
                    scope,
                    method,
                } = self.instance_vtable().get_full_method(disp_id).unwrap();
                let callee = FunctionObject::from_method(
                    activation,
                    method,
                    scope,
                    Some(receiver),
                    Some(class),
                );

                // We call getters, but return the actual function object for normal methods
                if matches!(property, Some(Property::Virtual { .. })) {
                    callee.call(receiver.into(), &[], activation)
                } else {
                    Ok(callee.into())
                }
            }
            Some(Property::Virtual { .. }) => Err(format!(
                "Attempting to use get_super on non-getter property {:?}",
                multiname
            )
            .into()),
            Some(Property::Slot { .. } | Property::ConstSlot { .. }) => {
                receiver.get_property(multiname, activation)
            }
            None => Err(format!(
                "Attempted to supercall method {:?}, which does not exist",
                multiname.local_name()
            )
            .into()),
        }
    }

    /// Supercall a setter defined in this class.
    ///
    /// This is intended to be called on the class object that is the
    /// superclass of the one that defined the currently called property. If no
    /// such superclass exists, you should use the class object for the
    /// receiver's actual type (i.e. the lowest in the chain). This ensures
    /// that repeated supercalls to the same setter will call parent and
    /// grandparent setter, and so on.
    ///
    /// If no setter exists with the given name, this falls back to setting a
    /// property of the `receiver`. This fallback only triggers if the property
    /// is associated with a trait. Dynamic properties will still error out.
    ///
    /// This function will search through the class object tree starting from
    /// this class up to `Object` for a setter trait with the given name. If it
    /// is found, it will be called with the receiver and value you provided,
    /// as if it were defined on the target instance object.
    ///
    /// The class that defined the setter being called will also be provided to
    /// the `Activation` that the setter runs on so that further supercalls
    /// will work as expected.
    ///
    /// This method corresponds directly to the AVM2 operation `setsuper`,
    /// with the caveat listed above about what object to call it on.
    #[allow(unused_mut)]
    pub fn set_super(
        self,
        multiname: &Multiname<'gc>,
        value: Value<'gc>,
        mut receiver: Object<'gc>,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<(), Error<'gc>> {
        let property = self.instance_vtable().get_trait(multiname);
        if property.is_none() {
            return Err(format!(
                "Attempted to supercall method {:?}, which does not exist",
                multiname.local_name()
            )
            .into());
        }

        match property {
            Some(Property::Virtual {
                set: Some(disp_id), ..
            }) => {
                // todo: handle errors
                let ClassBoundMethod {
                    class,
                    scope,
                    method,
                } = self.instance_vtable().get_full_method(disp_id).unwrap();
                let callee =
                    FunctionObject::from_method(activation, method, scope, Some(receiver), Some(class));

                callee.call(receiver.into(), &[value], activation)?;
                Ok(())
            }
            Some(Property::Slot { .. }) => {
                receiver.set_property(multiname, value, activation)?;
                Ok(())
            }
            _ => {
                Err(format!("set_super on {receiver:?} {multiname:?} with {value:?} resolved to unexpected property {property:?}").into())
            }
        }
    }

    pub fn add_application(
        &self,
        gc_context: &Mutation<'gc>,
        param: Option<ClassObject<'gc>>,
        cls: ClassObject<'gc>,
    ) {
        self.0.write(gc_context).applications.insert(param, cls);
    }

    pub fn translation_unit(self) -> Option<TranslationUnit<'gc>> {
        if let Method::Bytecode(bc) = self.0.read().constructor {
            Some(bc.txunit)
        } else {
            None
        }
    }

    pub fn constructor(self) -> Method<'gc> {
        self.0.read().constructor
    }

    pub fn instance_vtable(self) -> VTable<'gc> {
        self.0.read().instance_vtable
    }

    pub fn class_vtable(self) -> VTable<'gc> {
        self.0.read().class_vtable
    }

    /// Like `inner_class_definition`, but returns an `Err(BorrowError)` instead of panicking
    /// if our `GcCell` is already mutably borrowed. This is useful
    /// in contexts where panicking would be extremely undesirable,
    /// and there's a fallback if we cannot obtain the `Class`
    /// (such as `Debug` impls),
    pub fn try_inner_class_definition(&self) -> Result<GcCell<'gc, Class<'gc>>, BorrowError> {
        self.0.try_read().map(|c| c.class)
    }

    pub fn inner_class_definition(self) -> GcCell<'gc, Class<'gc>> {
        self.0.read().class
    }

    pub fn prototype(self) -> Object<'gc> {
        self.0.read().prototype.unwrap()
    }

    pub fn interfaces(self) -> Vec<GcCell<'gc, Class<'gc>>> {
        self.0.read().interfaces.clone()
    }

    pub fn class_scope(self) -> ScopeChain<'gc> {
        self.0.read().class_scope
    }

    pub fn instance_scope(self) -> ScopeChain<'gc> {
        self.0.read().instance_scope
    }

    pub fn superclass_object(self) -> Option<ClassObject<'gc>> {
        self.0.read().superclass_object
    }

    pub fn set_param(self, gc_context: &Mutation<'gc>, param: Option<Option<ClassObject<'gc>>>) {
        self.0.write(gc_context).params = param;
    }

    pub fn as_class_params(self) -> Option<Option<ClassObject<'gc>>> {
        self.0.read().params
    }

    fn instance_allocator(self) -> Option<AllocatorFn> {
        Some(self.0.read().instance_allocator.0)
    }

    /// Attempts to obtain the name of this class.
    /// If we are unable to read from a necessary `GcCell`,
    /// the returned value will be some kind of error message.
    ///
    /// This should only be used in a debug context, where
    /// we need infallible access to *something* to print
    /// out.
    pub fn debug_class_name(&self) -> Box<dyn Debug + 'gc> {
        let class_name = self
            .try_inner_class_definition()
            .and_then(|class| class.try_read().map(|c| c.name()));

        match class_name {
            Ok(class_name) => Box::new(class_name),
            Err(err) => Box::new(err),
        }
    }
}

impl<'gc> TObject<'gc> for ClassObject<'gc> {
    fn base(&self) -> Ref<ScriptObjectData<'gc>> {
        Ref::map(self.0.read(), |read| &read.base)
    }

    fn base_mut(&self, mc: &Mutation<'gc>) -> RefMut<ScriptObjectData<'gc>> {
        RefMut::map(self.0.write(mc), |write| &mut write.base)
    }

    fn as_ptr(&self) -> *const ObjectPtr {
        self.0.as_ptr() as *const ObjectPtr
    }

    fn to_string(&self, activation: &mut Activation<'_, 'gc>) -> Result<Value<'gc>, Error<'gc>> {
        Ok(AvmString::new_utf8(
            activation.context.gc_context,
            format!("[class {}]", self.0.read().class.read().name().local_name()),
        )
        .into())
    }

    fn to_locale_string(
        &self,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        self.to_string(activation)
    }

    fn value_of(&self, _mc: &Mutation<'gc>) -> Result<Value<'gc>, Error<'gc>> {
        Ok(Value::Object(Object::from(*self)))
    }

    fn call(
        self,
        receiver: Value<'gc>,
        arguments: &[Value<'gc>],
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        if let Some(call_handler) = self.0.read().call_handler {
            let scope = self.0.read().class_scope;
            let func = Executable::from_method(call_handler, scope, None, Some(self));

            func.exec(receiver, arguments, activation, self.into())
        } else if arguments.len() == 1 {
            arguments[0].coerce_to_type(activation, self.inner_class_definition())
        } else {
            Err(Error::AvmError(argument_error(
                activation,
                &format!(
                    "Error #1112: Argument count mismatch on class coercion.  Expected 1, got {}.",
                    arguments.len()
                ),
                1112,
            )?))
        }
    }

    fn construct(
        self,
        activation: &mut Activation<'_, 'gc>,
        arguments: &[Value<'gc>],
    ) -> Result<Object<'gc>, Error<'gc>> {
        let instance_allocator = self.0.read().instance_allocator.0;

        let instance = instance_allocator(self, activation)?;

        instance.install_instance_slots(activation.context.gc_context);

        self.call_init(instance.into(), arguments, activation)?;

        Ok(instance)
    }

    fn has_own_property(self, name: &Multiname<'gc>) -> bool {
        let read = self.0.read();

        read.base.has_own_dynamic_property(name) || self.class_vtable().has_trait(name)
    }

    fn as_class_object(&self) -> Option<ClassObject<'gc>> {
        Some(*self)
    }

    fn set_local_property_is_enumerable(
        &self,
        mc: &Mutation<'gc>,
        name: AvmString<'gc>,
        is_enumerable: bool,
    ) {
        self.0
            .write(mc)
            .base
            .set_local_property_is_enumerable(name, is_enumerable);
    }

    fn apply(
        &self,
        activation: &mut Activation<'_, 'gc>,
        nullable_params: &[Value<'gc>],
    ) -> Result<ClassObject<'gc>, Error<'gc>> {
        let self_class = self.inner_class_definition();

        if !self_class.read().is_generic() {
            return Err(make_error_1127(activation));
        }

        if nullable_params.len() != 1 {
            let class_name = self
                .inner_class_definition()
                .read()
                .name()
                .to_qualified_name(activation.context.gc_context);

            return Err(Error::AvmError(type_error(
                activation,
                &format!(
                    "Error #1128: Incorrect number of type parameters for {}. Expected 1, got {}.",
                    class_name,
                    nullable_params.len()
                ),
                1128,
            )?));
        }

        //Because `null` is a valid parameter, we have to accept values as
        //parameters instead of objects. We coerce them to objects now.
        let object_param = match nullable_params[0] {
            Value::Null => None,
            v => Some(v),
        };
        let object_param = match object_param {
            None => None,
            Some(cls) => Some(
                cls.as_object()
                    .and_then(|c| c.as_class_object())
                    .ok_or_else(|| {
                        // Note: FP throws VerifyError #1107 here
                        format!(
                            "Cannot apply class {:?} with non-class parameter",
                            self_class.read().name()
                        )
                    })?,
            ),
        };

        if let Some(application) = self.0.read().applications.get(&object_param) {
            return Ok(*application);
        }

        // if it's not a known application, then it's not int/uint/Number/*,
        // so it must be a simple Vector.<*>-derived class.

        let class_param = object_param.map(|c| c.inner_class_definition());

        let parameterized_class: GcCell<'_, Class<'_>> =
            Class::with_type_param(self_class, class_param, activation.context.gc_context);

        // NOTE: this isn't fully accurate, but much simpler.
        // FP's Vector is more of special case that literally copies some parent class's properties
        // main example: Vector.<Object>.prototype === Vector.<*>.prototype

        let vector_star_cls = activation.avm2().classes().object_vector;
        let class_object =
            Self::from_class(activation, parameterized_class, Some(vector_star_cls))?;

        class_object.0.write(activation.context.gc_context).params = Some(object_param);

        self.0
            .write(activation.context.gc_context)
            .applications
            .insert(object_param, class_object);

        Ok(class_object)
    }
}

impl<'gc> PartialEq for ClassObject<'gc> {
    fn eq(&self, other: &Self) -> bool {
        Object::ptr_eq(*self, *other)
    }
}

impl<'gc> Eq for ClassObject<'gc> {}

impl<'gc> Hash for ClassObject<'gc> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ptr().hash(state);
    }
}

impl<'gc> Debug for ClassObject<'gc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("ClassObject")
            .field("name", &self.debug_class_name())
            .field("ptr", &self.0.as_ptr())
            .finish()
    }
}
