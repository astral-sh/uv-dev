use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use gix::bstr::ByteSlice;
use gix_lfs::{Pointer, download};
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use uv_git_types::GitOid;
use uv_redacted::SafeUrl;

use super::{materialize_tree, open};
use crate::git::GitError;

struct Entry {
    path: PathBuf,
    oid: String,
    size: u64,
}

impl Entry {
    fn pointer(&self) -> Pointer<'_> {
        Pointer {
            oid: &self.oid,
            size: self.size,
        }
    }
}

/// Return the tracked LFS files, or `None` when this commit needs the Git LFS executable.
fn entries(repo: &gix::Repository, commit: gix::ObjectId) -> Result<Option<Vec<Entry>>> {
    let config = repo.config_snapshot();
    if config.plumbing().sections_by_name("lfs").is_some()
        || config.string("remote.origin.lfsurl").is_some()
    {
        return Ok(None);
    }
    let index = repo.index_from_tree(&repo.find_commit(commit)?.tree_id()?)?;
    if index
        .entry_by_path(".lfsconfig".as_bytes().as_bstr())
        .is_some()
        || index
            .entries()
            .iter()
            .any(|entry| entry.mode.is_submodule())
    {
        return Ok(None);
    }
    let mut attributes = repo.attributes_only(
        &index,
        gix::worktree::stack::state::attributes::Source::IdMapping,
    )?;
    let mut matched = attributes.selected_attribute_matches(["filter"]);
    let mut files = Vec::new();
    for entry in index.entries() {
        if !matches!(
            entry.mode,
            gix::index::entry::Mode::FILE | gix::index::entry::Mode::FILE_EXECUTABLE
        ) {
            continue;
        }
        let path = entry.path(&index);
        attributes
            .at_entry(path, Some(entry.mode))?
            .matching_attributes(&mut matched);
        if !matched
            .iter_selected()
            .any(|attr| attr.assignment.state.as_bstr() == Some(b"lfs".as_bstr()))
        {
            continue;
        }
        let size = repo.find_header(entry.id)?.size();
        if size == 0 {
            continue;
        }
        if size >= Pointer::SIZE_CUTOFF as u64 {
            return Ok(None);
        }
        let blob = repo.find_blob(entry.id)?;
        let Some(pointer) = Pointer::from_bytes(&blob.data) else {
            return Ok(None);
        };
        files.push(Entry {
            path: gix::path::from_bstr(path)?.into_owned(),
            oid: pointer.oid.to_owned(),
            size: pointer.size,
        });
    }
    Ok(Some(files))
}

fn object_path(repo: &gix::Repository, oid: &str) -> PathBuf {
    repo.common_dir()
        .join("lfs/objects")
        .join(&oid[..2])
        .join(&oid[2..4])
        .join(oid)
}

fn cached(repo: &gix::Repository, pointer: Pointer<'_>) -> Result<bool> {
    let path = object_path(repo, pointer.oid);
    let file = match fs_err::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    Ok(download::copy_verified(pointer, file, io::sink()).is_ok())
}

pub(super) fn checkout(
    source: &gix::Repository,
    checkout: &gix::Repository,
    commit: gix::ObjectId,
) -> Result<()> {
    let worktree = checkout
        .workdir()
        .context("Git LFS checkout needs a worktree")?;
    for entry in
        entries(source, commit)?.context("Git LFS checkout requires an extension filter")?
    {
        let from = object_path(source, &entry.oid);
        let local = object_path(checkout, &entry.oid);
        if !local.try_exists()? {
            fs_err::create_dir_all(local.parent().unwrap())?;
            if fs_err::hard_link(&from, &local).is_err() {
                fs_err::copy(&from, &local)?;
            }
        }
        let destination = worktree.join(entry.path);
        let pointer = fs_err::symlink_metadata(&destination)?;
        ensure!(
            pointer.is_file(),
            "Git LFS checkout target is not a regular file"
        );
        let file = uv_fs::tempfile_in(destination.parent().unwrap())?;
        fs_err::copy(&local, &file)?;
        fs_err::set_permissions(&file, pointer.permissions())?;
        file.persist(&destination)?;
    }
    Ok(())
}

pub(crate) fn contains(database: &Path, revision: GitOid) -> Result<bool> {
    let repo = open(database)?;
    let commit = gix::ObjectId::from_hex(revision.as_str().as_bytes())?;
    let Some(entries) = entries(&repo, commit)? else {
        return Ok(false);
    };
    for entry in &entries {
        if !cached(&repo, entry.pointer())? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn fetch(
    database: &Path,
    revision: GitOid,
    url: &SafeUrl,
    offline: bool,
) -> Result<bool> {
    let repo = open(database)?;
    let commit = gix::ObjectId::from_hex(revision.as_str().as_bytes())?;
    if repo
        .config_snapshot()
        .boolean("remote.uv-promisor.promisor")
        == Some(true)
    {
        materialize_tree(&repo, commit, url, offline)?;
    }
    let Some(entries) = entries(&repo, commit)? else {
        return Ok(false);
    };
    let mut missing = BTreeMap::new();
    for entry in &entries {
        if !missing.contains_key(&entry.oid) && !cached(&repo, entry.pointer())? {
            missing.insert(&entry.oid, entry.pointer());
        }
    }
    if missing.is_empty() {
        return Ok(true);
    }
    if offline {
        return Err(GitError::TransportNotAllowed.into());
    }

    let mut endpoint = url.without_credentials().into_owned();
    let path = endpoint.path().trim_end_matches('/');
    let repository = path.strip_suffix(".git").unwrap_or(path);
    endpoint.set_path(&format!("{repository}.git/info/lfs/objects/batch"));
    let mut builder = Client::builder().redirect(reqwest::redirect::Policy::none());
    if let Some(ca) = std::env::var_os("GIT_SSL_CAINFO")
        .map(PathBuf::from)
        .or_else(|| {
            repo.config_snapshot()
                .string("http.sslCAInfo")
                .and_then(|p| gix::path::from_bstring(p).ok())
        })
    {
        builder =
            builder.tls_certs_only(reqwest::Certificate::from_pem_bundle(&fs_err::read(ca)?)?);
    }
    let client = builder.build()?;
    let pointers = missing.into_values().collect::<Vec<_>>();
    let credentials = uv_auth::Credentials::from_url(url)?;
    for batch in pointers.chunks(100) {
        let mut request = client
            .post(endpoint.as_str())
            .header(ACCEPT, download::MEDIA_TYPE)
            .header(CONTENT_TYPE, download::MEDIA_TYPE)
            .body(download::request(batch)?);
        if let Some(credentials) = &credentials {
            ensure!(url.scheme() == "https", "Git LFS credentials require HTTPS");
            request = request.basic_auth(
                credentials.username().unwrap_or_default(),
                credentials.password(),
            );
        }
        let response = request.send().map_err(reqwest::Error::without_url)?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Ok(false);
        }
        ensure!(
            response.status().is_success(),
            "Git LFS batch request returned {}",
            response.status()
        );
        let mut json = Vec::new();
        response.take(1024 * 1024 + 1).read_to_end(&mut json)?;
        ensure!(
            json.len() <= 1024 * 1024,
            "Git LFS batch response exceeded 1 MiB"
        );
        let actions = download::actions(batch, &json)?;
        for &pointer in batch {
            let action = &actions[pointer.oid];
            let response = get_object(&client, &endpoint, action)?;
            let path = object_path(&repo, pointer.oid);
            let parent = path.parent().unwrap();
            fs_err::create_dir_all(parent)?;
            let mut file = uv_fs::tempfile_in(parent)?;
            download::copy_verified(pointer, response, &mut file)?;
            file.persist(path)?;
        }
    }
    Ok(true)
}

fn get_object(client: &Client, endpoint: &SafeUrl, action: &download::Action) -> Result<Response> {
    let url = SafeUrl::parse(&action.href)?;
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && endpoint.scheme() == "http"
                && endpoint.host() == url.host()
                && endpoint.port_or_known_default() == url.port_or_known_default()),
        "Git LFS download requires a secure URL or the original HTTP server"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Git LFS action URL embeds credentials"
    );
    let mut request = client.get(url.as_str());
    for (name, value) in &action.header {
        request = request.header(name, value);
    }
    let response = request.send().map_err(reqwest::Error::without_url)?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "Git LFS download returned {}",
        response.status()
    );
    Ok(response)
}
