//! Git support is derived from Cargo's implementation.
//! Cargo is dual-licensed under either Apache 2.0 or MIT, at the user's choice.
//! Source: <https://github.com/rust-lang/cargo/blob/23eb492cf920ce051abfc56bbaf838514dc8365c/src/cargo/sources/git/utils.rs>
use std::fmt::Display;
use std::path::{Path, PathBuf, absolute};
use std::str::{self};
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow, ensure};
use cargo_util::{ProcessBuilder, ProcessError, paths};
use owo_colors::OwoColorize;
use tracing::{debug, instrument, warn};
use url::Url;

use uv_fs::{Simplified, write_atomic_sync};
use uv_git_types::{GitOid, GitReference};
use uv_redacted::DisplaySafeUrl;
use uv_static::EnvVars;
use uv_warnings::warn_user_once;

use crate::GitFetchSettings;

/// Extension for the marker beside a completed checkout.
/// See [`GitCheckout::reset`] for why we need this.
const CHECKOUT_READY_EXTENSION: &str = "ok";

/// Filter used for partial Git fetches.
const PARTIAL_CLONE_FILTER: &str = "tree:0";

/// Remote configured for local checkout clones.
const CHECKOUT_REMOTE: &str = "origin";

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("Git executable not found. Ensure that Git is installed and available.")]
    GitNotFound,
    #[error("Git LFS extension not found. Ensure that Git LFS is installed and available.")]
    GitLfsNotFound,
    #[error("Is Git LFS configured? Run `{}` to initialize Git LFS.", "git lfs install".green())]
    GitLfsNotConfigured,
    #[error(transparent)]
    Other(#[from] which::Error),
    #[error(
        "Remote Git fetches are not allowed because network connectivity is disabled (i.e., with `--offline`)"
    )]
    TransportNotAllowed,
}

/// A global cache of the result of `which git` as a command
///
/// Caching the command allows us to avoid needing to remove environment
/// variables everywhere.
pub static GIT: LazyLock<Result<ProcessBuilder, GitError>> = LazyLock::new(|| {
    let path = which::which("git").map_err(|err| match err {
        which::Error::CannotFindBinaryPath => GitError::GitNotFound,
        err => GitError::Other(err),
    })?;

    let mut cmd = ProcessBuilder::new(path);

    // Certain git environment variables never make sense to inherit because
    // they affect what the current command will act on.

    // This can cause problems if for example uv is ran by git (for example, the
    // `exec` command in `git rebase`), the GIT_DIR is set by git and will point
    // to the wrong location (this takes precedence over the cwd).
    cmd.env_remove(EnvVars::GIT_DIR)
        .env_remove(EnvVars::GIT_WORK_TREE)
        .env_remove(EnvVars::GIT_INDEX_FILE)
        .env_remove(EnvVars::GIT_OBJECT_DIRECTORY)
        .env_remove(EnvVars::GIT_ALTERNATE_OBJECT_DIRECTORIES)
        .env_remove(EnvVars::GIT_COMMON_DIR);

    Ok(cmd)
});

/// Strategy when fetching refspecs for a [`GitReference`]
enum RefspecStrategy {
    /// All refspecs should be fetched, if any fail then the fetch will fail.
    All,
    /// Stop after the first successful fetch, if none succeed then the fetch will fail.
    First,
}

/// A Git reference (like a tag or branch) or a specific commit.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum ReferenceOrOid<'reference> {
    /// A Git reference, like a tag or branch.
    Reference(&'reference GitReference),
    /// A specific commit.
    Oid(GitOid),
}

impl ReferenceOrOid<'_> {
    /// Resolves the [`ReferenceOrOid`] to an object ID with objects the `repo` currently has.
    fn resolve(&self, repo: &GitRepository) -> Result<GitOid> {
        let refkind = self.kind_str();
        let result = match self {
            // Resolve the commit pointed to by the tag.
            //
            // `^0` recursively peels away from the revision to the underlying commit object.
            // This also verifies that the tag indeed refers to a commit.
            Self::Reference(GitReference::Tag(s)) => {
                repo.rev_parse(&format!("refs/remotes/origin/tags/{s}^0"))
            }

            // Resolve the commit pointed to by the branch.
            Self::Reference(GitReference::Branch(s)) => repo.rev_parse(&format!("origin/{s}^0")),

            // Attempt to resolve the branch, then the tag.
            Self::Reference(GitReference::BranchOrTag(s)) => repo
                .rev_parse(&format!("origin/{s}^0"))
                .or_else(|_| repo.rev_parse(&format!("refs/remotes/origin/tags/{s}^0"))),

            // Attempt to resolve the branch, then the tag, then the commit.
            Self::Reference(GitReference::BranchOrTagOrCommit(s)) => repo
                .rev_parse(&format!("origin/{s}^0"))
                .or_else(|_| repo.rev_parse(&format!("refs/remotes/origin/tags/{s}^0")))
                .or_else(|_| repo.rev_parse(&format!("{s}^0"))),

            // We'll be using the HEAD commit.
            Self::Reference(GitReference::DefaultBranch) => {
                repo.rev_parse("refs/remotes/origin/HEAD")
            }

            // Resolve a named reference.
            Self::Reference(GitReference::NamedRef(s)) => repo.rev_parse(&format!("{s}^0")),

            // Resolve a specific commit.
            Self::Oid(s) => repo.rev_parse(&format!("{s}^0")),
        };

        result.with_context(|| anyhow::format_err!("failed to find {refkind} `{self}`"))
    }

    /// Returns the kind of this [`ReferenceOrOid`].
    fn kind_str(&self) -> &str {
        match self {
            Self::Reference(reference) => reference.kind_str(),
            Self::Oid(_) => "commit",
        }
    }

    /// Converts the [`ReferenceOrOid`] to a `str` that can be used as a revision.
    fn as_rev(&self) -> &str {
        match self {
            Self::Reference(r) => r.as_rev(),
            Self::Oid(rev) => rev.as_str(),
        }
    }
}

impl Display for ReferenceOrOid<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reference(reference) => write!(f, "{reference}"),
            Self::Oid(oid) => write!(f, "{oid}"),
        }
    }
}

/// A remote repository. It gets cloned into a local [`GitDatabase`].
#[derive(PartialEq, Clone, Debug)]
pub(crate) struct GitRemote {
    /// URL to a remote repository.
    url: DisplaySafeUrl,
}

/// A local clone of a remote repository's database. Multiple [`GitCheckout`]s
/// can be cloned from a single [`GitDatabase`].
pub(crate) struct GitDatabase {
    /// The remote repository where this database is fetched from.
    remote: GitRemote,
    /// Underlying Git repository instance for this database.
    repo: GitRepository,
    /// Git LFS artifacts have been initialized (if requested).
    lfs_ready: Option<bool>,
}

/// A local checkout of a particular revision from a [`GitRepository`].
pub(crate) struct GitCheckout {
    /// The git revision this checkout is for.
    revision: GitOid,
    /// Underlying Git repository instance for this checkout.
    repo: GitRepository,
    /// Git LFS artifacts have been initialized (if requested).
    lfs_ready: Option<bool>,
}

/// Result of validating cached LFS objects with the installed Git LFS version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LfsValidation {
    Passed,
    Skipped,
    Failed,
}

/// A local Git repository.
pub(crate) struct GitRepository {
    /// Path to the underlying Git repository on the local filesystem.
    path: PathBuf,
}

impl GitRepository {
    /// Opens an existing Git repository at `path`.
    fn open(path: &Path) -> Result<Self> {
        // Make sure there is a Git repository at the specified path.
        GIT.as_ref()
            .cloned()?
            .arg("rev-parse")
            .cwd(path)
            .exec_with_output()?;

        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    /// Initializes a Git repository at `path`.
    fn init(path: &Path) -> Result<Self> {
        // TODO(ibraheem): see if this still necessary now that we no longer use libgit2
        // Skip anything related to templates, they just call all sorts of issues as
        // we really don't want to use them yet they insist on being used. See #6240
        // for an example issue that comes up.
        // opts.external_template(false);

        // Initialize the repository.
        GIT.as_ref()
            .cloned()?
            .arg("init")
            .cwd(path)
            .exec_with_output()?;

        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    /// Returns the configured Git remotes for this repository.
    fn remotes(&self) -> Result<Vec<String>> {
        let output = GIT
            .as_ref()
            .cloned()?
            .arg("remote")
            .cwd(&self.path)
            .exec_with_output()?;

        let output = String::from_utf8(output.stdout)?;
        Ok(output
            .lines()
            .map(str::trim)
            .filter(|remote| !remote.is_empty())
            .map(ToString::to_string)
            .collect())
    }

    /// Returns whether this repository can retrieve missing objects from a promisor remote.
    fn has_promisor_remote(&self) -> Result<bool> {
        let result = GIT
            .as_ref()
            .cloned()?
            .arg("config")
            .arg("--type=bool")
            .arg("--get-regexp")
            .arg(r"^remote\..*\.promisor$")
            .cwd(&self.path)
            .exec_with_output();
        match result {
            Ok(output) => Ok(str::from_utf8(&output.stdout)?
                .lines()
                .any(|line| line.ends_with(" true"))),
            Err(error)
                if error
                    .downcast_ref::<ProcessError>()
                    .is_some_and(|error| error.code == Some(1)) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    /// Configures the given remote as the promisor remote for this repository.
    fn configure_promisor_remote(&self, remote: &str, url: &DisplaySafeUrl) -> Result<()> {
        let url = without_credentials(url);
        let remotes = self.remotes()?;

        if remotes.iter().any(|existing| existing == remote) {
            GIT.as_ref()
                .cloned()?
                .arg("remote")
                .arg("set-url")
                .arg(remote)
                .arg(url.as_str())
                .cwd(&self.path)
                .exec_with_output()?;
        } else {
            GIT.as_ref()
                .cloned()?
                .arg("remote")
                .arg("add")
                .arg(remote)
                .arg(url.as_str())
                .cwd(&self.path)
                .exec_with_output()?;
        }

        GIT.as_ref()
            .cloned()?
            .arg("config")
            .arg(format!("remote.{remote}.promisor"))
            .arg("true")
            .cwd(&self.path)
            .exec_with_output()?;

        GIT.as_ref()
            .cloned()?
            .arg("config")
            .arg(format!("remote.{remote}.partialclonefilter"))
            .arg(PARTIAL_CLONE_FILTER)
            .cwd(&self.path)
            .exec_with_output()?;

        // Git creates a promisor remote whose name is the fetch URL when fetching
        // with `--filter` from a URL. Remove any such URL-named remotes so we
        // don't persist credentials in `.git/config`.
        for existing in remotes {
            if existing == remote {
                continue;
            }
            let Ok(existing_url) = existing.parse::<DisplaySafeUrl>() else {
                continue;
            };
            if without_credentials(&existing_url) == url {
                let result = GIT
                    .as_ref()
                    .cloned()?
                    .arg("config")
                    .arg("--remove-section")
                    .arg(format!("remote.{existing}"))
                    .cwd(&self.path)
                    .exec_with_output();
                if let Err(err) = result {
                    let err = redact_git_error(err, &existing_url);
                    debug!("Failed to remove URL-named Git remote `{existing_url}`: {err}");
                }
            }
        }

        Ok(())
    }

    /// Parses the object ID of the given `refname`.
    fn rev_parse(&self, refname: &str) -> Result<GitOid> {
        let result = GIT
            .as_ref()
            .cloned()?
            .arg("rev-parse")
            .arg(refname)
            // Avoid triggering dynamic object fetches when we are only checking
            // whether a revision resolves locally.
            .env("GIT_NO_LAZY_FETCH", "1")
            .cwd(&self.path)
            .exec_with_output()?;

        let mut result = String::from_utf8(result.stdout)?;
        result.truncate(result.trim_end().len());
        Ok(result.parse()?)
    }

    /// Expose the selected revision's LFS configuration to Git LFS's normal configuration loader.
    fn prepare_lfs_config(&self, revision: &GitOid) -> Result<()> {
        let entries = GIT
            .as_ref()
            .cloned()?
            .args(&[
                "ls-tree",
                "--name-only",
                revision.as_str(),
                "--",
                ".lfsconfig",
            ])
            .env("GIT_NO_LAZY_FETCH", "1")
            .env(EnvVars::GIT_ALLOW_PROTOCOL, "file")
            .cwd(&self.path)
            .exec_with_output()?;
        let path = self.path.join(".lfsconfig");
        if entries.stdout.is_empty() {
            match fs_err::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        } else {
            let contents = GIT
                .as_ref()
                .cloned()?
                .arg("show")
                .arg(format!("{revision}:.lfsconfig"))
                .env("GIT_NO_LAZY_FETCH", "1")
                .env(EnvVars::GIT_ALLOW_PROTOCOL, "file")
                .cwd(&self.path)
                .exec_with_output()?;
            write_atomic_sync(path, contents.stdout)?;
        }
        Ok(())
    }

    /// Verifies LFS artifacts have been initialized for a given `refname`.
    #[instrument(skip_all, fields(path = %self.path.user_display(), refname = %refname))]
    fn lfs_fsck_objects(&self, refname: &str) -> LfsValidation {
        let mut cmd = if let Ok(lfs) = GIT_LFS.as_ref() {
            lfs.clone()
        } else {
            warn!("Git LFS is not available, skipping LFS fetch");
            return LfsValidation::Failed;
        };

        // Requires Git LFS 3.x (2021 release)
        let result = cmd
            .arg("fsck")
            .arg("--objects")
            .arg(refname)
            .env("GIT_NO_LAZY_FETCH", "1")
            .env(EnvVars::GIT_ALLOW_PROTOCOL, "file")
            .cwd(&self.path)
            .exec_with_output();

        match result {
            Ok(_) => LfsValidation::Passed,
            Err(err) => {
                let lfs_error = err.to_string();
                if lfs_error.contains("unknown flag: --objects") {
                    warn_user_once!(
                        "Skipping Git LFS validation as Git LFS extension is outdated. \
                        Upgrade to `git-lfs>=3.0.2` or manually verify git-lfs objects were \
                        properly fetched after the current operation finishes."
                    );
                    LfsValidation::Skipped
                } else {
                    debug!("Git LFS validation failed: {err}");
                    LfsValidation::Failed
                }
            }
        }
    }
}

impl GitRemote {
    /// Creates an instance for a remote repository URL.
    pub(crate) fn new(url: DisplaySafeUrl) -> Self {
        Self { url }
    }

    /// Gets the remote repository URL.
    pub(crate) fn url(&self) -> &DisplaySafeUrl {
        &self.url
    }

    /// Fetches and checkouts to a reference or a revision from this remote
    /// into a local path.
    ///
    /// This ensures that it gets the up-to-date commit when a named reference
    /// is given (tag, branch, refs/*). Thus, network connection is involved.
    ///
    /// When `locked_rev` is provided, it takes precedence over `reference`.
    ///
    /// If we have a previous instance of [`GitDatabase`] then fetch into that
    /// if we can. If that can successfully load our revision then we've
    /// populated the database with the latest version of `reference`, so
    /// return that database and the rev we resolve to.
    pub(crate) fn checkout(
        self,
        into: &Path,
        db: Option<GitDatabase>,
        reference: &GitReference,
        locked_rev: Option<GitOid>,
        settings: GitFetchSettings,
        with_lfs: bool,
    ) -> Result<(GitDatabase, GitOid)> {
        let reference = locked_rev
            .or_else(|| {
                if let GitReference::BranchOrTagOrCommit(revision) = reference {
                    revision.parse::<GitOid>().ok()
                } else {
                    None
                }
            })
            .map(ReferenceOrOid::Oid)
            .unwrap_or(ReferenceOrOid::Reference(reference));
        if let Some(mut db) = db {
            fetch(&mut db.repo, &self.url, reference, settings)
                .with_context(|| format!("failed to fetch into: {}", into.user_display()))?;

            let resolved_commit_hash = match locked_rev {
                Some(rev) => db.contains(rev).then_some(rev),
                None => reference.resolve(&db.repo).ok(),
            };

            if let Some(rev) = resolved_commit_hash {
                if with_lfs {
                    let lfs_ready = fetch_lfs(
                        &db.repo,
                        &self.url,
                        &rev,
                        settings.disable_ssl,
                        settings.offline,
                        None,
                    )
                    .with_context(|| format!("failed to fetch LFS objects at {rev}"))?;
                    db = db.with_lfs_ready(Some(lfs_ready));
                }
                db.remote = self;
                return Ok((db, rev));
            }
        }

        // Otherwise start from scratch to handle corrupt git repositories.
        // After our fetch (which is interpreted as a clone now) we do the same
        // resolution to figure out what we cloned.
        match fs_err::remove_dir_all(into) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }

        fs_err::create_dir_all(into)?;
        let mut repo = GitRepository::init(into)?;
        fetch(&mut repo, &self.url, reference, settings)
            .with_context(|| format!("failed to clone into: {}", into.user_display()))?;
        let rev = match locked_rev {
            Some(rev) => rev,
            None => reference.resolve(&repo)?,
        };
        let lfs_ready = with_lfs
            .then(|| {
                fetch_lfs(
                    &repo,
                    &self.url,
                    &rev,
                    settings.disable_ssl,
                    settings.offline,
                    None,
                )
                .with_context(|| format!("failed to fetch LFS objects at {rev}"))
            })
            .transpose()?;

        Ok((
            GitDatabase {
                remote: self,
                repo,
                lfs_ready,
            },
            rev,
        ))
    }

    /// Creates a [`GitDatabase`] of this remote at `db_path`.
    pub(crate) fn db_at(&self, db_path: &Path) -> Result<GitDatabase> {
        let repo = GitRepository::open(db_path)?;

        Ok(GitDatabase {
            remote: self.clone(),
            repo,
            lfs_ready: None,
        })
    }
}

impl GitDatabase {
    /// Checkouts to a revision at `destination` from this database.
    pub(crate) fn copy_to(
        &self,
        rev: GitOid,
        destination: &Path,
        settings: GitFetchSettings,
    ) -> Result<GitCheckout> {
        // If the existing checkout exists, and it is fresh, use it.
        // A non-fresh checkout can happen if the checkout operation was
        // interrupted. In that case, the checkout gets deleted and a new
        // clone is created.
        let checkout = match GitRepository::open(destination)
            .ok()
            .map(|repo| GitCheckout::new(rev, repo))
            .filter(GitCheckout::is_fresh)
        {
            Some(co) => {
                if self.lfs_ready == Some(true) {
                    if co.repo.lfs_fsck_objects(rev.as_str()) == LfsValidation::Passed {
                        co.materialize_lfs()?;
                        co.with_lfs_ready(Some(true))
                    } else {
                        let lfs_ready = self.copy_lfs_to(&co)?;
                        let lfs_ready = co.reset(lfs_ready, self.remote.url(), settings)?;
                        co.with_lfs_ready(lfs_ready)
                    }
                } else {
                    co.with_lfs_ready(self.lfs_ready)
                }
            }
            None => GitCheckout::clone_into(destination, self, rev, settings)?,
        };
        Ok(checkout)
    }

    /// Copy LFS objects from the shared database into a checkout clone.
    fn copy_lfs_to(&self, checkout: &GitCheckout) -> Result<Option<bool>> {
        match self.lfs_ready {
            None => Ok(None),
            Some(false) => Ok(Some(false)),
            Some(true) => {
                let path = absolute(&self.repo.path)?;
                let url = DisplaySafeUrl::from_file_path(&path)
                    .map_err(|()| anyhow!("Invalid Git database path: {}", path.user_display()))?;
                fetch_lfs(
                    &checkout.repo,
                    &url,
                    &checkout.revision,
                    false,
                    true,
                    Some(&url),
                )
                .map(Some)
            }
        }
    }

    /// Get a short OID for a `revision`, usually 7 chars or more if ambiguous.
    pub(crate) fn to_short_id(&self, revision: GitOid) -> Result<String> {
        let output = GIT
            .as_ref()
            .cloned()?
            .arg("rev-parse")
            .arg("--short")
            .arg(revision.as_str())
            .cwd(&self.repo.path)
            .exec_with_output()?;

        let mut result = String::from_utf8(output.stdout)?;
        result.truncate(result.trim_end().len());
        Ok(result)
    }

    /// Checks if `oid` resolves to a commit in this database.
    pub(crate) fn contains(&self, oid: GitOid) -> bool {
        self.repo.rev_parse(&format!("{oid}^0")).is_ok()
    }

    /// Checks whether the shared database contains the revision's LFS objects.
    pub(crate) fn contains_lfs_artifacts(&self, oid: GitOid) -> bool {
        self.repo.lfs_fsck_objects(&format!("{oid}^0")) == LfsValidation::Passed
    }

    /// Set the Git LFS validation state (if any).
    #[must_use]
    pub(crate) fn with_lfs_ready(mut self, lfs: Option<bool>) -> Self {
        self.lfs_ready = lfs;
        self
    }
}

impl GitCheckout {
    /// Creates an instance of [`GitCheckout`]. This doesn't imply the checkout
    /// is done. Use [`GitCheckout::is_fresh`] to check.
    ///
    /// * The `repo` will be the checked out Git repository.
    fn new(revision: GitOid, repo: GitRepository) -> Self {
        Self {
            revision,
            repo,
            lfs_ready: None,
        }
    }

    /// Invalidate readiness before replacing or repairing a checkout.
    fn invalidate_ready(path: &Path) -> Result<()> {
        match fs_err::remove_file(path.with_extension(CHECKOUT_READY_EXTENSION)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Clone a repo for a `revision` into a local path from a `database`.
    /// This is a filesystem-to-filesystem clone.
    fn clone_into(
        into: &Path,
        database: &GitDatabase,
        revision: GitOid,
        settings: GitFetchSettings,
    ) -> Result<Self> {
        Self::invalidate_ready(into)?;

        // Local clones copy promisor packs without retaining the source's remote
        // configuration. They still need that remote to retrieve missing objects.
        let settings = settings
            .with_partial_fetches(settings.partial_fetches || database.repo.has_promisor_remote()?);
        let dirname = into.parent().unwrap();
        fs_err::create_dir_all(dirname)?;
        match fs_err::remove_dir_all(into) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }

        // Perform a local clone of the repository, which will attempt to use
        // hardlinks to set up the repository. This should speed up the clone operation
        // quite a bit if it works.
        let res = GIT
            .as_ref()
            .cloned()?
            .arg("clone")
            .arg("--no-checkout")
            .arg("--local")
            // Make sure to pass the local file path and not a file://... url. If given a url,
            // Git treats the repository as a remote origin and gets confused because we don't
            // have a HEAD checked out.
            .arg(database.repo.path.simplified_display().to_string())
            .arg(into.simplified_display().to_string())
            .exec_with_output();

        if let Err(e) = res {
            debug!("Cloning git repo with --local failed, retrying without hardlinks: {e}");

            GIT.as_ref()
                .cloned()?
                .arg("clone")
                .arg("--no-checkout")
                .arg("--no-hardlinks")
                .arg(database.repo.path.simplified_display().to_string())
                .arg(into.simplified_display().to_string())
                .exec_with_output()?;
        }

        let checkout = Self::new(revision, GitRepository::open(into)?);
        let lfs_ready = database.copy_lfs_to(&checkout)?;
        let lfs_ready = checkout.reset(lfs_ready, database.remote.url(), settings)?;
        Ok(checkout.with_lfs_ready(lfs_ready))
    }

    /// Checks if the `HEAD` of this checkout points to the expected revision.
    fn is_fresh(&self) -> bool {
        match self.repo.rev_parse("HEAD") {
            Ok(id) if id == self.revision => {
                // See comments in reset() for why we check this
                self.repo
                    .path
                    .with_extension(CHECKOUT_READY_EXTENSION)
                    .exists()
            }
            _ => false,
        }
    }

    /// Replace LFS pointers using local objects, including when smudge filters are absent.
    fn materialize_lfs(&self) -> Result<()> {
        let mut command = GIT_LFS
            .as_ref()
            .map_err(|_| GitError::GitLfsNotFound)?
            .clone();
        // Git LFS skips checkout unless a clean filter is configured, even with local objects.
        append_git_config(&mut command, "filter.lfs.clean", "git-lfs clean -- %f");
        command
            .arg("checkout")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env(EnvVars::GIT_ALLOW_PROTOCOL, "file")
            .env_remove(EnvVars::GIT_LFS_SKIP_SMUDGE)
            .cwd(&self.repo.path)
            .exec_with_output()?;
        Ok(())
    }

    /// Indicates Git LFS artifacts have been initialized (when requested).
    pub(crate) fn lfs_ready(&self) -> Option<bool> {
        self.lfs_ready
    }

    /// Set the Git LFS validation state (if any).
    #[must_use]
    fn with_lfs_ready(mut self, lfs: Option<bool>) -> Self {
        self.lfs_ready = lfs;
        self
    }

    /// This performs `git reset --hard` to the revision of this checkout and updates submodules,
    /// with additional interrupt protection by a marker file.
    ///
    /// If we're interrupted while performing any of the processes in this method (e.g., we die
    /// because of a signal) uv needs to be sure to try to check out this
    /// repo again on the next go-round.
    ///
    /// The marker sits beside the checkout with an [`.ok` extension], so tracked files cannot
    /// collide with it. It is removed before cloning and created only after preparation succeeds.
    /// For example, there may be the `checkouts/<repository-key>/0123456789abcdef/` contents
    /// directory containing `checkouts/<repository-key>/0123456789abcdef/pyproject.toml`, and the
    /// `checkouts/<repository-key>/0123456789abcdef.ok` marker file besides it.
    ///
    /// [`.ok` extension]: CHECKOUT_READY_EXTENSION
    /// `git reset --hard [<commit>]` can break relative submodule URLs, so we update submodules
    /// using the original remote URL.
    fn reset(
        &self,
        with_lfs: Option<bool>,
        original_remote_url: &DisplaySafeUrl,
        settings: GitFetchSettings,
    ) -> Result<Option<bool>> {
        Self::invalidate_ready(&self.repo.path)?;

        // A cache populated with partial fetches can still require missing objects
        // after the feature is disabled. Use the original remote for those objects.
        if settings.partial_fetches || self.repo.has_promisor_remote()? {
            self.repo
                .configure_promisor_remote(CHECKOUT_REMOTE, original_remote_url)?;
        }

        // We want to skip smudge if lfs was disabled for the repository
        // as smudge filters can trigger on a reset even if lfs artifacts
        // were not originally "fetched".
        let lfs_skip_smudge = if with_lfs == Some(true) { "0" } else { "1" };

        debug!("Reset `{}` to {}", self.repo.path.display(), self.revision);

        // Perform the hard reset.
        let mut reset = GIT.as_ref().cloned()?;
        configure_git_network(
            &mut reset,
            original_remote_url,
            settings.disable_ssl,
            settings.offline,
        );
        reset
            .arg("reset")
            .arg("--hard")
            .arg(self.revision.as_str())
            .env(EnvVars::GIT_LFS_SKIP_SMUDGE, lfs_skip_smudge)
            .cwd(&self.repo.path)
            .exec_with_output()
            .map_err(|err| git_command_error(err, original_remote_url, settings.offline))?;

        // Initialize direct submodules using the original remote URL so Git can resolve relative
        // submodule URLs, but don't write it to `remote.origin.url`. Git persists resolved submodule
        // URLs during initialization, so writing a credentialed parent remote can leak credentials
        // into checkout configuration.
        //
        // Do not use `--recursive` here: command-local `remote.origin.url` config is inherited by
        // Git commands run inside submodules, which would make nested relative URLs resolve against
        // the top-level remote instead of their immediate parent submodule.
        let mut submodule_update = GIT.as_ref().cloned()?;
        configure_git_network(
            &mut submodule_update,
            original_remote_url,
            settings.disable_ssl,
            settings.offline,
        );
        for config in submodule_update_config(original_remote_url) {
            submodule_update.arg("-c").arg(config);
        }

        submodule_update
            .arg("submodule")
            .arg("update")
            .arg("--init")
            .env(EnvVars::GIT_LFS_SKIP_SMUDGE, lfs_skip_smudge)
            .cwd(&self.repo.path)
            .exec_with_output()
            .map_err(|err| git_command_error(err, original_remote_url, settings.offline))
            .map(drop)?;

        // Recursively update nested submodules without overriding `remote.origin.url`, so each
        // nested relative URL resolves against its immediate parent submodule. The transient
        // credential rewrite is still safe to inherit because it only affects transport.
        let mut submodule_update = GIT.as_ref().cloned()?;
        configure_git_network(
            &mut submodule_update,
            original_remote_url,
            settings.disable_ssl,
            settings.offline,
        );
        for config in submodule_auth_config(original_remote_url) {
            submodule_update.arg("-c").arg(config);
        }

        submodule_update
            .arg("submodule")
            .arg("update")
            .arg("--recursive")
            .arg("--init")
            .env(EnvVars::GIT_LFS_SKIP_SMUDGE, lfs_skip_smudge)
            .cwd(&self.repo.path)
            .exec_with_output()
            .map_err(|err| git_command_error(err, original_remote_url, settings.offline))
            .map(drop)?;

        // Validate Git LFS objects (if needed) after the reset.
        // See `fetch_lfs` why we do this.
        let lfs_validation = match with_lfs {
            None => None,
            Some(false) => Some(false),
            Some(true) => {
                self.materialize_lfs()?;
                Some(match self.repo.lfs_fsck_objects(self.revision.as_str()) {
                    LfsValidation::Passed | LfsValidation::Skipped => true,
                    LfsValidation::Failed => false,
                })
            }
        };

        // The .ok file should be written when the reset is successful.
        // When Git LFS is enabled, the objects must also be fetched and
        // validated successfully as part of the corresponding checkout.
        if with_lfs.is_none() || lfs_validation == Some(true) {
            paths::create(self.repo.path.with_extension(CHECKOUT_READY_EXTENSION))?;
        }

        Ok(lfs_validation)
    }
}

/// Return command-local Git configuration for initializing direct submodules in a checkout.
///
/// Relative submodule URLs are resolved from `remote.origin.url`, but writing the original remote
/// URL into checkout configuration can persist credentials in the parent repository or submodule
/// remotes. Instead, callers pass these values via `git -c`, using a credential-stripped origin URL
/// for resolution and a transient `url.*.insteadOf` rewrite when credentials are needed for
/// transport.
fn submodule_update_config(original_remote_url: &DisplaySafeUrl) -> Vec<String> {
    let remote_url = original_remote_url.without_credentials();
    let mut config = vec![format!("remote.origin.url={}", remote_url.as_str())];

    config.extend(submodule_auth_config(original_remote_url));
    config
}

/// Return command-local Git authentication configuration for updating submodules.
///
/// Unlike `remote.origin.url`, these rewrites are safe to inherit during recursive submodule
/// updates: they rewrite transport URLs for authentication, but do not change the base URL that Git
/// uses to resolve nested relative submodule URLs.
fn submodule_auth_config(original_remote_url: &DisplaySafeUrl) -> Vec<String> {
    let remote_url = original_remote_url.without_credentials();
    let mut config = Vec::new();

    if remote_url.as_str() != original_remote_url.as_str() {
        let safe_root = remote_url_root(remote_url.into_owned());
        let credentialed_root = remote_url_root((**original_remote_url).clone());

        if safe_root.as_str() != credentialed_root.as_str() {
            config.push(format!(
                "url.{}.insteadOf={}",
                credentialed_root.as_str(),
                safe_root.as_str()
            ));
        }
    }

    config
}

/// Return the scheme, authority, and root path of a remote URL.
///
/// This is used as the rewrite prefix for `url.*.insteadOf`, so a credentialed parent URL can
/// authenticate sibling submodule URLs without making the credentials part of any persisted
/// submodule URL.
fn remote_url_root(mut url: Url) -> Url {
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    url
}

/// Returns the URL without embedded credentials for persistence in Git config.
fn without_credentials(url: &DisplaySafeUrl) -> DisplaySafeUrl {
    DisplaySafeUrl::from_url(url.without_credentials().into_owned())
}

/// Adds a one-shot Git URL rewrite so commands that perform lazy fetches can
/// authenticate without storing credentials in the repository config.
fn apply_url_rewrite(cmd: &mut ProcessBuilder, url: &DisplaySafeUrl) {
    let url_without_credentials = without_credentials(url);
    if url_without_credentials != *url {
        append_git_config(
            cmd,
            &format!("url.{}.insteadOf", url.as_str()),
            url_without_credentials.as_str(),
        );
    }
}

/// Append command-local configuration without replacing inherited entries.
fn append_git_config(cmd: &mut ProcessBuilder, key: &str, value: &str) {
    let config_index = cmd
        .get_env("GIT_CONFIG_COUNT")
        .and_then(|count| count.to_str()?.parse::<usize>().ok())
        .unwrap_or(0);
    cmd.env("GIT_CONFIG_COUNT", (config_index + 1).to_string())
        .env(&format!("GIT_CONFIG_KEY_{config_index}"), key)
        .env(&format!("GIT_CONFIG_VALUE_{config_index}"), value);
}

/// Applies transport settings to commands that can fetch Git objects.
fn configure_git_network(
    cmd: &mut ProcessBuilder,
    url: &DisplaySafeUrl,
    disable_ssl: bool,
    offline: bool,
) {
    // Terminal prompts would be hidden by the progress bar. GUI prompts remain available.
    cmd.env(EnvVars::GIT_TERMINAL_PROMPT, "0");
    apply_url_rewrite(cmd, url);
    if disable_ssl {
        let origin = remote_url_root(url.without_credentials().into_owned());
        append_git_config(cmd, &format!("http.{origin}.sslVerify"), "false");
    }
    if offline {
        cmd.env(EnvVars::GIT_ALLOW_PROTOCOL, "file");
    }
}

/// Converts an offline transport failure and redacts credentials from other errors.
fn git_command_error(error: anyhow::Error, url: &DisplaySafeUrl, offline: bool) -> anyhow::Error {
    let message = error.to_string();
    if offline && message.contains("transport '") && message.contains("' not allowed") {
        GitError::TransportNotAllowed.into()
    } else {
        redact_git_error(error, url)
    }
}

/// Attempts to fetch the given git `reference` for a Git repository.
///
/// This is the main entry for git clone/fetch. It does the following:
///
/// * Turns [`GitReference`] into refspecs accordingly.
/// * Dispatches `git fetch` using the git CLI.
///
/// The `remote_url` argument is the git remote URL where we want to fetch from.
fn fetch(
    repo: &mut GitRepository,
    remote_url: &DisplaySafeUrl,
    reference: ReferenceOrOid<'_>,
    settings: GitFetchSettings,
) -> Result<()> {
    if let ReferenceOrOid::Oid(rev) = reference {
        let local_object = reference.resolve(repo).ok();
        if let Some(local_object) = local_object {
            if rev == local_object {
                return Ok(());
            }
        }
    }

    // Translate the reference desired here into an actual list of refspecs
    // which need to get fetched. Additionally record if we're fetching tags.
    let mut refspecs = Vec::new();
    let mut tags = false;
    let mut refspec_strategy = RefspecStrategy::All;
    // The `+` symbol on the refspec means to allow a forced (fast-forward)
    // update which is needed if there is ever a force push that requires a
    // fast-forward.
    match reference {
        // For branches and tags we can fetch simply one reference and copy it
        // locally, no need to fetch other branches/tags.
        ReferenceOrOid::Reference(GitReference::Branch(branch)) => {
            refspecs.push(format!("+refs/heads/{branch}:refs/remotes/origin/{branch}"));
        }

        ReferenceOrOid::Reference(GitReference::Tag(tag)) => {
            refspecs.push(format!("+refs/tags/{tag}:refs/remotes/origin/tags/{tag}"));
        }

        ReferenceOrOid::Reference(GitReference::BranchOrTag(branch_or_tag)) => {
            refspecs.push(format!(
                "+refs/heads/{branch_or_tag}:refs/remotes/origin/{branch_or_tag}"
            ));
            refspecs.push(format!(
                "+refs/tags/{branch_or_tag}:refs/remotes/origin/tags/{branch_or_tag}"
            ));
            refspec_strategy = RefspecStrategy::First;
        }

        // Fetch all branches and tags so ambiguous references can resolve to either a named
        // reference or a short commit hash.
        ReferenceOrOid::Reference(GitReference::BranchOrTagOrCommit(_)) => {
            refspecs.push(String::from("+refs/heads/*:refs/remotes/origin/*"));
            refspecs.push(String::from("+HEAD:refs/remotes/origin/HEAD"));
            tags = true;
        }

        ReferenceOrOid::Reference(GitReference::DefaultBranch) => {
            refspecs.push(String::from("+HEAD:refs/remotes/origin/HEAD"));
        }

        ReferenceOrOid::Reference(GitReference::NamedRef(rev)) => {
            refspecs.push(format!("+{rev}:{rev}"));
        }

        ReferenceOrOid::Oid(rev) => {
            refspecs.push(format!("+{rev}:refs/commit/{rev}"));
        }
    }

    debug!("Performing a Git fetch for: {remote_url}");
    let result = match refspec_strategy {
        RefspecStrategy::All => {
            fetch_refspecs(repo, remote_url, refspecs.as_slice(), tags, settings)
        }
        RefspecStrategy::First => {
            // Try each refspec
            let mut errors = refspecs
                .iter()
                .map_while(|refspec| {
                    let fetch_result = fetch_refspecs(
                        repo,
                        remote_url,
                        std::slice::from_ref(refspec),
                        tags,
                        settings,
                    );

                    // Stop after the first success and log failures
                    match fetch_result {
                        Err(ref err) => {
                            debug!("Failed to fetch refspec `{refspec}`: {err}");
                            Some(fetch_result)
                        }
                        Ok(()) => None,
                    }
                })
                .collect::<Vec<_>>();

            if errors.len() == refspecs.len() {
                if let Some(result) = errors.pop() {
                    // Use the last error for the message
                    result
                } else {
                    // Can only occur if there were no refspecs to fetch
                    Ok(())
                }
            } else {
                Ok(())
            }
        }
    };
    match reference {
        // With the default branch, adding context is confusing
        ReferenceOrOid::Reference(GitReference::DefaultBranch) => result,
        _ => result.with_context(|| {
            format!(
                "failed to fetch {} `{}`",
                reference.kind_str(),
                reference.as_rev()
            )
        }),
    }
}

/// Attempts to use `git` CLI installed on the system to fetch the given refspecs from a remote repository.
fn fetch_refspecs(
    repo: &mut GitRepository,
    url: &DisplaySafeUrl,
    refspecs: &[String],
    tags: bool,
    settings: GitFetchSettings,
) -> Result<()> {
    if settings.partial_fetches {
        repo.configure_promisor_remote(CHECKOUT_REMOTE, url)?;
    }

    let mut cmd = GIT.as_ref().cloned()?;
    configure_git_network(&mut cmd, url, settings.disable_ssl, settings.offline);

    cmd.arg("fetch");
    if tags {
        cmd.arg("--tags");
    }
    cmd.arg("--force") // handle force pushes
        .arg("--update-head-ok"); // see discussion in #2078
    if settings.partial_fetches {
        // Build tools such as setuptools-scm can require the complete commit
        // history, while trees and blobs are only needed for selected revisions.
        cmd.arg(format!("--filter={PARTIAL_CLONE_FILTER}"))
            .arg(CHECKOUT_REMOTE);
    } else {
        // A repository populated with the preview feature may retain a remote
        // filter. Override it when fetching additional history without preview.
        cmd.arg("--no-filter").arg(url.as_str());
    }
    cmd.args(refspecs).cwd(&repo.path);

    // We capture the output to avoid streaming it to the user's console during clones.
    // The required `on...line` callbacks currently do nothing.
    // The output appears to be included in error messages by default.
    cmd.exec_with_output()
        .map_err(|err| git_command_error(err, url, settings.offline))?;

    Ok(())
}

/// A global cache of the `git lfs` command.
///
/// Returns an error if Git LFS isn't available.
/// Caching the command allows us to only check if LFS is installed once.
///
/// We also support a helper private environment variable to allow
/// controlling the LFS extension from being loaded for testing purposes.
/// Once installed, Git will always load `git-lfs` as a built-in alias
/// which takes priority over loading from `PATH` which prevents us
/// from shadowing the extension with other means.
pub static GIT_LFS: LazyLock<Result<ProcessBuilder>> = LazyLock::new(|| {
    if std::env::var_os(EnvVars::UV_INTERNAL__TEST_LFS_DISABLED).is_some() {
        return Err(anyhow!("Git LFS extension has been forcefully disabled."));
    }

    let mut cmd = GIT.as_ref()?.clone();
    cmd.arg("lfs");

    // Run a simple command to verify LFS is installed
    cmd.clone().arg("version").exec_with_output()?;
    Ok(cmd)
});

/// Fetch the Git objects needed to scan one revision's tree for LFS pointers.
fn fetch_lfs_git_objects(
    repo: &GitRepository,
    url: &DisplaySafeUrl,
    revision: &GitOid,
    disable_ssl: bool,
    offline: bool,
) -> Result<()> {
    if !repo.has_promisor_remote()? {
        return Ok(());
    }

    // Git LFS scans objects without retrieving missing promisor blobs. Fetch
    // the selected tree and its blobs in batches, without traversing history.
    let mut requested = Vec::new();
    loop {
        let output = GIT
            .as_ref()
            .cloned()?
            .arg("rev-list")
            .arg("--objects")
            .arg("--no-object-names")
            .arg("--missing=print")
            .arg("--no-walk")
            .arg(revision.as_str())
            .env("GIT_NO_LAZY_FETCH", "1")
            .env(EnvVars::GIT_ALLOW_PROTOCOL, "file")
            .cwd(&repo.path)
            .exec_with_output()?;
        let mut missing = str::from_utf8(&output.stdout)?
            .lines()
            .filter_map(|line| line.strip_prefix('?'))
            .map(str::parse::<GitOid>)
            .collect::<Result<Vec<_>, _>>()?;
        if missing.is_empty() {
            return Ok(());
        }
        missing.sort_unstable();
        ensure!(
            missing != requested,
            "Git did not provide the objects required by {revision}"
        );

        repo.configure_promisor_remote(CHECKOUT_REMOTE, url)?;
        let mut fetch = GIT.as_ref().cloned()?;
        configure_git_network(&mut fetch, url, disable_ssl, offline);
        fetch
            .arg("fetch")
            .arg("--no-tags")
            .arg("--filter=blob:none")
            .arg("--stdin")
            .arg(CHECKOUT_REMOTE)
            .stdin(
                missing
                    .iter()
                    .map(GitOid::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .cwd(&repo.path)
            .exec_with_output()
            .map_err(|err| git_command_error(err, url, offline))?;
        requested = missing;
    }
}

/// Attempts to use `git-lfs` CLI to fetch required LFS objects for a given revision.
fn fetch_lfs(
    repo: &GitRepository,
    url: &DisplaySafeUrl,
    revision: &GitOid,
    disable_ssl: bool,
    offline: bool,
    endpoint_override: Option<&DisplaySafeUrl>,
) -> Result<bool> {
    let mut cmd = if let Ok(lfs) = GIT_LFS.as_ref() {
        debug!("Fetching Git LFS objects");
        lfs.clone()
    } else {
        // Since this feature is opt-in, warn if not available
        warn!("Git LFS is not available, skipping LFS fetch");
        return Ok(false);
    };

    fetch_lfs_git_objects(repo, url, revision, disable_ssl, offline)?;
    repo.prepare_lfs_config(revision)?;
    configure_git_network(&mut cmd, url, disable_ssl, offline);
    if let Some(endpoint) = endpoint_override {
        // Local cache copies must not inherit remote endpoints from .lfsconfig or ambient config.
        append_git_config(&mut cmd, "lfs.url", endpoint.as_str());
    }
    // A named remote lets Git LFS apply revision-local endpoint settings, including file URLs.
    append_git_config(
        &mut cmd,
        &format!("remote.{CHECKOUT_REMOTE}.url"),
        url.as_str(),
    );

    cmd.arg("fetch")
        .arg(CHECKOUT_REMOTE)
        .arg(revision.as_str())
        // We should not support requesting LFS artifacts with skip smudge being set.
        // While this may not be necessary, it's added to avoid any potential future issues.
        .env_remove(EnvVars::GIT_LFS_SKIP_SMUDGE)
        .cwd(&repo.path);

    cmd.exec_with_output()
        .map_err(|err| git_command_error(err, url, offline))?;

    // We now validate the Git LFS objects explicitly (if supported). This is
    // needed to avoid issues with Git LFS not being installed or configured
    // on the system and giving the wrong impression to the user that Git LFS
    // objects were initialized correctly when installation finishes.
    // We may want to allow the user to skip validation in the future via
    // UV_GIT_LFS_NO_VALIDATION environment variable on rare cases where
    // validation costs outweigh the benefit.
    let validation_result = repo.lfs_fsck_objects(revision.as_str());

    Ok(match validation_result {
        LfsValidation::Passed | LfsValidation::Skipped => true,
        LfsValidation::Failed => false,
    })
}

/// Redact a credentialed remote URL from a Git process error.
fn redact_git_error(mut error: anyhow::Error, url: &DisplaySafeUrl) -> anyhow::Error {
    let credentialed_root = DisplaySafeUrl::from_url(remote_url_root((**url).clone()));
    let redact = |message: &str| credentialed_root.redact_in(&url.redact_in(message));

    if let Some(process_error) = error.downcast_mut::<ProcessError>() {
        process_error.desc = redact(&process_error.desc);
        return error;
    }

    anyhow!("{}", redact(&error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submodule_update_config_strips_credentials_from_origin_override() {
        let url = DisplaySafeUrl::parse("https://user:password@example.com/org/repo.git").unwrap();

        assert_eq!(
            submodule_update_config(&url),
            vec![
                "remote.origin.url=https://example.com/org/repo.git".to_string(),
                "url.https://user:password@example.com/.insteadOf=https://example.com/".to_string(),
            ]
        );
    }

    #[test]
    fn submodule_update_config_preserves_git_ssh_user() {
        let url = DisplaySafeUrl::parse("ssh://git@example.com/org/repo.git").unwrap();

        assert_eq!(
            submodule_update_config(&url),
            vec!["remote.origin.url=ssh://git@example.com/org/repo.git".to_string()]
        );
    }

    #[test]
    #[cfg(feature = "test-git")]
    fn git_tls_exception_is_scoped_to_remote_host() -> Result<()> {
        let url = DisplaySafeUrl::parse("https://user:password@allowed.example/org/repo.git")?;
        let mut command = GIT.as_ref().cloned()?;
        command
            .arg("-c")
            .arg("http.sslVerify=true")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_CONFIG_COUNT", "0")
            .env_remove(EnvVars::GIT_SSL_NO_VERIFY);
        configure_git_network(&mut command, &url, true, false);

        let allowed = command
            .clone()
            .args(&[
                "config",
                "--get-urlmatch",
                "http.sslVerify",
                "https://allowed.example/other.git",
            ])
            .exec_with_output()?;
        assert_eq!(str::from_utf8(&allowed.stdout)?.trim(), "false");

        let unrelated = command
            .args(&[
                "config",
                "--get-urlmatch",
                "http.sslVerify",
                "https://untrusted.example/submodule.git",
            ])
            .exec_with_output()?;
        assert_eq!(str::from_utf8(&unrelated.stdout)?.trim(), "true");
        Ok(())
    }

    #[test]
    fn git_process_error_redacts_credentials() -> Result<()> {
        let url = DisplaySafeUrl::parse("https://git:secret-token@example.com/org/repo.git")?;
        let stderr = format!("fatal: Authentication failed for '{}'", url.as_str());
        let error = ProcessError::new_raw(
            &format!(
                "process didn't exit successfully: `git fetch --force '{}' '+HEAD:refs/remotes/origin/HEAD'`",
                url.as_str()
            ),
            Some(128),
            "exit status: 128",
            Some(b"git output"),
            Some(stderr.as_bytes()),
        )
        .into();

        let error = redact_git_error(error, &url);
        let process_error = error
            .downcast_ref::<ProcessError>()
            .context("expected Git process error")?;

        assert_eq!(
            error.to_string(),
            "process didn't exit successfully: `git fetch --force 'https://git:****@example.com/org/repo.git' '+HEAD:refs/remotes/origin/HEAD'` (exit status: 128)\n--- stdout\ngit output\n--- stderr\nfatal: Authentication failed for 'https://git:****@example.com/org/repo.git'"
        );
        assert_eq!(process_error.code, Some(128));
        assert_eq!(
            process_error.stdout.as_deref(),
            Some(b"git output".as_slice())
        );
        assert_eq!(process_error.stderr.as_deref(), Some(stderr.as_bytes()));

        Ok(())
    }

    #[test]
    fn git_submodule_process_error_redacts_credentials() -> Result<()> {
        let url = DisplaySafeUrl::parse("https://git:secret-token@example.com/org/repo.git")?;

        for args in ["--init", "--recursive --init"] {
            let error = anyhow!(
                "process didn't exit successfully: `git -c 'url.https://git:secret-token@example.com/.insteadOf=https://example.com/' submodule update {args}` (exit status: 128)"
            );
            let redacted = redact_git_error(error, &url).to_string();

            assert!(!redacted.contains("secret-token"));
            assert_eq!(
                redacted,
                format!(
                    "process didn't exit successfully: `git -c 'url.https://git:****@example.com/.insteadOf=https://example.com/' submodule update {args}` (exit status: 128)"
                )
            );
        }

        Ok(())
    }
}
