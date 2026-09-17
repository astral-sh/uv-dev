use uv_normalize::ExtraName;
use uv_pep508::{ExtraOperator, MarkerExpression, MarkerTree, MarkerValueExtra};

const AXIS_PREFIX: &str = "uv-axis-a";
const CONTEXT_PREFIX: &str = "uv-axis-context-c";

/// Encode a workspace axis and section as a private marker atom.
///
/// Both names are encoded byte-for-byte so separators, normalization, and user-defined extras
/// cannot make two different axis assignments share an atom. These atoms are only used in uv's
/// universal lock graph; they are not PEP 508 requirement extras.
pub fn workspace_axis_extra(axis: &str, section: &str) -> ExtraName {
    let name = format!("{}{}-s{}", AXIS_PREFIX, hex_name(axis), hex_name(section));
    ExtraName::from_owned(name).expect("hexadecimal axis marker is a valid extra name")
}

/// Return a marker that is true when the named workspace axis selects this section.
pub fn workspace_axis_marker(axis: &str, section: &str) -> MarkerTree {
    extra_marker(workspace_axis_extra(axis, section))
}

/// Encode a lockfile's explicitly identified workspace context as a private wire-only atom.
pub fn workspace_axis_context_extra(context: u32) -> ExtraName {
    ExtraName::from_owned(format!("{CONTEXT_PREFIX}{context}"))
        .expect("decimal context marker is a valid extra name")
}

/// Return the wire-only marker for one persisted workspace context.
pub fn workspace_axis_context_marker(context: u32) -> MarkerTree {
    extra_marker(workspace_axis_context_extra(context))
}

/// Return whether an extra belongs to the private lock-context namespace.
pub fn is_workspace_axis_context_extra(extra: &ExtraName) -> bool {
    extra.as_str().starts_with(CONTEXT_PREFIX)
}

/// Decode a canonical lock-context identifier.
pub fn workspace_axis_context_id(extra: &ExtraName) -> Option<u32> {
    let digits = extra.as_str().strip_prefix(CONTEXT_PREFIX)?;
    let context = digits.parse::<u32>().ok()?;
    (digits == context.to_string()).then_some(context)
}

fn extra_marker(name: ExtraName) -> MarkerTree {
    MarkerTree::expression(MarkerExpression::Extra {
        operator: ExtraOperator::Equal,
        name: MarkerValueExtra::Extra(name),
    })
}

/// Return whether an extra name is in uv's private workspace-axis namespace.
pub fn is_workspace_axis_extra(extra: &ExtraName) -> bool {
    extra.as_str().starts_with(AXIS_PREFIX)
}

/// Evaluate one axis assignment without changing other axes or ordinary conflict markers.
pub fn select_workspace_axis_marker(marker: MarkerTree, axis: &str, section: &str) -> MarkerTree {
    let selected = workspace_axis_extra(axis, section);
    let prefix = format!("{}{}-s", AXIS_PREFIX, hex_name(axis));
    marker
        .simplify_extras_with(|candidate| *candidate == selected)
        .simplify_not_extras_with(|candidate| {
            candidate.as_str().starts_with(&prefix) && *candidate != selected
        })
}

/// Existentially remove workspace-axis atoms while retaining ordinary conflict markers.
///
/// The caller must first intersect the marker with the valid axis domain. In particular, merely
/// dropping an unspecified axis is not equivalent to choosing every one of its sections.
pub fn without_workspace_axis_markers(marker: MarkerTree) -> MarkerTree {
    marker.without_extras_with(is_workspace_axis_extra)
}

/// Existentially remove ordinary conflict atoms while retaining workspace-axis selectors.
pub fn without_non_workspace_axis_markers(marker: MarkerTree) -> MarkerTree {
    marker.without_extras_with(|extra| !is_workspace_axis_extra(extra))
}

fn hex_name(name: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(name.len() * 2);
    for byte in name.bytes() {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use uv_pep508::MarkerTree;

    use super::{
        select_workspace_axis_marker, without_workspace_axis_markers, workspace_axis_extra,
        workspace_axis_marker,
    };

    #[test]
    fn axis_atom_encoding_is_unambiguous() {
        assert_ne!(
            workspace_axis_extra("one-two", "three"),
            workspace_axis_extra("one", "two-three")
        );
        assert_ne!(
            workspace_axis_extra("a_b", "c"),
            workspace_axis_extra("a-b", "c")
        );
    }

    #[test]
    fn selecting_one_axis_retains_other_axes() {
        let python = workspace_axis_marker("python", "py313");
        let sqlalchemy = workspace_axis_marker("sqlalchemy", "v2");
        let combined = python.and(sqlalchemy);
        assert_eq!(
            select_workspace_axis_marker(combined, "python", "py313"),
            sqlalchemy
        );
        assert_eq!(
            select_workspace_axis_marker(combined, "python", "py312"),
            MarkerTree::FALSE
        );
    }

    #[test]
    fn projection_preserves_python_and_ordinary_conflicts() -> anyhow::Result<()> {
        let python: MarkerTree = "python_full_version >= '3.13'".parse()?;
        let conflict: MarkerTree = "extra == 'extra-3-foo-test'".parse()?;
        let axis = workspace_axis_marker("sqlalchemy", "v2");
        assert_eq!(
            without_workspace_axis_markers(python.and(conflict).and(axis.negate())),
            python.and(conflict)
        );
        Ok(())
    }
}
