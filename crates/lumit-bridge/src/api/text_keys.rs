//! Keyed words on a text layer: the words its keys point at, and the edits the
//! Text row's stopwatch and box make. The engine works out the word at a time
//! and where a key goes, so the row only shows and forwards.

use flutter_rust_bridge::frb;
use lumit_core::model::{LayerKind, TextDocument};
use lumit_core::text::keys::TextKeys;
use lumit_core::time::Rational;

use crate::api::{effect::BridgeScalar, layer::LayerReference, BridgeError};

/// The words a text layer's keys point at, and the keys. Each key's value is
/// a whole index into `words`. A held key switches the words at the next key,
/// a linear or eased one fades into them.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTextKeys {
    pub words: Vec<String>,
    /// On the composition's clock, like every other channel.
    pub index: BridgeScalar,
}

#[frb(ignore)]
pub(crate) fn read(keys: Option<&TextKeys>, offset: Rational) -> Option<BridgeTextKeys> {
    keys.map(|keys| BridgeTextKeys {
        words: keys.words.clone(),
        index: BridgeScalar::read_at(&keys.index, offset),
    })
}

/// The keys as the model holds them. A key value is rounded to a whole index,
/// so a key added partway through a fade points at a word rather than between.
#[frb(ignore)]
pub(crate) fn write(
    keys: Option<BridgeTextKeys>,
    offset: Rational,
) -> Result<Option<Box<TextKeys>>, BridgeError> {
    let Some(keys) = keys else {
        return Ok(None);
    };
    let mut animation = keys.index.animation_at(offset)?;
    #[allow(clippy::cast_precision_loss)] // a short list of words
    let last = keys.words.len().saturating_sub(1) as f64;
    if let lumit_core::anim::Animation::Keyframed(list) = &mut animation {
        for key in list {
            key.value = key.value.round().clamp(0.0, last);
        }
    }
    Ok(Some(Box::new(TextKeys {
        words: keys.words,
        index: lumit_core::anim::Property {
            animation,
            extra: serde_json::Map::new(),
        },
    })))
}

impl LayerReference {
    /// The text document and the layer time of comp frame `frame`.
    #[frb(ignore)]
    fn text_at_frame(&self, frame: i64) -> Result<(TextDocument, Rational), BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let layer = comp
            .layers
            .iter()
            .find(|l| l.id == self.layer_id)
            .ok_or(BridgeError::InvalidLayer)?;
        let LayerKind::Text { document } = &layer.kind else {
            return Err(BridgeError::NotText);
        };
        let lt = comp
            .frame_rate
            .time_of_frame(frame)
            .ok()
            .and_then(|t| t.0.checked_sub(layer.start_offset.0).ok())
            .ok_or(BridgeError::InvalidTime)?;
        Ok((document.clone(), lt))
    }

    #[frb(ignore)]
    fn put_text_document(&self, document: TextDocument) -> Result<(), BridgeError> {
        self.commit(lumit_core::ops::Op::SetTextDocument {
            comp: self.comp_id,
            layer: self.layer_id,
            document,
        })
    }

    /// The words typed or keyed at comp frame `frame`. During a fade, the word
    /// with more of the weight.
    #[frb(sync)]
    pub fn text_at(&self, frame: i64) -> Result<String, BridgeError> {
        let (document, lt) = self.text_at_frame(frame)?;
        Ok(match &document.keys {
            Some(keys) => keys.word_at(lt.to_f64()).to_owned(),
            None => document.text,
        })
    }

    /// Type `text` at comp frame `frame`: a held key there when the words are
    /// keyed, otherwise the words themselves. One undo step.
    #[frb(sync)]
    pub fn set_text_key_at(&self, frame: i64, text: String) -> Result<(), BridgeError> {
        let (mut document, lt) = self.text_at_frame(frame)?;
        match document.keys.take() {
            Some(keys) => document.keys = Some(Box::new(keys.with_word_at(lt, &text))),
            None => document.text = text,
        }
        self.put_text_document(document)
    }

    /// The stopwatch on: the words held from comp frame `frame`.
    #[frb(sync)]
    pub fn start_text_keys(&self, frame: i64) -> Result<(), BridgeError> {
        let (mut document, lt) = self.text_at_frame(frame)?;
        if document.keys.is_some() {
            return Ok(());
        }
        document.keys = Some(Box::new(TextKeys::start(&document.text, lt)));
        self.put_text_document(document)
    }

    /// The stopwatch off: the words shown at comp frame `frame` are kept.
    #[frb(sync)]
    pub fn stop_text_keys(&self, frame: i64) -> Result<(), BridgeError> {
        let (mut document, lt) = self.text_at_frame(frame)?;
        let Some(keys) = document.keys.take() else {
            return Ok(());
        };
        document.text = keys.stop(lt.to_f64());
        self.put_text_document(document)
    }
}
