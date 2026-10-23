use std::time::Duration;

use criterion::{Criterion, measurement::WallTime};

/// Keep expensive whole-command and filesystem workloads practical in continuous runs.
pub(crate) fn walltime_criterion() -> Criterion<WallTime> {
    Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
}
