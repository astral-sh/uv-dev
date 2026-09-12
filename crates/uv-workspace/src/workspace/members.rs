use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::slice;

use futures::{Stream, StreamExt, stream};
use glob::{Paths, Pattern, glob};
use rustc_hash::FxHashSet;

use uv_fs::{Simplified, normalize_path};

use crate::pyproject::{SerdePattern, ToolUvWorkspace};

use super::{MemberDiscovery, WorkspaceError, WorkspaceErrorKind, WorkspaceExclusions};

/// Maximum number of member reads buffered during discovery.
const MEMBER_READ_CONCURRENCY: usize = 8;

pub(super) enum MemberEvent<T> {
    Member(T),
    Ignored { root: PathBuf, reason: IgnoreReason },
    Error(WorkspaceError),
}

pub(super) enum IgnoreReason {
    Cache,
    Member,
}

pub(super) struct MemberPath<'workspace> {
    pub(super) root: PathBuf,
    pub(super) matched_by: &'workspace str,
}

pub(super) struct MemberRead<'workspace> {
    pub(super) member: MemberPath<'workspace>,
    pub(super) pyproject_path: PathBuf,
    pub(super) contents: io::Result<String>,
}

impl<'workspace> MemberPath<'workspace> {
    async fn read(self) -> MemberRead<'workspace> {
        let pyproject_path = self.root.join("pyproject.toml");
        let contents = fs_err::tokio::read_to_string(&pyproject_path).await;
        MemberRead {
            member: self,
            pyproject_path,
            contents,
        }
    }
}

struct MemberGlob<'workspace> {
    matched_by: &'workspace str,
    absolute_glob: String,
    paths: Paths,
}

/// Enumerate member paths lazily, retaining diagnostics in discovery order.
pub(super) struct MemberCandidates<'workspace> {
    workspace_root: &'workspace Path,
    workspace_definition: &'workspace ToolUvWorkspace,
    member_discovery: &'workspace MemberDiscovery,
    member_globs: slice::Iter<'workspace, SerdePattern>,
    current_glob: Option<MemberGlob<'workspace>>,
    external_cache_root: Option<PathBuf>,
    seen: FxHashSet<PathBuf>,
    exclusions: Option<Result<WorkspaceExclusions<'workspace>, WorkspaceError>>,
    finished: bool,
}

impl<'workspace> MemberCandidates<'workspace> {
    pub(super) fn new(
        workspace_root: &'workspace Path,
        workspace_definition: &'workspace ToolUvWorkspace,
        member_discovery: &'workspace MemberDiscovery,
        external_cache_root: Option<PathBuf>,
        seen: FxHashSet<PathBuf>,
    ) -> Self {
        Self {
            workspace_root,
            workspace_definition,
            member_discovery,
            member_globs: workspace_definition
                .members
                .as_deref()
                .unwrap_or_default()
                .iter(),
            current_glob: None,
            external_cache_root,
            seen,
            exclusions: None,
            finished: false,
        }
    }

    fn finish_with_error(
        &mut self,
        error: impl Into<WorkspaceError>,
    ) -> MemberEvent<MemberPath<'workspace>> {
        self.finished = true;
        MemberEvent::Error(error.into())
    }
}

impl<'workspace> Iterator for MemberCandidates<'workspace> {
    type Item = MemberEvent<MemberPath<'workspace>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        loop {
            let Some(current_glob) = &mut self.current_glob else {
                let member_glob = self.member_globs.next()?;
                let normalized_glob = normalize_path(Path::new(member_glob.as_str()));
                let absolute_glob = PathBuf::from(Pattern::escape(
                    self.workspace_root.simplified().to_string_lossy().as_ref(),
                ))
                .join(normalized_glob.as_ref())
                .to_string_lossy()
                .to_string();
                let paths =
                    match glob(&absolute_glob) {
                        Ok(paths) => paths,
                        Err(error) => {
                            return Some(self.finish_with_error(WorkspaceErrorKind::Pattern(
                                absolute_glob,
                                error,
                            )));
                        }
                    };
                self.current_glob = Some(MemberGlob {
                    matched_by: member_glob.as_str(),
                    absolute_glob,
                    paths,
                });
                continue;
            };

            let member_root = match current_glob.paths.next() {
                Some(Ok(member_root)) => member_root,
                Some(Err(error)) => {
                    let error =
                        WorkspaceErrorKind::GlobWalk(current_glob.absolute_glob.clone(), error);
                    return Some(self.finish_with_error(error));
                }
                None => {
                    self.current_glob = None;
                    continue;
                }
            };
            let matched_by = current_glob.matched_by;

            if self
                .external_cache_root
                .as_ref()
                .is_some_and(|cache_root| member_root.starts_with(cache_root))
            {
                return Some(MemberEvent::Ignored {
                    root: member_root,
                    reason: IgnoreReason::Cache,
                });
            }
            if !self.seen.insert(member_root.clone()) {
                continue;
            }
            let member_root = match std::path::absolute(&member_root) {
                Ok(member_root) => member_root,
                Err(error) => {
                    return Some(self.finish_with_error(WorkspaceErrorKind::Normalize(error)));
                }
            };

            let skip = match self.member_discovery {
                MemberDiscovery::All | MemberDiscovery::Existing => false,
                MemberDiscovery::None => true,
                MemberDiscovery::Ignore(ignore) => ignore.contains(member_root.as_path()),
            };
            if skip {
                return Some(MemberEvent::Ignored {
                    root: member_root,
                    reason: IgnoreReason::Member,
                });
            }

            // Exclusion errors matter only after a member survives explicit ignores.
            let excluded = match self.exclusions.get_or_insert_with(|| {
                WorkspaceExclusions::new(self.workspace_root, self.workspace_definition)
            }) {
                Ok(exclusions) => exclusions.matches(&member_root),
                Err(error) => {
                    let error = error.clone();
                    return Some(self.finish_with_error(error));
                }
            };
            if excluded {
                return Some(MemberEvent::Ignored {
                    root: member_root,
                    reason: IgnoreReason::Member,
                });
            }

            return Some(MemberEvent::Member(MemberPath {
                root: member_root,
                matched_by,
            }));
        }
    }
}

pub(super) fn read_members(
    candidates: MemberCandidates<'_>,
) -> impl Stream<Item = MemberEvent<MemberRead<'_>>> {
    read_members_with(candidates, MemberPath::read)
}

fn read_members_with<'workspace, Candidates, Reader, ReadFuture>(
    candidates: Candidates,
    mut read: Reader,
) -> impl Stream<Item = MemberEvent<MemberRead<'workspace>>>
where
    Candidates: Iterator<Item = MemberEvent<MemberPath<'workspace>>>,
    Reader: FnMut(MemberPath<'workspace>) -> ReadFuture,
    ReadFuture: Future<Output = MemberRead<'workspace>>,
{
    stream::iter(candidates)
        .map(move |event| {
            let pending = match event {
                MemberEvent::Member(member) => MemberEvent::Member(read(member)),
                MemberEvent::Ignored { root, reason } => MemberEvent::Ignored { root, reason },
                MemberEvent::Error(error) => MemberEvent::Error(error),
            };
            async move {
                match pending {
                    MemberEvent::Member(read) => MemberEvent::Member(read.await),
                    MemberEvent::Ignored { root, reason } => MemberEvent::Ignored { root, reason },
                    MemberEvent::Error(error) => MemberEvent::Error(error),
                }
            }
        })
        .buffered(MEMBER_READ_CONCURRENCY)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};
    use std::io;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use anyhow::{Context, Result, bail};
    use futures::channel::oneshot;
    use futures::{StreamExt, pin_mut, poll};
    use rustc_hash::FxHashSet;

    use crate::pyproject::{PyProjectToml, ToolUvWorkspace};
    use crate::workspace::{MemberDiscovery, WorkspaceErrorKind};

    use super::{
        IgnoreReason, MEMBER_READ_CONCURRENCY, MemberCandidates, MemberEvent, MemberPath,
        MemberRead, read_members_with,
    };

    fn candidate(index: usize) -> MemberEvent<MemberPath<'static>> {
        MemberEvent::Member(MemberPath {
            root: PathBuf::from(index.to_string()),
            matched_by: "members/*",
        })
    }

    fn read_result(member: MemberPath<'_>, contents: io::Result<String>) -> MemberRead<'_> {
        MemberRead {
            pyproject_path: member.root.join("pyproject.toml"),
            member,
            contents,
        }
    }

    fn contents(event: MemberEvent<MemberRead<'_>>) -> Result<String> {
        match event {
            MemberEvent::Member(read) => Ok(read.contents?),
            MemberEvent::Ignored { .. } => bail!("unexpected ignored member"),
            MemberEvent::Error(error) => Err(error.into()),
        }
    }

    fn next_member<'workspace>(
        candidates: &mut impl Iterator<Item = MemberEvent<MemberPath<'workspace>>>,
    ) -> Result<MemberPath<'workspace>> {
        match candidates.next().context("missing member")? {
            MemberEvent::Member(member) => Ok(member),
            MemberEvent::Ignored { .. } => bail!("unexpected ignored member"),
            MemberEvent::Error(error) => Err(error.into()),
        }
    }

    #[derive(Default)]
    struct Counts {
        enumerated: AtomicUsize,
        started: AtomicUsize,
        active: AtomicUsize,
        maximum: AtomicUsize,
    }

    #[tokio::test]
    async fn reads_are_bounded_and_ordered() -> Result<()> {
        let count = MEMBER_READ_CONCURRENCY * 2 + 1;
        let counts = Arc::new(Counts::default());
        let enumerated = counts.clone();
        let candidates = (0..count).map(candidate).inspect(move |_| {
            enumerated.enumerated.fetch_add(1, Ordering::Relaxed);
        });
        let (mut senders, mut receivers): (VecDeque<_>, VecDeque<_>) = (0..count)
            .map(|_| oneshot::channel::<io::Result<String>>())
            .unzip();
        let read_counts = counts.clone();
        let reads = read_members_with(candidates, move |member| {
            let receiver = receivers.pop_front().expect("one receiver per member");
            let counts = read_counts.clone();
            async move {
                counts.started.fetch_add(1, Ordering::Relaxed);
                let active = counts.active.fetch_add(1, Ordering::Relaxed) + 1;
                counts.maximum.fetch_max(active, Ordering::Relaxed);
                let contents = receiver.await.expect("member read completed");
                counts.active.fetch_sub(1, Ordering::Relaxed);
                read_result(member, contents)
            }
        });
        pin_mut!(reads);

        assert!(poll!(reads.next()).is_pending());
        assert_eq!(
            counts.enumerated.load(Ordering::Relaxed),
            MEMBER_READ_CONCURRENCY
        );
        assert_eq!(
            counts.started.load(Ordering::Relaxed),
            MEMBER_READ_CONCURRENCY
        );

        let first = senders.pop_front().context("first member sender")?;
        for index in 1..MEMBER_READ_CONCURRENCY {
            senders
                .pop_front()
                .context("prefetched member sender")?
                .send(Ok(index.to_string()))
                .expect("prefetched read is pending");
        }
        assert!(poll!(reads.next()).is_pending());
        assert_eq!(counts.active.load(Ordering::Relaxed), 1);
        assert_eq!(
            counts.enumerated.load(Ordering::Relaxed),
            MEMBER_READ_CONCURRENCY
        );

        first
            .send(Ok("0".to_owned()))
            .expect("first read is pending");
        let mut actual = vec![contents(reads.next().await.context("first member")?)?];
        for (index, sender) in (MEMBER_READ_CONCURRENCY..count).zip(senders) {
            sender
                .send(Ok(index.to_string()))
                .expect("remaining read is pending");
        }
        while let Some(event) = reads.next().await {
            actual.push(contents(event)?);
        }

        assert_eq!(
            actual,
            (0..count)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(counts.enumerated.load(Ordering::Relaxed), count);
        assert_eq!(counts.started.load(Ordering::Relaxed), count);
        assert_eq!(counts.active.load(Ordering::Relaxed), 0);
        assert_eq!(
            counts.maximum.load(Ordering::Relaxed),
            MEMBER_READ_CONCURRENCY
        );
        Ok(())
    }

    #[tokio::test]
    async fn reads_keep_the_first_error_and_diagnostic_order() {
        let candidates = [
            MemberEvent::Ignored {
                root: PathBuf::from("before"),
                reason: IgnoreReason::Member,
            },
            candidate(0),
            MemberEvent::Ignored {
                root: PathBuf::from("after"),
                reason: IgnoreReason::Cache,
            },
            candidate(1),
            MemberEvent::Error(WorkspaceErrorKind::Io(io::Error::other("later discovery")).into()),
        ]
        .into_iter();
        let (first_sender, first_receiver) = oneshot::channel::<io::Result<String>>();
        let (second_sender, second_receiver) = oneshot::channel::<io::Result<String>>();
        let mut receivers = VecDeque::from([first_receiver, second_receiver]);
        let reads = read_members_with(candidates, move |member| {
            let receiver = receivers.pop_front().expect("one receiver per member");
            async move { read_result(member, receiver.await.expect("member read completed")) }
        });
        pin_mut!(reads);

        let consume = async {
            let mut ignored = Vec::new();
            while let Some(event) = reads.next().await {
                let error = match event {
                    MemberEvent::Member(read) => match read.contents {
                        Ok(contents) => PyProjectToml::from_string(contents, read.pyproject_path)
                            .err()
                            .map(|error| error.to_string()),
                        Err(error) => Some(error.to_string()),
                    },
                    MemberEvent::Ignored { root, .. } => {
                        ignored.push(root);
                        None
                    }
                    MemberEvent::Error(error) => Some(error.to_string()),
                };
                if let Some(error) = error {
                    return (ignored, Some(error));
                }
            }
            (ignored, None)
        };
        pin_mut!(consume);
        assert!(poll!(consume.as_mut()).is_pending());
        second_sender
            .send(Err(io::Error::other("later read")))
            .expect("second read is pending");
        assert!(poll!(consume.as_mut()).is_pending());
        first_sender
            .send(Ok("[project\n".to_owned()))
            .expect("first read is pending");

        let (ignored, error) = consume.await;
        let expected = PyProjectToml::from_string("[project\n".to_owned(), "pyproject.toml")
            .expect_err("invalid first manifest");
        assert_eq!(ignored, [PathBuf::from("before")]);
        assert_eq!(error, Some(expected.to_string()));
    }

    #[test]
    fn candidate_globs_are_lazy_and_keep_the_first_match() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs_err::create_dir(root.join("first"))?;
        let definition: ToolUvWorkspace =
            toml::from_str(r#"members = ["first", "later", "first", "[a/b]/..", "never"]"#)?;
        let discovery = MemberDiscovery::All;
        let mut candidates =
            MemberCandidates::new(root, &definition, &discovery, None, FxHashSet::default());

        let first = next_member(&mut candidates)?;
        assert_eq!(first.root, std::path::absolute(root.join("first"))?);
        assert_eq!(first.matched_by, "first");

        fs_err::create_dir(root.join("later"))?;
        let later = next_member(&mut candidates)?;
        assert_eq!(later.root, std::path::absolute(root.join("later"))?);
        assert_eq!(later.matched_by, "later");

        // Normalization removes the closing bracket from this otherwise valid glob.
        let MemberEvent::Error(error) = candidates.next().context("invalid normalized glob")?
        else {
            bail!("expected a glob error");
        };
        let WorkspaceErrorKind::Pattern(_, _) = error.as_ref() else {
            bail!("expected a pattern error: {error}");
        };
        fs_err::create_dir(root.join("never"))?;
        assert!(candidates.next().is_none());
        Ok(())
    }

    #[test]
    fn explicitly_ignored_members_do_not_compile_exclusions() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs_err::create_dir(root.join("member"))?;
        let member_root = std::path::absolute(root.join("member"))?;
        let definition: ToolUvWorkspace = toml::from_str(
            r#"members = ["member"]
exclude = ["[a/b]/.."]"#,
        )?;

        for discovery in [
            MemberDiscovery::None,
            MemberDiscovery::Ignore(BTreeSet::from([member_root.clone()])),
        ] {
            let mut candidates =
                MemberCandidates::new(root, &definition, &discovery, None, FxHashSet::default());
            let MemberEvent::Ignored {
                root,
                reason: IgnoreReason::Member,
            } = candidates.next().context("ignored member")?
            else {
                bail!("expected an explicitly ignored member");
            };
            assert_eq!(root, member_root);
            assert!(candidates.next().is_none());
        }

        let discovery = MemberDiscovery::All;
        let mut candidates =
            MemberCandidates::new(root, &definition, &discovery, None, FxHashSet::default());
        let MemberEvent::Error(error) = candidates.next().context("invalid exclusion")? else {
            bail!("expected an exclusion error");
        };
        let WorkspaceErrorKind::Pattern(_, _) = error.as_ref() else {
            bail!("expected a pattern error: {error}");
        };
        assert!(candidates.next().is_none());
        Ok(())
    }
}
