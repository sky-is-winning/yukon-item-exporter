//! Browser-related platform functions

use crate::loader::Error;
use crate::socket::{ConnectionState, SocketAction, SocketHandle};
use crate::string::WStr;
use async_channel::Receiver;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Display;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::mpsc::Sender;
use std::time::Duration;
use swf::avm1::types::SendVarsMethod;
use url::{ParseError, Url};

/// Enumerates all possible navigation methods.
#[derive(Copy, Clone)]
pub enum NavigationMethod {
    /// Indicates that navigation should generate a GET request.
    Get,

    /// Indicates that navigation should generate a POST request.
    Post,
}

#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketMode {
    /// Allows movies to connect to any host using sockets.
    Allow,

    /// Refuse all socket connection requests
    Deny,

    /// Ask the user every time a socket connection is requested
    Ask,
}

/// The handling mode of links opening a new website.
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum OpenURLMode {
    /// Allow all links to open a new website.
    #[serde(rename = "allow")]
    Allow,

    /// A confirmation dialog opens with every link trying to open a new website.
    #[serde(rename = "confirm")]
    Confirm,

    /// Deny all links to open a new website.
    #[serde(rename = "deny")]
    Deny,
}

impl NavigationMethod {
    /// Convert an SWF method enum into a NavigationMethod.
    pub fn from_send_vars_method(s: SendVarsMethod) -> Option<Self> {
        match s {
            SendVarsMethod::None => None,
            SendVarsMethod::Get => Some(Self::Get),
            SendVarsMethod::Post => Some(Self::Post),
        }
    }

    pub fn from_method_str(method: &WStr) -> Option<Self> {
        // Methods seem to be case insensitive
        let method = method.to_ascii_lowercase();
        if &method == b"get" {
            Some(Self::Get)
        } else if &method == b"post" {
            Some(Self::Post)
        } else {
            None
        }
    }
}

impl fmt::Display for NavigationMethod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let method = match self {
            Self::Get => "GET",
            Self::Post => "POST",
        };
        f.write_str(method)
    }
}

/// A fetch request.
pub struct Request {
    /// The URL of the request.
    url: String,

    /// The HTTP method to be used to make the request.
    method: NavigationMethod,

    /// The contents of the request body, if the request's HTTP method supports
    /// having a body.
    ///
    /// The body consists of data and a mime type.
    body: Option<(Vec<u8>, String)>,

    /// The headers for the request, as (header_name, header_value) pairs.
    /// Flash appears to iterate over an internal hash table to determine
    /// the order of headers sent over the network. We just use an IndexMap
    /// to give us a consistent order - hopefully, no servers depend on
    /// the order of headers.
    headers: IndexMap<String, String>,
}

impl Request {
    /// Construct a GET request.
    pub fn get(url: String) -> Self {
        Self {
            url,
            method: NavigationMethod::Get,
            body: None,
            headers: Default::default(),
        }
    }

    /// Construct a POST request.
    pub fn post(url: String, body: Option<(Vec<u8>, String)>) -> Self {
        Self {
            url,
            method: NavigationMethod::Post,
            body,
            headers: Default::default(),
        }
    }

    /// Construct a request with the given method and data
    #[allow(clippy::self_named_constructors)]
    pub fn request(method: NavigationMethod, url: String, body: Option<(Vec<u8>, String)>) -> Self {
        Self {
            url,
            method,
            body,
            headers: Default::default(),
        }
    }

    /// Retrieve the URL of this request.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Retrieve the navigation method for this request.
    pub fn method(&self) -> NavigationMethod {
        self.method
    }

    /// Retrieve the body of this request, if it exists.
    pub fn body(&self) -> &Option<(Vec<u8>, String)> {
        &self.body
    }

    pub fn set_body(&mut self, body: (Vec<u8>, String)) {
        self.body = Some(body);
    }

    pub fn headers(&self) -> &IndexMap<String, String> {
        &self.headers
    }

    pub fn set_headers(&mut self, headers: IndexMap<String, String>) {
        self.headers = headers;
    }
}

/// A response to a successful fetch request.
pub struct SuccessResponse {
    /// The final URL obtained after any redirects.
    pub url: String,

    /// The contents of the response body.
    pub body: Vec<u8>,

    /// The status code of the response.
    pub status: u16,

    /// The field to indicate if the request has been redirected.
    pub redirected: bool,
}

/// A response to a non-successful fetch request.
pub struct ErrorResponse {
    /// The final URL obtained after any redirects.
    pub url: String,

    /// The error that occurred during the request.
    pub error: Error,
}

/// Type alias for pinned, boxed, and owned futures that output a falliable
/// result of type `Result<T, E>`.
pub type OwnedFuture<T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + 'static>>;

/// A backend interacting with a browser environment.
pub trait NavigatorBackend {
    /// Cause a browser navigation to a given URL.
    ///
    /// The URL given may be any URL scheme a browser can support. This may not
    /// be meaningful for all environments: for example, `javascript:` URLs may
    /// not be executable in a desktop context.
    ///
    /// The `target` parameter, should be treated identically to the `target`
    /// parameter on an HTML `<a>nchor` tag.
    ///
    /// This function may be used to send variables to an eligible target. If
    /// desired, the `vars_method` will be specified with a suitable
    /// `NavigationMethod` and a key-value representation of the variables to
    /// be sent. What the backend needs to do depends on the `NavigationMethod`:
    ///
    /// * `GET` - Variables are appended onto the query parameters of the given
    ///   URL.
    /// * `POST` - Variables are sent as form data in a POST request, as if the
    ///   user had filled out and submitted an HTML form.
    ///
    /// Flash Player implemented sandboxing to prevent certain kinds of XSS
    /// attacks. The `NavigatorBackend` is not responsible for enforcing this
    /// sandbox.
    fn navigate_to_url(
        &self,
        url: &str,
        target: &str,
        vars_method: Option<(NavigationMethod, IndexMap<String, String>)>,
    );

    /// Fetch data and return it some time in the future.
    fn fetch(&self, request: Request) -> OwnedFuture<SuccessResponse, ErrorResponse>;

    /// Take a URL string and resolve it to the actual URL from which a file
    /// can be fetched. This includes handling of relative links and pre-processing.
    ///
    /// If the URL is local, this equals the URL returned by fetch. Otherwise,
    /// fetch may return a different URL, e.g. considering redirections.
    fn resolve_url(&self, url: &str) -> Result<Url, ParseError>;

    /// Arrange for a future to be run at some point in the... well, future.
    ///
    /// This function must be called to ensure a future is actually computed.
    /// The future must output an empty value and not hold any stack references
    /// which would cause it to become invalidated.
    ///
    /// TODO: For some reason, `wasm_bindgen_futures` wants unpinnable futures.
    /// This seems highly limiting.
    fn spawn_future(&mut self, future: OwnedFuture<(), Error>);

    /// Handle any context specific pre-processing
    ///
    /// Changing http -> https for example. This function may alter any part of the
    /// URL (generally only if configured to do so by the user).
    fn pre_process_url(&self, url: Url) -> Url;

    /// Handle any Socket connection request
    ///
    /// Use [SocketAction::Connect] to notify AVM that the connection failed or succeeded.
    ///
    /// Use [SocketAction::Close] to close the connection on AVM side.
    ///
    /// Use [SocketAction::Data] to send data to AVM side.
    ///
    /// When the Sender of the Receiver is dropped then this task should end.
    fn connect_socket(
        &mut self,
        host: String,
        port: u16,
        timeout: Duration,
        handle: SocketHandle,
        receiver: Receiver<Vec<u8>>,
        sender: Sender<SocketAction>,
    );
}

#[cfg(not(target_family = "wasm"))]
pub struct NullExecutor(futures::executor::LocalPool);

#[cfg(not(target_family = "wasm"))]
impl NullExecutor {
    pub fn new() -> Self {
        Self(futures::executor::LocalPool::new())
    }

    pub fn spawner(&self) -> NullSpawner {
        NullSpawner(self.0.spawner())
    }

    pub fn run(&mut self) {
        self.0.run_until_stalled();
    }
}

impl Default for NullExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_family = "wasm"))]
pub struct NullSpawner(futures::executor::LocalSpawner);

#[cfg(not(target_family = "wasm"))]
impl NullSpawner {
    pub fn spawn_local(&self, future: OwnedFuture<(), Error>) {
        use futures::task::LocalSpawnExt;
        let _ = self.0.spawn_local(async move {
            if let Err(e) = future.await {
                tracing::error!("Asynchronous error occurred: {}", e);
            }
        });
    }
}

#[cfg(target_family = "wasm")]
pub struct NullExecutor;

#[cfg(target_family = "wasm")]
impl NullExecutor {
    pub fn new() -> Self {
        Self
    }

    pub fn spawner(&self) -> NullSpawner {
        NullSpawner
    }

    pub fn run(&mut self) {}
}

#[cfg(target_family = "wasm")]
pub struct NullSpawner;

#[cfg(target_family = "wasm")]
impl NullSpawner {
    pub fn spawn_local(&self, future: OwnedFuture<(), Error>) {
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = future.await {
                tracing::error!("Asynchronous error occurred: {}", e);
            }
        });
    }
}

/// A null implementation for platforms that do not live in a web browser.
///
/// The NullNavigatorBackend includes a trivial executor that holds owned
/// futures and runs them to completion, blockingly.
pub struct NullNavigatorBackend {
    spawner: NullSpawner,

    /// The base path for all relative fetches.
    relative_base_path: PathBuf,
}

impl NullNavigatorBackend {
    pub fn new() -> Self {
        let executor = NullExecutor::new();
        Self {
            spawner: executor.spawner(),
            relative_base_path: PathBuf::new(),
        }
    }

    pub fn with_base_path(path: &Path, executor: &NullExecutor) -> Result<Self, std::io::Error> {
        Ok(Self {
            spawner: executor.spawner(),
            relative_base_path: path.canonicalize()?,
        })
    }
}

impl Default for NullNavigatorBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl NavigatorBackend for NullNavigatorBackend {
    fn navigate_to_url(
        &self,
        _url: &str,
        _target: &str,
        _vars_method: Option<(NavigationMethod, IndexMap<String, String>)>,
    ) {
    }

    fn fetch(&self, request: Request) -> OwnedFuture<SuccessResponse, ErrorResponse> {
        fetch_path(self, "NullNavigatorBackend", request.url())
    }

    fn resolve_url(&self, url: &str) -> Result<Url, ParseError> {
        resolve_url_with_relative_base_path(self, self.relative_base_path.clone(), url)
    }

    fn spawn_future(&mut self, future: OwnedFuture<(), Error>) {
        self.spawner.spawn_local(future);
    }

    fn pre_process_url(&self, url: Url) -> Url {
        url
    }

    fn connect_socket(
        &mut self,
        _host: String,
        _port: u16,
        _timeout: Duration,
        handle: SocketHandle,
        _receiver: Receiver<Vec<u8>>,
        sender: Sender<SocketAction>,
    ) {
        sender
            .send(SocketAction::Connect(handle, ConnectionState::Failed))
            .expect("working channel send");
    }
}

// The following functions are helper functions used in different
// NavigatorBackend implementations.
// To avoid duplicated code, they are placed here as public functions.

/// Converts a given result into an OwnedFuture and returns it.
pub fn async_return<SuccessType: 'static, ErrorType: 'static>(
    return_value: Result<SuccessType, ErrorType>,
) -> OwnedFuture<SuccessType, ErrorType> {
    Box::pin(async move { return_value })
}

/// This creates and returns the generic ErrorResponse for an invalid URL
/// used in the NavigatorBackend fetch methods.
pub fn create_fetch_error<ErrorType: Display>(
    url: &str,
    error: ErrorType,
) -> Result<SuccessResponse, ErrorResponse> {
    create_specific_fetch_error("Invalid URL", url, error)
}

/// This creates and returns a specific ErrorResponse with a given reason
/// used in the NavigatorBackend fetch methods.
pub fn create_specific_fetch_error<ErrorType: Display>(
    reason: &str,
    url: &str,
    error: ErrorType,
) -> Result<SuccessResponse, ErrorResponse> {
    let message = if error.to_string() == "" {
        format!("{reason} {url}")
    } else {
        format!("{reason} {url}: {error}")
    };
    let error = Error::FetchError(message);
    Err(ErrorResponse {
        url: url.to_string(),
        error,
    })
}

// Url doesn't implement from_file_path and to_file_path for WASM targets.
// Therefore, we need to use cfg to make Ruffle compile for all targets.

#[cfg(any(unix, windows, target_os = "redox"))]
fn url_from_file_path(path: &Path) -> Result<Url, ()> {
    Url::from_file_path(path)
}

#[cfg(not(any(unix, windows, target_os = "redox")))]
fn url_from_file_path(_path: &Path) -> Result<Url, ()> {
    Err(())
}

#[cfg(any(unix, windows, target_os = "redox"))]
fn url_to_file_path(url: &Url) -> Result<PathBuf, ()> {
    Url::to_file_path(url)
}

#[cfg(not(any(unix, windows, target_os = "redox")))]
fn url_to_file_path(_path: &Url) -> Result<PathBuf, ()> {
    Err(())
}

// The following functions are implementations used in multiple places.
// To avoid duplicated code, they are placed here as public functions.

/// This is the resolve implementation for the TestNavigatorBackend and the
/// NullNavigatorBackend.
///
/// It resolves the given URL with the given relative base path.
pub fn resolve_url_with_relative_base_path<NavigatorType: NavigatorBackend>(
    navigator: &NavigatorType,
    base_path: PathBuf,
    url: &str,
) -> Result<Url, ParseError> {
    /// This is a helper function used to resolve just the request url.
    /// It is used if the base url and the request url can't be combined.
    fn resolve_request_url<NavigatorType: NavigatorBackend>(
        url: &str,
        navigator: &NavigatorType,
    ) -> Result<Url, ParseError> {
        match Url::parse(url) {
            Ok(parsed_url) => Ok(navigator.pre_process_url(parsed_url)),
            Err(error) => Err(error),
        }
    }

    if let Ok(mut base_url) = url_from_file_path(base_path.as_path()) {
        // Make sure we have a trailing slash, so that joining a request url like 'data.txt'
        // gets appended, rather than replacing the last component.
        base_url.path_segments_mut().unwrap().push("");
        if let Ok(parsed_url) = base_url.join(url) {
            Ok(navigator.pre_process_url(parsed_url))
        } else {
            resolve_request_url(url, navigator)
        }
    } else {
        resolve_request_url(url, navigator)
    }
}

/// This is the fetch implementation for the TestNavigatorBackend and the
/// NullNavigatorBackend.
///
/// It tries to fetch the given URL as a local path and read and return
/// its content. It returns an ErrorResponse if the URL is not valid, not
/// local or a local path that can't be read.
pub fn fetch_path<NavigatorType: NavigatorBackend>(
    navigator: &NavigatorType,
    navigator_name: &str,
    url: &str,
) -> OwnedFuture<SuccessResponse, ErrorResponse> {
    let url = match navigator.resolve_url(url) {
        Ok(url) => url,
        Err(e) => return async_return(create_fetch_error(url, e)),
    };
    let path = if url.scheme() == "file" {
        // Flash supports query parameters with local urls.
        // SwfMovie takes care of exposing those to ActionScript -
        // when we actually load a filesystem url, strip them out.
        let mut filesystem_url = url.clone();
        filesystem_url.set_query(None);

        match url_to_file_path(&filesystem_url) {
            Ok(path) => path,
            Err(_) => {
                return async_return(create_specific_fetch_error(
                    "Unable to create path out of URL",
                    url.as_str(),
                    "",
                ))
            }
        }
    } else {
        return async_return(create_specific_fetch_error(
            &format!("{navigator_name} can't fetch non-local URL"),
            url.as_str(),
            "",
        ));
    };

    Box::pin(async move {
        let body = match std::fs::read(path) {
            Ok(body) => body,
            Err(e) => return create_specific_fetch_error("Can't open file", url.as_str(), e),
        };
        Ok(SuccessResponse {
            url: url.to_string(),
            body,
            status: 0,
            redirected: false,
        })
    })
}
