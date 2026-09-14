use std::cell::Cell;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};

use anyhow::Result;
use uv_install_wheel::Layout;
use uv_pypi_types::Scheme;

use super::fixture::OwnedLeafTrial;
use super::{leaf, settings, timing};

#[test]
fn mutation_settings_require_one_complete_explicit_tuple() -> Result<()> {
    assert!(settings::read([None, None, None])?.is_none());
    for mask in 1..7 {
        let supplied = std::array::from_fn(|index| {
            (mask & (1 << index) != 0).then(|| OsString::from("supplied"))
        });
        let error = settings::read(supplied).expect_err("a partial opt-in must fail");
        assert_eq!(
            error.to_string(),
            format!("set all of {} together", settings::INPUT_NAMES.join(", "))
        );
    }

    let root = tempfile::tempdir()?;
    let wheel = root.path().join("input.whl");
    fs_err::write(&wheel, b"input bytes")?;
    let hash = "00".repeat(32);
    let tuple = || {
        [
            Some(root.path().as_os_str().to_owned()),
            Some(wheel.as_os_str().to_owned()),
            Some(OsString::from(&hash)),
        ]
    };
    let inputs = settings::read(tuple())?.expect("a complete tuple enables measurements");
    assert_eq!(inputs.scratch, fs_err::canonicalize(root.path())?);
    assert_eq!(inputs.wheel, wheel);
    assert_eq!(inputs.sha256, hash);
    let mut malformed = tuple();
    malformed[2] = Some(OsString::from("not-a-sha256"));
    assert_eq!(
        settings::read(malformed)
            .expect_err("a malformed full digest must fail before fixture creation")
            .to_string(),
        "UV_BENCH_WHEEL_SHA256 must contain 64 hexadecimal digits"
    );
    let mut missing_scratch = tuple();
    missing_scratch[0] = Some(root.path().join("missing").into_os_string());
    assert!(settings::read(missing_scratch).is_err());
    Ok(())
}

fn error_trial() -> Result<OwnedLeafTrial> {
    let root = tempfile::tempdir()?;
    fs_err::write(root.path().join("regular"), b"regular\n")?;
    fs_err::create_dir(root.path().join("directory"))?;
    fs_err::write(root.path().join("directory/child"), b"unrelated\n")?;
    fs_err::write(root.path().join("later"), b"later\n")?;
    OwnedLeafTrial::new(
        root,
        ["regular", "missing", "directory", "later"]
            .map(PathBuf::from)
            .into(),
    )
}

#[test]
fn portable_backends_attempt_later_leaves_after_independent_errors() -> Result<()> {
    let trial = error_trial()?;
    let raw = leaf::raw(&trial);
    let expected = raw.iter().map(leaf::comparable_raw).collect::<Vec<_>>();
    trial.assert_leaf_results(&raw)?;
    assert_eq!(leaf::successful(&raw), 2);
    assert_eq!(
        expected[1].map_err(|(kind, _)| kind),
        Err(io::ErrorKind::NotFound)
    );
    assert!(expected[2].is_err());
    assert_eq!(expected[3], Ok(()));

    for threads in [None, Some(1), Some(4), Some(16)] {
        let trial = error_trial()?;
        let actual = if let Some(threads) = threads {
            leaf::with_workers(&trial, &leaf::worker_pool(threads))
        } else {
            leaf::ordinary(&trial)
        };
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_eq!(leaf::comparable(actual), expected.map_err(|(kind, _)| kind));
        }
        assert_eq!(actual.len(), expected.len());
        assert_eq!(leaf::successful(&actual), 2);
        trial.assert_leaf_results(&actual)?;
    }
    Ok(())
}

#[cfg(unix)]
fn symlink_trial() -> Result<OwnedLeafTrial> {
    let root = tempfile::tempdir()?;
    fs_err::create_dir_all(root.path().join("targets/directory"))?;
    fs_err::write(root.path().join("targets/file"), b"target\n")?;
    fs_err::write(root.path().join("targets/directory/child"), b"child\n")?;
    for (target, name) in [
        ("targets/file", "file-link"),
        ("targets/directory", "directory-link"),
        ("targets/missing", "dangling-link"),
    ] {
        fs_err::os::unix::fs::symlink(target, root.path().join(name))?;
    }
    OwnedLeafTrial::new(
        root,
        ["file-link", "directory-link", "dangling-link"]
            .map(PathBuf::from)
            .into(),
    )
}

#[cfg(unix)]
#[test]
fn portable_backends_remove_leaf_symlinks_without_following_them() -> Result<()> {
    let trial = symlink_trial()?;
    let expected = leaf::raw(&trial);
    assert_eq!(leaf::successful(&expected), 3);
    trial.assert_leaf_results(&expected)?;
    for threads in [None, Some(1), Some(4), Some(16)] {
        let trial = symlink_trial()?;
        let actual = if let Some(threads) = threads {
            leaf::with_workers(&trial, &leaf::worker_pool(threads))
        } else {
            leaf::ordinary(&trial)
        };
        assert_eq!(leaf::successful(&actual), 3);
        trial.assert_leaf_results(&actual)?;
    }
    Ok(())
}

fn layout(root: &Path) -> Layout {
    let site_packages = root.join("site-packages");
    let scripts = root.join("bin");
    Layout {
        sys_executable: scripts.join(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        }),
        python_version: (3, 12),
        os_name: if cfg!(windows) { "nt" } else { "posix" }.to_owned(),
        scheme: Scheme {
            purelib: site_packages.clone(),
            platlib: site_packages,
            scripts,
            data: root.to_path_buf(),
            include: root.join("include"),
        },
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "The production error is compared with the original operating-system error"
)]
fn production_stops_at_an_unrecoverable_record_path_error() -> Result<()> {
    let root = tempfile::tempdir()?;
    let layout = layout(root.path());
    let dist_info = layout.scheme.purelib.join("fixture-1.0.0.dist-info");
    fs_err::create_dir_all(&dist_info)?;
    fs_err::write(layout.scheme.purelib.join("before"), b"before\n")?;
    fs_err::write(layout.scheme.purelib.join("blocker"), b"not a directory\n")?;
    fs_err::write(layout.scheme.purelib.join("after"), b"after\n")?;
    fs_err::write(
        dist_info.join("RECORD"),
        b"before,,\nblocker/child,,\nafter,,\nfixture-1.0.0.dist-info/RECORD,,\n",
    )?;
    let raw = std::fs::remove_file(layout.scheme.purelib.join("blocker/child"))
        .expect_err("a file cannot be traversed as a directory");
    assert_eq!(raw.kind(), io::ErrorKind::NotADirectory);
    assert!(raw.raw_os_error().is_some());

    let error = uv_install_wheel::uninstall_wheel(&dist_info, "fixture", &layout)
        .expect_err("the serial production loop must stop at the failed path");
    let uv_install_wheel::Error::Io(error) = error else {
        anyhow::bail!("production returned a non-I/O error");
    };
    assert_eq!(error.kind(), raw.kind());
    assert!(!layout.scheme.purelib.join("before").try_exists()?);
    assert_eq!(
        fs_err::read(layout.scheme.purelib.join("after"))?,
        b"after\n"
    );
    assert_eq!(
        fs_err::read(layout.scheme.purelib.join("blocker"))?,
        b"not a directory\n"
    );
    assert!(dist_info.join("RECORD").try_exists()?);
    Ok(())
}

#[test]
fn production_parses_the_complete_record_before_mutating() -> Result<()> {
    let root = tempfile::tempdir()?;
    let layout = layout(root.path());
    let dist_info = layout.scheme.purelib.join("fixture-1.0.0.dist-info");
    fs_err::create_dir_all(&dist_info)?;
    fs_err::write(layout.scheme.purelib.join("before"), b"before\n")?;
    fs_err::write(dist_info.join("RECORD"), b"before,,\ninvalid,,not-a-size\n")?;
    assert!(matches!(
        uv_install_wheel::uninstall_wheel(&dist_info, "fixture", &layout),
        Err(uv_install_wheel::Error::RecordCsv(_))
    ));
    assert_eq!(
        fs_err::read(layout.scheme.purelib.join("before"))?,
        b"before\n"
    );
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Event {
    Setup,
    Materialize(u64),
    Operation(u64),
    Audit(u64),
    OutputDropped(u64),
    TrialDropped(u64),
    Finish,
    StateDropped,
}

type Events = Arc<Mutex<Vec<(ThreadId, Event)>>>;

fn record(events: &Events, event: Event) {
    events
        .lock()
        .expect("Failed to lock wheel timing events")
        .push((thread::current().id(), event));
}

struct State {
    next: Rc<Cell<u64>>,
    events: Events,
}

impl Drop for State {
    fn drop(&mut self) {
        record(&self.events, Event::StateDropped);
    }
}

struct TimingTrial {
    index: u64,
    events: Events,
}

impl Drop for TimingTrial {
    fn drop(&mut self) {
        record(&self.events, Event::TrialDropped(self.index));
    }
}

struct Output {
    next: Rc<Cell<u64>>,
    index: u64,
    events: Events,
}

impl Drop for Output {
    fn drop(&mut self) {
        assert_eq!(self.next.get(), self.index + 1);
        record(&self.events, Event::OutputDropped(self.index));
    }
}

#[test]
fn timing_uses_fresh_trials_and_keeps_non_send_state_on_one_thread() {
    let caller = thread::current().id();
    let events = Events::default();
    let setup_events = Arc::clone(&events);
    let trial_events = Arc::clone(&events);
    let mut next_trial = 0;
    let _elapsed = timing::isolated_trials(
        3,
        move || {
            record(&setup_events, Event::Setup);
            State {
                next: Rc::new(Cell::new(0)),
                events: setup_events,
            }
        },
        move || {
            let index = next_trial;
            next_trial += 1;
            record(&trial_events, Event::Materialize(index));
            TimingTrial {
                index,
                events: Arc::clone(&trial_events),
            }
        },
        |state, trial| {
            assert_eq!(state.next.get(), trial.index);
            state.next.set(trial.index + 1);
            record(&state.events, Event::Operation(trial.index));
            Output {
                next: Rc::clone(&state.next),
                index: trial.index,
                events: Arc::clone(&state.events),
            }
        },
        |trial, output| {
            assert_eq!(trial.index, output.index);
            record(&trial.events, Event::Audit(trial.index));
        },
        |state| record(&state.events, Event::Finish),
    );

    let events = events.lock().expect("Failed to lock wheel timing events");
    let submitting_thread = events[0].0;
    assert_ne!(submitting_thread, caller);
    assert!(
        events
            .iter()
            .all(|(thread, _)| *thread == submitting_thread)
    );
    assert_eq!(
        events.iter().map(|(_, event)| event).collect::<Vec<_>>(),
        [
            Event::Setup,
            Event::Materialize(0),
            Event::Operation(0),
            Event::Audit(0),
            Event::OutputDropped(0),
            Event::TrialDropped(0),
            Event::Materialize(1),
            Event::Operation(1),
            Event::Audit(1),
            Event::OutputDropped(1),
            Event::TrialDropped(1),
            Event::Materialize(2),
            Event::Operation(2),
            Event::Audit(2),
            Event::OutputDropped(2),
            Event::TrialDropped(2),
            Event::Finish,
            Event::StateDropped,
        ]
        .iter()
        .collect::<Vec<_>>()
    );
    let second_thread = timing::isolated(|| thread::current().id());
    assert_ne!(second_thread, submitting_thread);
    assert_ne!(second_thread, caller);
}
