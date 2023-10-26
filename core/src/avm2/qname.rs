use crate::avm2::script::TranslationUnit;
use crate::avm2::{Activation, Error, Namespace};
use crate::context::GcContext;
use crate::either::Either;
use crate::string::{AvmString, WStr, WString};
use gc_arena::{Collect, Mutation};
use std::fmt::Debug;
use swf::avm2::types::{Index, Multiname as AbcMultiname};

/// A `QName`, likely "qualified name", consists of a namespace and name string.
///
/// This is technically interchangeable with `xml::XMLName`, as they both
/// implement `QName`; however, AVM2 and XML have separate representations.
///
/// A property cannot be retrieved or set without first being resolved into a
/// `QName`. All other forms of names and multinames are either versions of
/// `QName` with unspecified parameters, or multiple names to be checked in
/// order.
#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
pub struct QName<'gc> {
    ns: Namespace<'gc>,
    name: AvmString<'gc>,
}

impl<'gc> PartialEq for QName<'gc> {
    fn eq(&self, other: &Self) -> bool {
        // Implemented by hand to enforce order of comparisons for perf
        self.name == other.name && self.ns == other.ns
    }
}

impl<'gc> Eq for QName<'gc> {}

impl<'gc> QName<'gc> {
    pub fn new(ns: Namespace<'gc>, name: impl Into<AvmString<'gc>>) -> Self {
        Self {
            ns,
            name: name.into(),
        }
    }

    /// Pull a `QName` from the multiname pool.
    ///
    /// This function returns an Err if the multiname does not exist or is not
    /// a `QName`.
    pub fn from_abc_multiname(
        translation_unit: TranslationUnit<'gc>,
        multiname_index: Index<AbcMultiname>,
        context: &mut GcContext<'_, 'gc>,
    ) -> Result<Self, Error<'gc>> {
        if multiname_index.0 == 0 {
            return Err("Attempted to load a trait name of index zero".into());
        }

        let actual_index = multiname_index.0 as usize - 1;
        let abc = translation_unit.abc();
        let abc_multiname: Result<_, Error<'gc>> = abc
            .constant_pool
            .multinames
            .get(actual_index)
            .ok_or_else(|| format!("Unknown multiname constant {}", multiname_index.0).into());

        Ok(match abc_multiname? {
            AbcMultiname::QName { namespace, name } => Self {
                ns: translation_unit.pool_namespace(*namespace, context)?,
                name: translation_unit.pool_string(name.0, context)?.into(),
            },
            _ => return Err("Attempted to pull QName from non-QName multiname".into()),
        })
    }

    /// Constructs a `QName` from a fully qualified name.
    ///
    /// A fully qualified name can be any of the following formats:
    /// NAMESPACE::LOCAL_NAME
    /// NAMESPACE.LOCAL_NAME (Where the LAST dot is used to split the namespace & local_name)
    /// LOCAL_NAME (Use the public namespace)
    ///
    /// This does *not* handle `Vector.<SomeTypeParam>` - use `get_defined_value_handling_vector` for that
    pub fn from_qualified_name(name: AvmString<'gc>, activation: &mut Activation<'_, 'gc>) -> Self {
        let parts = name
            .rsplit_once(WStr::from_units(b"::"))
            .or_else(|| name.rsplit_once(WStr::from_units(b".")));

        if let Some((package_name, local_name)) = parts {
            let mut context = activation.borrow_gc();
            let package_name = context
                .interner
                .intern_wstr(context.gc_context, package_name);

            Self {
                ns: Namespace::package(package_name, &mut context),
                name: AvmString::new(context.gc_context, local_name),
            }
        } else {
            Self {
                ns: activation.avm2().public_namespace,
                name,
            }
        }
    }

    /// Converts this `QName` to a fully qualified name.
    pub fn to_qualified_name(self, mc: &Mutation<'gc>) -> AvmString<'gc> {
        match self.to_qualified_name_no_mc() {
            Either::Left(avm_string) => avm_string,
            Either::Right(wstring) => AvmString::new(mc, wstring),
        }
    }

    /// Like `to_qualified_name`, but avoids the need for a `Mutation`
    /// by returning `Either::Right(wstring)` when it would otherwise
    /// be necessary to allocate a new `AvmString`.
    ///
    /// This method is intended for contexts like `Debug` impls where
    /// a `Mutation` is not available. Normally, you should
    /// use `to_qualified_name`
    pub fn to_qualified_name_no_mc(self) -> Either<AvmString<'gc>, WString> {
        let uri = self.namespace().as_uri();
        let name = self.local_name();
        if uri.is_empty() {
            Either::Left(name)
        } else {
            Either::Right({
                let mut buf = WString::from(uri.as_wstr());
                buf.push_str(WStr::from_units(b"::"));
                buf.push_str(&name);
                buf
            })
        }
    }

    // Like `to_qualified_name`, but uses a `.` instead of `::` separate
    // the namespace and local name. This matches the output produced by
    // Flash Player in error messages
    pub fn to_qualified_name_err_message(self, mc: &Mutation<'gc>) -> AvmString<'gc> {
        let mut buf = WString::new();
        let uri = self.namespace().as_uri();
        if !uri.is_empty() {
            buf.push_str(&uri);
            buf.push_char('.');
        }
        buf.push_str(&self.local_name());
        AvmString::new(mc, buf)
    }

    pub fn local_name(&self) -> AvmString<'gc> {
        self.name
    }

    pub fn namespace(self) -> Namespace<'gc> {
        self.ns
    }

    /// Get the string value of this QName, including the namespace URI.
    pub fn as_uri(&self, mc: &Mutation<'gc>) -> AvmString<'gc> {
        let ns_uri = self.ns.as_uri_opt();
        let ns = match &ns_uri {
            Some(s) if s.is_empty() => return self.name,
            Some(s) => s,
            None => WStr::from_units(b"*"),
        };

        let mut uri = WString::from(ns);
        uri.push_str(WStr::from_units(b"::"));
        uri.push_str(&self.name);
        AvmString::new(mc, uri)
    }
}

impl<'gc> Debug for QName<'gc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self.to_qualified_name_no_mc() {
            Either::Left(name) => write!(f, "{name}"),
            Either::Right(name) => write!(f, "{name}"),
        }
    }
}
