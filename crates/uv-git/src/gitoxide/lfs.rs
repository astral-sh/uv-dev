use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use gix::bstr::{BString, ByteSlice};
use gix_lfs::{Pointer, download, filter::Filter};
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use uv_git_types::GitOid;
use uv_redacted::SafeUrl;

use super::{materialize_tree, open};
use crate::git::{GitError, GitRemote};

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

struct Selection {
    files: Vec<Entry>,
    endpoint: Option<BString>,
}

/// Read LFS pointers and their endpoint. Only regular, tracked `.lfsconfig` files are read,
/// and their include directives are ignored.
fn entries(repo: &gix::Repository, commit: gix::ObjectId) -> Result<Option<Selection>> {
    let index = repo.index_from_tree(&repo.find_commit(commit)?.tree_id()?)?;
    let mut tracked = gix::config::File::default();
    if let Some(entry) = index.entry_by_path(b".lfsconfig".as_bstr())
        && matches!(
            entry.mode,
            gix::index::entry::Mode::FILE | gix::index::entry::Mode::FILE_EXECUTABLE
        )
    {
        tracked = gix::config::File::try_from(repo.find_blob(entry.id)?.data.as_bstr())?;
    }
    let config = repo.config_snapshot();
    let get = |key: &str| config.string(key).or_else(|| tracked.string(key));
    let endpoint = get("lfs.url").or_else(|| get("remote.origin.lfsurl"));
    let include = get("lfs.fetchinclude").unwrap_or_default();
    let exclude = get("lfs.fetchexclude").unwrap_or_default();
    let filter = Filter::new(
        &include,
        &exclude,
        config.boolean("core.ignoreCase").unwrap_or(false),
    );
    if config
        .plumbing()
        .sections_by_name("lfs")
        .into_iter()
        .flatten()
        .any(|section| match section.header().subsection_name() {
            None => section.value_names().any(|name| {
                !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "url" | "pushurl" | "fetchinclude" | "fetchexclude"
                )
            }),
            Some(sub) if sub == "transfer" => {
                section
                    .value_names()
                    .any(|name| !name.eq_ignore_ascii_case("maxretries"))
                    || config
                        .string("lfs.transfer.maxretries")
                        .is_none_or(|value| value != "0")
            }
            Some(_) => true,
        })
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
        if !filter.allows(path) {
            continue;
        }
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
    Ok(Some(Selection { files, endpoint }))
}

fn object_path(oid: &str) -> PathBuf {
    Path::new("lfs/objects")
        .join(&oid[..2])
        .join(&oid[2..4])
        .join(oid)
}

fn cached(repo: &gix::Repository, pointer: Pointer<'_>) -> Result<bool> {
    let path = repo.common_dir().join(object_path(pointer.oid));
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
    for entry in entries(source, commit)?
        .context("Git LFS checkout requires an extension filter")?
        .files
    {
        let path = object_path(&entry.oid);
        let from = source.common_dir().join(&path);
        let local = checkout.common_dir().join(&path);
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

pub(super) fn checkout_submodules(repo: &gix::Repository, parent: &GitRemote) -> Result<()> {
    let Some(modules) = repo.submodules()? else {
        return Ok(());
    };
    for module in modules {
        let (Some(commit), Some(child)) = (module.index_id()?, module.open()?) else {
            continue;
        };
        let url = module.resolved_url()?;
        let url = if url.scheme == gix::url::Scheme::File && url.host().is_none() {
            SafeUrl::from_file_path(gix::path::from_bstr(&url.path)?)
                .map_err(|()| anyhow::anyhow!("Invalid local submodule path"))?
        } else {
            SafeUrl::parse(url.to_bstring().to_str()?)?
        };
        let insecure = parent.disable_ssl && same_origin(parent.url(), &url);
        let remote = GitRemote::new(url, insecure, parent.offline);
        if fetch_objects(&child, commit, &remote)? {
            checkout(&child, &child, commit)?;
        } else {
            crate::git::checkout_lfs_extension(&child, commit, &remote)?;
        }
        checkout_submodules(&child, &remote)?;
    }
    Ok(())
}

pub(crate) fn contains(database: &Path, revision: GitOid) -> Result<bool> {
    let repo = open(database)?;
    let commit = gix::ObjectId::from_hex(revision.as_str().as_bytes())?;
    let Some(entries) = entries(&repo, commit)? else {
        return Ok(false);
    };
    for entry in &entries.files {
        if !cached(&repo, entry.pointer())? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn fetch(database: &Path, revision: GitOid, source: &GitRemote) -> Result<bool> {
    let repo = gix::open_opts(database, source.network_options([]))?;
    let commit = gix::ObjectId::from_hex(revision.as_str().as_bytes())?;
    fetch_objects(&repo, commit, source)
}

fn fetch_objects(
    repo: &gix::Repository,
    commit: gix::ObjectId,
    source: &GitRemote,
) -> Result<bool> {
    if repo
        .config_snapshot()
        .boolean("remote.uv-promisor.promisor")
        == Some(true)
    {
        materialize_tree(repo, commit, source)?;
    }
    let Some(entries) = entries(repo, commit)? else {
        return Ok(false);
    };
    let mut missing = BTreeMap::new();
    for entry in &entries.files {
        if !missing.contains_key(&entry.oid) && !cached(repo, entry.pointer())? {
            missing.insert(&entry.oid, entry.pointer());
        }
    }
    if missing.is_empty() {
        return Ok(true);
    }
    let (remote, _) = repo
        .remote_at(source.url.as_str())?
        .sanitized_url_and_version(gix::remote::Direction::Fetch)?;
    if remote.scheme == gix::url::Scheme::File && remote.host().is_none() {
        if let Some(endpoint) = entries
            .endpoint
            .as_ref()
            .map(|v| gix::url::parse(v))
            .transpose()?
        {
            if endpoint.scheme == gix::url::Scheme::File {
                let local = gix::path::from_bstr(&remote.path)?.canonicalize()?;
                let target = gix::path::from_bstr(&endpoint.path)?.canonicalize()?;
                if endpoint.host().is_some()
                    || (target != local
                        && !(local.ends_with(".git") && local.parent() == Some(target.as_path())))
                {
                    return Ok(false);
                }
                return fetch_local(repo, &remote, missing.into_values());
            }
        } else {
            return fetch_local(repo, &remote, missing.into_values());
        }
    }
    let remote = SafeUrl::parse(remote.to_bstring().to_str()?)?;
    let Some(endpoint) = batch_endpoint(&remote, entries.endpoint)? else {
        return Ok(false);
    };
    if source.offline {
        return Err(GitError::TransportNotAllowed.into());
    }

    let ca = std::env::var_os("GIT_SSL_CAINFO")
        .map(PathBuf::from)
        .or_else(|| {
            repo.config_snapshot()
                .string("http.sslCAInfo")
                .and_then(|p| gix::path::from_bstring(p).ok())
        })
        .map(fs_err::read)
        .transpose()?
        .map(|pem| reqwest::Certificate::from_pem_bundle(&pem))
        .transpose()?;
    let client = |verify: bool| {
        let mut builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .tls_danger_accept_invalid_certs(!verify);
        if let Some(certs) = &ca {
            builder = builder.tls_certs_only(certs.clone());
        }
        builder.build()
    };
    let verified = client(true)?;
    let unverified = source.disable_ssl.then(|| client(false)).transpose()?;
    let client_for = |url: &SafeUrl| {
        if same_origin(source.url(), url) {
            unverified.as_ref().unwrap_or(&verified)
        } else {
            &verified
        }
    };
    let pointers = missing.into_values().collect::<Vec<_>>();
    let credentials = if same_origin(&remote, &endpoint) {
        uv_auth::Credentials::from_url(&remote)?
    } else {
        None
    };
    for batch in pointers.chunks(100) {
        let mut request = client_for(&endpoint)
            .post(endpoint.as_str())
            .header(ACCEPT, download::MEDIA_TYPE)
            .header(CONTENT_TYPE, download::MEDIA_TYPE)
            .body(download::request(batch)?);
        if let Some(credentials) = &credentials {
            ensure!(
                endpoint.scheme() == "https",
                "Git LFS credentials require HTTPS"
            );
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
            let url = SafeUrl::parse(&action.href)?;
            let response = get_object(client_for(&url), &endpoint, &url, action)?;
            store(repo, pointer, response)?;
        }
    }
    Ok(true)
}

fn same_origin(a: &SafeUrl, b: &SafeUrl) -> bool {
    a.scheme() == b.scheme()
        && a.host() == b.host()
        && a.port_or_known_default() == b.port_or_known_default()
}

fn batch_endpoint(remote: &SafeUrl, configured: Option<BString>) -> Result<Option<SafeUrl>> {
    let mut endpoint = if let Some(configured) = configured {
        let Ok(endpoint) = SafeUrl::parse(configured.to_str()?) else {
            return Ok(None);
        };
        if !endpoint.username().is_empty() || endpoint.password().is_some() {
            return Ok(None);
        }
        endpoint
    } else {
        let mut url = remote.without_credentials().into_owned();
        let path = url.path().trim_end_matches('/');
        url.set_path(&format!(
            "{}.git/info/lfs",
            path.strip_suffix(".git").unwrap_or(path)
        ));
        url
    };
    if !matches!(endpoint.scheme(), "http" | "https") {
        return Ok(None);
    }
    ensure!(
        remote.scheme() != "https" || endpoint.scheme() == "https",
        "Git LFS endpoint requires HTTPS"
    );
    endpoint.set_path(&format!(
        "{}/objects/batch",
        endpoint.path().trim_end_matches('/')
    ));
    Ok(Some(endpoint))
}

fn fetch_local<'a>(
    repo: &gix::Repository,
    remote: &gix::Url,
    missing: impl Iterator<Item = Pointer<'a>>,
) -> Result<bool> {
    let source = gix::open_opts(
        gix::path::from_bstr(&remote.path)?,
        super::options().bail_if_untrusted(true),
    )?;
    if source.config_snapshot().string("lfs.storage").is_some() {
        return Ok(false);
    }
    // Keep object paths inside the source's common Git directory, even through directory symlinks.
    let dir =
        cap_std::fs::Dir::open_ambient_dir(source.common_dir(), cap_std::ambient_authority())?;
    for pointer in missing {
        let path = object_path(pointer.oid);
        ensure!(
            dir.symlink_metadata(&path)?.is_file(),
            "Git LFS object {} is not a regular file",
            pointer.oid
        );
        let file = dir.open(&path)?;
        ensure!(
            file.metadata()?.is_file(),
            "Git LFS object {} is not a regular file",
            pointer.oid
        );
        store(repo, pointer, file)?;
    }
    Ok(true)
}

fn store(repo: &gix::Repository, pointer: Pointer<'_>, reader: impl Read) -> Result<()> {
    let path = repo.common_dir().join(object_path(pointer.oid));
    let parent = path.parent().unwrap();
    fs_err::create_dir_all(parent)?;
    let mut file = uv_fs::tempfile_in(parent)?;
    download::copy_verified(pointer, reader, &mut file)?;
    file.persist(path)?;
    Ok(())
}

fn get_object(
    client: &Client,
    endpoint: &SafeUrl,
    url: &SafeUrl,
    action: &download::Action,
) -> Result<Response> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lfs_endpoints_scope_repository_credentials_to_their_origin() -> Result<()> {
        let remote = SafeUrl::parse("https://alice:password@repo.test/project")?;
        let default = batch_endpoint(&remote, None)?.unwrap();
        assert_eq!(
            default.as_str(),
            "https://repo.test/project.git/info/lfs/objects/batch"
        );
        assert!(same_origin(&remote, &default));
        for (base, same) in [
            ("https://repo.test/lfs", true),
            ("https://repo.test:444/lfs", false),
            ("https://objects.test/lfs", false),
        ] {
            let endpoint = batch_endpoint(&remote, Some(base.into()))?.unwrap();
            assert_eq!(endpoint.as_str(), format!("{base}/objects/batch"));
            assert_eq!(same_origin(&remote, &endpoint), same);
        }
        assert!(batch_endpoint(&remote, Some("http://repo.test/lfs".into())).is_err());
        assert!(batch_endpoint(&remote, Some("file:///private/data".into()))?.is_none());
        Ok(())
    }
}
