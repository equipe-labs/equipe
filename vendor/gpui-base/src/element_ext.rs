use gpui::{App, Bounds, IntoElement, ParentElement, Pixels, Styled as _, Window, canvas};

use crate::TextSelectionScopeId;

/// Extends a GPUI parent element with post-layout prepaint observation.
pub trait ElementExt: ParentElement + Sized {
    /// Marks this element subtree as belonging to a text-selection scope.
    fn text_selection_scope(self, scope: TextSelectionScopeId) -> impl IntoElement
    where
        Self: IntoElement,
    {
        crate::text_selection::text_selection_scope(scope, self)
    }

    /// Invokes `callback` during prepaint with this element's resolved bounds.
    fn on_prepaint<F>(self, callback: F) -> Self
    where
        F: FnOnce(Bounds<Pixels>, &mut Window, &mut App) + 'static,
    {
        self.child(
            canvas(
                move |bounds, window, cx| callback(bounds, window, cx),
                |_, _, _, _| {},
            )
            // equipe: pinned to the element's top-left. Without insets an
            // absolute child sits where it would have flowed — in a block
            // `div`, under the children before it — so the bounds reported
            // were the element's size moved down by its content's height
            // (the ticket card's, for the pointer: `views/ticket_card.rs`).
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    }
}

impl<T: ParentElement> ElementExt for T {}
