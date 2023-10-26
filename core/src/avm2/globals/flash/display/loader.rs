//! `flash.display.Loader` builtin/prototype

use indexmap::IndexMap;

use crate::avm2::activation::Activation;
use crate::avm2::globals::flash::display::display_object::initialize_for_allocator;
use crate::avm2::object::LoaderInfoObject;
use crate::avm2::object::LoaderStream;
use crate::avm2::object::TObject;
use crate::avm2::parameters::ParametersExt;
use crate::avm2::value::Value;
use crate::avm2::ClassObject;
use crate::avm2::Multiname;
use crate::avm2::{Error, Object};
use crate::avm2_stub_method;
use crate::backend::navigator::{NavigationMethod, Request};
use crate::display_object::LoaderDisplay;
use crate::display_object::MovieClip;
use crate::loader::MovieLoaderVMData;
use crate::tag_utils::SwfMovie;
use std::sync::Arc;

pub fn loader_allocator<'gc>(
    class: ClassObject<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Object<'gc>, Error<'gc>> {
    // Loader does not have an associated `Character` variant, and can never be
    // instantiated from the timeline.
    let display_object = LoaderDisplay::empty(activation, activation.context.swf.clone()).into();
    let loader = initialize_for_allocator(activation, display_object, class)?;

    // Note that the initialization of `_contentLoaderInfo` is intentionally done here,
    // and not in the Loader constructor - subclasess of Loader can observe 'contentLoaderInfo'
    // being set before super() is called.

    // Some LoaderInfo properties (such as 'bytesLoaded' and 'bytesTotal') are always
    // accessible, even before the 'init' event has fired. Using an empty movie gives
    // us the correct value (0) for them.
    let loader_info = LoaderInfoObject::not_yet_loaded(
        activation,
        Arc::new(SwfMovie::empty(activation.context.swf.version())),
        Some(loader),
        None,
        false,
    )?;
    loader.set_property(
        &Multiname::new(
            activation.avm2().flash_display_internal,
            "_contentLoaderInfo",
        ),
        loader_info.into(),
        activation,
    )?;
    Ok(loader)
}

pub fn load<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let url_request = args.get_object(activation, 0, "request")?;
    let context = args.try_get_object(activation, 1);

    // This is a dummy MovieClip, which will get overwritten in `Loader`
    let content = MovieClip::new(
        Arc::new(SwfMovie::empty(activation.context.swf.version())),
        activation.context.gc_context,
    );

    let loader_info = this
        .get_property(
            &Multiname::new(
                activation.avm2().flash_display_internal,
                "_contentLoaderInfo",
            ),
            activation,
        )?
        .as_object()
        .unwrap();

    // Update the LoaderStream - we still have a fake SwfMovie, but we now have the real target clip.
    loader_info
        .as_loader_info_object()
        .unwrap()
        .set_loader_stream(
            LoaderStream::NotYetLoaded(
                Arc::new(SwfMovie::empty(activation.context.swf.version())),
                Some(content.into()),
                false,
            ),
            activation.context.gc_context,
        );

    let request = request_from_url_request(activation, url_request)?;

    let url = request.url().to_string();
    let future = activation.context.load_manager.load_movie_into_clip(
        activation.context.player.clone(),
        content.into(),
        request,
        Some(url),
        MovieLoaderVMData::Avm2 {
            loader_info,
            context,
            default_domain: activation
                .caller_domain()
                .expect("Missing caller domain in Loader.load"),
        },
    );
    activation.context.navigator.spawn_future(future);

    Ok(Value::Undefined)
}

pub fn request_from_url_request<'gc>(
    activation: &mut Activation<'_, 'gc>,
    url_request: Object<'gc>,
) -> Result<Request, Error<'gc>> {
    // FIXME: set `followRedirects`  and `userAgent`
    // from the `URLRequest`

    let mut url = url_request
        .get_public_property("url", activation)?
        .coerce_to_string(activation)?
        .to_string();

    let method = url_request
        .get_public_property("method", activation)?
        .coerce_to_string(activation)?;

    let headers = url_request
        .get_public_property("requestHeaders", activation)?
        .coerce_to_object(activation)?
        .as_array_object()
        .unwrap();

    let headers = headers.as_array_storage().unwrap();
    let mut string_headers = IndexMap::default();
    for i in 0..headers.length() {
        let Some(header) = headers.get(i).and_then(|val| val.as_object()) else {
            continue;
        };

        let name = header
            .get_public_property("name", activation)?
            .coerce_to_string(activation)?
            .to_string();
        let value = header
            .get_public_property("value", activation)?
            .coerce_to_string(activation)?
            .to_string();

        // Note - testing with Flash Player shows that later entries in the array
        // overwrite earlier ones with the same name. Flash Player never sends an HTTP
        // request with duplicate headers
        string_headers.insert(name, value);
    }

    // TODO: URLRequest.method should not be able to have invalid types.
    // We should throw an error there on set.
    let method = NavigationMethod::from_method_str(&method).unwrap_or(NavigationMethod::Get);
    let data = url_request.get_public_property("data", activation)?;
    let body = match (method, data) {
        (_, Value::Null | Value::Undefined) => None,
        (NavigationMethod::Get, data) => {
            // This looks "wrong" but it's Flash-correct.
            // It simply appends the data to the URL if there's already a query,
            // otherwise it adds ?data.
            // This does mean that if there's a #fragment in the URL after the query,
            // the new data gets appended to *that* - which is totally wrong but whatcha gonna do?
            if !url.contains('?') {
                url.push('?');
            }
            url.push_str(&data.coerce_to_string(activation)?.to_string());
            None
        }
        (NavigationMethod::Post, data) => {
            let content_type = url_request
                .get_public_property("contentType", activation)?
                .coerce_to_string(activation)?
                .to_string();
            if let Some(ba) = data.as_object().and_then(|o| o.as_bytearray_object()) {
                // Note that this does *not* respect or modify the position.
                Some((ba.storage().bytes().to_vec(), content_type))
            } else {
                Some((
                    data.coerce_to_string(activation)?
                        .to_utf8_lossy()
                        .as_bytes()
                        .to_vec(),
                    content_type,
                ))
            }
        }
    };

    let mut request = Request::request(method, url.to_string(), body);
    request.set_headers(string_headers);

    Ok(request)
}

pub fn load_bytes<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let arg0 = args.get_object(activation, 0, "data")?;
    let bytearray = arg0.as_bytearray().unwrap();
    let context = args.try_get_object(activation, 1);

    // This is a dummy MovieClip, which will get overwritten in `Loader`
    let content = MovieClip::new(
        Arc::new(SwfMovie::empty(activation.context.swf.version())),
        activation.context.gc_context,
    );

    let loader_info = this
        .get_property(
            &Multiname::new(
                activation.avm2().flash_display_internal,
                "_contentLoaderInfo",
            ),
            activation,
        )?
        .as_object()
        .unwrap();

    let future = activation.context.load_manager.load_movie_into_clip_bytes(
        activation.context.player.clone(),
        content.into(),
        bytearray.bytes().to_vec(),
        MovieLoaderVMData::Avm2 {
            loader_info,
            context,
            default_domain: activation
                .caller_domain()
                .expect("Missing caller domain in Loader.loadBytes"),
        },
    );
    activation.context.navigator.spawn_future(future);

    Ok(Value::Undefined)
}

pub fn unload<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    // TODO: Broadcast an "unload" event on the LoaderInfo and reset LoaderInfo properties
    avm2_stub_method!(activation, "flash.display.Loader", "unload");
    let _ = crate::avm2::globals::flash::display::display_object_container::remove_child_at(
        activation,
        this,
        &[0.into()],
    );

    let loader_info = this
        .get_property(
            &Multiname::new(
                activation.avm2().flash_display_internal,
                "_contentLoaderInfo",
            ),
            activation,
        )?
        .as_object()
        .unwrap();

    let loader_info_object = loader_info.as_loader_info_object().unwrap();

    loader_info_object.unload(activation);

    Ok(Value::Undefined)
}
