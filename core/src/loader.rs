//! Management of async loaders

use crate::avm1::ExecutionReason;
use crate::avm1::{Activation, ActivationIdentifier};
use crate::avm1::{Attribute, Avm1};
use crate::avm1::{Object, SoundObject, TObject, Value};
use crate::avm2::bytearray::ByteArrayStorage;
use crate::avm2::object::{
    BitmapDataObject, ByteArrayObject, EventObject as Avm2EventObject, LoaderStream, TObject as _,
};
use crate::avm2::{
    Activation as Avm2Activation, Avm2, Domain as Avm2Domain, Object as Avm2Object,
    Value as Avm2Value,
};
use crate::backend::navigator::{OwnedFuture, Request};
use crate::bitmap::bitmap_data::Color;
use crate::bitmap::bitmap_data::{BitmapData, BitmapDataWrapper};
use crate::context::{ActionQueue, ActionType, UpdateContext};
use crate::display_object::{
    DisplayObject, MovieClip, TDisplayObject, TDisplayObjectContainer, TInteractiveObject,
};
use crate::events::ClipEvent;
use crate::frame_lifecycle::catchup_display_object_to_frame;
use crate::limits::ExecutionLimit;
use crate::player::Player;
use crate::streams::NetStream;
use crate::string::AvmString;
use crate::tag_utils::SwfMovie;
use crate::vminterface::Instantiator;
use encoding_rs::UTF_8;
use gc_arena::{Collect, GcCell};
use generational_arena::{Arena, Index};
use ruffle_render::utils::{determine_jpeg_tag_format, JpegTagFormat};
use std::fmt;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use swf::read::{extract_swz, read_compression_type};
use thiserror::Error;
use url::{form_urlencoded, ParseError, Url};

pub type Handle = Index;

/// How Ruffle should load movies.
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoadBehavior {
    /// Allow movies to execute before they have finished loading.
    ///
    /// Frames/bytes loaded values will tick up normally and progress events
    /// will be fired at regular intervals. Movie preload animations will play
    /// normally.
    Streaming,

    /// Delay execution of loaded movies until they have finished loading.
    ///
    /// Movies will see themselves load immediately. Preload animations will be
    /// skipped. This may break movies that depend on loading during execution.
    Delayed,

    /// Block Ruffle until movies have finished loading.
    ///
    /// This has the same implications as `Delay`, but tag processing will be
    /// done synchronously. Complex movies will visibly block the player from
    /// accepting user input and the application will appear to freeze.
    Blocking,
}

/// Enumeration of all content types that `Loader` can handle.
///
/// This is a superset of `JpegTagFormat`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentType {
    Swf,
    Jpeg,
    Png,
    Gif,
    Unknown,
}

impl From<JpegTagFormat> for ContentType {
    fn from(jtf: JpegTagFormat) -> Self {
        match jtf {
            JpegTagFormat::Jpeg => Self::Jpeg,
            JpegTagFormat::Png => Self::Png,
            JpegTagFormat::Gif => Self::Gif,
            JpegTagFormat::Unknown => Self::Unknown,
        }
    }
}

impl fmt::Display for ContentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Swf => write!(f, "SWF"),
            Self::Jpeg => write!(f, "JPEG"),
            Self::Png => write!(f, "PNG"),
            Self::Gif => write!(f, "GIF"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

impl ContentType {
    fn sniff(data: &[u8]) -> ContentType {
        if read_compression_type(data).is_ok() {
            ContentType::Swf
        } else {
            determine_jpeg_tag_format(data).into()
        }
    }

    /// Assert that content is of a given type, and error otherwise.
    fn expect(self, expected: Self) -> Result<Self, Error> {
        if self == expected {
            Ok(self)
        } else {
            Err(Error::UnexpectedData(expected, self))
        }
    }
}

#[derive(Clone, Collect, Copy)]
#[collect(no_drop)]
pub enum DataFormat {
    Binary,
    Text,
    Variables,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Load cancelled")]
    Cancelled,

    #[error("Non-root-movie loader spawned as root movie loader")]
    NotRootMovieLoader,

    #[error("Non-movie loader spawned as movie loader")]
    NotMovieLoader,

    #[error("Non-form loader spawned as form loader")]
    NotFormLoader,

    #[error("Non-load vars loader spawned as load vars loader")]
    NotLoadVarsLoader,

    #[error("Non-data loader spawned as data loader")]
    NotLoadDataLoader,

    #[error("Non-sound loader spawned as sound loader")]
    NotSoundLoader,

    #[error("Non-NetStream loader spawned as NetStream loader")]
    NotNetStreamLoader,

    #[error("Other Loader spawned as Movie unloader")]
    NotMovieUnloader,

    #[error("HTTP Status is not OK: {0} redirected: {1}")]
    HttpNotOk(String, u16, bool),

    #[error("Could not fetch: {0}")]
    FetchError(String),

    #[error("Invalid SWF: {0}")]
    InvalidSwf(#[from] crate::tag_utils::Error),

    #[error("Invalid bitmap")]
    InvalidBitmap(#[from] ruffle_render::error::Error),

    #[error("Invalid sound: {0}")]
    InvalidSound(#[from] crate::backend::audio::DecodeError),

    #[error("Unexpected content of type {1}, expected {0}")]
    UnexpectedData(ContentType, ContentType),

    // TODO: We can't support lifetimes on this error object yet (or we'll need some backends inside
    // the GC arena). We're losing info here. How do we fix that?
    #[error("Error running avm1 script: {0}")]
    Avm1Error(String),

    // TODO: We can't support lifetimes on this error object yet (or we'll need some backends inside
    // the GC arena). We're losing info here. How do we fix that?
    #[error("Error running avm2 script: {0}")]
    Avm2Error(String),
}

impl From<crate::avm1::Error<'_>> for Error {
    fn from(error: crate::avm1::Error<'_>) -> Self {
        Error::Avm1Error(error.to_string())
    }
}

/// Holds all in-progress loads for the player.
pub struct LoadManager<'gc>(Arena<Loader<'gc>>);

unsafe impl<'gc> Collect for LoadManager<'gc> {
    fn trace(&self, cc: &gc_arena::Collection) {
        for (_, loader) in self.0.iter() {
            loader.trace(cc)
        }
    }
}

impl<'gc> LoadManager<'gc> {
    /// Construct a new `LoadManager`.
    pub fn new() -> Self {
        Self(Arena::new())
    }

    /// Add a new loader to the `LoadManager`.
    ///
    /// Returns the loader handle for later inspection. A loader handle is
    /// valid for as long as the load operation.
    ///
    /// After the load finishes, the loader should be removed (and the handle
    /// invalidated). This can be done with remove_loader.
    /// Movie loaders are removed automatically after the loader status is set
    /// accordingly.
    pub fn add_loader(&mut self, loader: Loader<'gc>) -> Handle {
        let handle = self.0.insert(loader);
        match self.get_loader_mut(handle).unwrap() {
            Loader::RootMovie { self_handle, .. }
            | Loader::Movie { self_handle, .. }
            | Loader::Form { self_handle, .. }
            | Loader::LoadVars { self_handle, .. }
            | Loader::LoadURLLoader { self_handle, .. }
            | Loader::SoundAvm1 { self_handle, .. }
            | Loader::SoundAvm2 { self_handle, .. }
            | Loader::NetStream { self_handle, .. }
            | Loader::MovieUnloader { self_handle, .. } => *self_handle = Some(handle),
        }
        handle
    }

    /// Remove a completed loader.
    /// This is used to remove a loader after the loading or unloading process has completed.
    pub fn remove_loader(&mut self, handle: Handle) {
        self.0.remove(handle);
    }

    /// Retrieve a loader by handle.
    pub fn get_loader(&self, handle: Handle) -> Option<&Loader<'gc>> {
        self.0.get(handle)
    }

    /// Retrieve a loader by handle for mutation.
    pub fn get_loader_mut(&mut self, handle: Handle) -> Option<&mut Loader<'gc>> {
        self.0.get_mut(handle)
    }

    /// Kick off the root movie load.
    ///
    /// The root movie is special because it determines a few bits of player
    /// state, such as the size of the stage and the current frame rate. Ergo,
    /// this method should only be called once, by the player that is trying to
    /// kick off its root movie load.
    pub fn load_root_movie(
        &mut self,
        player: Weak<Mutex<Player>>,
        request: Request,
        parameters: Vec<(String, String)>,
        on_metadata: Box<dyn FnOnce(&swf::HeaderExt)>,
    ) -> OwnedFuture<(), Error> {
        let loader = Loader::RootMovie { self_handle: None };
        let handle = self.add_loader(loader);
        let loader = self.get_loader_mut(handle).unwrap();
        loader.root_movie_loader(player, request, parameters, on_metadata)
    }

    /// Kick off a movie clip load.
    ///
    /// Returns the loader's async process, which you will need to spawn.
    pub fn load_movie_into_clip(
        &mut self,
        player: Weak<Mutex<Player>>,
        target_clip: DisplayObject<'gc>,
        request: Request,
        loader_url: Option<String>,
        vm_data: MovieLoaderVMData<'gc>,
    ) -> OwnedFuture<(), Error> {
        let loader = Loader::Movie {
            self_handle: None,
            target_clip,
            vm_data,
            loader_status: LoaderStatus::Pending,
            movie: None,
        };
        let handle = self.add_loader(loader);
        let loader = self.get_loader_mut(handle).unwrap();
        loader.movie_loader(player, request, loader_url)
    }

    /// Kick off a movie clip load.
    ///
    /// Returns the loader's async process, which you will need to spawn.
    pub fn load_movie_into_clip_bytes(
        &mut self,
        player: Weak<Mutex<Player>>,
        target_clip: DisplayObject<'gc>,
        bytes: Vec<u8>,
        vm_data: MovieLoaderVMData<'gc>,
    ) -> OwnedFuture<(), Error> {
        let loader = Loader::Movie {
            self_handle: None,
            target_clip,
            vm_data,
            loader_status: LoaderStatus::Pending,
            movie: None,
        };
        let handle = self.add_loader(loader);
        let loader = self.get_loader_mut(handle).unwrap();
        loader.movie_loader_bytes(player, bytes)
    }

    /// Fires the `onLoad` listener event for every MovieClip that has been
    /// initialized (ran its first frame).
    ///
    /// This also removes all movie loaders that have completed.
    pub fn movie_clip_on_load(&mut self, queue: &mut ActionQueue<'gc>) {
        let mut invalidated_loaders = vec![];

        for (index, loader) in self.0.iter_mut().rev() {
            if loader.movie_clip_loaded(queue) {
                invalidated_loaders.push(index);
            }
        }

        for index in invalidated_loaders {
            self.0.remove(index);
        }
    }

    /// Kick off a form data load into an AVM1 object.
    ///
    /// Returns the loader's async process, which you will need to spawn.
    pub fn load_form_into_object(
        &mut self,
        player: Weak<Mutex<Player>>,
        target_object: Object<'gc>,
        request: Request,
    ) -> OwnedFuture<(), Error> {
        let loader = Loader::Form {
            self_handle: None,
            target_object,
        };
        let handle = self.add_loader(loader);
        let loader = self.get_loader_mut(handle).unwrap();
        loader.form_loader(player, request)
    }

    /// Kick off a form data load into an AVM1 object.
    ///
    /// Returns the loader's async process, which you will need to spawn.
    pub fn load_form_into_load_vars(
        &mut self,
        player: Weak<Mutex<Player>>,
        target_object: Object<'gc>,
        request: Request,
    ) -> OwnedFuture<(), Error> {
        let loader = Loader::LoadVars {
            self_handle: None,
            target_object,
        };
        let handle = self.add_loader(loader);
        let loader = self.get_loader_mut(handle).unwrap();
        loader.load_vars_loader(player, request)
    }

    /// Kick off a data load into a `URLLoader`, updating
    /// its `data` property when the load completes.
    ///
    /// Returns the loader's async process, which you will need to spawn.
    pub fn load_data_into_url_loader(
        &mut self,
        player: Weak<Mutex<Player>>,
        target_object: Avm2Object<'gc>,
        request: Request,
        data_format: DataFormat,
    ) -> OwnedFuture<(), Error> {
        let loader = Loader::LoadURLLoader {
            self_handle: None,
            target_object,
        };
        let handle = self.add_loader(loader);
        let loader = self.get_loader_mut(handle).unwrap();
        loader.load_url_loader(player, request, data_format)
    }

    /// Kick off an AVM1 audio load.
    ///
    /// Returns the loader's async process, which you will need to spawn.
    pub fn load_sound_avm1(
        &mut self,
        player: Weak<Mutex<Player>>,
        target_object: SoundObject<'gc>,
        request: Request,
        is_streaming: bool,
    ) -> OwnedFuture<(), Error> {
        let loader = Loader::SoundAvm1 {
            self_handle: None,
            target_object,
        };
        let handle = self.add_loader(loader);
        let loader = self.get_loader_mut(handle).unwrap();
        loader.sound_loader_avm1(player, request, is_streaming)
    }

    /// Kick off an AVM2 audio load.
    ///
    /// Returns the loader's async process, which you will need to spawn.
    pub fn load_sound_avm2(
        &mut self,
        player: Weak<Mutex<Player>>,
        target_object: Avm2Object<'gc>,
        request: Request,
    ) -> OwnedFuture<(), Error> {
        let loader = Loader::SoundAvm2 {
            self_handle: None,
            target_object,
        };
        let handle = self.add_loader(loader);
        let loader = self.get_loader_mut(handle).unwrap();
        loader.sound_loader_avm2(player, request)
    }

    pub fn load_netstream(
        &mut self,
        player: Weak<Mutex<Player>>,
        target_stream: NetStream<'gc>,
        request: Request,
    ) -> OwnedFuture<(), Error> {
        let loader = Loader::NetStream {
            self_handle: None,
            target_stream,
        };
        let handle = self.add_loader(loader);
        let loader = self.get_loader_mut(handle).unwrap();
        loader.stream_loader(player, request)
    }

    /// Process tags on all loaders in the Parsing phase.
    ///
    /// Returns true if *all* loaders finished preloading.
    pub fn preload_tick(context: &mut UpdateContext<'_, 'gc>, limit: &mut ExecutionLimit) -> bool {
        let mut did_finish = true;
        let handles: Vec<_> = context.load_manager.0.iter().map(|(h, _)| h).collect();

        for handle in handles {
            let status = match context.load_manager.get_loader(handle) {
                Some(Loader::Movie { loader_status, .. }) => Some(loader_status),
                _ => None,
            };

            if matches!(status, Some(LoaderStatus::Parsing)) {
                match Loader::preload_tick(handle, context, limit, 0, false) {
                    Ok(f) => did_finish = did_finish && f,
                    Err(e) => tracing::error!("Error encountered while preloading movie: {}", e),
                }
            }
        }

        did_finish
    }
}

impl<'gc> Default for LoadManager<'gc> {
    fn default() -> Self {
        Self::new()
    }
}

/// The completion status of a `Loader` loading a movie.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoaderStatus {
    /// The movie hasn't been loaded yet.
    Pending,
    /// The movie is currently being parsed (e.g. mc.preload)
    Parsing,
    /// The movie loaded successfully.
    Succeeded,
    /// An error occurred while loading the movie.
    Failed,
}

#[derive(Collect, Clone, Copy)]
#[collect(no_drop)]
pub enum MovieLoaderVMData<'gc> {
    Avm1 {
        broadcaster: Option<Object<'gc>>,
    },
    Avm2 {
        loader_info: Avm2Object<'gc>,

        /// The context of the SWF being loaded.
        context: Option<Avm2Object<'gc>>,

        /// The default domain this SWF will use.
        default_domain: Avm2Domain<'gc>,
    },
}

/// A struct that holds garbage-collected pointers for asynchronous code.
#[derive(Collect)]
#[collect(no_drop)]
pub enum Loader<'gc> {
    /// Loader that is loading the root movie of a player.
    RootMovie {
        /// The handle to refer to this loader instance.
        #[collect(require_static)]
        self_handle: Option<Handle>,
    },

    /// Loader that is loading a new movie into a MovieClip.
    Movie {
        /// The handle to refer to this loader instance.
        #[collect(require_static)]
        self_handle: Option<Handle>,

        /// The target movie clip to load the movie into.
        target_clip: DisplayObject<'gc>,

        // Virtual-machine specific data (AVM1 or AVM2)
        vm_data: MovieLoaderVMData<'gc>,

        /// Indicates the completion status of this loader.
        ///
        /// This flag exists to prevent a situation in which loading a movie
        /// into a clip that has not yet fired its Load event causes the
        /// loader to be prematurely removed. This flag is only set when either
        /// the movie has been replaced (and thus Load events can be trusted)
        /// or an error has occurred (in which case we don't care about the
        /// loader anymore).
        #[collect(require_static)]
        loader_status: LoaderStatus,

        /// The SWF being loaded.
        ///
        /// This is only available if the asynchronous loader path has
        /// completed and we expect the Player to periodically tick preload
        /// until loading completes.
        movie: Option<Arc<SwfMovie>>,
    },

    /// Loader that is loading form data into an AVM1 object scope.
    Form {
        /// The handle to refer to this loader instance.
        #[collect(require_static)]
        self_handle: Option<Handle>,

        /// The target AVM1 object to load form data into.
        target_object: Object<'gc>,
    },

    /// Loader that is loading form data into an AVM1 LoadVars object.
    LoadVars {
        /// The handle to refer to this loader instance.
        #[collect(require_static)]
        self_handle: Option<Handle>,

        /// The target AVM1 object to load form data into.
        target_object: Object<'gc>,
    },

    /// Loader that is loading data into a `URLLoader`'s `data` property
    /// The `data` property is only updated after the data is loaded completely
    LoadURLLoader {
        /// The handle to refer to this loader instance.
        #[collect(require_static)]
        self_handle: Option<Handle>,

        /// The target `URLLoader` to load data into.
        target_object: Avm2Object<'gc>,
    },

    /// Loader that is loading an MP3 into an AVM1 Sound object.
    SoundAvm1 {
        /// The handle to refer to this loader instance.
        #[collect(require_static)]
        self_handle: Option<Handle>,

        /// The target AVM1 object to load the audio into.
        target_object: SoundObject<'gc>,
    },

    /// Loader that is loading an MP3 into an AVM2 Sound object.
    SoundAvm2 {
        /// The handle to refer to this loader instance.
        #[collect(require_static)]
        self_handle: Option<Handle>,

        /// The target AVM1 object to load the audio into.
        target_object: Avm2Object<'gc>,
    },

    /// Loader that is buffering video or audio into a NetStream.
    NetStream {
        /// The handle to refer to this loader instance.
        #[collect(require_static)]
        self_handle: Option<Handle>,

        /// The stream to buffer data into.
        target_stream: NetStream<'gc>,
    },

    /// Loader that is unloading a MovieClip.
    MovieUnloader {
        /// The handle to refer to this loader instance.
        #[collect(require_static)]
        self_handle: Option<Handle>,

        /// The target MovieClip to unload.
        target_clip: DisplayObject<'gc>,
    },
}

impl<'gc> Loader<'gc> {
    /// Process tags on a loaded movie.
    ///
    /// Is only callable on Movie loaders, panics otherwise. Will
    /// do nothing unless the movie is ready to be preloaded. Movies which
    /// complete their preload will fire all events and be removed from the
    /// load manager queue.
    ///
    /// Returns true if the movie finished preloading.
    ///
    /// Returns any AVM errors encountered while sending events to user code.
    fn preload_tick(
        handle: Handle,
        context: &mut UpdateContext<'_, 'gc>,
        limit: &mut ExecutionLimit,
        status: u16,
        redirected: bool,
    ) -> Result<bool, Error> {
        let mc = match context.load_manager.get_loader_mut(handle) {
            Some(Self::Movie {
                target_clip, movie, ..
            }) => {
                if movie.is_none() {
                    //Non-SWF load or file not loaded yet
                    return Ok(false);
                }

                if target_clip.as_movie_clip().is_none() {
                    // Non-movie-clip loads should not be handled in preload_tick
                    tracing::error!("Cannot preload non-movie-clip loader");
                    return Ok(false);
                }

                *target_clip
            }
            None => return Err(Error::Cancelled),
            Some(_) => panic!("Attempted to preload a non-SWF loader"),
        };

        let mc = mc.as_movie_clip().unwrap();

        let did_finish = mc.preload(context, limit);

        Loader::movie_loader_progress(
            handle,
            context,
            mc.compressed_loaded_bytes() as usize,
            mc.compressed_total_bytes() as usize,
        )?;

        if did_finish {
            Loader::movie_loader_complete(handle, context, Some(mc.into()), status, redirected)?;
        }

        Ok(did_finish)
    }

    /// Construct a future for the root movie loader.
    fn root_movie_loader(
        &mut self,
        player: Weak<Mutex<Player>>,
        request: Request,
        parameters: Vec<(String, String)>,
        on_metadata: Box<dyn FnOnce(&swf::HeaderExt)>,
    ) -> OwnedFuture<(), Error> {
        let _handle = match self {
            Loader::RootMovie { self_handle, .. } => {
                self_handle.expect("Loader not self-introduced")
            }
            _ => return Box::pin(async { Err(Error::NotMovieLoader) }),
        };

        let player = player
            .upgrade()
            .expect("Could not upgrade weak reference to player");

        Box::pin(async move {
            let fetch = player.lock().unwrap().navigator().fetch(request);

            let response = fetch.await.map_err(|error| {
                player
                    .lock()
                    .unwrap()
                    .ui()
                    .display_root_movie_download_failed_message();
                error.error
            })?;

            // The spoofed root movie URL takes precedence over the actual URL.
            let swf_url = player
                .lock()
                .unwrap()
                .compatibility_rules()
                .rewrite_swf_url(response.url);
            let spoofed_or_swf_url = player
                .lock()
                .unwrap()
                .spoofed_url()
                .map(|u| u.to_string())
                .unwrap_or(swf_url);

            let mut movie = SwfMovie::from_data(&response.body, spoofed_or_swf_url, None)?;
            on_metadata(movie.header());
            movie.append_parameters(parameters);
            player.lock().unwrap().set_root_movie(movie);
            Ok(())
        })
    }

    /// Construct a future for the given movie loader.
    ///
    /// The given future should be passed immediately to an executor; it will
    /// take responsibility for running the loader to completion.
    ///
    /// If the loader is not a movie then the returned future will yield an
    /// error immediately once spawned.
    fn movie_loader(
        &mut self,
        player: Weak<Mutex<Player>>,
        request: Request,
        loader_url: Option<String>,
    ) -> OwnedFuture<(), Error> {
        let handle = match self {
            Loader::Movie { self_handle, .. } => self_handle.expect("Loader not self-introduced"),
            _ => return Box::pin(async { Err(Error::NotMovieLoader) }),
        };

        let player = player
            .upgrade()
            .expect("Could not upgrade weak reference to player");

        Box::pin(async move {
            let request_url = request.url().to_string();
            let resolved_url = player.lock().unwrap().navigator().resolve_url(&request_url);

            let fetch = player.lock().unwrap().navigator().fetch(request);

            let mut replacing_root_movie = false;
            player.lock().unwrap().update(|uc| -> Result<(), Error> {
                let clip = match uc.load_manager.get_loader(handle) {
                    Some(Loader::Movie { target_clip, .. }) => *target_clip,
                    None => return Err(Error::Cancelled),
                    _ => unreachable!(),
                };

                replacing_root_movie = uc
                    .stage
                    .root_clip()
                    .map(|root| DisplayObject::ptr_eq(clip, root))
                    .unwrap_or(false);

                if let Some(mut mc) = clip.as_movie_clip() {
                    if !uc.is_action_script_3() {
                        mc.avm1_unload(uc);
                    }

                    // Before the actual SWF is loaded, an initial loading state is entered.
                    Loader::load_initial_loading_swf(&mut mc, uc, &request_url, resolved_url);
                }

                Loader::movie_loader_start(handle, uc)
            })?;

            match fetch.await {
                Ok(response) if replacing_root_movie => {
                    ContentType::sniff(&response.body).expect(ContentType::Swf)?;

                    let movie = SwfMovie::from_data(&response.body, response.url, loader_url)?;
                    player.lock().unwrap().set_root_movie(movie);
                    return Ok(());
                }
                Ok(response) => {
                    Loader::movie_loader_data(
                        handle,
                        player,
                        &response.body,
                        response.url,
                        response.status,
                        response.redirected,
                        loader_url,
                    )?;
                }
                Err(response) => {
                    tracing::error!(
                        "Error during movie loading of {:?}: {:?}",
                        response.url,
                        response.error
                    );
                    player.lock().unwrap().update(|uc| -> Result<(), Error> {
                        // FIXME - match Flash's error message

                        let (status_code, redirected) =
                            if let Error::HttpNotOk(_, status_code, redirected) = response.error {
                                (status_code, redirected)
                            } else {
                                (0, false)
                            };
                        Loader::movie_loader_error(
                            handle,
                            uc,
                            "Movie loader error".into(),
                            status_code,
                            redirected,
                            response.url,
                        )
                    })?;
                }
            }

            Ok(())
        })
    }

    fn movie_loader_bytes(
        &mut self,
        player: Weak<Mutex<Player>>,
        bytes: Vec<u8>,
    ) -> OwnedFuture<(), Error> {
        let handle = match self {
            Loader::Movie { self_handle, .. } => self_handle.expect("Loader not self-introduced"),
            _ => return Box::pin(async { Err(Error::NotMovieLoader) }),
        };

        let player = player
            .upgrade()
            .expect("Could not upgrade weak reference to player");

        Box::pin(async move {
            let mut replacing_root_movie = false;
            player.lock().unwrap().update(|uc| -> Result<(), Error> {
                let clip = match uc.load_manager.get_loader(handle) {
                    Some(Loader::Movie { target_clip, .. }) => *target_clip,
                    None => return Err(Error::Cancelled),
                    _ => unreachable!(),
                };

                replacing_root_movie = uc
                    .stage
                    .root_clip()
                    .map(|root| DisplayObject::ptr_eq(clip, root))
                    .unwrap_or(false);

                if let Some(mc) = clip.as_movie_clip() {
                    if !uc.is_action_script_3() {
                        mc.avm1_unload(uc);
                    }
                    mc.replace_with_movie(uc, None, false, None);
                }

                // NOTE: We do NOT call `movie_loader_start` as `loadBytes` does
                // not emit `open`
                Ok(())
            })?;

            if replacing_root_movie {
                ContentType::sniff(&bytes).expect(ContentType::Swf)?;

                let movie = SwfMovie::from_data(&bytes, "file:///".into(), None)?;
                player.lock().unwrap().set_root_movie(movie);
                return Ok(());
            }

            Loader::movie_loader_data(handle, player, &bytes, "file:///".into(), 0, false, None)
        })
    }

    fn form_loader(
        &mut self,
        player: Weak<Mutex<Player>>,
        request: Request,
    ) -> OwnedFuture<(), Error> {
        let handle = match self {
            Loader::Form { self_handle, .. } => self_handle.expect("Loader not self-introduced"),
            _ => return Box::pin(async { Err(Error::NotFormLoader) }),
        };

        let player = player
            .upgrade()
            .expect("Could not upgrade weak reference to player");

        Box::pin(async move {
            let fetch = player.lock().unwrap().navigator().fetch(request);

            let response = fetch.await.map_err(|e| e.error)?;

            // Fire the load handler.
            player.lock().unwrap().update(|uc| {
                let loader = uc.load_manager.get_loader(handle);
                let that = match loader {
                    Some(&Loader::Form { target_object, .. }) => target_object,
                    None => return Err(Error::Cancelled),
                    _ => return Err(Error::NotFormLoader),
                };

                let mut activation = Activation::from_stub(
                    uc.reborrow(),
                    ActivationIdentifier::root("[Form Loader]"),
                );

                for (k, v) in form_urlencoded::parse(&response.body) {
                    let k = AvmString::new_utf8(activation.context.gc_context, k);
                    let v = AvmString::new_utf8(activation.context.gc_context, v);
                    that.set(k, v.into(), &mut activation)?;
                }

                // Fire the onData method and event.
                if let Some(display_object) = that.as_display_object() {
                    if let Some(movie_clip) = display_object.as_movie_clip() {
                        activation.context.action_queue.queue_action(
                            movie_clip.into(),
                            ActionType::Method {
                                object: that,
                                name: "onData",
                                args: vec![],
                            },
                            false,
                        );
                        movie_clip.event_dispatch(&mut activation.context, ClipEvent::Data);
                    }
                }

                Ok(())
            })
        })
    }

    /// Creates a future for a LoadVars load call.
    fn load_vars_loader(
        &mut self,
        player: Weak<Mutex<Player>>,
        request: Request,
    ) -> OwnedFuture<(), Error> {
        let handle = match self {
            Loader::LoadVars { self_handle, .. } => {
                self_handle.expect("Loader not self-introduced")
            }
            _ => return Box::pin(async { Err(Error::NotLoadVarsLoader) }),
        };

        let player = player
            .upgrade()
            .expect("Could not upgrade weak reference to player");

        Box::pin(async move {
            let fetch = player.lock().unwrap().navigator().fetch(request);

            let data = fetch.await;

            // Fire the load handler.
            player.lock().unwrap().update(|uc| {
                let loader = uc.load_manager.get_loader(handle);
                let that = match loader {
                    Some(&Loader::LoadVars { target_object, .. }) => target_object,
                    None => return Err(Error::Cancelled),
                    _ => return Err(Error::NotLoadVarsLoader),
                };

                let mut activation =
                    Activation::from_stub(uc.reborrow(), ActivationIdentifier::root("[Loader]"));

                match data {
                    Ok(response) => {
                        let length = response.body.len();

                        // Set the properties used by the getBytesTotal and getBytesLoaded methods.
                        that.set("_bytesTotal", length.into(), &mut activation)?;
                        if length > 0 {
                            that.set("_bytesLoaded", length.into(), &mut activation)?;
                        }

                        let _ = that.call_method(
                            "onHTTPStatus".into(),
                            &[response.status.into()],
                            &mut activation,
                            ExecutionReason::Special,
                        );

                        // Fire the onData method with the loaded string.
                        // If the loaded data is an empty string, the load is considered unsuccessful.
                        let value_data = if length == 0 {
                            Value::Undefined
                        } else {
                            AvmString::new_utf8(
                                activation.context.gc_context,
                                UTF_8.decode(&response.body).0,
                            )
                            .into()
                        };
                        let _ = that.call_method(
                            "onData".into(),
                            &[value_data],
                            &mut activation,
                            ExecutionReason::Special,
                        );
                    }
                    Err(response) => {
                        // TODO: Log "Error opening URL" trace similar to the Flash Player?

                        let status_code =
                            if let Error::HttpNotOk(_, status_code, _) = response.error {
                                status_code
                            } else {
                                0
                            };

                        let _ = that.call_method(
                            "onHTTPStatus".into(),
                            &[status_code.into()],
                            &mut activation,
                            ExecutionReason::Special,
                        );

                        // Fire the onData method with no data to indicate an unsuccessful load.
                        let _ = that.call_method(
                            "onData".into(),
                            &[Value::Undefined],
                            &mut activation,
                            ExecutionReason::Special,
                        );
                    }
                }

                Ok(())
            })
        })
    }

    /// Creates a future for a LoadURLLoader load call.
    fn load_url_loader(
        &mut self,
        player: Weak<Mutex<Player>>,
        request: Request,
        data_format: DataFormat,
    ) -> OwnedFuture<(), Error> {
        let handle = match self {
            Loader::LoadURLLoader { self_handle, .. } => {
                self_handle.expect("Loader not self-introduced")
            }
            _ => return Box::pin(async { Err(Error::NotLoadDataLoader) }),
        };

        let player = player
            .upgrade()
            .expect("Could not upgrade weak reference to player");

        Box::pin(async move {
            let fetch = player.lock().unwrap().navigator().fetch(request);
            let response = fetch.await;

            player.lock().unwrap().update(|uc| {
                let loader = uc.load_manager.get_loader(handle);
                let target = match loader {
                    Some(&Loader::LoadURLLoader { target_object, .. }) => target_object,
                    // We would have already returned after the previous 'update' call
                    _ => unreachable!(),
                };

                let mut activation = Avm2Activation::from_nothing(uc.reborrow());

                fn set_data<'a, 'gc: 'a>(
                    body: Vec<u8>,
                    activation: &mut Avm2Activation<'a, 'gc>,
                    target: Avm2Object<'gc>,
                    data_format: DataFormat,
                ) {
                    let data_object = match data_format {
                        DataFormat::Binary => {
                            let storage = ByteArrayStorage::from_vec(body);
                            let bytearray =
                                ByteArrayObject::from_storage(activation, storage).unwrap();
                            bytearray.into()
                        }
                        DataFormat::Text => Avm2Value::String(AvmString::new_utf8_bytes(
                            activation.context.gc_context,
                            &body,
                        )),
                        DataFormat::Variables => {
                            tracing::warn!(
                                "Support for URLLoaderDataFormat.VARIABLES not yet implemented"
                            );
                            Avm2Value::Undefined
                        }
                    };

                    target
                        .set_public_property("data", data_object, activation)
                        .unwrap();
                }

                match response {
                    Ok(response) => {
                        let total_len = response.body.len();

                        // FIXME - the "open" event should be fired earlier, just before
                        // we start to fetch the data.
                        // However, the "open" event should not be fired if an IO error
                        // occurs opening the connection (e.g. if a file does not exist on disk).
                        // We currently have no way of detecting this, so we settle for firing
                        // the event after the entire fetch is complete. This causes there
                        // to a longer delay between the initial load triggered by the script
                        // and the "load" event firing, but it ensures that we match
                        // the Flash behavior w.r.t when an event is fired vs not fired.
                        let open_evt =
                            Avm2EventObject::bare_default_event(&mut activation.context, "open");
                        Avm2::dispatch_event(&mut activation.context, open_evt, target);
                        set_data(response.body, &mut activation, target, data_format);

                        // FIXME - we should fire "progress" events as we receive data, not
                        // just at the end
                        let progress_evt = activation
                            .avm2()
                            .classes()
                            .progressevent
                            .construct(
                                &mut activation,
                                &[
                                    "progress".into(),
                                    false.into(),
                                    false.into(),
                                    total_len.into(),
                                    total_len.into(),
                                ],
                            )
                            .map_err(|e| Error::Avm2Error(e.to_string()))?;

                        Avm2::dispatch_event(&mut activation.context, progress_evt, target);

                        let http_status_evt = activation
                            .avm2()
                            .classes()
                            .httpstatusevent
                            .construct(
                                &mut activation,
                                &[
                                    "httpStatus".into(),
                                    false.into(),
                                    false.into(),
                                    response.status.into(),
                                    response.redirected.into(),
                                ],
                            )
                            .map_err(|e| Error::Avm2Error(e.to_string()))?;

                        Avm2::dispatch_event(&mut activation.context, http_status_evt, target);

                        let complete_evt = Avm2EventObject::bare_default_event(
                            &mut activation.context,
                            "complete",
                        );
                        Avm2::dispatch_event(uc, complete_evt, target);
                    }
                    Err(response) => {
                        // Testing with Flash shoes that the 'data' property is cleared
                        // when an error occurs

                        set_data(Vec::new(), &mut activation, target, data_format);

                        let (status_code, redirected) =
                            if let Error::HttpNotOk(_, status_code, redirected) = response.error {
                                (status_code, redirected)
                            } else {
                                (0, false)
                            };
                        let http_status_evt = activation
                            .avm2()
                            .classes()
                            .httpstatusevent
                            .construct(
                                &mut activation,
                                &[
                                    "httpStatus".into(),
                                    false.into(),
                                    false.into(),
                                    status_code.into(),
                                    redirected.into(),
                                ],
                            )
                            .map_err(|e| Error::Avm2Error(e.to_string()))?;

                        Avm2::dispatch_event(&mut activation.context, http_status_evt, target);

                        // FIXME - Match the exact error message generated by Flash

                        let io_error_evt_cls = activation.avm2().classes().ioerrorevent;
                        let io_error_evt = io_error_evt_cls
                            .construct(
                                &mut activation,
                                &[
                                    "ioError".into(),
                                    false.into(),
                                    false.into(),
                                    "Error #2032: Stream Error".into(),
                                    2032.into(),
                                ],
                            )
                            .map_err(|e| Error::Avm2Error(e.to_string()))?;

                        Avm2::dispatch_event(uc, io_error_evt, target);
                    }
                }

                Ok(())
            })
        })
    }

    /// Creates a future for a Sound load call.
    fn sound_loader_avm1(
        &mut self,
        player: Weak<Mutex<Player>>,
        request: Request,
        is_streaming: bool,
    ) -> OwnedFuture<(), Error> {
        let handle = match self {
            Loader::SoundAvm1 { self_handle, .. } => {
                self_handle.expect("Loader not self-introduced")
            }
            _ => return Box::pin(async { Err(Error::NotLoadVarsLoader) }),
        };

        let player = player
            .upgrade()
            .expect("Could not upgrade weak reference to player");

        Box::pin(async move {
            let fetch = player.lock().unwrap().navigator().fetch(request);
            let data = fetch.await;

            // Fire the load handler.
            player.lock().unwrap().update(|uc| {
                let loader = uc.load_manager.get_loader(handle);
                let sound_object = match loader {
                    Some(&Loader::SoundAvm1 { target_object, .. }) => target_object,
                    None => return Err(Error::Cancelled),
                    _ => return Err(Error::NotSoundLoader),
                };

                let success = data
                    .map_err(|e| e.error)
                    .and_then(|data| {
                        let handle = uc.audio.register_mp3(&data.body)?;
                        sound_object.set_sound(uc.gc_context, Some(handle));
                        let duration = uc
                            .audio
                            .get_sound_duration(handle)
                            .map(|d| d.round() as u32);
                        sound_object.set_duration(uc.gc_context, duration);
                        Ok(())
                    })
                    .is_ok();

                let mut activation =
                    Activation::from_stub(uc.reborrow(), ActivationIdentifier::root("[Loader]"));
                let _ = sound_object.call_method(
                    "onLoad".into(),
                    &[success.into()],
                    &mut activation,
                    ExecutionReason::Special,
                );

                // Streaming sounds should auto-play.
                if is_streaming {
                    crate::avm1::start_sound(&mut activation, sound_object.into(), &[])?;
                }

                Ok(())
            })
        })
    }

    /// Creates a future for a LoadURLLoader load call.
    fn sound_loader_avm2(
        &mut self,
        player: Weak<Mutex<Player>>,
        request: Request,
    ) -> OwnedFuture<(), Error> {
        let handle = match self {
            Loader::SoundAvm2 { self_handle, .. } => {
                self_handle.expect("Loader not self-introduced")
            }
            _ => return Box::pin(async { Err(Error::NotLoadDataLoader) }),
        };

        let player = player
            .upgrade()
            .expect("Could not upgrade weak reference to player");

        Box::pin(async move {
            let fetch = player.lock().unwrap().navigator().fetch(request);
            let response = fetch.await;

            player.lock().unwrap().update(|uc| {
                let loader = uc.load_manager.get_loader(handle);
                let sound_object = match loader {
                    Some(&Loader::SoundAvm2 { target_object, .. }) => target_object,
                    None => return Err(Error::Cancelled),
                    _ => return Err(Error::NotSoundLoader),
                };

                match response {
                    Ok(response) => {
                        let handle = uc.audio.register_mp3(&response.body)?;
                        if let Err(e) = sound_object
                            .as_sound_object()
                            .expect("Not a sound object")
                            .set_sound(uc, handle)
                        {
                            tracing::error!("Encountered AVM2 error when setting sound: {}", e);
                        }

                        // FIXME - the "open" event should be fired earlier, and not fired in case of ioerror.
                        let mut activation = Avm2Activation::from_nothing(uc.reborrow());
                        let open_evt =
                            Avm2EventObject::bare_default_event(&mut activation.context, "open");
                        Avm2::dispatch_event(&mut activation.context, open_evt, sound_object);

                        let complete_evt = Avm2EventObject::bare_default_event(
                            &mut activation.context,
                            "complete",
                        );
                        Avm2::dispatch_event(uc, complete_evt, sound_object);
                    }
                    Err(_err) => {
                        // FIXME: Match the exact error message generated by Flash.
                        let mut activation = Avm2Activation::from_nothing(uc.reborrow());
                        let io_error_evt_cls = activation.avm2().classes().ioerrorevent;
                        let io_error_evt = io_error_evt_cls
                            .construct(
                                &mut activation,
                                &[
                                    "ioError".into(),
                                    false.into(),
                                    false.into(),
                                    "Error #2032: Stream Error".into(),
                                    2032.into(),
                                ],
                            )
                            .map_err(|e| Error::Avm2Error(e.to_string()))?;

                        Avm2::dispatch_event(uc, io_error_evt, sound_object);
                    }
                }

                Ok(())
            })
        })
    }

    fn stream_loader(
        &mut self,
        player: Weak<Mutex<Player>>,
        request: Request,
    ) -> OwnedFuture<(), Error> {
        let handle = match self {
            Loader::NetStream { self_handle, .. } => {
                self_handle.expect("Loader not self-introduced")
            }
            _ => return Box::pin(async { Err(Error::NotNetStreamLoader) }),
        };

        let player = player
            .upgrade()
            .expect("Could not upgrade weak reference to player");

        Box::pin(async move {
            let fetch = player.lock().unwrap().navigator().fetch(request);
            let response = fetch.await;

            player.lock().unwrap().update(|uc| {
                let loader = uc.load_manager.get_loader(handle);
                let stream = match loader {
                    Some(&Loader::NetStream { target_stream, .. }) => target_stream,
                    None => return Err(Error::Cancelled),
                    _ => return Err(Error::NotNetStreamLoader),
                };

                match response {
                    Ok(mut response) => {
                        stream.reset_buffer(uc);
                        stream.load_buffer(uc, &mut response.body);
                    }
                    Err(response) => {
                        stream.report_error(response.error);
                    }
                }

                Ok(())
            })
        })
    }

    /// Report a movie loader start event to script code.
    fn movie_loader_start(handle: Index, uc: &mut UpdateContext<'_, 'gc>) -> Result<(), Error> {
        let me = uc.load_manager.get_loader_mut(handle);
        if me.is_none() {
            return Err(Error::Cancelled);
        }

        let me = me.unwrap();

        let (clip, vm_data) = match me {
            Loader::Movie {
                target_clip,
                vm_data,
                ..
            } => (*target_clip, *vm_data),
            _ => unreachable!(),
        };

        match vm_data {
            MovieLoaderVMData::Avm1 { broadcaster } => {
                if let Some(broadcaster) = broadcaster {
                    Avm1::run_stack_frame_for_method(
                        clip,
                        broadcaster,
                        uc,
                        "broadcastMessage".into(),
                        &["onLoadStart".into(), clip.object()],
                    );
                }
            }
            MovieLoaderVMData::Avm2 { loader_info, .. } => {
                let mut activation = Avm2Activation::from_nothing(uc.reborrow());

                let open_evt = Avm2EventObject::bare_default_event(&mut activation.context, "open");
                Avm2::dispatch_event(uc, open_evt, loader_info);
            }
        }

        Ok(())
    }

    /// Load data into a movie loader.
    fn movie_loader_data(
        handle: Handle,
        player: Arc<Mutex<Player>>,
        data: &[u8],
        url: String,
        status: u16,
        redirected: bool,
        loader_url: Option<String>,
    ) -> Result<(), Error> {
        let sniffed_type = ContentType::sniff(data);
        let length = data.len();

        if sniffed_type == ContentType::Unknown {
            if let Ok(data) = extract_swz(data) {
                return Self::movie_loader_data(
                    handle, player, &data, url, status, redirected, loader_url,
                );
            }
        }
        player.lock().unwrap().update(|uc| {
            let (clip, vm_data) = match uc.load_manager.get_loader(handle) {
                Some(Loader::Movie {
                    target_clip,
                    vm_data,
                    ..
                }) => (*target_clip, *vm_data),
                None => return Err(Error::Cancelled),
                _ => unreachable!(),
            };

            let mut activation = Avm2Activation::from_nothing(uc.reborrow());

            let domain = if let MovieLoaderVMData::Avm2 {
                context,
                default_domain,
                ..
            } = vm_data
            {
                let domain = context
                    .and_then(|o| {
                        o.get_public_property("applicationDomain", &mut activation)
                            .ok()
                    })
                    .and_then(|v| v.coerce_to_object(&mut activation).ok())
                    .and_then(|o| o.as_application_domain())
                    .unwrap_or_else(|| {
                        let parent_domain = default_domain;
                        Avm2Domain::movie_domain(&mut activation, parent_domain)
                    });
                Some(domain)
            } else {
                None
            };

            let movie = match sniffed_type {
                ContentType::Swf => Arc::new(SwfMovie::from_data(data, url.clone(), loader_url)?),
                ContentType::Gif | ContentType::Jpeg | ContentType::Png => {
                    Arc::new(SwfMovie::from_loaded_image(url.clone(), length))
                }
                ContentType::Unknown => Arc::new(SwfMovie::error_movie(url.clone())),
            };

            match activation.context.load_manager.get_loader_mut(handle) {
                Some(Loader::Movie {
                    movie: old,
                    loader_status,
                    ..
                }) => {
                    *loader_status = LoaderStatus::Parsing;
                    *old = Some(movie.clone())
                }
                _ => unreachable!(),
            };

            if let MovieLoaderVMData::Avm2 { loader_info, .. } = vm_data {
                let fake_movie = Arc::new(SwfMovie::empty_fake_compressed_len(
                    activation.context.swf.version(),
                    length,
                ));

                // Expose 'bytesTotal' (via the fake movie) during the first 'progress' event,
                // but nothing else (in particular, the `parameters` and `url` properties are not set
                // to their real values)
                loader_info
                    .as_loader_info_object()
                    .unwrap()
                    .set_loader_stream(
                        LoaderStream::NotYetLoaded(fake_movie, Some(clip), false),
                        activation.context.gc_context,
                    );

                // Flash always fires an initial 'progress' event with
                // bytesLoaded=0 and bytesTotal set to the proper value.
                // This only seems to happen for an AVM2 event handler
                Loader::movie_loader_progress(handle, &mut activation.context, 0, length)?;

                // Update the LoaderStream - we now have a real SWF movie and a real target clip
                // This is intentionally set *after* the first 'progress' event, to match Flash's behavior
                // (`LoaderInfo.parameters` is always empty during the first 'progress' event)
                loader_info
                    .as_loader_info_object()
                    .unwrap()
                    .set_loader_stream(
                        LoaderStream::NotYetLoaded(movie.clone(), Some(clip), false),
                        activation.context.gc_context,
                    );
            }

            match sniffed_type {
                ContentType::Swf => {
                    let library = activation
                        .context
                        .library
                        .library_for_movie_mut(movie.clone());

                    if let Some(domain) = domain {
                        library.set_avm2_domain(domain);
                    }

                    if let Some(mc) = clip.as_movie_clip() {
                        let loader_info =
                            if let MovieLoaderVMData::Avm2 { loader_info, .. } = vm_data {
                                Some(*loader_info.as_loader_info_object().unwrap())
                            } else {
                                None
                            };

                        // Store our downloaded `SwfMovie` into our target `MovieClip`,
                        // and initialize it.

                        mc.replace_with_movie(
                            &mut activation.context,
                            Some(movie),
                            true,
                            loader_info,
                        );
                    }

                    // NOTE: Certain tests specifically expect small files to preload immediately
                    Loader::preload_tick(
                        handle,
                        uc,
                        &mut ExecutionLimit::with_max_ops_and_time(10000, Duration::from_millis(1)),
                        status,
                        redirected,
                    )?;

                    return Ok(());
                }
                ContentType::Gif | ContentType::Jpeg | ContentType::Png => {
                    let library = activation.context.library.library_for_movie_mut(movie);

                    if let Some(domain) = domain {
                        library.set_avm2_domain(domain);
                    }

                    // This will construct AVM2-side objects even under AVM1, but it doesn't matter,
                    // since Bitmap and BitmapData never have AVM1-side objects.
                    let bitmap = ruffle_render::utils::decode_define_bits_jpeg(data, None)?;

                    let transparency = true;
                    let bitmap_data = BitmapData::new_with_pixels(
                        bitmap.width(),
                        bitmap.height(),
                        transparency,
                        bitmap.as_colors().map(Color::from).collect(),
                    );
                    let bitmapdata_wrapper = BitmapDataWrapper::new(GcCell::new(
                        activation.context.gc_context,
                        bitmap_data,
                    ));
                    let bitmapdata_class = activation.context.avm2.classes().bitmapdata;
                    let bitmapdata_avm2 = BitmapDataObject::from_bitmap_data_internal(
                        &mut activation,
                        bitmapdata_wrapper,
                        bitmapdata_class,
                    )
                    .unwrap();

                    let bitmap_avm2 = activation
                        .avm2()
                        .classes()
                        .bitmap
                        .construct(&mut activation, &[bitmapdata_avm2.into()])
                        .unwrap();
                    let bitmap_obj = bitmap_avm2.as_display_object().unwrap();

                    Loader::movie_loader_progress(handle, &mut activation.context, length, length)?;
                    Loader::movie_loader_complete(
                        handle,
                        &mut activation.context,
                        Some(bitmap_obj),
                        status,
                        redirected,
                    )?;
                }
                ContentType::Unknown => {
                    if activation.context.is_action_script_3() {
                        Loader::movie_loader_progress(
                            handle,
                            &mut activation.context,
                            length,
                            length,
                        )?;
                        Loader::movie_loader_error(
                            handle,
                            uc,
                            AvmString::new_utf8(
                                uc.gc_context,
                                &format!("Error #2124: Loaded file is an unknown type. URL: {url}"),
                            ),
                            status,
                            redirected,
                            url,
                        )?;
                    } else {
                        // If the file is no valid supported file, the MovieClip enters the error state
                        if let Some(mut mc) = clip.as_movie_clip() {
                            Loader::load_error_swf(&mut mc, &mut activation.context, url.clone());
                        }

                        // AVM1 fires the event with the current and total length as 0
                        Loader::movie_loader_progress(handle, &mut activation.context, 0, 0)?;
                        Loader::movie_loader_complete(
                            handle,
                            &mut activation.context,
                            None,
                            status,
                            redirected,
                        )?;
                    }
                }
            }

            Ok(())
        }) //TODO: content sniffing errors need to be reported somehow
    }

    /// Report a movie loader progress event to script code.
    ///
    /// The current and total length are always reported as compressed lengths.
    fn movie_loader_progress(
        handle: Index,
        uc: &mut UpdateContext<'_, 'gc>,
        cur_len: usize,
        total_len: usize,
    ) -> Result<(), Error> {
        let me = uc.load_manager.get_loader_mut(handle);
        if me.is_none() {
            return Err(Error::Cancelled);
        }

        let me = me.unwrap();

        let (clip, vm_data) = match me {
            Loader::Movie {
                target_clip,
                vm_data,
                ..
            } => (*target_clip, *vm_data),
            _ => unreachable!(),
        };

        match vm_data {
            MovieLoaderVMData::Avm1 { broadcaster } => {
                if let Some(broadcaster) = broadcaster {
                    Avm1::run_stack_frame_for_method(
                        clip,
                        broadcaster,
                        uc,
                        "broadcastMessage".into(),
                        &[
                            "onLoadProgress".into(),
                            clip.object(),
                            cur_len.into(),
                            total_len.into(),
                        ],
                    );
                }
            }
            MovieLoaderVMData::Avm2 { loader_info, .. } => {
                let mut activation = Avm2Activation::from_nothing(uc.reborrow());

                let progress_evt = activation
                    .avm2()
                    .classes()
                    .progressevent
                    .construct(
                        &mut activation,
                        &[
                            "progress".into(),
                            false.into(),
                            false.into(),
                            cur_len.into(),
                            total_len.into(),
                        ],
                    )
                    .map_err(|e| Error::Avm2Error(e.to_string()))?;

                Avm2::dispatch_event(uc, progress_evt, loader_info);
            }
        }

        Ok(())
    }

    /// Report a movie loader completion to script code.
    fn movie_loader_complete(
        handle: Index,
        uc: &mut UpdateContext<'_, 'gc>,
        dobj: Option<DisplayObject<'gc>>,
        status: u16,
        redirected: bool,
    ) -> Result<(), Error> {
        let (target_clip, vm_data, movie) = match uc.load_manager.get_loader_mut(handle) {
            Some(Loader::Movie {
                target_clip,
                movie,
                vm_data,
                ..
            }) => (*target_clip, *vm_data, movie.clone()),
            None => return Err(Error::Cancelled),
            _ => unreachable!(),
        };

        let loader_info = if let MovieLoaderVMData::Avm2 { loader_info, .. } = vm_data {
            Some(*loader_info.as_loader_info_object().unwrap())
        } else {
            None
        };

        if let Some(loader_info) = loader_info {
            // Store the real movie into the `LoaderStream`, so that
            // 'bytesTotal' starts returning the correct value
            // (we previously had a fake empty SwfMovie).
            // However, we still use `LoaderStream::NotYetLoaded`, since
            // the actual MovieClip display object has not run its first
            // frame yet.
            loader_info.set_loader_stream(
                LoaderStream::NotYetLoaded(movie.clone().unwrap(), Some(dobj.unwrap()), false),
                uc.gc_context,
            );
        }

        if let Some(dobj) = dobj {
            if dobj.as_movie_clip().is_some() {
                // We call these methods after we initialize the `LoaderInfo`, but before
                // we add the loaded clip as a child. The frame constructor should see
                // 'this.parent == null' and 'this.stage == null'
                dobj.post_instantiation(uc, None, Instantiator::Movie, false);
                catchup_display_object_to_frame(uc, dobj);
                // Movie clips created from ActionScript (including from a Loader) skip the next enterFrame,
                // and consequently are observed to have their currentFrame lag one
                // frame behind objects placed by the timeline (even if they were
                // both placed in the same frame to begin with).
                dobj.base_mut(uc.gc_context).set_skip_next_enter_frame(true);

                let flashvars = movie.clone().unwrap().parameters().to_owned();
                if !flashvars.is_empty() {
                    let mut activation = Activation::from_nothing(
                        uc.reborrow(),
                        ActivationIdentifier::root("[Loader]"),
                        dobj,
                    );
                    let object = dobj.object().coerce_to_object(&mut activation);
                    for (key, value) in flashvars.iter() {
                        object.define_value(
                            activation.context.gc_context,
                            AvmString::new_utf8(activation.context.gc_context, key),
                            AvmString::new_utf8(activation.context.gc_context, value).into(),
                            Attribute::empty(),
                        );
                    }
                }
            }
        }

        if let MovieLoaderVMData::Avm2 { loader_info, .. } = vm_data {
            let domain = uc
                .library
                .library_for_movie(movie.unwrap())
                .unwrap()
                .avm2_domain();
            let mut activation = Avm2Activation::from_domain(uc.reborrow(), domain);
            let mut loader = loader_info
                .get_public_property("loader", &mut activation)
                .map_err(|e| Error::Avm2Error(e.to_string()))?
                .as_object()
                .unwrap()
                .as_display_object()
                .unwrap()
                .as_container()
                .unwrap();

            // Note that we do *not* use the 'addChild' method here:
            // Per the flash docs, our implementation always throws
            // an 'unsupported' error. Also, the AVM2 side of our movie
            // clip does not yet exist. Any children added inside the movie
            // frame constructor will see an 'added' event immediately, and
            // an 'addedToStage' event *after* the constructor finishes
            // when we add the movie as a child of the loader.
            loader.insert_at_index(&mut activation.context, dobj.unwrap(), 0);
        } else if let Some(dobj) = dobj {
            // This is a load of an image into AVM1 - add it as a child of the target clip.
            if dobj.as_movie_clip().is_none() {
                let mc = target_clip.as_movie_clip().unwrap();
                mc.replace_with_movie(uc, Some(movie.unwrap()), true, None);
                mc.replace_at_depth(uc, dobj, 1);

                // This sets the MovieClip image state correctly.
                mc.set_current_frame(uc.gc_context, 1);
                mc.set_cur_preload_frame(uc.gc_context, 2);
            }
        }

        match vm_data {
            MovieLoaderVMData::Avm1 { broadcaster } => {
                if let Some(broadcaster) = broadcaster {
                    Avm1::run_stack_frame_for_method(
                        target_clip,
                        broadcaster,
                        uc,
                        "broadcastMessage".into(),
                        // TODO: Pass an actual httpStatus argument instead of 0.
                        &["onLoadComplete".into(), target_clip.object(), status.into()],
                    );
                }
            }
            // This is fired after we process the movie's first frame,
            // in `MovieClip.on_exit_frame`
            MovieLoaderVMData::Avm2 { loader_info, .. } => {
                let loader_info_obj = loader_info.as_loader_info_object().unwrap();
                loader_info_obj.set_loader_stream(
                    LoaderStream::Swf(target_clip.as_movie_clip().unwrap().movie(), dobj.unwrap()),
                    uc.gc_context,
                );

                if let Some(dobj) = dobj {
                    if dobj.as_movie_clip().is_none() {
                        loader_info_obj.fire_init_and_complete_events(uc, status, redirected);
                    }
                }
            }
        }

        if let Loader::Movie { loader_status, .. } = uc.load_manager.get_loader_mut(handle).unwrap()
        {
            *loader_status = LoaderStatus::Succeeded;
        };

        Ok(())
    }

    /// Report a movie loader error to script code.
    ///
    /// This is an associated function because we cannot borrow both the update
    /// context and one of it's loaders.
    fn movie_loader_error(
        handle: Index,
        uc: &mut UpdateContext<'_, 'gc>,
        msg: AvmString<'gc>,
        status: u16,
        redirected: bool,
        swf_url: String,
    ) -> Result<(), Error> {
        //TODO: Inspect the fetch error.
        //This requires cooperation from the backend to send abstract
        //error types we can actually inspect.
        //This also can get errors from decoding an invalid SWF file,
        //too. We should distinguish those to player code.
        let (clip, vm_data) = match uc.load_manager.get_loader_mut(handle) {
            Some(Loader::Movie {
                target_clip,
                vm_data,
                ..
            }) => (*target_clip, *vm_data),
            None => return Err(Error::Cancelled),
            _ => unreachable!(),
        };

        // If the SWF can't be loaded, the MovieClip enters the error state
        if let Some(mut mc) = clip.as_movie_clip() {
            Loader::load_error_swf(&mut mc, uc, swf_url);
        }

        match vm_data {
            MovieLoaderVMData::Avm1 { broadcaster } => {
                if let Some(broadcaster) = broadcaster {
                    Avm1::run_stack_frame_for_method(
                        clip,
                        broadcaster,
                        uc,
                        "broadcastMessage".into(),
                        &[
                            "onLoadError".into(),
                            clip.object(),
                            "LoadNeverCompleted".into(),
                        ],
                    );
                }
            }
            MovieLoaderVMData::Avm2 { loader_info, .. } => {
                let mut activation = Avm2Activation::from_nothing(uc.reborrow());

                let http_status_evt = activation
                    .avm2()
                    .classes()
                    .httpstatusevent
                    .construct(
                        &mut activation,
                        &[
                            "httpStatus".into(),
                            false.into(),
                            false.into(),
                            status.into(),
                            redirected.into(),
                        ],
                    )
                    .map_err(|e| Error::Avm2Error(e.to_string()))?;

                Avm2::dispatch_event(&mut activation.context, http_status_evt, loader_info);

                // FIXME - Match the exact error message generated by Flash

                let io_error_evt_cls = activation.avm2().classes().ioerrorevent;
                let io_error_evt = io_error_evt_cls
                    .construct(
                        &mut activation,
                        &[
                            "ioError".into(),
                            false.into(),
                            false.into(),
                            msg.into(),
                            0.into(),
                        ],
                    )
                    .map_err(|e| Error::Avm2Error(e.to_string()))?;

                Avm2::dispatch_event(uc, io_error_evt, loader_info);
            }
        }

        if let Loader::Movie { loader_status, .. } = uc.load_manager.get_loader_mut(handle).unwrap()
        {
            *loader_status = LoaderStatus::Failed;
        };

        Ok(())
    }

    /// This makes the MovieClip enter the initial loading state in which some
    /// attributes have certain initial loading values to signal that the file is
    /// currently being loaded and neither an error has occurred nor the first frame
    /// has been successfully loaded yet.
    fn load_initial_loading_swf(
        mc: &mut MovieClip<'gc>,
        uc: &mut UpdateContext<'_, 'gc>,
        request_url: &str,
        resolved_url: Result<Url, ParseError>,
    ) {
        match resolved_url {
            Err(_) => {
                Loader::load_error_swf(mc, uc, request_url.to_string());
            }
            Ok(url) => {
                // If the loaded SWF is a local file, the initial loading state equals the error state.
                if url.scheme() == "file" {
                    Loader::load_error_swf(mc, uc, url.to_string());
                } else {
                    // Replacing the movie sets total_frames and and frames_loaded correctly.
                    // The movie just needs to be the default empty movie with the correct URL.
                    // In this loading state, the URL is the URL of the parent movie / doesn't change.

                    let current_movie = mc.movie();
                    let current_version = current_movie.version();
                    let current_url = current_movie.url();
                    let mut initial_loading_movie = SwfMovie::empty(current_version);
                    initial_loading_movie.set_url(current_url.to_string());

                    mc.replace_with_movie(uc, Some(Arc::new(initial_loading_movie)), true, None);

                    // Maybe this (keeping the current URL) should be the default behaviour
                    // of replace_with_movie?
                    // TODO: See where it gets invoked without a movie as well and what the
                    // correct URL result is in these cases.
                }
            }
        }
    }

    /// This makes the MovieClip enter the error state in which some attributes have
    /// certain error values to signal that no valid file could be loaded.
    /// An error state movie stub which provides the correct values is created and
    /// loaded.
    ///
    /// This happens if no file could be loaded or if the loaded content is no valid
    /// supported content.
    ///
    /// swf_url is always the final URL obtained after any redirects.
    fn load_error_swf(
        mc: &mut MovieClip<'gc>,
        uc: &mut UpdateContext<'_, 'gc>,
        mut swf_url: String,
    ) {
        // If a local URL is fetched using the flash plugin, the _url property
        // won't be changed => It keeps being the parent SWF URL.
        if cfg!(target_family = "wasm") {
            if let Ok(url) = Url::parse(&swf_url) {
                if url.scheme() == "file" {
                    swf_url = mc.movie().url().to_string();
                }
            }
        };

        let error_movie = SwfMovie::error_movie(swf_url);
        // This also sets total_frames correctly
        mc.replace_with_movie(uc, Some(Arc::new(error_movie)), true, None);
        mc.set_cur_preload_frame(uc.gc_context, 0);
    }

    /// Event handler morally equivalent to `onLoad` on a movie clip.
    ///
    /// Returns `true` if the loader has completed and should be removed.
    ///
    /// Used to fire listener events on clips and terminate completed loaders.
    fn movie_clip_loaded(&mut self, queue: &mut ActionQueue<'gc>) -> bool {
        let (clip, vm_data, loader_status) = match self {
            Loader::Movie {
                target_clip,
                vm_data,
                loader_status,
                ..
            } => (*target_clip, *vm_data, *loader_status),
            _ => return false,
        };

        match loader_status {
            LoaderStatus::Pending => false,
            LoaderStatus::Parsing => false,
            LoaderStatus::Failed => true,
            LoaderStatus::Succeeded => {
                // AVM2 is handled separately
                if let MovieLoaderVMData::Avm1 {
                    broadcaster: Some(broadcaster),
                } = vm_data
                {
                    queue.queue_action(
                        clip,
                        ActionType::Method {
                            object: broadcaster,
                            name: "broadcastMessage",
                            args: vec!["onLoadInit".into(), clip.object()],
                        },
                        false,
                    );
                }
                true
            }
        }
    }
}
