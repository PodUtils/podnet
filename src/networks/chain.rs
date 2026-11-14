//! A chain of networks in one spec: `TOP over BELOW` (T-117).
//!
//! The words ` over ` cut the spec into layers. The first layer is the top,
//! as [`crate::network::Chain::label`] writes it. Each layer comes up over
//! the way of the layer below it ([`crate::network::Ctx::below`]). Example:
//! `wg:/etc/wg0.conf over warp` sends the UDP of the WireGuard tunnel
//! through the datagrams of a WARP session.
//!
//! A spec with `+exec:` is not cut: a command can hold the word `over`.

use std::sync::Arc;

use super::{parse, usage, SpecNetwork};
use crate::network::{Caps, Chain, Failure, Network};

/// The most layers of one spec.
pub const MAX_LAYERS: usize = 8;

/// The word between two layers.
const OVER: &str = " over ";

/// True when `spec` names more than one layer.
pub(super) fn is_chain(spec: &str) -> bool {
    format!(" {spec} ").contains(OVER) && !spec.contains("+exec:")
}

/// The layers of `spec`, the lowest first. An error is one line that names
/// the layer.
pub(super) fn parse_layers(spec: &str) -> Result<Vec<SpecNetwork>, Failure> {
    // The spaces around the spec keep a word over at an end as a cut.
    let padded = format!(" {spec} ");
    let parts: Vec<&str> = padded.split(OVER).map(str::trim).collect();
    if parts.len() > MAX_LAYERS {
        return Err(usage(format!(
            "a chain has at most {MAX_LAYERS} layers, and {} are given",
            parts.len()
        )));
    }
    let mut layers = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            return Err(usage(
                "a chain needs a network on each side of the word over (TOP over BELOW)".into(),
            ));
        }
        let layer = parse(part).map_err(|e| {
            Failure::new(
                e.stage,
                e.class,
                format!("layer {} of the chain: {}", i + 1, e.text()),
            )
        })?;
        layers.push(layer);
    }
    layers.reverse();
    Ok(layers)
}

/// The label of the layers (the lowest first): the top first, with ` over `.
pub(super) fn label(layers: &[SpecNetwork]) -> String {
    let names: Vec<String> = layers.iter().rev().map(SpecNetwork::label).collect();
    names.join(OVER)
}

/// What the top layer gives.
pub(super) fn caps(layers: &[SpecNetwork]) -> Caps {
    layers.last().map_or(Caps::NONE, |top| top.caps())
}

/// The chain of the layers.
pub(super) fn of(layers: &[SpecNetwork]) -> Result<Chain, Failure> {
    let layers = layers
        .iter()
        .map(|l| Arc::new(l.clone()) as Arc<dyn Network>)
        .collect();
    Chain::new(layers)
}
