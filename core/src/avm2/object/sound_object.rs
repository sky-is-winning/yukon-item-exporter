//! Object representation for sounds

use crate::avm2::activation::Activation;
use crate::avm2::object::script_object::ScriptObjectData;
use crate::avm2::object::{ClassObject, Object, ObjectPtr, TObject};
use crate::avm2::value::Value;
use crate::avm2::Error;
use crate::backend::audio::SoundHandle;
use crate::context::UpdateContext;
use crate::display_object::SoundTransform;
use core::fmt;
use gc_arena::{Collect, GcCell, GcWeakCell, Mutation};
use std::cell::{Ref, RefMut};
use swf::SoundInfo;

use super::SoundChannelObject;

/// A class instance allocator that allocates Sound objects.
pub fn sound_allocator<'gc>(
    class: ClassObject<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Object<'gc>, Error<'gc>> {
    let base = ScriptObjectData::new(class);

    Ok(SoundObject(GcCell::new(
        activation.context.gc_context,
        SoundObjectData {
            base,
            sound_data: SoundData::NotLoaded {
                queued_plays: Vec::new(),
            },
        },
    ))
    .into())
}

#[derive(Clone, Collect, Copy)]
#[collect(no_drop)]
pub struct SoundObject<'gc>(pub GcCell<'gc, SoundObjectData<'gc>>);

#[derive(Clone, Collect, Copy, Debug)]
#[collect(no_drop)]
pub struct SoundObjectWeak<'gc>(pub GcWeakCell<'gc, SoundObjectData<'gc>>);

impl fmt::Debug for SoundObject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SoundObject")
            .field("ptr", &self.0.as_ptr())
            .finish()
    }
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct SoundObjectData<'gc> {
    /// Base script object
    base: ScriptObjectData<'gc>,

    /// The sound this object holds.
    sound_data: SoundData<'gc>,
}

#[derive(Collect)]
#[collect(no_drop)]
pub enum SoundData<'gc> {
    NotLoaded {
        queued_plays: Vec<QueuedPlay<'gc>>,
    },
    Loaded {
        #[collect(require_static)]
        sound: SoundHandle,
    },
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct QueuedPlay<'gc> {
    #[collect(require_static)]
    pub sound_info: SoundInfo,
    #[collect(require_static)]
    pub sound_transform: Option<SoundTransform>,
    pub sound_channel: SoundChannelObject<'gc>,
    pub position: f64,
}

impl<'gc> SoundObject<'gc> {
    pub fn sound_handle(self) -> Option<SoundHandle> {
        let this = self.0.read();
        match this.sound_data {
            SoundData::NotLoaded { .. } => None,
            SoundData::Loaded { sound } => Some(sound),
        }
    }

    /// Returns `true` if a `SoundChannel` should be returned back to the AVM2 caller.
    pub fn play(
        self,
        queued: QueuedPlay<'gc>,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<bool, Error<'gc>> {
        let mut this = self.0.write(activation.context.gc_context);
        match &mut this.sound_data {
            SoundData::NotLoaded { queued_plays } => {
                queued_plays.push(queued);
                // We don't know the length yet, so return the `SoundChannel`
                Ok(true)
            }
            SoundData::Loaded { sound } => play_queued(queued, *sound, activation),
        }
    }

    pub fn set_sound(
        self,
        context: &mut UpdateContext<'_, 'gc>,
        sound: SoundHandle,
    ) -> Result<(), Error<'gc>> {
        let mut this = self.0.write(context.gc_context);
        let mut activation = Activation::from_nothing(context.reborrow());
        match &mut this.sound_data {
            SoundData::NotLoaded { queued_plays } => {
                for queued in std::mem::take(queued_plays) {
                    play_queued(queued, sound, &mut activation)?;
                }
                this.sound_data = SoundData::Loaded { sound };
            }
            SoundData::Loaded { sound: old_sound } => {
                panic!("Tried to replace sound {old_sound:?} with {sound:?}")
            }
        }
        Ok(())
    }
}

/// Returns `true` if the sound had a valid position, and `false` otherwise
fn play_queued<'gc>(
    queued: QueuedPlay<'gc>,
    sound: SoundHandle,
    activation: &mut Activation<'_, 'gc>,
) -> Result<bool, Error<'gc>> {
    if let Some(duration) = activation.context.audio.get_sound_duration(sound) {
        if queued.position > duration {
            tracing::error!(
                "Sound.play: position={} is greater than duration={}",
                queued.position,
                duration
            );
            return Ok(false);
        }
    }

    if let Some(instance) = activation
        .context
        .start_sound(sound, &queued.sound_info, None, None)
    {
        if let Some(sound_transform) = queued.sound_transform {
            activation
                .context
                .set_local_sound_transform(instance, sound_transform);
        }

        queued
            .sound_channel
            .as_sound_channel()
            .unwrap()
            .set_sound_instance(activation, instance);

        activation
            .context
            .attach_avm2_sound_channel(instance, queued.sound_channel);
    }
    Ok(true)
}

impl<'gc> TObject<'gc> for SoundObject<'gc> {
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

    fn as_sound_object(self) -> Option<SoundObject<'gc>> {
        Some(self)
    }
}
