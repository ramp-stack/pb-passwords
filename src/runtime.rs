//! Runtime-only clipboard expiry. All visual composition belongs to CHK.
use crate::secret::Secret;
use chk::{Action, ActionItem, Context, Icons, Theme};
use prism::{
    drawable::{Drawable, Instruction, LayoutTree, Offset, Rect, Size},
    events::{ChildEvent, DynEvent, Event, ListenCallback, ListenFuture, Timer},
};
use std::{any::Any, future::poll_fn, task::Poll, time::Duration};
use zeroize::Zeroize;

#[derive(Debug, Clone)]
struct CopyText(Secret);
impl Event for CopyText {
    fn pass(&self, _: &mut Context, children: &mut [ChildEvent<'_, Self>]) {
        for child in children {
            *child.event = Some(self.clone());
        }
    }
}
pub fn copy_action(label: &str, value: &str) -> ActionItem {
    let secret = Secret::new(value);
    ActionItem::new_with_active(
        Action::custom(move |ctx: &mut Context, _: &Theme| ctx.emit(CopyText(secret.clone()))),
        label,
        "Copied",
        Icons::Copy,
    )
}
#[derive(Clone, Debug)]
pub struct ClipboardUi {
    body: Box<dyn Drawable>,
    clipboard: Secret,
    timer: Timer,
}
impl ClipboardUi {
    pub fn new(body: Box<dyn Drawable>) -> Self {
        Self {
            body,
            clipboard: Secret::default(),
            timer: Timer::default(),
        }
    }
    #[cfg(test)]
    pub(crate) fn expire_copy_now(&mut self) {
        self.timer.reset(Duration::ZERO);
    }

    pub(crate) fn clear_clipboard(&mut self, ctx: &mut Context) {
        if !self.clipboard.0.borrow().is_empty()
            && let Some(prism::hardware::Data::Text(mut current)) = ctx.hardware.get_clipboard()
        {
            if current.as_str() == self.clipboard.0.borrow().as_str() {
                ctx.hardware
                    .set_clipboard(&prism::hardware::Data::Text(String::new()));
            }
            current.zeroize();
        }
        self.clipboard.clear();
        self.timer.cancel();
    }
}
impl Drawable for ClipboardUi {
    fn request_width(&mut self) -> (f32, f32) {
        self.body.request_width()
    }
    fn request_height(&mut self, width: f32) -> (f32, f32) {
        self.body.request_height(width)
    }
    fn layout(&mut self, size: Size) -> LayoutTree {
        self.body.layout(size)
    }
    fn draw(&self, layout: &LayoutTree, offset: Offset, bound: Rect) -> Vec<Instruction> {
        self.body.draw(layout, offset, bound)
    }
    fn event(&mut self, ctx: &mut Context, layout: &LayoutTree, event: Box<dyn DynEvent>) {
        if let Some(CopyText(value)) = event.downcast_ref::<CopyText>() {
            self.clipboard.clear();
            self.clipboard = Secret::new(&value.0.borrow());
            ctx.hardware
                .set_clipboard(&prism::hardware::Data::Text(value.0.borrow().to_string()));
            self.timer.reset(Duration::from_secs(30));
        } else {
            self.body.event(ctx, layout, event);
        }
    }
    fn changed(&mut self) -> ListenFuture<'_> {
        Box::pin(async move {
            let mut body = self.body.changed();
            let mut timer = Box::pin(self.timer.next());
            let callback = poll_fn(|ctx| {
                if let Poll::Ready(()) = timer.as_mut().poll(ctx) {
                    return Poll::Ready(None);
                }
                body.as_mut().poll(ctx).map(Some)
            })
            .await;
            Box::new(move |target: &mut dyn Any, ctx: &mut Context| {
                let target = target.downcast_mut::<Self>().unwrap();
                if let Some(callback) = callback {
                    callback(&mut target.body as &mut dyn Any, ctx);
                } else {
                    target.clear_clipboard(ctx);
                }
            }) as ListenCallback
        })
    }
}
