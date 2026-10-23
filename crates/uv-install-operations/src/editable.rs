use std::sync::Arc;

use uv_configuration::EditableMode;
use uv_distribution_types::{DirectorySourceDist, Dist, Resolution, ResolvedDist, SourceDist};

/// Apply editable installation overrides to local directory distributions.
pub fn apply_editable_mode(resolution: Resolution, editable: Option<EditableMode>) -> Resolution {
    let Some(editable) = editable else {
        return resolution;
    };

    resolution.map(|dist| {
        let ResolvedDist::Installable { dist, version } = dist else {
            return None;
        };
        let Dist::Source(SourceDist::Directory(DirectorySourceDist {
            name,
            install_path,
            mode: current_mode,
            first_party,
            url,
        })) = dist.as_ref()
        else {
            return None;
        };

        let editable = editable.for_package(name)?;
        let mode = current_mode.with_editable(editable);
        if *current_mode == mode {
            return None;
        }

        Some(ResolvedDist::Installable {
            dist: Arc::new(Dist::Source(SourceDist::Directory(DirectorySourceDist {
                name: name.clone(),
                install_path: install_path.clone(),
                mode,
                first_party: *first_party,
                url: url.clone(),
            }))),
            version: version.clone(),
        })
    })
}
