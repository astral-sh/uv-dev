use uv_cache::Removal;

#[test]
fn aggregate_removal_summaries() {
    for (left, right, expected) in [
        ((None, false), (None, false), (None, false)),
        ((None, false), (Some(5), false), (None, false)),
        ((None, false), (Some(5), true), (None, false)),
        ((Some(3), false), (None, false), (None, false)),
        ((Some(3), true), (None, false), (None, false)),
        ((Some(3), false), (Some(5), false), (Some(8), false)),
        ((Some(3), false), (Some(5), true), (Some(8), true)),
        ((Some(3), true), (Some(5), false), (Some(8), true)),
        ((Some(3), true), (Some(5), true), (Some(8), true)),
    ] {
        let mut summary = Removal {
            num_files: 2,
            num_dirs: 3,
            coarse_bytes: 7,
            fine_bytes: left.0,
            fine_bytes_incomplete: left.1,
        };
        summary += Removal {
            num_files: 5,
            num_dirs: 11,
            coarse_bytes: 13,
            fine_bytes: right.0,
            fine_bytes_incomplete: right.1,
        };

        assert_eq!(summary.num_files, 7);
        assert_eq!(summary.num_dirs, 14);
        assert_eq!(summary.coarse_bytes, 20);
        assert_eq!(
            (summary.fine_bytes, summary.fine_bytes_incomplete),
            expected,
            "{left:?} + {right:?}",
        );
    }
}

#[test]
fn fine_removal_bytes_saturate() {
    for (left, right, expected) in [
        (0, u64::MAX, u64::MAX),
        (u64::MAX, 0, u64::MAX),
        (u64::MAX - 1, 1, u64::MAX),
        (1, u64::MAX - 1, u64::MAX),
        (u64::MAX - 1, 2, u64::MAX),
        (2, u64::MAX - 1, u64::MAX),
        (u64::MAX, u64::MAX, u64::MAX),
    ] {
        let mut summary = Removal {
            fine_bytes: Some(left),
            ..Removal::default()
        };
        summary += Removal {
            fine_bytes: Some(right),
            ..Removal::default()
        };

        assert_eq!(summary.fine_bytes, Some(expected), "{left} + {right}");
        assert!(!summary.fine_bytes_incomplete);
    }
}
