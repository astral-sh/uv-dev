//! Validate complete preview JSONL command streams.

use std::collections::BTreeMap;
use std::process::{ExitStatus, Output};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

use crate::json_schema::JsonSchema;

/// Whether this invocation is expected to emit a final result.
#[derive(Debug, Clone, Copy)]
pub enum JsonlResultExpectation {
    /// A completed report is required, even if the command exits unsuccessfully.
    Required,
    /// No result is expected, such as for silent output or a setup failure.
    Forbidden,
    /// A successful command must report a result; a failed command may omit it.
    OptionalOnFailure,
}

/// The observed lifecycle of one process-wide progress operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonlOperation {
    /// The phase shared by every record for this operation.
    pub phase: String,
    /// Whether a completion record was observed, not whether the operation succeeded.
    pub completed: bool,
}

/// A schema-valid, newline-delimited command stream and its actual process status.
#[derive(Debug)]
pub struct JsonlOutput {
    pub status: ExitStatus,
    pub progress: Vec<Value>,
    pub result: Option<Value>,
    pub operations: BTreeMap<u64, JsonlOperation>,
}

impl JsonlOutput {
    /// Validate framing, individual records, final-result order, and operation IDs.
    ///
    /// Retries and failed operations can leave an ID without a completion record, even
    /// when the command eventually succeeds. Counts, totals, and optional metadata are
    /// observations, not generic monotonicity or accounting guarantees. Tests with
    /// controlled inputs can impose stronger assertions on the returned operations.
    pub fn parse(
        schema: &JsonSchema,
        output: &Output,
        expectation: JsonlResultExpectation,
    ) -> Result<Self> {
        let contents =
            std::str::from_utf8(&output.stdout).context("JSONL output is not valid UTF-8")?;
        ensure!(
            contents.is_empty() || contents.ends_with('\n'),
            "incomplete final JSONL record"
        );

        let mut parsed = Self {
            status: output.status,
            progress: Vec::new(),
            result: None,
            operations: BTreeMap::new(),
        };
        if !contents.is_empty() {
            for (index, line) in contents.split_terminator('\n').enumerate() {
                let line_number = index + 1;
                ensure!(!line.trim().is_empty(), "empty JSONL record {line_number}");
                ensure!(
                    parsed.result.is_none(),
                    "JSONL record {line_number} follows the final result"
                );
                let record = schema
                    .parse(line.as_bytes())
                    .with_context(|| format!("invalid JSONL record {line_number}"))?;
                match record.get("type").and_then(Value::as_str) {
                    Some("progress") => {
                        parsed
                            .observe_progress(&record)
                            .with_context(|| format!("invalid JSONL record {line_number}"))?;
                        parsed.progress.push(record);
                    }
                    Some("result") => parsed.result = Some(record),
                    Some(event_type) => {
                        bail!("JSONL record {line_number} has unknown type `{event_type}`")
                    }
                    None => bail!("JSONL record {line_number} has no string type"),
                }
            }
        }

        match expectation {
            JsonlResultExpectation::Required => {
                ensure!(parsed.result.is_some(), "missing final JSONL result");
            }
            JsonlResultExpectation::Forbidden => {
                ensure!(parsed.result.is_none(), "unexpected final JSONL result");
            }
            JsonlResultExpectation::OptionalOnFailure => {
                ensure!(
                    !parsed.status.success() || parsed.result.is_some(),
                    "successful command omitted its final JSONL result"
                );
            }
        }
        Ok(parsed)
    }

    fn observe_progress(&mut self, record: &Value) -> Result<()> {
        let phase = record
            .get("phase")
            .and_then(Value::as_str)
            .context("progress record has no string phase")?;
        let status = record
            .get("status")
            .and_then(Value::as_str)
            .context("progress record has no string status")?;
        let Some(id) = record.get("id") else {
            // Top-level phases can repeat and interleave across internal reporters.
            return Ok(());
        };
        let id = id
            .as_u64()
            .context("progress ID is not an unsigned integer")?;

        match status {
            "started" => {
                ensure!(
                    !self.operations.contains_key(&id),
                    "operation {id} has more than one start"
                );
                self.operations.insert(
                    id,
                    JsonlOperation {
                        phase: phase.to_owned(),
                        completed: false,
                    },
                );
            }
            "updated" | "completed" => {
                let operation = self
                    .operations
                    .get_mut(&id)
                    .with_context(|| format!("operation {id} has no observed start"))?;
                ensure!(
                    operation.phase == phase,
                    "operation {id} changed phase from `{}` to `{phase}`",
                    operation.phase
                );
                ensure!(!operation.completed, "operation {id} was already completed");
                operation.completed = status == "completed";
            }
            _ => bail!("unknown progress status `{status}`"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    use anyhow::Result;
    use serde_json::{Value, json};

    use super::{JsonlOperation, JsonlOutput, JsonlResultExpectation};
    use crate::json_schema::JsonSchema;

    fn schema() -> Result<JsonSchema> {
        JsonSchema::new(include_str!(
            "../../../docs/reference/internals/lock-jsonl.schema.json"
        ))
    }

    fn output(code: u8, stdout: Vec<u8>) -> Output {
        #[cfg(unix)]
        let status = ExitStatus::from_raw(i32::from(code) << 8);
        #[cfg(windows)]
        let status = ExitStatus::from_raw(u32::from(code));
        Output {
            status,
            stdout,
            stderr: Vec::new(),
        }
    }

    fn capture(code: u8, records: &[Value]) -> Result<Output> {
        let mut stdout = Vec::new();
        for record in records {
            serde_json::to_writer(&mut stdout, record)?;
            stdout.push(b'\n');
        }
        Ok(output(code, stdout))
    }

    fn result() -> Value {
        json!({
            "type": "result",
            "schema": {"version": "preview"},
            "status": "fresh",
            "dry_run": false
        })
    }

    fn progress(phase: &str, status: &str, id: u64) -> Value {
        json!({"type": "progress", "phase": phase, "status": status, "id": id})
    }

    #[test]
    fn correlates_interleaved_operations() -> Result<()> {
        let records = [
            progress("download", "started", 9),
            progress("checkout", "started", 3),
            progress("download", "updated", 9),
            progress("checkout", "completed", 3),
            progress("download", "completed", 9),
            result(),
        ];
        let output = capture(0, &records)?;
        let parsed = JsonlOutput::parse(&schema()?, &output, JsonlResultExpectation::Required)?;
        assert!(parsed.status.success());
        assert_eq!(parsed.progress, records[..5]);
        assert_eq!(parsed.result, Some(result()));
        assert_eq!(
            parsed.operations,
            [
                (
                    3,
                    JsonlOperation {
                        phase: "checkout".to_owned(),
                        completed: true
                    }
                ),
                (
                    9,
                    JsonlOperation {
                        phase: "download".to_owned(),
                        completed: true
                    }
                ),
            ]
            .into()
        );
        Ok(())
    }

    #[test]
    fn retains_abandoned_operations_and_observed_metadata() -> Result<()> {
        let records = [
            progress("download", "started", 20),
            json!({"type":"progress","phase":"download","status":"updated","id":20,"completed":8,"total":4}),
            progress("download", "started", 4),
            json!({"type":"progress","phase":"download","status":"updated","id":4,"completed":8,"total":4}),
            json!({"type":"progress","phase":"download","status":"updated","id":4,"completed":2}),
            progress("download", "completed", 4),
            json!({"type":"progress","phase":"checkout","status":"started","id":21,"revision":"main"}),
            json!({"type":"progress","phase":"checkout","status":"completed","id":21,"revision":"0123456"}),
            result(),
        ];
        let parsed = JsonlOutput::parse(
            &schema()?,
            &capture(0, &records)?,
            JsonlResultExpectation::Required,
        )?;
        assert_eq!(parsed.progress, records[..8]);
        assert_eq!(
            parsed.operations.get(&20),
            Some(&JsonlOperation {
                phase: "download".to_owned(),
                completed: false
            })
        );
        assert_eq!(
            parsed
                .operations
                .get(&4)
                .map(|operation| operation.completed),
            Some(true)
        );
        assert_eq!(
            parsed
                .operations
                .get(&21)
                .map(|operation| operation.completed),
            Some(true)
        );
        Ok(())
    }

    #[test]
    fn accepts_repeated_idless_phases() -> Result<()> {
        let records = [
            json!({"type":"progress","phase":"resolve","status":"started"}),
            json!({"type":"progress","phase":"prepare","status":"started"}),
            json!({"type":"progress","phase":"resolve","status":"completed"}),
            json!({"type":"progress","phase":"resolve","status":"started"}),
            json!({"type":"progress","phase":"prepare","status":"updated","completed":2}),
            json!({"type":"progress","phase":"prepare","status":"updated","completed":1}),
            json!({"type":"progress","phase":"resolve","status":"completed"}),
            result(),
        ];
        let parsed = JsonlOutput::parse(
            &schema()?,
            &capture(0, &records)?,
            JsonlResultExpectation::Required,
        )?;
        assert_eq!(parsed.progress, records[..7]);
        assert!(parsed.operations.is_empty());
        Ok(())
    }

    #[test]
    fn keeps_result_expectations_separate_from_exit_status() -> Result<()> {
        let schema = schema()?;
        let completed = capture(1, &[result()])?;
        let parsed = JsonlOutput::parse(&schema, &completed, JsonlResultExpectation::Required)?;
        assert_eq!(parsed.status.code(), Some(1));
        assert!(parsed.result.is_some());

        let partial = capture(2, &[progress("download", "started", 17)])?;
        let parsed =
            JsonlOutput::parse(&schema, &partial, JsonlResultExpectation::OptionalOnFailure)?;
        assert_eq!(parsed.status.code(), Some(2));
        assert!(parsed.result.is_none());
        assert_eq!(
            parsed
                .operations
                .get(&17)
                .map(|operation| operation.completed),
            Some(false)
        );
        JsonlOutput::parse(&schema, &partial, JsonlResultExpectation::Forbidden)?;
        JsonlOutput::parse(
            &schema,
            &capture(0, &[])?,
            JsonlResultExpectation::Forbidden,
        )?;

        let errors = [
            JsonlOutput::parse(&schema, &partial, JsonlResultExpectation::Required)
                .expect_err("a completed result is required")
                .to_string(),
            JsonlOutput::parse(&schema, &completed, JsonlResultExpectation::Forbidden)
                .expect_err("a result is forbidden")
                .to_string(),
            JsonlOutput::parse(
                &schema,
                &capture(0, &[])?,
                JsonlResultExpectation::OptionalOnFailure,
            )
            .expect_err("successful output requires a result")
            .to_string(),
        ];
        insta::assert_json_snapshot!(errors, @r#"
        [
          "missing final JSONL result",
          "unexpected final JSONL result",
          "successful command omitted its final JSONL result"
        ]
        "#);
        Ok(())
    }

    #[test]
    fn rejects_incomplete_or_malformed_records() -> Result<()> {
        let schema = schema()?;
        let complete = capture(0, &[result()])?;
        let mut crlf = serde_json::to_vec(&result())?;
        crlf.extend_from_slice(b"\r\n");
        JsonlOutput::parse(&schema, &output(0, crlf), JsonlResultExpectation::Required)?;
        let mut incomplete = complete.stdout.clone();
        incomplete.pop();
        let mut trailing_blank = complete.stdout;
        trailing_blank.push(b'\n');
        let inputs = [
            vec![0xff, b'\n'],
            incomplete,
            b"\n".to_vec(),
            b" \r\n".to_vec(),
            trailing_blank,
            b"{\n".to_vec(),
            b"{\"type\":\"progress\",\"phase\":\"unknown\",\"status\":\"started\"}\n".to_vec(),
            b"{\"type\":\"result\"}\n".to_vec(),
        ];
        let errors = inputs
            .into_iter()
            .map(|contents| {
                JsonlOutput::parse(
                    &schema,
                    &output(2, contents),
                    JsonlResultExpectation::OptionalOnFailure,
                )
                .expect_err("invalid stream must be rejected")
                .to_string()
            })
            .collect::<Vec<_>>();
        insta::assert_json_snapshot!(errors, @r#"
        [
          "JSONL output is not valid UTF-8",
          "incomplete final JSONL record",
          "empty JSONL record 1",
          "empty JSONL record 1",
          "empty JSONL record 2",
          "invalid JSONL record 1",
          "invalid JSONL record 1",
          "invalid JSONL record 1"
        ]
        "#);
        Ok(())
    }

    #[test]
    fn rejects_records_after_the_result() -> Result<()> {
        let schema = schema()?;
        let errors = [result(), progress("download", "started", 1)]
            .into_iter()
            .map(|following| -> Result<String> {
                let error = JsonlOutput::parse(
                    &schema,
                    &capture(0, &[result(), following])?,
                    JsonlResultExpectation::Required,
                )
                .expect_err("a result must be the last record");
                Ok(error.to_string())
            })
            .collect::<Result<Vec<_>>>()?;
        insta::assert_json_snapshot!(errors, @r#"
        [
          "JSONL record 2 follows the final result",
          "JSONL record 2 follows the final result"
        ]
        "#);
        Ok(())
    }

    #[test]
    fn rejects_invalid_operation_lifecycles() -> Result<()> {
        let schema = schema()?;
        let cases = [
            vec![progress("download", "updated", 7)],
            vec![progress("download", "completed", 7)],
            vec![
                progress("download", "started", 7),
                progress("download", "started", 7),
            ],
            vec![
                progress("download", "started", 7),
                progress("extract", "updated", 7),
            ],
            vec![
                progress("download", "started", 7),
                progress("download", "completed", 7),
                progress("download", "updated", 7),
            ],
            vec![
                progress("download", "started", 7),
                progress("download", "completed", 7),
                progress("build", "started", 7),
            ],
        ];
        let errors = cases
            .into_iter()
            .map(|records| -> Result<String> {
                let error = JsonlOutput::parse(
                    &schema,
                    &capture(2, &records)?,
                    JsonlResultExpectation::OptionalOnFailure,
                )
                .expect_err("invalid operation lifecycle must be rejected");
                Ok(format!("{error:#}"))
            })
            .collect::<Result<Vec<_>>>()?;
        insta::assert_json_snapshot!(errors, @r#"
        [
          "invalid JSONL record 1: operation 7 has no observed start",
          "invalid JSONL record 1: operation 7 has no observed start",
          "invalid JSONL record 2: operation 7 has more than one start",
          "invalid JSONL record 2: operation 7 changed phase from `download` to `extract`",
          "invalid JSONL record 3: operation 7 was already completed",
          "invalid JSONL record 3: operation 7 has more than one start"
        ]
        "#);
        Ok(())
    }
}
