use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use gix::progress::Discard;
use gix::remote::Direction;
use uv_preview::PreviewFeature;
use uv_redacted::SafeUrl;

use crate::git::GitError;

pub(crate) mod lfs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    Cli,
    Gitoxide,
}

impl Backend {
    pub(crate) fn for_remote(url: &SafeUrl, disable_ssl: bool) -> Self {
        if uv_preview::is_enabled(PreviewFeature::Gitoxide) && Self::supports(url, disable_ssl) {
            Self::Gitoxide
        } else {
            Self::Cli
        }
    }

    fn supports(url: &SafeUrl, disable_ssl: bool) -> bool {
        if disable_ssl || url.as_str().contains('?') || url.fragment().is_some() {
            return false;
        }
        match url.scheme() {
            "http" | "https" => true,
            "file" => !url.has_host(),
            "ssh" => {
                url.has_host() && matches!(url.username(), "" | "git") && url.password().is_none()
            }
            _ => false,
        }
    }
}

pub(super) fn options() -> gix::open::Options {
    gix::open::Options::default().without_repository_environment_overrides()
}

pub(crate) fn open(path: &Path) -> gix::Result<gix::Repository> {
    gix::open_opts(path, options())
}

pub(crate) fn init(path: &Path) -> gix::Result<gix::Repository> {
    Ok(gix::ThreadSafeRepository::init_opts(
        path,
        gix::create::Kind::WithWorktree,
        gix::create::Options::default(),
        options(),
    )?
    .to_thread_local())
}

/// Return `false` when the local request cannot use the in-process fetch.
pub(crate) fn fetch(
    path: &Path,
    url: &SafeUrl,
    refspecs: &[String],
    all_tags: bool,
    offline: bool,
) -> Result<bool> {
    let options = options().config_overrides(["gitoxide.credentials.terminalPrompt=false"]);
    let mut repo = gix::open_opts(path, options)?;
    repo.committer_or_set_generic_fallback()?;
    let mut remote = repo
        .remote_at(url.as_str())?
        .with_refspecs(refspecs.iter().map(String::as_str), Direction::Fetch)?;
    if all_tags {
        remote = remote.with_fetch_tags(gix::remote::fetch::Tags::All);
    }
    if remote
        .url(Direction::Fetch)
        .is_some_and(|url| url.scheme == gix::url::Scheme::File)
    {
        return Ok(remote.try_fetch_local(&AtomicBool::new(false))?.is_some());
    }
    if offline {
        return Err(GitError::TransportNotAllowed.into());
    }
    let outcome = remote
        .connect(Direction::Fetch)?
        .prepare_fetch(Discard, gix::remote::ref_map::Options::default())?
        .with_filter(gix::remote::fetch::Filter::TreeDepth(0))
        .receive(Discard, &AtomicBool::new(false))?;
    if let gix::remote::fetch::Status::Change {
        write_pack_bundle, ..
    } = outcome.status
        && write_pack_bundle
            .data_path
            .is_some_and(|pack| pack.with_extension("promisor").is_file())
    {
        set_promisor(&mut repo, url)?;
    }
    Ok(true)
}

fn set_promisor(repo: &mut gix::Repository, url: &SafeUrl) -> Result<()> {
    let mut config = repo.config_edit()?;
    config.set_raw_value("remote.uv-promisor.url", url.without_credentials().as_str())?;
    config.set_raw_value("remote.uv-promisor.promisor", "true")?;
    config.set_raw_value("remote.uv-promisor.partialclonefilter", "tree:0")?;
    config.commit()?;
    Ok(())
}

/// Fetch the missing part of the selected tree before copying the database into a checkout.
fn materialize_tree(
    repo: &gix::Repository,
    commit: gix::ObjectId,
    url: &SafeUrl,
    offline: bool,
) -> Result<()> {
    let root = repo.find_commit(commit)?.tree_id()?.detach();
    loop {
        let mut trees = vec![root];
        let mut seen = HashSet::new();
        let mut missing = Vec::new();
        while let Some(tree_id) = trees.pop() {
            if !seen.insert(tree_id) {
                continue;
            }
            if !repo.has_object(tree_id) {
                missing.push(tree_id);
                continue;
            }
            for entry in repo.find_tree(tree_id)?.iter() {
                let entry = entry?;
                if entry.mode().is_tree() {
                    trees.push(entry.object_id());
                } else if entry.mode().is_blob_or_symlink()
                    && !repo.has_object(entry.object_id())
                    && seen.insert(entry.object_id())
                {
                    missing.push(entry.object_id());
                }
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        if offline {
            return Err(GitError::TransportNotAllowed.into());
        }
        // Commits in a partial clone can have incomplete trees. Advertising them as haves
        // would let the server omit the very objects required for the checkout.
        let repo = gix::open_opts(
            repo.git_dir(),
            options().config_overrides([
                "fetch.negotiationAlgorithm=noop",
                "gitoxide.credentials.terminalPrompt=false",
            ]),
        )?;
        let objects = missing.iter().map(ToString::to_string).collect::<Vec<_>>();
        let outcome = repo
            .remote_at(url.as_str())?
            .with_refspecs(objects.iter().map(String::as_str), Direction::Fetch)?
            .with_fetch_tags(gix::remote::fetch::Tags::None)
            .connect(Direction::Fetch)?
            .prepare_fetch(Discard, gix::remote::ref_map::Options::default())?
            .receive(Discard, &AtomicBool::new(false))?;
        anyhow::ensure!(
            missing.iter().all(|id| repo.has_object(id)),
            "Git server did not supply the requested checkout objects"
        );
        if let gix::remote::fetch::Status::Change {
            write_pack_bundle, ..
        } = outcome.status
            && let Some(keep) = write_pack_bundle.keep_path
        {
            fs_err::remove_file(keep)?;
        }
    }
}

pub(crate) fn checkout(
    database: &Path,
    destination: &Path,
    revision: uv_git_types::GitOid,
    url: &SafeUrl,
    offline: bool,
    with_lfs: bool,
) -> Result<()> {
    let source = open(database)?;
    let commit = gix::hash::ObjectId::from_hex(revision.as_str().as_bytes())?;
    let partial = source
        .config_snapshot()
        .boolean("remote.uv-promisor.promisor")
        == Some(true);
    if partial {
        materialize_tree(&source, commit, url, offline)?;
    }
    let mut repo = init(destination)?;
    copy_cached_objects(&source, &repo)?;
    if partial {
        set_promisor(&mut repo, url)?;
    }
    repo.committer_or_set_generic_fallback()?;
    let mut config = repo.config_edit()?;
    config.set_raw_value("remote.origin.url", gix::path::into_bstr(database)?)?;
    config.set_raw_value("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")?;
    config.commit()?;

    for tag in source.references()?.tags()? {
        let mut tag = tag?;
        let id = tag.follow_to_object()?.detach();
        repo.reference(
            tag.name(),
            id,
            gix::refs::transaction::PreviousValue::MustNotExist,
            "clone: tag",
        )?;
    }

    let options = |overrides: Vec<String>| {
        options().config_overrides(overrides.into_iter().chain([
            "gitoxide.credentials.terminalPrompt=false".to_owned(),
            "filter.lfs.process=".to_owned(),
            "filter.lfs.smudge=".to_owned(),
            "filter.lfs.required=false".to_owned(),
        ]))
    };
    let mut repo = gix::open_opts(
        destination,
        options(crate::git::submodule_update_config(url)),
    )?;
    repo.committer_or_set_generic_fallback()?;
    repo.reset(commit, gix::repository::reset::ResetMode::Hard)?;
    if with_lfs {
        lfs::checkout(&source, &repo, commit)?;
    }
    if let Some(modules) = repo.submodules()? {
        for module in modules {
            if module.index_id()?.is_some() {
                if offline {
                    return Err(GitError::TransportNotAllowed.into());
                }
                module.init(false)?;
            }
        }
        // Nested modules resolve relative URLs against their own origins.
        let repo = gix::open_opts(destination, options(crate::git::submodule_auth_config(url)))?;
        if let Some(modules) = repo.submodules()? {
            for module in modules {
                module.update_submodule(
                    Discard,
                    &AtomicBool::new(false),
                    &gix::submodule::update::Options::new(true, true),
                )?;
            }
        }
    }
    Ok(())
}

/// Copy cached object files into the checkout, using hard links where possible, so they
/// remain available if the cache database is removed.
fn copy_cached_objects(source: &gix::Repository, destination: &gix::Repository) -> Result<()> {
    let store = source.objects.store_ref();
    let destination_objects = destination.objects.store_ref().path();
    let hash_len = source.object_hash().len_in_hex();
    for objects in std::iter::once(store.path().to_owned()).chain(store.alternate_db_paths()?) {
        anyhow::ensure!(
            fs_err::symlink_metadata(&objects)?.is_dir(),
            "invalid Git object directory"
        );
        for entry in fs_err::read_dir(&objects)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(directory) = name.to_str() else {
                continue;
            };
            let packed = directory == "pack";
            let loose = directory.len() == 2 && directory.bytes().all(|b| b.is_ascii_hexdigit());
            if !(packed || loose) {
                continue;
            }
            anyhow::ensure!(
                entry.file_type()?.is_dir(),
                "invalid Git object directory: {}",
                entry.path().display()
            );
            fs_err::create_dir_all(destination_objects.join(directory))?;
            for object in fs_err::read_dir(entry.path())? {
                let object = object?;
                let name = object.file_name();
                let Some(name) = name.to_str() else { continue };
                let hex = if packed {
                    let Some(stem) = name
                        .strip_suffix(".pack")
                        .or_else(|| name.strip_suffix(".idx"))
                        .or_else(|| name.strip_suffix(".promisor"))
                    else {
                        continue;
                    };
                    let Some(hex) = stem.strip_prefix("pack-") else {
                        continue;
                    };
                    if hex.len() != hash_len {
                        continue;
                    }
                    hex
                } else {
                    if name.len() != hash_len - 2 {
                        continue;
                    }
                    name
                };
                if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    continue;
                }
                anyhow::ensure!(
                    object.file_type()?.is_file(),
                    "invalid Git object file: {}",
                    object.path().display()
                );
                let target = destination_objects.join(directory).join(name);
                match fs_err::hard_link(object.path(), &target) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => {
                        fs_err::copy(object.path(), target)?;
                    }
                }
            }
        }
    }
    if source.is_shallow()? {
        fs_err::copy(source.shallow_file()?, destination.shallow_file()?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_supports_usernames_preserved_in_promisor_urls() -> Result<()> {
        for url in [
            "ssh://forge.example/pkg",
            "ssh://git@forge.example:2222/pkg",
        ] {
            let url = SafeUrl::parse(url)?;
            assert!(Backend::supports(&url, false));
            assert_eq!(url.without_credentials().as_ref(), &url);
        }
        for url in [
            "ssh://user@forge.example/pkg",
            "ssh://git:secret@forge.example/pkg",
        ] {
            assert!(!Backend::supports(&SafeUrl::parse(url)?, false));
        }
        Ok(())
    }
}
