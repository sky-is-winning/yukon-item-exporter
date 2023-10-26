pub use crate::avm2::globals::flash::utils::get_qualified_class_name;
use crate::avm2::metadata::Metadata;
use crate::avm2::method::Method;
use crate::avm2::object::{ArrayObject, TObject};
use crate::avm2::parameters::ParametersExt;
use crate::avm2::property::Property;
use crate::avm2::ClassObject;

use crate::avm2::{Activation, Error, Object, Value};
use crate::avm2_stub_method;

// Implements `avmplus.describeTypeJSON`
pub fn describe_type_json<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let value = args[0].coerce_to_object(activation)?;
    let flags = DescribeTypeFlags::from_bits(args.get_u32(activation, 1)?).expect("Invalid flags!");

    let class_obj = value.as_class_object().or_else(|| value.instance_of());
    let object = activation
        .avm2()
        .classes()
        .object
        .construct(activation, &[])?;
    let Some(class_obj) = class_obj else {
        return Ok(Value::Null);
    };

    let is_static = value.as_class_object().is_some();
    if !is_static && flags.contains(DescribeTypeFlags::USE_ITRAITS) {
        return Ok(Value::Null);
    }

    let class = class_obj.inner_class_definition();
    let class = class.read();

    let qualified_name = class
        .name()
        .to_qualified_name(activation.context.gc_context);

    object.set_public_property("name", qualified_name.into(), activation)?;

    object.set_public_property(
        "isDynamic",
        (is_static || !class.is_sealed()).into(),
        activation,
    )?;
    object.set_public_property(
        "isFinal",
        (is_static || class.is_final()).into(),
        activation,
    )?;
    object.set_public_property("isStatic", is_static.into(), activation)?;

    let traits = describe_internal_body(activation, class_obj, is_static, flags)?;
    if flags.contains(DescribeTypeFlags::INCLUDE_TRAITS) {
        object.set_public_property("traits", traits.into(), activation)?;
    } else {
        object.set_public_property("traits", Value::Null, activation)?;
    }

    Ok(object.into())
}

bitflags::bitflags! {
    #[derive(Copy, Clone)]
    pub struct DescribeTypeFlags: u32 {
        const HIDE_NSURI_METHODS      = 1 << 0;
        const INCLUDE_BASES           = 1 << 1;
        const INCLUDE_INTERFACES      = 1 << 2;
        const INCLUDE_VARIABLES       = 1 << 3;
        const INCLUDE_ACCESSORS       = 1 << 4;
        const INCLUDE_METHODS         = 1 << 5;
        const INCLUDE_METADATA        = 1 << 6;
        const INCLUDE_CONSTRUCTOR     = 1 << 7;
        const INCLUDE_TRAITS          = 1 << 8;
        const USE_ITRAITS             = 1 << 9;
        const HIDE_OBJECT             = 1 << 10;
    }
}

fn describe_internal_body<'gc>(
    activation: &mut Activation<'_, 'gc>,
    class_obj: ClassObject<'gc>,
    is_static: bool,
    flags: DescribeTypeFlags,
) -> Result<Object<'gc>, Error<'gc>> {
    // If we were passed a non-ClassObject, or the caller specifically requested it, then
    // look at the instance "traits" (our implementation is different than avmplus)

    let use_instance_traits = !is_static || flags.contains(DescribeTypeFlags::USE_ITRAITS);
    let traits = activation
        .avm2()
        .classes()
        .object
        .construct(activation, &[])?;

    let bases = ArrayObject::empty(activation)?.as_array_object().unwrap();
    let interfaces = ArrayObject::empty(activation)?.as_array_object().unwrap();
    let variables = ArrayObject::empty(activation)?.as_array_object().unwrap();
    let accessors = ArrayObject::empty(activation)?.as_array_object().unwrap();
    let methods = ArrayObject::empty(activation)?.as_array_object().unwrap();

    if flags.contains(DescribeTypeFlags::INCLUDE_BASES) {
        traits.set_public_property("bases", bases.into(), activation)?;
    } else {
        traits.set_public_property("bases", Value::Null, activation)?;
    }

    if flags.contains(DescribeTypeFlags::INCLUDE_INTERFACES) {
        traits.set_public_property("interfaces", interfaces.into(), activation)?;
    } else {
        traits.set_public_property("interfaces", Value::Null, activation)?;
    }

    if flags.contains(DescribeTypeFlags::INCLUDE_VARIABLES) {
        traits.set_public_property("variables", variables.into(), activation)?;
    } else {
        traits.set_public_property("variables", Value::Null, activation)?;
    }

    if flags.contains(DescribeTypeFlags::INCLUDE_ACCESSORS) {
        traits.set_public_property("accessors", accessors.into(), activation)?;
    } else {
        traits.set_public_property("accessors", Value::Null, activation)?;
    }

    if flags.contains(DescribeTypeFlags::INCLUDE_METHODS) {
        traits.set_public_property("methods", methods.into(), activation)?;
    } else {
        traits.set_public_property("methods", Value::Null, activation)?;
    }

    let mut bases_array = bases
        .as_array_storage_mut(activation.context.gc_context)
        .unwrap();
    let mut interfaces_array = interfaces
        .as_array_storage_mut(activation.context.gc_context)
        .unwrap();
    let mut variables_array = variables
        .as_array_storage_mut(activation.context.gc_context)
        .unwrap();
    let mut accessors_array = accessors
        .as_array_storage_mut(activation.context.gc_context)
        .unwrap();
    let mut methods_array = methods
        .as_array_storage_mut(activation.context.gc_context)
        .unwrap();

    let superclass = if use_instance_traits {
        class_obj.superclass_object()
    } else {
        Some(activation.avm2().classes().class)
    };

    if flags.contains(DescribeTypeFlags::INCLUDE_BASES) {
        let mut current_super_obj = superclass;
        while let Some(super_obj) = current_super_obj {
            let super_name = super_obj
                .inner_class_definition()
                .read()
                .name()
                .to_qualified_name(activation.context.gc_context);
            bases_array.push(super_name.into());
            current_super_obj = super_obj.superclass_object();
        }
    }

    // When we're describing a Class object, we use the class vtable (which hides instance properties)
    let vtable = if use_instance_traits {
        class_obj.instance_vtable()
    } else {
        class_obj.class_vtable()
    };

    let super_vtable = if use_instance_traits {
        class_obj.superclass_object().map(|c| c.instance_vtable())
    } else {
        class_obj.instance_of().map(|c| c.instance_vtable())
    };

    if flags.contains(DescribeTypeFlags::INCLUDE_INTERFACES) && use_instance_traits {
        for interface in class_obj.interfaces() {
            let interface_name = interface
                .read()
                .name()
                .to_qualified_name(activation.context.gc_context);
            interfaces_array.push(interface_name.into());
        }
    }

    // Implement the weird 'HIDE_NSURI_METHODS' behavior from avmplus:
    // https://github.com/adobe/avmplus/blob/858d034a3bd3a54d9b70909386435cf4aec81d21/core/TypeDescriber.cpp#L237
    let mut skip_ns = Vec::new();
    if let Some(super_vtable) = super_vtable {
        for (_, ns, prop) in super_vtable.resolved_traits().iter() {
            if !ns.as_uri().is_empty() {
                if let Property::Method { disp_id } = prop {
                    let method = super_vtable
                        .get_full_method(*disp_id)
                        .unwrap_or_else(|| panic!("Missing method for id {disp_id:?}"));
                    let is_playerglobals = method
                        .class
                        .class_scope()
                        .domain()
                        .is_playerglobals_domain(activation);

                    if !skip_ns.contains(&(ns, is_playerglobals)) {
                        skip_ns.push((ns, is_playerglobals));
                    }
                }
            }
        }
    }

    let class_is_playerglobals = class_obj
        .class_scope()
        .domain()
        .is_playerglobals_domain(activation);

    // FIXME - avmplus iterates over their own hashtable, so the order in the final XML
    // is different
    for (prop_name, ns, prop) in vtable.resolved_traits().iter() {
        if !ns.is_public_ignoring_ns() {
            continue;
        }

        // Hack around our lack of namespace versioning.
        // This is hack to work around the fact that we don't have namespace versioning
        // Once we do, methods from playerglobals should end up distinct public and AS3
        // namespaces, due to the special `kApiVersion_VM_ALLVERSIONS` used:
        // https://github.com/adobe/avmplus/blob/858d034a3bd3a54d9b70909386435cf4aec81d21/core/AbcParser.cpp#L1497
        //
        // The main way this is
        // observable is by having a class like this:
        //
        // ``
        // class SubClass extends SuperClass {
        //   AS3 function subclassMethod {}
        // }
        // class SuperClass {}
        // ```
        //
        // Here, `subclassMethod` will not get hidden - even though `Object`
        // has AS3 methods, they are in the playerglobal AS3 namespace
        // (with version kApiVersion_VM_ALLVERSIONS), which is distinct
        // from the AS3 namespace used by SubClass. However, if we have any
        // user-defined classes in the inheritance chain, then the namespace
        // *should* match (if the swf version numbers match).
        //
        // For now, we approximate this by checking if the declaring class
        // and our starting class are both in the playerglobals domain
        // or both not in the playerglobals domain. If not, then we ignore
        // `skip_ns`, since we should really have two different namespaces here.
        if flags.contains(DescribeTypeFlags::HIDE_NSURI_METHODS)
            && skip_ns.contains(&(ns, class_is_playerglobals))
        {
            continue;
        }

        let uri = if ns.as_uri().is_empty() {
            None
        } else {
            Some(ns.as_uri())
        };

        match prop {
            Property::ConstSlot { slot_id } | Property::Slot { slot_id } => {
                if !flags.contains(DescribeTypeFlags::INCLUDE_VARIABLES) {
                    continue;
                }
                let prop_class_name = vtable
                    .slot_class_name(*slot_id, activation.context.gc_context)?
                    .to_qualified_name_or_star(activation.context.gc_context);

                let access = match prop {
                    Property::ConstSlot { .. } => "readonly",
                    Property::Slot { .. } => "readwrite",
                    _ => unreachable!(),
                };

                let trait_metadata = vtable.get_metadata_for_slot(slot_id);

                let variable = activation
                    .avm2()
                    .classes()
                    .object
                    .construct(activation, &[])?;
                variable.set_public_property("name", prop_name.into(), activation)?;
                variable.set_public_property("type", prop_class_name.into(), activation)?;
                variable.set_public_property("access", access.into(), activation)?;
                variable.set_public_property(
                    "uri",
                    uri.map_or(Value::Null, |u| u.into()),
                    activation,
                )?;

                variable.set_public_property("metadata", Value::Null, activation)?;

                if flags.contains(DescribeTypeFlags::INCLUDE_METADATA) {
                    let metadata_object = ArrayObject::empty(activation)?;
                    if let Some(metadata) = trait_metadata {
                        write_metadata(metadata_object, &metadata, activation)?;
                    }
                    variable.set_public_property("metadata", metadata_object.into(), activation)?;
                }

                variables_array.push(variable.into());
            }
            Property::Method { disp_id } => {
                if !flags.contains(DescribeTypeFlags::INCLUDE_METHODS) {
                    continue;
                }
                let method = vtable
                    .get_full_method(*disp_id)
                    .unwrap_or_else(|| panic!("Missing method for id {disp_id:?}"));
                let return_type_name = method
                    .method
                    .return_type()
                    .to_qualified_name_or_star(activation.context.gc_context);
                let declared_by = method.class;

                if flags.contains(DescribeTypeFlags::HIDE_OBJECT)
                    && declared_by == activation.avm2().classes().object
                {
                    continue;
                }

                let declared_by_name = declared_by
                    .inner_class_definition()
                    .read()
                    .name()
                    .to_qualified_name(activation.context.gc_context);

                let trait_metadata = vtable.get_metadata_for_disp(disp_id);

                let method_obj = activation
                    .avm2()
                    .classes()
                    .object
                    .construct(activation, &[])?;

                method_obj.set_public_property("name", prop_name.into(), activation)?;
                method_obj.set_public_property(
                    "returnType",
                    return_type_name.into(),
                    activation,
                )?;
                method_obj.set_public_property(
                    "declaredBy",
                    declared_by_name.into(),
                    activation,
                )?;

                method_obj.set_public_property(
                    "uri",
                    uri.map_or(Value::Null, |u| u.into()),
                    activation,
                )?;

                let params = write_params(&method.method, activation)?;
                method_obj.set_public_property("parameters", params.into(), activation)?;

                method_obj.set_public_property("metadata", Value::Null, activation)?;

                if flags.contains(DescribeTypeFlags::INCLUDE_METADATA) {
                    let metadata_object = ArrayObject::empty(activation)?;
                    if let Some(metadata) = trait_metadata {
                        write_metadata(metadata_object, &metadata, activation)?;
                    }
                    method_obj.set_public_property(
                        "metadata",
                        metadata_object.into(),
                        activation,
                    )?;
                }
                methods_array.push(method_obj.into());
            }
            Property::Virtual { get, set } => {
                if !flags.contains(DescribeTypeFlags::INCLUDE_ACCESSORS) {
                    continue;
                }
                let access = match (get, set) {
                    (Some(_), Some(_)) => "readwrite",
                    (Some(_), None) => "readonly",
                    (None, Some(_)) => "writeonly",
                    (None, None) => unreachable!(),
                };

                // For getters, obtain the type by looking at the getter return type.
                // For setters, obtain the type by looking at the setter's first parameter.
                let (method_type, defining_class) = if let Some(get) = get {
                    let getter = vtable
                        .get_full_method(*get)
                        .unwrap_or_else(|| panic!("Missing 'get' method for id {get:?}"));
                    (getter.method.return_type(), getter.class)
                } else if let Some(set) = set {
                    let setter = vtable
                        .get_full_method(*set)
                        .unwrap_or_else(|| panic!("Missing 'set' method for id {set:?}"));
                    (
                        setter.method.signature()[0].param_type_name.clone(),
                        setter.class,
                    )
                } else {
                    unreachable!();
                };

                let uri = if ns.as_uri().is_empty() {
                    None
                } else {
                    Some(ns.as_uri())
                };

                let accessor_type =
                    method_type.to_qualified_name_or_star(activation.context.gc_context);
                let declared_by = defining_class
                    .inner_class_definition()
                    .read()
                    .name()
                    .to_qualified_name(activation.context.gc_context);

                let accessor_obj = activation
                    .avm2()
                    .classes()
                    .object
                    .construct(activation, &[])?;
                accessor_obj.set_public_property("name", prop_name.into(), activation)?;
                accessor_obj.set_public_property("access", access.into(), activation)?;
                accessor_obj.set_public_property("type", accessor_type.into(), activation)?;
                accessor_obj.set_public_property("declaredBy", declared_by.into(), activation)?;
                accessor_obj.set_public_property(
                    "uri",
                    uri.map_or(Value::Null, |u| u.into()),
                    activation,
                )?;

                let metadata_object = ArrayObject::empty(activation)?;

                if let Some(get_disp_id) = get {
                    if let Some(metadata) = vtable.get_metadata_for_disp(get_disp_id) {
                        write_metadata(metadata_object, &metadata, activation)?;
                    }
                }

                if let Some(set_disp_id) = set {
                    if let Some(metadata) = vtable.get_metadata_for_disp(set_disp_id) {
                        write_metadata(metadata_object, &metadata, activation)?;
                    }
                }

                if flags.contains(DescribeTypeFlags::INCLUDE_METADATA)
                    && metadata_object.as_array_storage().unwrap().length() > 0
                {
                    accessor_obj.set_public_property(
                        "metadata",
                        metadata_object.into(),
                        activation,
                    )?;
                } else {
                    accessor_obj.set_public_property("metadata", Value::Null, activation)?;
                }

                accessors_array.push(accessor_obj.into());
            }
        }
    }

    let constructor = class_obj.constructor();
    // Flash only shows a <constructor> element if it has at least one parameter
    if flags.contains(DescribeTypeFlags::INCLUDE_CONSTRUCTOR)
        && use_instance_traits
        && !constructor.signature().is_empty()
    {
        let params = write_params(&constructor, activation)?;
        traits.set_public_property("constructor", params.into(), activation)?;
    } else {
        // This is needed to override the normal 'constructor' property
        traits.set_public_property("constructor", Value::Null, activation)?;
    }

    if flags.contains(DescribeTypeFlags::INCLUDE_METADATA) {
        avm2_stub_method!(
            activation,
            "avmplus",
            "describeTypeJSON",
            "with top-level metadata"
        );

        let metadata_object = ArrayObject::empty(activation)?;
        traits.set_public_property("metadata", metadata_object.into(), activation)?;
    } else {
        traits.set_public_property("metadata", Value::Null, activation)?;
    }

    Ok(traits)
}

fn write_params<'gc>(
    method: &Method<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Object<'gc>, Error<'gc>> {
    let params = ArrayObject::empty(activation)?;
    let mut params_array = params
        .as_array_storage_mut(activation.context.gc_context)
        .unwrap();
    for param in method.signature() {
        let param_type_name = param
            .param_type_name
            .to_qualified_name_or_star(activation.context.gc_context);
        let optional = param.default_value.is_some();
        let param_obj = activation
            .avm2()
            .classes()
            .object
            .construct(activation, &[])?;
        param_obj.set_public_property("type", param_type_name.into(), activation)?;
        param_obj.set_public_property("optional", optional.into(), activation)?;
        params_array.push(param_obj.into());
    }
    Ok(params)
}

fn write_metadata<'gc>(
    metadata_object: Object<'gc>,
    trait_metadata: &[Metadata<'gc>],
    activation: &mut Activation<'_, 'gc>,
) -> Result<(), Error<'gc>> {
    let mut metadata_array = metadata_object
        .as_array_storage_mut(activation.context.gc_context)
        .unwrap();

    for single_trait in trait_metadata.iter() {
        metadata_array.push(single_trait.as_json_object(activation)?.into());
    }
    Ok(())
}
