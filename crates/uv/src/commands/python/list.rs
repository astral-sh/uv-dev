use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::Write;
use uv_cli::PythonListFormat;
use uv_pep440::Version;

use anyhow::Result;
use itertools::Either;
use owo_colors::OwoColorize;
use rustc_hash::FxHashSet;
use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_fs::Simplified;
use uv_python::downloads::{
    Error as PythonDownloadError, ManagedPythonDownloadList, PythonDownloadRequest,
};
use uv_python::{
    EnvironmentPreference, PythonDownloads, PythonPreference, PythonRequest, PythonSource,
    find_all_python_installations,
};

use crate::commands::ExitStatus;
use crate::commands::python::PythonVersionParts;
use crate::printer::{Printer, jsonl_result_data};
use crate::settings::PythonListKinds;

#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
enum Kind {
    Download,
    Managed,
    System,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct PythonListEntry {
    key: String,
    #[cfg_attr(feature = "schemars", schemars(with = "String"))]
    version: Version,
    version_parts: PythonVersionParts,
    path: Option<String>,
    symlink: Option<String>,
    url: Option<String>,
    os: String,
    variant: String,
    implementation: String,
    arch: String,
    libc: String,
}

/// Generate the JSON schema for Python installation listings.
#[cfg(feature = "schemars")]
pub fn json_schema() -> schemars::Schema {
    let mut schema = schemars::generate::SchemaSettings::draft07()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<Vec<PythonListEntry>>();
    schema.insert("title".to_owned(), "uv python list".into());
    schema
}

/// Generate the per-record schema for preview JSONL Python listings.
#[cfg(feature = "schemars")]
pub fn jsonl_schema() -> schemars::Schema {
    crate::commands::report::jsonl_array_schema::<PythonListEntry>("uv python list JSONL (preview)")
}

/// List available Python installations.
#[expect(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub(crate) async fn list(
    request: Option<String>,
    kinds: PythonListKinds,
    all_versions: bool,
    all_platforms: bool,
    all_arches: bool,
    show_urls: bool,
    output_format: PythonListFormat,
    python_downloads_json_url: Option<String>,
    python_install_mirror: Option<String>,
    pypy_install_mirror: Option<String>,
    python_preference: PythonPreference,
    python_downloads: PythonDownloads,
    client_builder: &BaseClientBuilder<'_>,
    cache: &Cache,
    printer: Printer,
) -> Result<ExitStatus> {
    let request = request.as_deref().map(PythonRequest::parse);
    let base_download_request = if python_preference == PythonPreference::OnlySystem {
        None
    } else {
        // If the user request cannot be mapped to a download request, we won't show any downloads
        PythonDownloadRequest::from_request(request.as_ref().unwrap_or(&PythonRequest::Any))
    };

    let download_list =
        ManagedPythonDownloadList::new(client_builder, cache, python_downloads_json_url.as_deref())
            .await?;
    let mut output = BTreeSet::new();
    if let Some(base_download_request) = base_download_request {
        let download_request = match kinds {
            PythonListKinds::Installed => None,
            PythonListKinds::Downloads => Some(if all_platforms {
                base_download_request
            } else if all_arches {
                base_download_request.fill_platform()?.with_any_arch()
            } else {
                base_download_request.fill_platform()?
            }),
            PythonListKinds::Default => {
                if python_downloads.is_automatic() {
                    Some(if all_platforms {
                        base_download_request
                    } else if all_arches {
                        base_download_request.fill_platform()?.with_any_arch()
                    } else {
                        base_download_request.fill_platform()?
                    })
                } else {
                    // If fetching is not automatic, then don't show downloads as available by default
                    None
                }
            }
        }
        // Include pre-release versions
        .map(|request| request.with_prereleases(true));

        let downloads = download_request
            .as_ref()
            .map(|request| download_list.iter_matching(request))
            .into_iter()
            .flatten()
            // TODO(zanieb): Add a way to show debug downloads, we just hide them for now
            .filter(|download| !download.key().variant().is_debug());

        for download in downloads {
            output.insert((
                download.key().clone(),
                Kind::Download,
                Either::Right(
                    download
                        .download_urls(
                            python_install_mirror.as_deref(),
                            pypy_install_mirror.as_deref(),
                        )?
                        .into_iter()
                        .next()
                        .ok_or(PythonDownloadError::NoPythonDownloadUrlFound)?,
                ),
            ));
        }
    }

    let installed = match kinds {
        PythonListKinds::Installed | PythonListKinds::Default => {
            // While usually [`PythonPreference::OnlyManaged`] means we can skip searching the
            // `PATH`, in `uv python list` we want to enumerate links to managed Python
            // interpreters for inspection. Consequently, we widen the preference here and
            // perform post-filtering.
            let discovery_preference = if python_preference == PythonPreference::OnlyManaged {
                PythonPreference::Managed
            } else {
                python_preference
            };
            let mut installations = find_all_python_installations(
                request.as_ref().unwrap_or(&PythonRequest::Any),
                EnvironmentPreference::OnlySystem,
                discovery_preference,
                cache,
            )?;
            // Apply the original `PythonPreference` to discovered interpreters, since we may
            // have expanded it above.
            installations
                .retain(|installation| python_preference.allows_installation(installation));
            Some(installations)
        }
        PythonListKinds::Downloads => None,
    };

    if let Some(installed) = installed {
        for installation in installed {
            let kind = if matches!(installation.source(), PythonSource::Managed) {
                Kind::Managed
            } else {
                Kind::System
            };
            output.insert((
                installation.key(),
                kind,
                Either::Left(installation.interpreter().real_executable().to_path_buf()),
            ));
        }
    }

    let mut seen_minor = FxHashSet::default();
    let mut seen_patch = FxHashSet::default();
    let mut seen_paths = FxHashSet::default();
    let mut include = Vec::new();
    for (key, kind, uri) in output.iter().rev() {
        // Do not show the same path more than once
        if let Either::Left(path) = uri {
            if !seen_paths.insert(path) {
                continue;
            }
        }

        // Only show the latest patch version for each download unless all were requested.
        //
        // We toggle off platforms/arches based unless all_platforms/all_arches because
        // we want to only show the "best" option for each version by default, even
        // if e.g. the x86_32 build would also work on x86_64.
        if !matches!(kind, Kind::System) {
            if let [major, minor, ..] = *key.version().release() {
                if !seen_minor.insert((
                    all_platforms.then_some(*key.os()),
                    major,
                    minor,
                    key.variant(),
                    key.implementation(),
                    all_arches.then_some(*key.arch()),
                    *key.libc(),
                )) {
                    if matches!(kind, Kind::Download) && !all_versions {
                        continue;
                    }
                }
            }
            if let [major, minor, patch] = *key.version().release() {
                if !seen_patch.insert((
                    all_platforms.then_some(*key.os()),
                    major,
                    minor,
                    patch,
                    key.variant(),
                    key.implementation(),
                    all_arches.then_some(*key.arch()),
                    key.libc(),
                )) {
                    if matches!(kind, Kind::Download) {
                        continue;
                    }
                }
            }
        }
        include.push((key, uri));
    }

    match output_format {
        PythonListFormat::Json | PythonListFormat::Jsonl => {
            let data = include
                .iter()
                .map(|(key, uri)| -> Result<_> {
                    let mut path_or_none: Option<String> = None;
                    let mut symlink_or_none: Option<String> = None;
                    let mut url_or_none: Option<String> = None;
                    match uri {
                        Either::Left(path) => {
                            path_or_none = Some(path.user_display().to_string());

                            let is_symlink = fs_err::symlink_metadata(path)?.is_symlink();
                            if is_symlink {
                                symlink_or_none =
                                    Some(path.read_link()?.user_display().to_string());
                            }
                        }
                        Either::Right(url) => {
                            url_or_none = Some((*url).to_string());
                        }
                    }
                    let version = key.version();

                    Ok(PythonListEntry {
                        key: key.to_string(),
                        version: version.version().clone(),
                        version_parts: (*key).into(),
                        path: path_or_none,
                        symlink: symlink_or_none,
                        url: url_or_none,
                        arch: key.arch().to_string(),
                        implementation: key.implementation().to_string(),
                        os: key.os().to_string(),
                        variant: key.variant().to_string(),
                        libc: key.libc().to_string(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let output = if matches!(output_format, PythonListFormat::Jsonl) {
                jsonl_result_data(&data)?
            } else {
                serde_json::to_string(&data)?
            };
            writeln!(printer.stdout_important_raw(), "{output}")?;
        }
        PythonListFormat::Text => {
            // Compute the width of the first column.
            let width = include
                .iter()
                .fold(0usize, |acc, (key, _)| acc.max(key.to_string().len()));

            for (key, uri) in include {
                let key = key.to_string();
                match uri {
                    Either::Left(path) => {
                        let is_symlink = fs_err::symlink_metadata(path)?.is_symlink();
                        if is_symlink {
                            writeln!(
                                printer.stdout(),
                                "{key:width$}    {} -> {}",
                                path.user_display().cyan(),
                                path.read_link()?.user_display().cyan()
                            )?;
                        } else {
                            writeln!(
                                printer.stdout(),
                                "{key:width$}    {}",
                                path.user_display().cyan()
                            )?;
                        }
                    }
                    Either::Right(url) => {
                        if show_urls {
                            writeln!(printer.stdout(), "{key:width$}    {}", url.dimmed())?;
                        } else {
                            writeln!(
                                printer.stdout(),
                                "{key:width$}    {}",
                                "<download available>".dimmed()
                            )?;
                        }
                    }
                }
            }
        }
    }

    Ok(ExitStatus::Success)
}

#[cfg(all(test, feature = "schemars"))]
mod tests {
    use anyhow::{Context, Result};
    use serde_json::{Value, json};
    use uv_test::json_schema::JsonSchema;

    use super::{PythonListEntry, PythonVersionParts, json_schema, jsonl_schema};
    use crate::printer::jsonl_result_data;

    fn entry() -> Result<PythonListEntry> {
        Ok(PythonListEntry {
            key: "cpython-3.12.0-linux-x86_64-gnu".to_owned(),
            version: "3.12.0".parse()?,
            version_parts: PythonVersionParts {
                major: 3,
                minor: 12,
                patch: 0,
            },
            path: None,
            symlink: None,
            url: Some("https://example.org/cpython-3.12.0.tar.gz".to_owned()),
            os: "linux".to_owned(),
            variant: "default".to_owned(),
            implementation: "cpython".to_owned(),
            arch: "x86_64".to_owned(),
            libc: "gnu".to_owned(),
        })
    }

    #[test]
    fn json_schemas_describe_serialized_entries() -> Result<()> {
        let mut installed = entry()?;
        installed.path = Some("/python/bin/python".to_owned());
        installed.symlink = Some("python3.12".to_owned());
        installed.url = None;
        let entries = [entry()?, installed];
        let payload = serde_json::to_value(&entries)?;
        assert_eq!(payload[0]["version"], "3.12.0");

        let document = serde_json::to_value(json_schema())?;
        assert_eq!(document["title"], "uv python list");
        assert_eq!(document["type"], "array");
        let validator = JsonSchema::new(&serde_json::to_string(&document)?)?;
        validator.parse(&serde_json::to_vec(&entries)?)?;
        validator.parse(b"[]")?;
        for invalid in [b"null".as_slice(), b"{}", b"[null]", b"[{}]"] {
            assert!(validator.parse(invalid).is_err());
        }

        for field in [
            "key",
            "version",
            "version_parts",
            "path",
            "symlink",
            "url",
            "os",
            "variant",
            "implementation",
            "arch",
            "libc",
        ] {
            let mut missing = payload.clone();
            missing[0]
                .as_object_mut()
                .context("expected a Python list entry")?
                .remove(field);
            assert!(validator.parse(&serde_json::to_vec(&missing)?).is_err());
        }

        let parts = &document["definitions"]["PythonVersionParts"]["properties"];
        for field in ["major", "minor", "patch"] {
            assert_eq!(parts[field]["type"], "integer");
            assert_eq!(parts[field]["minimum"], 0);
            assert_eq!(parts[field]["maximum"], u64::MAX);
            let mut maximum = payload.clone();
            maximum[0]["version_parts"][field] = json!(u64::MAX);
            let maximum = serde_json::to_string(&maximum)?;
            validator.parse(maximum.as_bytes())?;
            let maximum_value = u64::MAX.to_string();
            assert_eq!(maximum.matches(&maximum_value).count(), 1);
            let above_maximum =
                maximum.replacen(&maximum_value, &(u128::from(u64::MAX) + 1).to_string(), 1);
            assert!(validator.parse(above_maximum.as_bytes()).is_err());
            for invalid_value in [Value::Null, json!(-1), json!(0.5), json!("3")] {
                let mut invalid = payload.clone();
                invalid[0]["version_parts"][field] = invalid_value;
                assert!(validator.parse(&serde_json::to_vec(&invalid)?).is_err());
            }
        }

        let record_document = serde_json::to_value(jsonl_schema())?;
        assert_eq!(record_document["title"], "uv python list JSONL (preview)");
        let records = JsonSchema::new(&serde_json::to_string(&record_document)?)?;
        records.parse(jsonl_result_data(&entries)?.as_bytes())?;
        records.parse(br#"{"type":"result","data":[]}"#)?;
        records.parse(br#"{"type":"progress","phase":"resolve","status":"started"}"#)?;
        for invalid in [
            br#"{"type":"result"}"#.as_slice(),
            br#"{"type":"result","data":null}"#,
            br#"{"type":"result","data":{}}"#,
            br#"{"type":"unknown","data":[]}"#,
            br#"{"type":"progress","phase":"resolve"}"#,
        ] {
            assert!(records.parse(invalid).is_err());
        }
        Ok(())
    }
}
