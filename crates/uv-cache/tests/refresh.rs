use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, UNIX_EPOCH};

use uv_cache::Refresh;
use uv_cache_info::Timestamp;
use uv_normalize::PackageName;

#[derive(Debug, PartialEq, Eq)]
enum Selection {
    None,
    Packages(Vec<PackageName>, Vec<Box<Path>>),
    All,
}

fn split(refresh: Refresh) -> (Selection, Timestamp) {
    match refresh {
        Refresh::None(timestamp) => (Selection::None, timestamp),
        Refresh::Packages(packages, paths, timestamp) => {
            (Selection::Packages(packages, paths), timestamp)
        }
        Refresh::All(timestamp) => (Selection::All, timestamp),
    }
}

#[test]
fn combine_refresh_policies() {
    let older = Timestamp::from(UNIX_EPOCH);
    let newer = Timestamp::from(UNIX_EPOCH + Duration::from_secs(1));
    let packages = |names: &[&str]| {
        names
            .iter()
            .map(|name| PackageName::from_str(name).unwrap())
            .collect()
    };
    let paths = |names: &[&str]| names.iter().map(|name| Path::new(name).into()).collect();
    let left = |timestamp| {
        Refresh::Packages(
            packages(&["left", "shared"]),
            paths(&["left", "shared"]),
            timestamp,
        )
    };
    let right = |timestamp| {
        Refresh::Packages(
            packages(&["shared", "right"]),
            paths(&["shared", "right"]),
            timestamp,
        )
    };
    let combined = |timestamp| {
        Refresh::Packages(
            packages(&["left", "shared", "shared", "right"]),
            paths(&["left", "shared", "shared", "right"]),
            timestamp,
        )
    };
    let empty = |timestamp| Refresh::Packages(vec![], vec![], timestamp);

    for (left_timestamp, right_timestamp) in [(older, newer), (newer, older), (newer, newer)] {
        let cases = [
            (
                Refresh::None(left_timestamp),
                Refresh::None(right_timestamp),
                Refresh::None(newer),
            ),
            (
                Refresh::None(left_timestamp),
                right(right_timestamp),
                right(newer),
            ),
            (
                Refresh::None(left_timestamp),
                Refresh::All(right_timestamp),
                Refresh::All(newer),
            ),
            (
                left(left_timestamp),
                Refresh::None(right_timestamp),
                left(newer),
            ),
            (
                left(left_timestamp),
                right(right_timestamp),
                combined(newer),
            ),
            (
                left(left_timestamp),
                Refresh::All(right_timestamp),
                Refresh::All(newer),
            ),
            (
                Refresh::All(left_timestamp),
                Refresh::None(right_timestamp),
                Refresh::All(newer),
            ),
            (
                Refresh::All(left_timestamp),
                right(right_timestamp),
                Refresh::All(newer),
            ),
            (
                Refresh::All(left_timestamp),
                Refresh::All(right_timestamp),
                Refresh::All(newer),
            ),
            (
                Refresh::None(left_timestamp),
                empty(right_timestamp),
                empty(newer),
            ),
            (
                empty(left_timestamp),
                Refresh::None(right_timestamp),
                empty(newer),
            ),
            (empty(left_timestamp), empty(right_timestamp), empty(newer)),
        ];

        for (left, right, expected) in cases {
            assert_eq!(split(left.combine(right)), split(expected));
        }
    }
}
