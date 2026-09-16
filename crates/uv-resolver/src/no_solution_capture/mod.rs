//! Internal, opt-in, bounded evidence for the original resolver failure.
//!
//! This protocol is for source-bound resolver tests, not ordinary diagnostics or a public lock
//! format. A complete capture is not a verified PubGrub proof. In particular, its observations
//! cannot establish that an independently served package inventory was empty.

mod budget;
mod producer;
mod reader;
mod wire;

#[cfg(test)]
mod test_support;

use std::fmt;

pub use reader::{CaptureReadError, CaptureWriteError};
pub use wire::{
    CaptureMetadata, CaptureOperation, CaptureOptions, CaptureScope, CaptureStatus, CaptureToken,
};

pub(crate) use producer::CaptureContext;

/// A checked internal capture. Its wire representation can only be reconstructed through the
/// allocation-bounded reader.
pub struct NoSolutionEvidence(wire::EvidenceWire);

impl NoSolutionEvidence {
    /// Read a complete envelope under the fixed version-1 resource limits.
    pub fn from_json(bytes: &[u8], token: &CaptureToken) -> Result<Self, CaptureReadError> {
        reader::read(bytes, token, budget::CaptureLimits::V1)
    }

    /// Serialize the envelope without exceeding its byte budget.
    pub fn to_json(&self) -> Result<Vec<u8>, CaptureWriteError> {
        reader::encode(&self.0)
    }

    pub fn matches(&self, token: &CaptureToken) -> bool {
        self.0.request == token.request() && self.0.producer_pid == token.producer_pid()
    }

    pub const fn status(&self) -> CaptureStatus {
        self.0.status
    }
}

impl fmt::Debug for NoSolutionEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NoSolutionEvidence")
            .field("status", &self.0.status)
            .field("reason", &self.0.reason)
            .field("usage", &self.0.usage)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod api_boundary {
    use serde::de::DeserializeOwned;

    use super::NoSolutionEvidence;

    #[test]
    fn evidence_has_no_unchecked_deserialize_implementation() {
        trait AmbiguousIfDeserialize<Marker> {
            fn check() {}
        }
        impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
        struct HasDeserialize;
        impl<T: DeserializeOwned> AmbiguousIfDeserialize<HasDeserialize> for T {}

        // Type inference becomes ambiguous if a Deserialize derive is added to the public handle.
        let _ = <NoSolutionEvidence as AmbiguousIfDeserialize<_>>::check;
    }
}
