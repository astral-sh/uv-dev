use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{Context, Result, bail};
use syn::{Attribute, Expr, ImplItem, Item, LitStr, Type};

use crate::ROOT_DIR;

const ENV_VARS_PATH: &str = "crates/uv-static/src/env_vars.rs";

#[derive(clap::Args)]
pub(crate) struct Args {
    /// The Git revision to compare against.
    #[arg(long)]
    base: String,
}

pub(crate) fn main(args: &Args) -> Result<()> {
    let current = fs_err::read_to_string(format!("{ROOT_DIR}/{ENV_VARS_PATH}"))
        .with_context(|| format!("failed to read `{ENV_VARS_PATH}`"))?;
    let base = read_from_git(&args.base)?;

    check_new_env_vars(&base, &current)
}

fn read_from_git(revision: &str) -> Result<String> {
    let output = Command::new("git")
        .current_dir(ROOT_DIR)
        .arg("show")
        .arg(format!("{revision}:{ENV_VARS_PATH}"))
        .output()
        .with_context(|| format!("failed to run `git show {revision}:{ENV_VARS_PATH}`"))?;

    if !output.status.success() {
        bail!(
            "failed to read `{ENV_VARS_PATH}` at `{revision}`: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    String::from_utf8(output.stdout)
        .with_context(|| format!("`{ENV_VARS_PATH}` at `{revision}` is not valid UTF-8"))
}

fn check_new_env_vars(base: &str, current: &str) -> Result<()> {
    let base = env_vars(base)?;
    let current = env_vars(current)?;
    let mut invalid = Vec::new();

    for (name, added_in) in current {
        if base.contains_key(&name) || added_in.as_deref() == Some("next release") {
            continue;
        }

        if let Some(added_in) = added_in {
            invalid.push(format!("`{name}` uses `{added_in}`"));
        } else {
            invalid.push(format!("`{name}` is missing `#[attr_added_in(...)]`"));
        }
    }

    if invalid.is_empty() {
        return Ok(());
    }

    bail!(
        "new environment variables must use `#[attr_added_in(\"next release\")]`:\n{}",
        invalid.join("\n")
    )
}

fn env_vars(source: &str) -> Result<BTreeMap<String, Option<String>>> {
    let file = syn::parse_file(source).context("failed to parse environment variables")?;
    let mut env_vars = BTreeMap::new();

    for item in file.items {
        let Item::Impl(item) = item else {
            continue;
        };
        let Type::Path(self_type) = item.self_ty.as_ref() else {
            continue;
        };
        if !self_type.path.is_ident("EnvVars") {
            continue;
        }

        for item in item.items {
            let (name, attrs) = match item {
                ImplItem::Const(item) => {
                    let Expr::Lit(expression) = item.expr else {
                        continue;
                    };
                    let syn::Lit::Str(name) = expression.lit else {
                        continue;
                    };
                    (name.value(), item.attrs)
                }
                ImplItem::Fn(item) => {
                    let Some(name) = attribute_value(&item.attrs, "attr_env_var_pattern")? else {
                        continue;
                    };
                    (name, item.attrs)
                }
                _ => continue,
            };
            let added_in = attribute_value(&attrs, "attr_added_in")?;
            env_vars.insert(name, added_in);
        }
    }

    Ok(env_vars)
}

fn attribute_value(attrs: &[Attribute], name: &str) -> Result<Option<String>> {
    attrs
        .iter()
        .find(|attr| attr.path().is_ident(name))
        .map(|attr| attr.parse_args::<LitStr>().map(|value| value.value()))
        .transpose()
        .with_context(|| format!("failed to parse `#[{name}(...)]`"))
}

#[cfg(test)]
mod tests {
    use super::check_new_env_vars;

    const BASE: &str = r#"
        struct EnvVars;

        impl EnvVars {
            #[attr_added_in("0.12.6")]
            pub const EXISTING: &'static str = "EXISTING";
        }
    "#;

    #[test]
    fn accepts_new_env_var_for_next_release() {
        let current = format!(
            r#"{BASE}
            impl EnvVars {{
                #[attr_added_in("next release")]
                pub const NEW: &'static str = "NEW";
            }}
            "#
        );

        check_new_env_vars(BASE, &current).unwrap();
    }

    #[test]
    fn rejects_released_version_on_new_hidden_env_var() {
        let current = format!(
            r#"{BASE}
            impl EnvVars {{
                #[attr_hidden]
                #[attr_added_in("0.12.7")]
                pub const NEW: &'static str = "NEW";
            }}
            "#
        );

        let error = check_new_env_vars(BASE, &current).unwrap_err();
        assert_eq!(
            error.to_string(),
            "new environment variables must use `#[attr_added_in(\"next release\")]`:\n`NEW` uses `0.12.7`"
        );
    }

    #[test]
    fn rejects_missing_annotation_on_new_hidden_env_var() {
        let current = format!(
            r#"{BASE}
            impl EnvVars {{
                #[attr_hidden]
                pub const NEW: &'static str = "NEW";
            }}
            "#
        );

        let error = check_new_env_vars(BASE, &current).unwrap_err();
        assert_eq!(
            error.to_string(),
            "new environment variables must use `#[attr_added_in(\"next release\")]`:\n`NEW` is missing `#[attr_added_in(...)]`"
        );
    }

    #[test]
    fn accepts_release_version_on_existing_env_var() {
        let current = BASE.replace("0.12.6", "0.12.7");

        check_new_env_vars(BASE, &current).unwrap();
    }

    #[test]
    fn checks_patterned_env_vars() {
        let current = format!(
            r#"{BASE}
            impl EnvVars {{
                #[attr_added_in("0.12.7")]
                #[attr_env_var_pattern("UV_INDEX_{{name}}_TOKEN")]
                pub fn index_token(name: &str) -> String {{
                    format!("UV_INDEX_{{name}}_TOKEN")
                }}
            }}
            "#
        );

        let error = check_new_env_vars(BASE, &current).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("`UV_INDEX_{name}_TOKEN` uses `0.12.7`")
        );
    }
}
