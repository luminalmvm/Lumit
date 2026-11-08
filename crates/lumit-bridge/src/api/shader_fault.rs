//! Why an inner shader graph will not compile, as facts rather than a sentence
//! (docs/impl/custom-shader.md §2.2).
//!
//! The compiler's own words name a box by its stored id, which nobody can see
//! on the canvas, and they are English. So the canvas is told the kind of
//! fault and the box and socket it is about, and says it in the user's
//! language while drawing the wire it means.

use flutter_rust_bridge::frb;
use lumit_core::fx::shader::compile::GraphError;

/// The kinds of fault a graph can have.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeShaderFault {
    /// The stored graph is not one this build can read: it does not parse,
    /// names a kind of box this build has never heard of, gives two boxes one
    /// id, or has a wire to a box or socket that is not there.
    Unreadable,
    NoResult,
    ManyResults,
    DoubleInput,
    Cycle,
    /// A wire carries a value its input cannot take.
    TypeMismatch,
    /// A Parameter box, or a swizzle's pattern, holds a setting the shader
    /// cannot carry.
    BadParameter,
}

/// One fault, with the box and the input it is about where it is about one.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeShaderGraphError {
    pub fault: BridgeShaderFault,
    pub node: Option<u32>,
    /// The input's id, as the box's ports name it.
    pub port: Option<String>,
}

impl From<BridgeShaderFault> for BridgeShaderGraphError {
    fn from(fault: BridgeShaderFault) -> Self {
        Self {
            fault,
            node: None,
            port: None,
        }
    }
}

impl From<GraphError> for BridgeShaderGraphError {
    fn from(why: GraphError) -> Self {
        match why {
            GraphError::TypeMismatch { node, port } => Self {
                fault: BridgeShaderFault::TypeMismatch,
                node: Some(node),
                port: Some(port),
            },
            GraphError::NoResult => BridgeShaderFault::NoResult.into(),
            GraphError::ManyResults => BridgeShaderFault::ManyResults.into(),
            GraphError::DoubleInput => BridgeShaderFault::DoubleInput.into(),
            GraphError::Cycle => BridgeShaderFault::Cycle.into(),
            GraphError::BadParameter(_) => BridgeShaderFault::BadParameter.into(),
            GraphError::UnknownKind(_) | GraphError::DuplicateNode(_) | GraphError::Dangling => {
                BridgeShaderFault::Unreadable.into()
            }
        }
    }
}
