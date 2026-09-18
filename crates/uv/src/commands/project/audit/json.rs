//! JSON layout models for `uv audit`.

use serde::Serialize;
use uv_normalize::PackageName;

use super::AuditResults;

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub(crate) struct Report {
    schema: Schema,
    #[serde(flatten)]
    body: ReportBody,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct ReportBody {
    summary: Summary,
    vulnerabilities: Vec<Vulnerability>,
    adverse_statuses: Vec<AdverseStatus>,
}

/// Generate the JSON schema for preview project audit reports.
#[cfg(feature = "schemars")]
pub fn project_json_schema() -> schemars::Schema {
    let mut schema = schemars::generate::SchemaSettings::draft07()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<Report>();
    schema.insert("title".to_owned(), "uv audit (preview)".into());
    schema
}

/// Generate the per-record schema for preview JSONL project audit reports.
#[cfg(feature = "schemars")]
pub fn project_jsonl_schema() -> schemars::Schema {
    crate::commands::report::jsonl_object_schema::<Report>("uv audit JSONL (preview)")
}

impl Report {
    pub(crate) fn from_findings(
        n_packages: usize,
        vulnerabilities: &[&uv_audit::Vulnerability],
        statuses: &[&uv_audit::ProjectStatus],
    ) -> Self {
        let mut vulnerabilities = vulnerabilities
            .iter()
            .copied()
            .map(Vulnerability::from)
            .collect::<Vec<_>>();
        vulnerabilities.sort_by(|first, second| {
            first
                .dependency
                .name
                .cmp(&second.dependency.name)
                .then_with(|| first.dependency.version.cmp(&second.dependency.version))
                .then_with(|| first.display_id.cmp(&second.display_id))
        });

        let mut adverse_statuses = statuses
            .iter()
            .copied()
            .map(AdverseStatus::from)
            .collect::<Vec<_>>();
        adverse_statuses.sort_by(|first, second| {
            first
                .name
                .cmp(&second.name)
                .then_with(|| first.status.cmp(&second.status))
        });

        Self {
            schema: Schema::default(),
            body: ReportBody {
                summary: Summary {
                    audited_packages: n_packages,
                    vulnerabilities: vulnerabilities.len(),
                    adverse_statuses: adverse_statuses.len(),
                },
                vulnerabilities,
                adverse_statuses,
            },
        }
    }
}

/// JSON report containing separate findings for each audited tool.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub(crate) struct ToolReports {
    schema: Schema,
    tools: Vec<ToolReport>,
}

/// Generate the JSON schema for preview tool audit reports.
#[cfg(feature = "schemars")]
pub fn tool_json_schema() -> schemars::Schema {
    let mut schema = schemars::generate::SchemaSettings::draft07()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<ToolReports>();
    schema.insert("title".to_owned(), "uv tool audit (preview)".into());
    schema
}

/// Generate the per-record schema for preview JSONL tool audit reports.
#[cfg(feature = "schemars")]
pub fn tool_jsonl_schema() -> schemars::Schema {
    crate::commands::report::jsonl_object_schema::<ToolReports>("uv tool audit JSONL (preview)")
}

impl ToolReports {
    pub(crate) fn from_audits(audits: &[(PackageName, AuditResults)]) -> Self {
        let tools = audits
            .iter()
            .map(|(name, results)| {
                let (vulnerabilities, statuses) = results.split_findings();
                let report = Report::from_findings(results.n_packages, &vulnerabilities, &statuses);

                ToolReport {
                    name: name.to_string(),
                    body: report.body,
                }
            })
            .collect();

        Self {
            schema: Schema::default(),
            tools,
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct ToolReport {
    name: String,
    #[serde(flatten)]
    body: ReportBody,
}

#[derive(Debug, Serialize, Default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct Schema {
    version: SchemaVersion,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
enum SchemaVersion {
    #[default]
    Preview,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct Summary {
    #[cfg_attr(feature = "schemars", schemars(range(max = u64::MAX)))]
    audited_packages: usize,
    #[cfg_attr(feature = "schemars", schemars(range(max = u64::MAX)))]
    vulnerabilities: usize,
    #[cfg_attr(feature = "schemars", schemars(range(max = u64::MAX)))]
    adverse_statuses: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct Dependency {
    name: String,
    version: String,
}

impl From<&uv_audit::Dependency> for Dependency {
    fn from(dependency: &uv_audit::Dependency) -> Self {
        Self {
            name: dependency.name().to_string(),
            version: dependency.version().to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct Vulnerability {
    dependency: Dependency,
    id: String,
    display_id: String,
    aliases: Vec<String>,
    summary: Option<String>,
    description: Option<String>,
    link: Option<String>,
    fix_versions: Vec<String>,
    published: Option<String>,
    modified: Option<String>,
}

impl From<&uv_audit::Vulnerability> for Vulnerability {
    fn from(vulnerability: &uv_audit::Vulnerability) -> Self {
        Self {
            dependency: Dependency::from(&vulnerability.dependency),
            id: vulnerability.id.as_str().to_string(),
            display_id: vulnerability.best_id().as_str().to_string(),
            aliases: vulnerability
                .aliases
                .iter()
                .map(|id| id.as_str().to_string())
                .collect(),
            summary: vulnerability.summary.clone(),
            description: vulnerability.description.clone(),
            link: vulnerability
                .link
                .as_ref()
                .map(|link| link.as_str().to_string()),
            fix_versions: vulnerability
                .fix_versions
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            published: vulnerability
                .published
                .as_ref()
                .map(std::string::ToString::to_string),
            modified: vulnerability
                .modified
                .as_ref()
                .map(std::string::ToString::to_string),
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct AdverseStatus {
    name: String,
    status: String,
    reason: Option<String>,
}

impl From<&uv_audit::ProjectStatus> for AdverseStatus {
    fn from(status: &uv_audit::ProjectStatus) -> Self {
        Self {
            name: status.name.to_string(),
            status: status.status.to_string(),
            reason: status.reason.clone(),
        }
    }
}

#[cfg(all(test, feature = "schemars"))]
mod tests {
    use anyhow::{Context, Result};
    use serde_json::{Value, json};
    use uv_audit::{
        AdverseStatus, Dependency, Finding, ProjectStatus, Vulnerability, VulnerabilityID,
    };
    use uv_cli::AuditOutputFormat;
    use uv_redacted::DisplaySafeUrl;
    use uv_test::json_schema::JsonSchema;

    use super::{
        AuditResults, Report, ToolReports, project_json_schema, project_jsonl_schema,
        tool_json_schema, tool_jsonl_schema,
    };
    use crate::printer::{Printer, jsonl_result};

    fn audit() -> Result<AuditResults> {
        Ok(AuditResults {
            printer: Printer::Silent,
            n_packages: 3,
            output_format: AuditOutputFormat::Json,
            findings: vec![
                Finding::Vulnerability(Box::new(Vulnerability {
                    dependency: Dependency::new("example".parse()?, "1.0".parse()?),
                    id: VulnerabilityID::new("SOURCE-123"),
                    summary: Some("Example finding".to_owned()),
                    description: Some("Finding details".to_owned()),
                    link: Some(DisplaySafeUrl::parse("https://example.org/advisory/123")?),
                    fix_versions: vec!["1.1".parse()?],
                    aliases: vec![VulnerabilityID::new("CVE-2026-12345")],
                    published: Some("2026-01-01T00:00:00Z".parse()?),
                    modified: Some("2026-02-01T00:00:00Z".parse()?),
                })),
                Finding::ProjectStatus(ProjectStatus {
                    name: "archived-example".parse()?,
                    status: AdverseStatus::Archived,
                    reason: None,
                }),
            ],
            artifact_uri: "uv.lock".to_owned(),
        })
    }

    #[test]
    fn project_schemas_describe_serialized_findings() -> Result<()> {
        let results = audit()?;
        let (vulnerabilities, statuses) = results.split_findings();
        let report = Report::from_findings(results.n_packages, &vulnerabilities, &statuses);
        let payload = serde_json::to_value(&report)?;
        assert_eq!(
            payload["vulnerabilities"][0]["display_id"],
            "CVE-2026-12345"
        );
        assert_eq!(payload["adverse_statuses"][0]["reason"], Value::Null);

        let document = serde_json::to_value(project_json_schema())?;
        assert_eq!(document["title"], "uv audit (preview)");
        let validator = JsonSchema::new(&serde_json::to_string(&document)?)?;
        validator.parse(&serde_json::to_vec(&report)?)?;
        validator.parse(&serde_json::to_vec(&Report::from_findings(0, &[], &[]))?)?;
        let record_validator = JsonSchema::new(&serde_json::to_string(&project_jsonl_schema())?)?;
        record_validator.parse(jsonl_result(&report)?.as_bytes())?;

        for field in ["summary", "description", "link", "published", "modified"] {
            let mut nullable = payload.clone();
            nullable["vulnerabilities"][0][field] = Value::Null;
            validator.parse(&serde_json::to_vec(&nullable)?)?;
        }
        for field in [
            "dependency",
            "id",
            "display_id",
            "aliases",
            "summary",
            "description",
            "link",
            "fix_versions",
            "published",
            "modified",
        ] {
            let mut missing = payload.clone();
            missing["vulnerabilities"][0]
                .as_object_mut()
                .context("expected a vulnerability")?
                .remove(field);
            assert!(validator.parse(&serde_json::to_vec(&missing)?).is_err());
        }
        for field in ["audited_packages", "vulnerabilities", "adverse_statuses"] {
            assert_eq!(
                document["definitions"]["Summary"]["properties"][field]["maximum"],
                u64::MAX
            );
            let mut maximum = payload.clone();
            maximum["summary"][field] = json!(u64::MAX);
            let maximum = serde_json::to_string(&maximum)?;
            validator.parse(maximum.as_bytes())?;
            let above = maximum.replacen(
                &u64::MAX.to_string(),
                &(u128::from(u64::MAX) + 1).to_string(),
                1,
            );
            assert!(validator.parse(above.as_bytes()).is_err());
            let mut negative = payload.clone();
            negative["summary"][field] = json!(-1);
            assert!(validator.parse(&serde_json::to_vec(&negative)?).is_err());
        }
        let mut invalid_version = payload;
        invalid_version["schema"]["version"] = json!(1);
        assert!(
            validator
                .parse(&serde_json::to_vec(&invalid_version)?)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn tool_schemas_describe_serialized_findings() -> Result<()> {
        let report = ToolReports::from_audits(&[
            ("first-tool".parse()?, audit()?),
            ("second-tool".parse()?, audit()?),
        ]);
        let payload = serde_json::to_value(&report)?;
        assert_eq!(payload["tools"][0]["name"], "first-tool");
        assert_eq!(payload["tools"][1]["name"], "second-tool");
        assert_eq!(
            payload["tools"][0]["vulnerabilities"][0]["display_id"],
            "CVE-2026-12345"
        );
        assert_eq!(
            payload["tools"][0]["adverse_statuses"][0]["reason"],
            Value::Null
        );

        let document = serde_json::to_value(tool_json_schema())?;
        assert_eq!(document["title"], "uv tool audit (preview)");
        let validator = JsonSchema::new(&serde_json::to_string(&document)?)?;
        validator.parse(&serde_json::to_vec(&report)?)?;
        validator.parse(&serde_json::to_vec(&ToolReports::from_audits(&[]))?)?;
        let record_validator = JsonSchema::new(&serde_json::to_string(&tool_jsonl_schema())?)?;
        record_validator.parse(jsonl_result(&report)?.as_bytes())?;

        for field in ["name", "summary", "vulnerabilities", "adverse_statuses"] {
            let mut missing = payload.clone();
            missing["tools"][0]
                .as_object_mut()
                .context("expected a tool report")?
                .remove(field);
            assert!(validator.parse(&serde_json::to_vec(&missing)?).is_err());
        }
        for field in ["audited_packages", "vulnerabilities", "adverse_statuses"] {
            assert_eq!(
                document["definitions"]["Summary"]["properties"][field]["maximum"],
                u64::MAX
            );
            let mut maximum = payload.clone();
            maximum["tools"][0]["summary"][field] = json!(u64::MAX);
            let maximum = serde_json::to_string(&maximum)?;
            validator.parse(maximum.as_bytes())?;
            let above = maximum.replacen(
                &u64::MAX.to_string(),
                &(u128::from(u64::MAX) + 1).to_string(),
                1,
            );
            assert!(validator.parse(above.as_bytes()).is_err());
            let mut negative = payload.clone();
            negative["tools"][0]["summary"][field] = json!(-1);
            assert!(validator.parse(&serde_json::to_vec(&negative)?).is_err());
        }
        let mut invalid_version = payload;
        invalid_version["schema"]["version"] = json!(1);
        assert!(
            validator
                .parse(&serde_json::to_vec(&invalid_version)?)
                .is_err()
        );
        Ok(())
    }
}
