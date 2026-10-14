use std::sync::Arc;

use uv_configuration::EditableMode;
use uv_distribution_types::{Dist, Resolution, ResolvedDist, SourceDist};

/// Apply editable installation overrides to local directory distributions.
pub fn apply_editable_mode(resolution: Resolution, editable: Option<EditableMode>) -> Resolution {
    let Some(editable) = editable else {
        return resolution;
    };

    resolution.map(|dist| {
        let ResolvedDist::Installable { dist, version } = dist else {
            return None;
        };
        let Dist::Source(SourceDist::Directory(directory)) = dist.as_ref() else {
            return None;
        };

        let editable = editable.for_package(&directory.name)?;
        if directory.editable == Some(editable) {
            return None;
        }

        let mut directory = directory.clone();
        directory.editable = Some(editable);
        Some(ResolvedDist::Installable {
            dist: Arc::new(Dist::Source(SourceDist::Directory(directory))),
            version: version.clone(),
        })
    })
}
