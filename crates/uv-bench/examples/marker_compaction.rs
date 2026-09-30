//! Estimate lockfile marker size when only valid conflict selections are represented.
//!
//! This produces a JSON mapping for representation experiments. The compact markers can change
//! invalid-selection diagnostics, so they cannot replace lockfile markers without changing how
//! readers discover transitive conflicts.

use std::collections::BTreeMap;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use clap::Parser;
use fs_err::File;
use toml::Value;

use uv_pep508::MarkerTree;
use uv_pypi_types::Conflicts;
use uv_resolver_types::{ConflictMarker, UniversalMarker};

#[derive(Parser)]
struct Args {
    /// The lockfile to inspect.
    input: PathBuf,
    /// Where to write the JSON marker mapping and size counts.
    output: PathBuf,
}

fn add_marker<'a>(value: &'a Value, markers: &mut BTreeMap<&'a str, usize>) -> Result<()> {
    let marker = value.as_str().context("expected a marker string")?;
    *markers.entry(marker).or_default() += 1;
    Ok(())
}

fn add_marker_list<'a>(value: &'a Value, markers: &mut BTreeMap<&'a str, usize>) -> Result<()> {
    for value in value.as_array().context("expected a marker array")? {
        add_marker(value, markers)?;
    }
    Ok(())
}

fn add_dependency_markers<'a>(
    value: &'a Value,
    markers: &mut BTreeMap<&'a str, usize>,
) -> Result<()> {
    for dependency in value.as_array().context("expected a dependency array")? {
        if let Some(marker) = dependency.get("marker") {
            add_marker(marker, markers)?;
        }
    }
    Ok(())
}

fn collect_markers<'a>(lock: &'a Value, markers: &mut BTreeMap<&'a str, usize>) -> Result<()> {
    for key in [
        "resolution-markers",
        "supported-markers",
        "required-markers",
    ] {
        if let Some(value) = lock.get(key) {
            add_marker_list(value, markers)?;
        }
    }
    for package in lock
        .get("package")
        .and_then(Value::as_array)
        .context("expected lockfile packages")?
    {
        if let Some(value) = package.get("resolution-markers") {
            add_marker_list(value, markers)?;
        }
        if let Some(value) = package.get("dependencies") {
            add_dependency_markers(value, markers)?;
        }
        for key in [
            "optional-dependencies",
            "dev-dependencies",
            "dependency-groups",
        ] {
            if let Some(groups) = package.get(key) {
                for dependencies in groups
                    .as_table()
                    .context("expected dependency groups")?
                    .values()
                {
                    add_dependency_markers(dependencies, markers)?;
                }
            }
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let lock: Value = toml::from_str(&fs_err::read_to_string(args.input)?)?;
    let conflicts: Conflicts = lock
        .get("conflicts")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()))
        .try_into()?;
    let world = UniversalMarker::new(MarkerTree::TRUE, ConflictMarker::from_conflicts(&conflicts))
        .combined();
    let mut counts = BTreeMap::new();
    collect_markers(&lock, &mut counts)?;
    let mut before_bytes = 0;
    let mut after_bytes = 0;
    let mut changed_occurrences = 0;
    let mut markers = BTreeMap::new();
    for (source, count) in counts {
        let original = MarkerTree::from_str(source)?;
        let compact = original.and(world).restrict(world);
        if original.and(world) != compact.and(world) {
            bail!("changed an allowed selection");
        }
        // A tautology keeps every replacement representable as a marker string.
        let rendered = compact
            .try_to_string()
            .unwrap_or_else(|| "os_name == 'posix' or os_name != 'posix'".to_string());
        before_bytes += source.len() * count;
        after_bytes += rendered.len() * count;
        if original != compact {
            changed_occurrences += count;
        }
        markers.insert(source, rendered);
    }
    let mut output = BufWriter::new(File::create(args.output)?);
    serde_json::to_writer_pretty(
        &mut output,
        &serde_json::json!({
            "valid_conflict_world_only": true,
            "before_bytes": before_bytes,
            "after_bytes": after_bytes,
            "changed_occurrences": changed_occurrences,
            "markers": markers,
        }),
    )?;
    output.flush()?;
    Ok(())
}
