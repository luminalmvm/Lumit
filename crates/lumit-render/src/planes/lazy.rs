//! A model opened the first time it is asked for anything. An analysis opens its
//! frames first, and for a comp that makes a render device. On DirectML, making
//! one after a model has opened removed the model's device (887A0005), so the
//! first Analyse of a comp in a run of Lumit failed and the next one worked.

use super::{open_model, PlaneFailure, PlaneModel, PlaneOut, PlaneSettings};
use std::cell::OnceCell;

pub(super) struct Lazy {
    settings: PlaneSettings,
    opened: OnceCell<Result<Box<dyn PlaneModel>, PlaneFailure>>,
}

impl Lazy {
    pub(super) fn new(settings: PlaneSettings) -> Self {
        Lazy {
            settings,
            opened: OnceCell::new(),
        }
    }

    fn open(&self) -> &Result<Box<dyn PlaneModel>, PlaneFailure> {
        self.opened.get_or_init(|| open_model(self.settings))
    }
}

impl PlaneModel for Lazy {
    fn run(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<PlaneOut, PlaneFailure> {
        self.open();
        match self.opened.get_mut() {
            Some(Ok(model)) => model.run(rgba, w, h),
            Some(Err(why)) => Err(*why),
            None => Err(PlaneFailure::ModelFailed),
        }
    }

    // A model that won't open has made nothing, and `run` says why.
    fn made_with(&self) -> String {
        self.open()
            .as_ref()
            .map_or_else(|_| String::new(), |model| model.made_with())
    }
}
