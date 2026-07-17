//! `uv-audit` provides types and interfaces for auditing Python dependencies.

pub use service::{ProjectStatusAudit, VulnerabilityServiceFormat, osv};
pub use types::{
    AdverseStatus, Dependency, Finding, ProjectStatus, Vulnerability, VulnerabilityID,
};

mod service;
mod types;
