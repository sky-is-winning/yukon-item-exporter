use crate::avm2::Activation;
use crate::avm2::Object as Avm2Object;
use crate::context::RenderContext;
use crate::context::UpdateContext;
use crate::display_object::InteractiveObject;
use crate::display_object::TInteractiveObject;
use crate::display_object::{DisplayObjectBase, DisplayObjectPtr, TDisplayObject};
use crate::events::{ClipEvent, ClipEventResult};
use crate::prelude::*;

use crate::display_object::container::ChildContainer;
use crate::display_object::interactive::InteractiveObjectBase;
use crate::tag_utils::SwfMovie;
use core::fmt;
use gc_arena::GcWeakCell;
use gc_arena::{Collect, GcCell, Mutation};
use std::cell::{Ref, RefMut};
use std::sync::Arc;

use super::interactive::Avm2MousePick;

#[derive(Clone, Collect, Copy)]
#[collect(no_drop)]
pub struct LoaderDisplay<'gc>(GcCell<'gc, LoaderDisplayData<'gc>>);

impl fmt::Debug for LoaderDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoaderDisplay")
            .field("ptr", &self.0.as_ptr())
            .finish()
    }
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct LoaderDisplayData<'gc> {
    base: InteractiveObjectBase<'gc>,
    container: ChildContainer<'gc>,
    avm2_object: Option<Avm2Object<'gc>>,
    movie: Arc<SwfMovie>,
}

impl<'gc> LoaderDisplay<'gc> {
    pub fn empty(activation: &mut Activation<'_, 'gc>, movie: Arc<SwfMovie>) -> Self {
        let obj = LoaderDisplay(GcCell::new(
            activation.context.gc_context,
            LoaderDisplayData {
                base: Default::default(),
                container: ChildContainer::new(),
                avm2_object: None, // Set later after assignment
                movie,
            },
        ));

        obj.set_placed_by_script(activation.context.gc_context, true);
        activation.context.avm2.add_orphan_obj(obj.into());
        obj
    }

    pub fn downgrade(self) -> LoaderDisplayWeak<'gc> {
        LoaderDisplayWeak(GcCell::downgrade(self.0))
    }
}

impl<'gc> TDisplayObject<'gc> for LoaderDisplay<'gc> {
    fn base(&self) -> Ref<DisplayObjectBase<'gc>> {
        Ref::map(self.0.read(), |r| &r.base.base)
    }

    fn base_mut<'a>(&'a self, mc: &Mutation<'gc>) -> RefMut<'a, DisplayObjectBase<'gc>> {
        RefMut::map(self.0.write(mc), |w| &mut w.base.base)
    }

    fn instantiate(&self, gc_context: &Mutation<'gc>) -> DisplayObject<'gc> {
        Self(GcCell::new(gc_context, self.0.read().clone())).into()
    }

    fn as_ptr(&self) -> *const DisplayObjectPtr {
        self.0.as_ptr() as *const DisplayObjectPtr
    }

    fn id(&self) -> CharacterId {
        u16::MAX
    }

    fn render_self(&self, context: &mut RenderContext<'_, 'gc>) {
        self.render_children(context);
    }

    fn self_bounds(&self) -> Rectangle<Twips> {
        Default::default()
    }

    fn object2(&self) -> Avm2Value<'gc> {
        self.0
            .read()
            .avm2_object
            .map(Avm2Value::from)
            .unwrap_or(Avm2Value::Null)
    }

    fn set_object2(&self, context: &mut UpdateContext<'_, 'gc>, to: Avm2Object<'gc>) {
        self.0.write(context.gc_context).avm2_object = Some(to);
    }

    fn as_container(self) -> Option<DisplayObjectContainer<'gc>> {
        Some(self.into())
    }

    fn as_interactive(self) -> Option<InteractiveObject<'gc>> {
        Some(self.into())
    }

    fn enter_frame(&self, context: &mut UpdateContext<'_, 'gc>) {
        let skip_frame = self.base().should_skip_next_enter_frame();
        for child in self.iter_render_list() {
            // See MovieClip::enter_frame for an explanation of this.
            if skip_frame {
                child
                    .base_mut(context.gc_context)
                    .set_skip_next_enter_frame(true);
            }
            child.enter_frame(context);
        }
        self.base_mut(context.gc_context)
            .set_skip_next_enter_frame(false);
    }

    fn construct_frame(&self, context: &mut UpdateContext<'_, 'gc>) {
        for child in self.iter_render_list() {
            child.construct_frame(context);
        }
    }

    fn movie(&self) -> Arc<SwfMovie> {
        self.0.read().movie.clone()
    }

    fn set_parent(&self, context: &mut UpdateContext<'_, 'gc>, parent: Option<DisplayObject<'gc>>) {
        let had_parent = self.parent().is_some();
        self.base_mut(context.gc_context)
            .set_parent_ignoring_orphan_list(parent);
        let has_parent = self.parent().is_some();

        if context.is_action_script_3() && had_parent && !has_parent {
            context.avm2.add_orphan_obj((*self).into())
        }
    }
}

impl<'gc> TInteractiveObject<'gc> for LoaderDisplay<'gc> {
    fn raw_interactive(&self) -> Ref<InteractiveObjectBase<'gc>> {
        Ref::map(self.0.read(), |r| &r.base)
    }

    fn raw_interactive_mut(&self, mc: &Mutation<'gc>) -> RefMut<InteractiveObjectBase<'gc>> {
        RefMut::map(self.0.write(mc), |w| &mut w.base)
    }

    fn as_displayobject(self) -> DisplayObject<'gc> {
        self.into()
    }

    fn filter_clip_event(
        self,
        _context: &mut UpdateContext<'_, 'gc>,
        _event: ClipEvent,
    ) -> ClipEventResult {
        ClipEventResult::NotHandled
    }
    fn event_dispatch(
        self,
        _context: &mut UpdateContext<'_, 'gc>,
        _event: ClipEvent<'gc>,
    ) -> ClipEventResult {
        ClipEventResult::NotHandled
    }

    fn mouse_pick_avm1(
        &self,
        context: &mut UpdateContext<'_, 'gc>,
        point: Point<Twips>,
        require_button_mode: bool,
    ) -> Option<InteractiveObject<'gc>> {
        for child in self.iter_render_list().rev() {
            if let Some(int) = child.as_interactive() {
                if let Some(result) = int.mouse_pick_avm1(context, point, require_button_mode) {
                    return Some(result);
                }
            }
        }

        None
    }

    fn mouse_pick_avm2(
        &self,
        context: &mut UpdateContext<'_, 'gc>,
        point: Point<Twips>,
        require_button_mode: bool,
    ) -> Avm2MousePick<'gc> {
        // We have at most one child
        if let Some(child) = self.iter_render_list().next() {
            if let Some(int) = child.as_interactive() {
                return int
                    .mouse_pick_avm2(context, point, require_button_mode)
                    .combine_with_parent((*self).into());
            }
        }
        Avm2MousePick::Miss
    }
}

impl<'gc> TDisplayObjectContainer<'gc> for LoaderDisplay<'gc> {
    fn raw_container(&self) -> Ref<'_, ChildContainer<'gc>> {
        Ref::map(self.0.read(), |this| &this.container)
    }

    fn raw_container_mut(&self, gc_context: &Mutation<'gc>) -> RefMut<'_, ChildContainer<'gc>> {
        RefMut::map(self.0.write(gc_context), |this| &mut this.container)
    }
}

#[derive(Clone, Debug, Collect, Copy)]
#[collect(no_drop)]
pub struct LoaderDisplayWeak<'gc>(GcWeakCell<'gc, LoaderDisplayData<'gc>>);

impl<'gc> LoaderDisplayWeak<'gc> {
    pub fn upgrade(self, mc: &Mutation<'gc>) -> Option<LoaderDisplay<'gc>> {
        self.0.upgrade(mc).map(LoaderDisplay)
    }

    pub fn as_ptr(self) -> *const DisplayObjectPtr {
        self.0.as_ptr() as *const DisplayObjectPtr
    }
}
