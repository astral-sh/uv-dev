use uv_errors::Hints;
use uv_normalize::PackageName;
use uv_warnings::warn_user_with_chain;

pub(crate) fn warn_malformed_tool(name: &PackageName, error: uv_tool::Error) {
    warn_user_with_chain!(
        anyhow::Error::from(error)
            .context(format!("Ignoring malformed tool `{name}`"))
            .as_ref(),
        Hints::from(format!(
            "Run `uv tool uninstall {name}` to remove the tool."
        )),
    );
}

pub(crate) fn warn_invalid_environment(name: &PackageName, error: uv_tool::Error) {
    warn_user_with_chain!(
        anyhow::Error::from(error)
            .context(format!(
                "Ignoring tool `{name}` with an invalid environment"
            ))
            .as_ref(),
        Hints::from(format!(
            "Run `uv tool install {name} --reinstall` to reinstall the tool."
        )),
    );
}
