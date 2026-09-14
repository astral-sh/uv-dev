//! Recognize conservative semantic no-solution evidence from a real uv process.

use std::collections::BTreeSet;
use std::ops::Bound;
use std::str::FromStr;

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep440::{MIN_VERSION, Version};
use uv_pep508::{MarkerExpression, MarkerTree};

use super::project::ScenarioProject;
use super::scenario::Scenario;

const BEFORE: &str = "Resolver derivation tree before reduction";
const AFTER: &str = "Resolver derivation tree after reduction";
const HEADER: &str = "error: No solution found when resolving dependencies";
const MAX_DIAGNOSTIC_BYTES: usize = 4 * 1024 * 1024;

/// A recognized derivation whose absent-version claims agree with the closed-world index.
///
/// This establishes the kind of subprocess failure, not graph satisfiability. A separate
/// whole-domain certificate is required before calling the result a resolver contradiction.
#[derive(Debug, Serialize)]
pub(super) struct SemanticNoSolution {
    grammar: &'static str,
    header: String,
    project: String,
    project_version: String,
    project_inventory_shown: bool,
    before_external_leaves: usize,
    after_external_leaves: usize,
    no_versions: Vec<EmptyRange>,
}

#[derive(Debug, Serialize)]
struct EmptyRange {
    block: &'static str,
    package: String,
    range: String,
}

/// Recognize the deliberately narrow diagnostic protocol enabled by
/// `UV_INTERNAL__SHOW_DERIVATION_TREE`.
pub(super) fn certify_no_solution(
    scenario: &Scenario,
    status: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<SemanticNoSolution> {
    ensure!(status == Some(1), "the lock did not exit with status 1");
    ensure!(stdout.is_empty(), "the failed lock wrote unexpected stdout");
    ensure!(
        stderr.len() <= MAX_DIAGNOSTIC_BYTES,
        "the no-solution diagnostic exceeds the parser bound"
    );
    let stderr = std::str::from_utf8(stderr).context("the lock diagnostic is not UTF-8")?;
    ensure!(
        !stderr.contains('\u{1b}'),
        "the lock diagnostic contains terminal escapes"
    );
    let lines = stderr.lines().collect::<Vec<_>>();
    let before = unique_position(&lines, BEFORE)?;
    let after = unique_position(&lines, AFTER)?;
    ensure!(before < after, "the derivation trees are out of order");
    for line in &lines[..before] {
        ensure!(
            line.is_empty() || interpreter_preamble(line),
            "unrecognized lock diagnostic preamble: {line}"
        );
    }
    let header = lines[after + 1..]
        .iter()
        .position(|line| line.starts_with("error: "))
        .map(|offset| after + 1 + offset)
        .context("the derivation has no terminal error header")?;
    validate_header(lines[header])?;
    validate_terminal(&lines[header + 1..])?;

    let inventory = Inventory::new(scenario)?;
    let project_inventory = inventory.project_inventory_is_shown(&lines[before + 1..after])?;
    let mut no_versions = Vec::new();
    let before_external_leaves = inventory.check_tree(
        &lines[before + 1..after],
        "before",
        project_inventory,
        &mut no_versions,
    )?;
    let after_external_leaves = inventory.check_tree(
        &lines[after + 1..header],
        "after",
        project_inventory,
        &mut no_versions,
    )?;
    Ok(SemanticNoSolution {
        grammar: "uv-derivation-inventory-v1",
        header: lines[header].to_owned(),
        project: inventory.project_name.to_string(),
        project_version: inventory.project_version.to_string(),
        project_inventory_shown: project_inventory,
        before_external_leaves,
        after_external_leaves,
        no_versions,
    })
}

fn unique_position(lines: &[&str], expected: &str) -> Result<usize> {
    let mut positions = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (*line == expected).then_some(index));
    let position = positions
        .next()
        .with_context(|| format!("missing `{expected}`"))?;
    ensure!(positions.next().is_none(), "repeated `{expected}`");
    Ok(position)
}

fn interpreter_preamble(line: &str) -> bool {
    let Some((version, path)) = line
        .strip_prefix("Using CPython ")
        .and_then(|line| line.split_once(" interpreter at: "))
    else {
        return false;
    };
    !path.is_empty() && Version::from_str(version).is_ok()
}

fn validate_header(header: &str) -> Result<()> {
    if header == HEADER {
        return Ok(());
    }
    let marker = header
        .strip_prefix(HEADER)
        .and_then(|suffix| suffix.strip_prefix(" for split (markers: "))
        .and_then(|suffix| suffix.strip_suffix(')'))
        .context("unrecognized no-solution error header")?;
    ordinary_marker(marker)?;
    Ok(())
}

fn validate_terminal(lines: &[&str]) -> Result<()> {
    let Some((first, remaining)) = lines.split_first() else {
        bail!("the no-solution diagnostic has no cause report");
    };
    ensure!(
        first
            .strip_prefix("  cause: ")
            .is_some_and(|cause| !cause.is_empty()),
        "the no-solution diagnostic has no cause report"
    );
    for line in lines {
        ensure!(
            !unavailable_diagnostic(line),
            "the no-solution diagnostic contains an availability failure: {line}"
        );
    }
    let mut hints_started = false;
    for line in remaining {
        if line.is_empty() {
            continue;
        }
        if let Some(hint) = line.strip_prefix("hint: ") {
            ensure!(semantic_hint(hint), "unrecognized no-solution hint: {hint}");
            hints_started = true;
        } else {
            ensure!(
                !hints_started
                    && line.starts_with("         ")
                    && !line.trim_start().starts_with("error:")
                    && !line.trim_start().starts_with("warning:"),
                "unrecognized no-solution diagnostic line: {line}"
            );
        }
    }
    Ok(())
}

fn unavailable_diagnostic(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    [
        "invalid metadata",
        "inconsistent metadata",
        "invalid package format",
        "not found in the cache",
        "downloaded from a registry",
        "could not be parsed",
        "could not be fetched",
        "could not be queried",
        "network was disabled",
        "authentication credentials",
        "failed to fetch",
        "failed to download",
        "failed to build",
    ]
    .iter()
    .any(|message| line.contains(message))
}

fn semantic_hint(hint: &str) -> bool {
    if hint
        == "The resolution failed for an environment that is not the current one, consider limiting the environments with `tool.uv.environments`."
    {
        return true;
    }
    hint.strip_prefix("While the active Python version is ")
        .and_then(|hint| {
            hint.strip_suffix(
                ", the resolution failed for other Python versions supported by your project. Consider limiting your project's supported Python versions using `requires-python`.",
            )
        })
        .is_some_and(|version| Version::from_str(version).is_ok())
}

struct Inventory<'a> {
    scenario: &'a Scenario,
    project_name: PackageName,
    project_version: Version,
    names: BTreeSet<PackageName>,
}

impl<'a> Inventory<'a> {
    fn new(scenario: &'a Scenario) -> Result<Self> {
        // A local-only `NoVersions` leaf can disappear before either derivation tree is printed.
        // Even an otherwise recognized tree cannot exclude an unavailable local candidate.
        for (name, package) in &scenario.packages {
            for version in package.versions.keys() {
                ensure!(
                    !version.is_local(),
                    "the derivation classifier cannot exclude hidden local-version leaves: {name}=={version}"
                );
            }
        }
        let project = ScenarioProject::new(scenario)?;
        let requirements = project.requirements(&project.all_selection())?;
        let names = requirements
            .iter()
            .chain(scenario.packages.values().flat_map(|package| {
                package.versions.values().flat_map(|metadata| {
                    metadata
                        .requires
                        .iter()
                        .chain(metadata.extras.values().flatten())
                })
            }))
            .map(|requirement| requirement.name.clone())
            .collect::<BTreeSet<_>>();
        ensure!(
            !names.contains(project.name()),
            "the generated project name is targeted by scenario metadata"
        );
        let names = names
            .into_iter()
            .chain(scenario.packages.keys().cloned())
            .chain(std::iter::once(project.name().clone()))
            .collect();
        Ok(Self {
            scenario,
            project_name: project.name().clone(),
            project_version: Version::new([0, 0, 0]),
            names,
        })
    }

    /// The generated directory project has one static version. Its only incoming ranges are the
    /// unversioned workspace requirement and exact-version extra/group proxies; scenario metadata
    /// is forbidden from naming it. The root edge and a dependency of that exact version also
    /// corroborate that uv actually read the local project metadata.
    fn project_inventory_is_shown(&self, lines: &[&str]) -> Result<bool> {
        let root_edge = format!("root=={} depends on {}*", *MIN_VERSION, self.project_name);
        let project_pin = format!("=={}", self.project_version);
        let mut root_seen = false;
        let mut metadata_seen = false;
        for line in lines {
            let line = line.trim_start_matches(' ');
            root_seen |= line == root_edge;
            if let Some((parent, _)) = split_dependency(line) {
                let (parent, range) = self.package_range(parent)?;
                metadata_seen |=
                    parent.display == self.project_name.as_ref() && range.display == project_pin;
            }
        }
        Ok(root_seen && metadata_seen)
    }

    fn package_range(&self, contents: &str) -> Result<(PackageDisplay, DiagnosticRange)> {
        let (package, range) = package_range(contents)?;
        if let PackageKind::Named(name) = &package.kind {
            ensure!(
                self.names.contains(name),
                "derivation package `{name}` is outside the scenario name inventory"
            );
        }
        Ok((package, range))
    }

    fn check_tree(
        &self,
        lines: &[&str],
        block: &'static str,
        project_inventory: bool,
        no_versions: &mut Vec<EmptyRange>,
    ) -> Result<usize> {
        let mut external_leaves = 0;
        for line in lines {
            let line = line.trim_start_matches(' ');
            ensure!(!line.is_empty(), "empty {block} derivation line");
            if let Some(term) = line.strip_prefix("term ") {
                self.package_range(term.strip_prefix("not ").unwrap_or(term))?;
                continue;
            }
            if let Some(claim) = line.strip_prefix("no versions of ") {
                let (package, range) = self.package_range(claim)?;
                self.check_empty_range(&package, &range, project_inventory)?;
                no_versions.push(EmptyRange {
                    block,
                    package: package.display,
                    range: range.display,
                });
            } else if let Some(root) = line.strip_prefix("not root ") {
                ensure!(
                    root == format!("root{}", *MIN_VERSION),
                    "unrecognized synthetic-root derivation leaf: {line}"
                );
            } else if let Some((package, dependency)) = split_dependency(line) {
                self.package_range(package)?;
                self.package_range(dependency)?;
            } else {
                bail!("unsupported {block} derivation leaf: {line}");
            }
            external_leaves += 1;
        }
        ensure!(
            external_leaves > 0,
            "the {block} derivation contains no external leaves"
        );
        Ok(external_leaves)
    }

    fn check_empty_range(
        &self,
        package: &PackageDisplay,
        range: &DiagnosticRange,
        project_inventory: bool,
    ) -> Result<()> {
        let PackageKind::Named(name) = &package.kind else {
            bail!(
                "no-version claims for `{}` are outside the finite inventory",
                package.display
            );
        };
        if name == &self.project_name {
            ensure!(
                project_inventory
                    && package.display == self.project_name.as_ref()
                    && range.is_complement_of_singleton(&self.project_version),
                "unrecognized generated-project absence claim `{}{}` for version {}",
                package.display,
                range.display,
                self.project_version
            );
        } else if let Some(candidate) = self
            .scenario
            .packages
            .get(name)
            .and_then(|package| package.versions.keys().next())
        {
            bail!(
                "the lossy absence claim `{}{}` cannot certify a listed scenario candidate {name}=={candidate}",
                package.display,
                range.display
            );
        }
        Ok(())
    }
}

/// Find only separators outside package marker strings.
fn split_dependency(line: &str) -> Option<(&str, &str)> {
    let mut quote = None;
    let mut escaped = false;
    let mut marker_depth = 0_u8;
    for (index, character) in line.char_indices() {
        if let Some(expected) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == expected {
                quote = None;
            }
        } else {
            match character {
                '\'' | '"' if marker_depth > 0 => quote = Some(character),
                '{' => marker_depth = marker_depth.checked_add(1)?,
                '}' => marker_depth = marker_depth.checked_sub(1)?,
                ' ' if marker_depth == 0 && line[index..].starts_with(" depends on ") => {
                    return Some((&line[..index], &line[index + " depends on ".len()..]));
                }
                _ => {}
            }
        }
    }
    None
}

#[derive(Debug)]
enum PackageKind {
    SyntheticRoot,
    Python,
    Named(PackageName),
}

#[derive(Debug)]
struct PackageDisplay {
    kind: PackageKind,
    display: String,
}

fn package_range(contents: &str) -> Result<(PackageDisplay, DiagnosticRange)> {
    let (package, suffix) = package_display(contents)?;
    let range = diagnostic_range(suffix)?;
    if let PackageKind::SyntheticRoot = &package.kind {
        ensure!(
            range.display == format!("=={}", *MIN_VERSION) && range.is_singleton(&MIN_VERSION),
            "unrecognized synthetic-root version range: {contents}"
        );
    }
    Ok((package, range))
}

fn package_display(contents: &str) -> Result<(PackageDisplay, &str)> {
    let name_end = contents
        .find(|character: char| !name_character(character))
        .unwrap_or(contents.len());
    let name = &contents[..name_end];
    ensure!(!name.is_empty(), "missing derivation package name");
    let kind = match name {
        "root" => PackageKind::SyntheticRoot,
        "Python" => PackageKind::Python,
        name => {
            let parsed = PackageName::from_str(name)?;
            ensure!(
                parsed.as_ref() == name,
                "non-normalized package name: {name}"
            );
            PackageKind::Named(parsed)
        }
    };
    let mut rest = &contents[name_end..];
    if let Some(extra) = rest.strip_prefix('[') {
        let (extra, suffix) = extra
            .split_once(']')
            .context("unclosed derivation package extra")?;
        let parsed = ExtraName::from_str(extra)?;
        ensure!(
            parsed.as_ref() == extra,
            "non-normalized extra name: {extra}"
        );
        ensure!(
            matches!(&kind, PackageKind::Named(_)),
            "special derivation vertices cannot have extras"
        );
        rest = suffix;
    } else if let Some(group) = rest.strip_prefix(':') {
        ensure!(
            name != "system",
            "system derivation vertices are unsupported"
        );
        let end = group
            .find(|character: char| !name_character(character))
            .unwrap_or(group.len());
        let parsed = GroupName::from_str(&group[..end])?;
        ensure!(
            parsed.as_ref() == &group[..end],
            "non-normalized dependency group name"
        );
        ensure!(
            matches!(&kind, PackageKind::Named(_)),
            "special derivation vertices cannot have groups"
        );
        rest = &group[end..];
    }
    if rest.starts_with('{') {
        ensure!(
            matches!(&kind, PackageKind::Named(_)),
            "special derivation vertices cannot have markers"
        );
        let end = marker_end(rest)?;
        ordinary_marker(&rest[1..end - 1])?;
        rest = &rest[end..];
    }
    let display = contents[..contents.len() - rest.len()].to_owned();
    Ok((PackageDisplay { kind, display }, rest))
}

fn name_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
}

fn marker_end(contents: &str) -> Result<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in contents.char_indices().skip(1) {
        if let Some(expected) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == expected {
                quote = None;
            }
        } else {
            match character {
                '\'' | '"' => quote = Some(character),
                '}' => return Ok(index + 1),
                '{' => bail!("nested derivation marker braces"),
                _ => {}
            }
        }
    }
    bail!("unclosed derivation marker")
}

fn ordinary_marker(contents: &str) -> Result<MarkerTree> {
    let marker = MarkerTree::from_str(contents)?;
    let mut has_extras = false;
    marker.visit_extras(|_, _| has_extras = true);
    ensure!(
        !has_extras,
        "selection markers are outside the derivation grammar"
    );
    ensure!(
        !marker
            .to_dnf()
            .iter()
            .flatten()
            .any(|expression| matches!(expression, MarkerExpression::List { .. })),
        "list markers are outside the derivation grammar"
    );
    Ok(marker)
}

#[derive(Debug)]
struct DiagnosticRange {
    display: String,
    segments: Vec<(Bound<Version>, Bound<Version>)>,
}

impl DiagnosticRange {
    // This is only the literal ordered range, not an inverse of uv's lossy diagnostic projection.
    #[cfg(test)]
    fn contains_displayed(&self, version: &Version) -> bool {
        self.segments.iter().any(|(lower, upper)| {
            let lower = match lower {
                Bound::Included(lower) => version >= lower,
                Bound::Excluded(lower) => version > lower,
                Bound::Unbounded => true,
            };
            let upper = match upper {
                Bound::Included(upper) => version <= upper,
                Bound::Excluded(upper) => version < upper,
                Bound::Unbounded => true,
            };
            lower && upper
        })
    }

    fn is_singleton(&self, version: &Version) -> bool {
        match self.segments.as_slice() {
            [(Bound::Included(lower), Bound::Included(upper))] => {
                lower == version && upper == version
            }
            _ => false,
        }
    }

    fn is_complement_of_singleton(&self, version: &Version) -> bool {
        self.display == format!("<{version} | >{version}")
            && matches!(
                self.segments.as_slice(),
                [(Bound::Unbounded, Bound::Excluded(lower)), (Bound::Excluded(upper), Bound::Unbounded)]
                    if lower == version && upper == version
            )
    }
}

#[derive(Clone, Copy)]
enum Comparison {
    Equal,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

fn diagnostic_range(contents: &str) -> Result<DiagnosticRange> {
    let mut segments = Vec::new();
    if contents != "∅" {
        for segment in contents.split(" | ") {
            if segment == "*" {
                ensure!(contents == "*", "a full range cannot have other segments");
                segments.push((Bound::Unbounded, Bound::Unbounded));
                continue;
            }
            let segment = match segment.split(", ").collect::<Vec<_>>().as_slice() {
                [one] => match comparison(one)? {
                    (Comparison::Equal, version) => {
                        (Bound::Included(version.clone()), Bound::Included(version))
                    }
                    (Comparison::Less, version) => (Bound::Unbounded, Bound::Excluded(version)),
                    (Comparison::LessEqual, version) => {
                        (Bound::Unbounded, Bound::Included(version))
                    }
                    (Comparison::Greater, version) => (Bound::Excluded(version), Bound::Unbounded),
                    (Comparison::GreaterEqual, version) => {
                        (Bound::Included(version), Bound::Unbounded)
                    }
                },
                [lower, upper] => {
                    let lower = match comparison(lower)? {
                        (Comparison::Greater, version) => Bound::Excluded(version),
                        (Comparison::GreaterEqual, version) => Bound::Included(version),
                        (Comparison::Equal | Comparison::Less | Comparison::LessEqual, _) => {
                            bail!("invalid lower bound in diagnostic range: {contents}")
                        }
                    };
                    let upper = match comparison(upper)? {
                        (Comparison::Less, version) => Bound::Excluded(version),
                        (Comparison::LessEqual, version) => Bound::Included(version),
                        (Comparison::Equal | Comparison::Greater | Comparison::GreaterEqual, _) => {
                            bail!("invalid upper bound in diagnostic range: {contents}")
                        }
                    };
                    (lower, upper)
                }
                _ => bail!("invalid diagnostic range: {contents}"),
            };
            ensure!(
                nonempty_segment(&segment),
                "empty segment in diagnostic range: {contents}"
            );
            segments.push(segment);
        }
    }
    Ok(DiagnosticRange {
        display: contents.to_owned(),
        segments,
    })
}

fn nonempty_segment(segment: &(Bound<Version>, Bound<Version>)) -> bool {
    match segment {
        (Bound::Included(lower), Bound::Included(upper)) => lower <= upper,
        (
            Bound::Included(lower) | Bound::Excluded(lower),
            Bound::Included(upper) | Bound::Excluded(upper),
        ) => lower < upper,
        (Bound::Unbounded, _) | (_, Bound::Unbounded) => true,
    }
}

fn comparison(contents: &str) -> Result<(Comparison, Version)> {
    for (prefix, comparison) in [
        ("==", Comparison::Equal),
        ("<=", Comparison::LessEqual),
        (">=", Comparison::GreaterEqual),
        ("<", Comparison::Less),
        (">", Comparison::Greater),
    ] {
        if let Some(version) = contents.strip_prefix(prefix) {
            return Ok((comparison, diagnostic_version(version)?));
        }
    }
    bail!("unrecognized diagnostic comparison: {contents}")
}

fn diagnostic_version(contents: &str) -> Result<Version> {
    let version = Version::from_str(contents)?;
    ensure!(
        version.to_string() == contents,
        "non-canonical diagnostic version: {contents}"
    );
    Ok(version)
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    // The ordinary disjoint-marker failure emitted by uv before adaptive universal recovery.
    const DISJOINT: &str = indoc! {"
        Using CPython 3.12.13 interpreter at: /python
        Resolver derivation tree before reduction
        term root==0a0.dev0
          root==0a0.dev0 depends on uv-scenario-root*
          term uv-scenario-root*
            term uv-scenario-root==0.0.0
              uv-scenario-root==0.0.0 depends on a{sys_platform == 'win32'}*
              term a*
                a* depends on missing{sys_platform != 'win32'}*
                no versions of missing{sys_platform != 'win32'}*
            no versions of uv-scenario-root<0.0.0 | >0.0.0
        Resolver derivation tree after reduction
        term uv-scenario-root==0.0.0
          uv-scenario-root==0.0.0 depends on a{sys_platform == 'win32'}*
          term a*
            a* depends on missing{sys_platform != 'win32'}*
            no versions of missing{sys_platform != 'win32'}*
        error: No solution found when resolving dependencies
          cause: Because there are no versions of missing{sys_platform != 'win32'} and all versions of a depend on missing{sys_platform != 'win32'}, we can conclude that all versions of a cannot be used.
                 And because your project depends on a{sys_platform == 'win32'}, we can conclude that your project's requirements are unsatisfiable.
    "};

    // A cold offline cache produces the same ordinary NoVersions shape without an offline hint.
    const UNAVAILABLE: &str = indoc! {"
        Using CPython 3.12.13 interpreter at: /python
        Resolver derivation tree before reduction
        term root==0a0.dev0
          root==0a0.dev0 depends on uv-scenario-root*
          term uv-scenario-root*
            term uv-scenario-root==0.0.0
              uv-scenario-root==0.0.0 depends on a{sys_platform == 'win32'}*
              no versions of a{sys_platform == 'win32'}*
            no versions of uv-scenario-root<0.0.0 | >0.0.0
        Resolver derivation tree after reduction
        term uv-scenario-root==0.0.0
          uv-scenario-root==0.0.0 depends on a{sys_platform == 'win32'}*
          no versions of a{sys_platform == 'win32'}*
        error: No solution found when resolving dependencies
          cause: Because there are no versions of a{sys_platform == 'win32'} and your project depends on a{sys_platform == 'win32'}, we can conclude that your project's requirements are unsatisfiable.
    "};

    fn scenario() -> Result<Scenario> {
        Ok(toml::from_str(include_str!(
            "../../../../test/scenarios/fork/non-local-fork-marker-unreachable.toml"
        ))?)
    }

    #[test]
    fn recognizes_the_real_disjoint_marker_derivation() -> Result<()> {
        let proof = certify_no_solution(&scenario()?, Some(1), b"", DISJOINT.as_bytes())?;
        assert_eq!(proof.before_external_leaves, 5);
        assert_eq!(proof.after_external_leaves, 3);
        assert_eq!(proof.no_versions.len(), 3);
        assert_eq!(
            proof.no_versions[0].package,
            "missing{sys_platform != 'win32'}"
        );
        assert_eq!(proof.no_versions[0].range, "*");
        assert_eq!(proof.no_versions[1].package, "uv-scenario-root");
        assert!(proof.project_inventory_shown);
        Ok(())
    }

    #[test]
    fn rejects_silent_cache_misses_from_the_candidate_inventory() -> Result<()> {
        let error = certify_no_solution(&scenario()?, Some(1), b"", UNAVAILABLE.as_bytes())
            .expect_err("the supposedly absent candidate exists");
        insta::assert_snapshot!(error, @"the lossy absence claim `a{sys_platform == 'win32'}*` cannot certify a listed scenario candidate a==1.0.0");
        for package in ["a[missing]", "a:dev"] {
            let output = UNAVAILABLE.replace("a{sys_platform == 'win32'}", package);
            assert!(certify_no_solution(&scenario()?, Some(1), b"", output.as_bytes()).is_err());
        }
        Ok(())
    }

    #[test]
    fn rejects_nonempty_inventories_and_uses_the_actual_project_identity() -> Result<()> {
        let mut scenario = scenario()?;
        let name: PackageName = "a".parse()?;
        let package = scenario.packages.get_mut(&name).context("package a")?;
        package
            .versions
            .insert("2.0.0".parse()?, toml::from_str("sdist = false")?);
        let inventory = Inventory::new(&scenario)?;
        for display in ["a>1.0.0", "a[unknown]>1.0.0", "a:dev>1.0.0"] {
            let (package, range) = package_range(display)?;
            assert!(inventory.check_empty_range(&package, &range, true).is_err());
        }

        // A registry package may use the usual project name. Its versions must not be confused
        // with the independently named, fixed-version first-party project.
        scenario.packages.insert(
            "uv-scenario-root".parse()?,
            super::super::scenario::Package {
                versions: [("2.0.0".parse()?, toml::from_str("sdist = false")?)]
                    .into_iter()
                    .collect(),
            },
        );
        let project = Inventory::new(&scenario)?.project_name;
        assert_eq!(project.as_ref(), "uv-scenario-root-root");
        let output = DISJOINT.replace("uv-scenario-root", project.as_ref());
        certify_no_solution(&scenario, Some(1), b"", output.as_bytes())?;
        assert!(certify_no_solution(&scenario, Some(1), b"", DISJOINT.as_bytes()).is_err());
        Ok(())
    }

    #[test]
    fn rejects_lossy_ranges_for_nonempty_version_inventories() -> Result<()> {
        for candidate in ["1.0", "1.0+local", "1.0.post0"] {
            let scenario: Scenario = toml::from_str(&format!(
                "name = 'lossy-inventory'\n[root]\nrequires_python = '>=3.12,<3.15'\nrequires = ['a']\n[expected]\nsatisfiable = true\n[packages.a.versions.'{candidate}']\nsdist = false\n"
            ))?;
            for display in [
                "a==1.0",
                "a<1.0",
                "a<=1.0",
                "a>1.0",
                "a>=1.0",
                "a<1.0 | >2.0",
                "a∅",
            ] {
                let (package, range) = package_range(display)?;
                assert!(
                    Inventory::new(&scenario)
                        .and_then(|inventory| {
                            inventory.check_empty_range(&package, &range, true)
                        })
                        .is_err(),
                    "{candidate} versus {display}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn rejects_local_inventories_when_an_absence_leaf_is_hidden() -> Result<()> {
        let mut scenario = scenario()?;
        scenario.packages.insert(
            "local-only".parse()?,
            super::super::scenario::Package {
                versions: [("1.0+local".parse()?, toml::from_str("sdist = false")?)]
                    .into_iter()
                    .collect(),
            },
        );
        let output = DISJOINT
            .lines()
            .filter(|line| !line.trim_start().starts_with("no versions of missing"))
            .collect::<Vec<_>>()
            .join("\n");
        let error = certify_no_solution(&scenario, Some(1), b"", output.as_bytes())
            .expect_err("the printed trees cannot certify a local-version inventory");
        insta::assert_snapshot!(error, @"the derivation classifier cannot exclude hidden local-version leaves: local-only==1.0+local");
        Ok(())
    }

    #[test]
    fn rejects_real_unavailable_diagnostics() -> Result<()> {
        let unauthorized = format!(
            "{UNAVAILABLE}\nhint: An index URL (http://127.0.0.1:1234/simple/) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)\n"
        );
        let malformed_wheel = UNAVAILABLE.replace(
            "no versions of a{sys_platform == 'win32'}*",
            "a==1.0.0 an invalid package format",
        );
        let cases = [
            (Some(1), unauthorized.as_str()),
            (Some(1), malformed_wheel.as_str()),
            (
                Some(1),
                "error: Received some unexpected JSON from http://127.0.0.1:1234/simple/a/\n  cause: invalid type: string \"invalid\", expected a sequence of files\n",
            ),
            (
                Some(2),
                "error: Failed to fetch: `http://127.0.0.1:1234/simple/a/`\n  cause: HTTP status server error (503 Service Unavailable)\n",
            ),
            (
                Some(1),
                "error: Failed to build `a==1.0.0`\n  cause: the build backend returned an error\n",
            ),
        ];
        for (status, output) in cases {
            assert!(certify_no_solution(&scenario()?, status, b"", output.as_bytes()).is_err());
        }
        Ok(())
    }

    #[test]
    fn rejects_unknown_leaves_and_incomplete_envelopes() -> Result<()> {
        for output in [
            DISJOINT.replace(
                "no versions of missing{sys_platform != 'win32'}*",
                "missing* unsupported availability reason",
            ),
            DISJOINT.replace(
                "no versions of missing{sys_platform != 'win32'}*",
                "no versions of Python<3.12",
            ),
            DISJOINT.replace("missing{sys_platform != 'win32'}*", "unrelated*"),
            DISJOINT.replace(BEFORE, "unknown derivation heading"),
            DISJOINT.replace(AFTER, BEFORE),
            DISJOINT.replace(
                HEADER,
                "error: No solution found when resolving build dependencies",
            ),
            format!("warning: an unknown setting was applied\n{DISJOINT}"),
            format!("{DISJOINT}\nhint: an unknown hint\n"),
            format!("{DISJOINT}\nerror: another failure\n"),
            DISJOINT.replace(
                "missing{sys_platform != 'win32'}*",
                "missing{sys_platform != 'win32'}>=1,<=2",
            ),
        ] {
            assert!(certify_no_solution(&scenario()?, Some(1), b"", output.as_bytes()).is_err());
        }
        assert!(certify_no_solution(&scenario()?, Some(2), b"", DISJOINT.as_bytes()).is_err());
        assert!(
            certify_no_solution(&scenario()?, Some(1), b"unexpected", DISJOINT.as_bytes()).is_err()
        );
        Ok(())
    }

    #[test]
    fn permits_only_known_semantic_hints_and_headers() -> Result<()> {
        let output = DISJOINT.replace(
            HEADER,
            &format!("{HEADER} for split (markers: python_full_version >= '3.13' and sys_platform == 'darwin')"),
        );
        let output = format!(
            "{output}\nhint: While the active Python version is 3.12, the resolution failed for other Python versions supported by your project. Consider limiting your project's supported Python versions using `requires-python`.\nhint: The resolution failed for an environment that is not the current one, consider limiting the environments with `tool.uv.environments`.\n"
        );
        certify_no_solution(&scenario()?, Some(1), b"", output.as_bytes())?;
        for header in [
            " for split (included: a[feature])",
            " for split (markers: extra == 'feature')",
            " for split (markers: invalid marker)",
        ] {
            let output = DISJOINT.replace(HEADER, &format!("{HEADER}{header}"));
            assert!(certify_no_solution(&scenario()?, Some(1), b"", output.as_bytes()).is_err());
        }
        Ok(())
    }

    #[test]
    fn parses_finite_diagnostic_ranges() -> Result<()> {
        let one = Version::from_str("1")?;
        for (range, contains) in [
            ("∅", false),
            ("*", true),
            ("==1.0", true),
            ("<1", false),
            (">1", false),
            (">=1, <=2", true),
            ("<1 | >1", false),
        ] {
            assert_eq!(
                diagnostic_range(range)?.contains_displayed(&one),
                contains,
                "{range}"
            );
        }
        for range in [
            "",
            "~=1",
            "==1.*",
            ">=1,<=2",
            ">2, <1",
            "==1, <2",
            "* | <1",
            "∅ | ==1",
            ">=1, <1+",
            ">1+, <2",
            "==1+local+",
        ] {
            assert!(diagnostic_range(range).is_err(), "{range}");
        }
        Ok(())
    }

    #[test]
    fn keeps_marker_strings_inside_the_package_display() -> Result<()> {
        let leaf = "a{sys_platform == 'x depends on y'}* depends on missing{os_name == '}'}*";
        let (package, dependency) = split_dependency(leaf).context("dependency separator")?;
        package_range(package)?;
        package_range(dependency)?;
        assert_eq!(package, "a{sys_platform == 'x depends on y'}*");
        assert!(
            split_dependency("a{sys_platform == 'x depends on y'}* invalid metadata").is_none()
        );
        assert!(package_range("a{sys_platform == 'unterminated}*").is_err());
        Ok(())
    }

    #[test]
    fn restricts_synthetic_root_claims() -> Result<()> {
        let scenario = scenario()?;
        let inventory = Inventory::new(&scenario)?;
        inventory.check_tree(&["not root root0a0.dev0"], "before", false, &mut Vec::new())?;
        for leaf in [
            "not root other1.0.0",
            "not root root==0a0.dev0",
            "not root root1.0.0",
            "root* depends on a*",
            "term root==1",
            "no versions of root==0a0.dev0",
            "system:libc* depends on a*",
        ] {
            assert!(
                inventory
                    .check_tree(&[leaf], "before", false, &mut Vec::new())
                    .is_err(),
                "{leaf}"
            );
        }
        Ok(())
    }

    #[test]
    fn restricts_the_generated_project_inventory_claim() -> Result<()> {
        let scenario = scenario()?;
        let inventory = Inventory::new(&scenario)?;
        let leaf = "no versions of uv-scenario-root<0.0.0 | >0.0.0";
        inventory.check_tree(&[leaf], "before", true, &mut Vec::new())?;
        assert!(
            inventory
                .check_tree(&[leaf], "before", false, &mut Vec::new())
                .is_err()
        );
        for range in ["*", "∅", "==0.0.0", "<0.0.0", ">0.0.0", "<0 | >0"] {
            let leaf = format!("no versions of uv-scenario-root{range}");
            assert!(
                inventory
                    .check_tree(&[&leaf], "before", true, &mut Vec::new())
                    .is_err()
            );
        }
        let missing_metadata = DISJOINT.replace(
            "uv-scenario-root==0.0.0 depends on a{sys_platform == 'win32'}*",
            "uv-scenario-root* depends on a{sys_platform == 'win32'}*",
        );
        assert!(certify_no_solution(&scenario, Some(1), b"", missing_metadata.as_bytes()).is_err());

        let scenario: Scenario = toml::from_str(&format!(
            "{}\n[packages.b.versions.'1.0.0']\nrequires = [\"uv-scenario-root; sys_platform == 'nonexistent'\"]\nsdist = false\n",
            include_str!("../../../../test/scenarios/fork/non-local-fork-marker-unreachable.toml")
        ))?;
        assert!(Inventory::new(&scenario).is_err());
        Ok(())
    }
}
