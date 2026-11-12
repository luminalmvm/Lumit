//! The Viewer's matte view: one node's key shown in a view, preview only
//! ([`lumit_core::grade::matte_view`]).

use flutter_rust_bridge::frb;
use uuid::Uuid;

use lumit_core::grade::matte_view::{MatteMode, MatteView};
use lumit_core::grade::Shot;

use crate::api::composition::CompositionReference;
use crate::api::worker_thread::WorkerRequest;
use crate::api::BridgeError;

/// How a view shows a node's key.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMatteView {
    Off,
    /// The picture where the key is, flat grey elsewhere.
    Highlight,
    /// The key itself, black and white.
    Key,
}

/// What Highlight shows where the key is not.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMatteBackdrop {
    Grey,
    Black,
    White,
    /// The picture in black and white.
    Mono,
}

impl CompositionReference {
    /// Show the key of the Correction `effect` in Viewer view `view`: the
    /// shot (`layer`, or with `clip` one clip on it) drawn alone, its stack
    /// stopping at that node, Highlight over `backdrop`. Drag previews show
    /// it too, and an export never does. The frontend asks for the frame under the playhead after a
    /// change, as it does after a look.
    #[frb(sync)]
    pub fn set_matte_view(
        &self,
        view: u32,
        mode: BridgeMatteView,
        layer: Uuid,
        clip: Option<Uuid>,
        effect: Uuid,
        backdrop: BridgeMatteBackdrop,
    ) -> Result<(), BridgeError> {
        let mode = match (mode, backdrop) {
            (BridgeMatteView::Off, _) => None,
            (BridgeMatteView::Key, _) => Some(MatteMode::Key),
            (BridgeMatteView::Highlight, BridgeMatteBackdrop::Grey) => Some(MatteMode::Highlight),
            (BridgeMatteView::Highlight, BridgeMatteBackdrop::Black) => {
                Some(MatteMode::HighlightBlack)
            }
            (BridgeMatteView::Highlight, BridgeMatteBackdrop::White) => {
                Some(MatteMode::HighlightWhite)
            }
            (BridgeMatteView::Highlight, BridgeMatteBackdrop::Mono) => {
                Some(MatteMode::HighlightMono)
            }
        };
        let matte = mode.map(|mode| MatteView {
            shot: Shot {
                comp: self.id,
                layer,
                clip,
            },
            effect,
            mode,
        });
        self.dispatch(WorkerRequest::SetMatteView { view, matte })
    }
}
