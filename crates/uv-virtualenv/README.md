# uv-virtualenv

`uv-virtualenv` is a Rust library that creates Python virtual environments. It also includes a CLI.

## Syncing with upstream virtualenv activation scripts

This crate follows the activation scripts from the pypa/virtualenv project. Some differences support
this crate's implementation.

### License disclaimers added

Each activation script starts with license information, as the pypa/virtualenv project's MIT License
requires. Do not remove the license comments from this crate's activation scripts.

### Placeholder names are slightly different

The activation scripts are templates. When uv creates a virtual environment, it fills each template
with the required values.

The upstream project defines its placeholder names in
[`virtualenv.activation.ViaTemplateActivator.replacements()`][upstream-placeholders].

This crate defines its placeholder names in
[`uv_virtualenv::virtualenv::create()`][crate-placeholders].

[upstream-placeholders]:
  https://github.com/pypa/virtualenv/blob/dad9369e97f5aef7e33777b18dcdb51b1fdac7bd/src/virtualenv/activation/via_template.py#L43
[crate-placeholders]:
  https://github.com/astral-sh/uv/blob/d8f3f03198308be53de51a3a297c85566eabb084/crates/uv-virtualenv/src/virtualenv.rs#L462

The placeholder names in the activation scripts must match those in [this crate's
source][crate-placeholders].

### Relocatable virtual environments

This crate modifies its activation scripts to support relocatable virtual environments. Keep the
patch in [astral-sh/uv#5640].

[astral-sh/uv#5640]: https://github.com/astral-sh/uv/pull/5640

### TCL/TK library locations

The upstream virtualenv patches [pypa/virtualenv#2928] and [pypa/virtualenv#2940] locate the TCL/TK
libraries of a base Python distribution dynamically. See the [upstream
approach][upstream-tcl/tk-approach].

[pypa/virtualenv#2928]: https://github.com/pypa/virtualenv/pull/2928
[pypa/virtualenv#2940]: https://github.com/pypa/virtualenv/pull/2940
[upstream-tcl/tk-approach]:
  https://github.com/pypa/virtualenv/blob/dad9369e97f5aef7e33777b18dcdb51b1fdac7bd/src/virtualenv/discovery/py_info.py#L140

This project does not use the upstream implementation because it adds complexity. When you sync
activation scripts from upstream, omit the TCL/TK patches.
