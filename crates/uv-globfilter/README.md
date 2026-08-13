# globfilter

Walks directories and applies include and exclude patterns across platforms.

For example, use patterns to select paths within a project:

```toml
include = ["src", "License.txt", "resources/icons/*.svg"]
exclude = ["target", "/dist", ".cache", "*.tmp"]
```

When you traverse a directory, call `GlobDirFilter::from_globs(...)?.match_directory(&relative)` to
skip directories that cannot match. Use this method in the `filter_entry` method of `WalkDir`.

## Syntax

This crate supports the restricted, cross-language glob syntax from
[PEP 639](https://packaging.python.org/en/latest/specifications/glob-patterns/):

- Alphanumeric characters, underscores (`_`), hyphens (`-`), and dots (`.`) match exactly.
- Special glob characters match as follows:
  - `*` matches any number of characters except path separators.
  - `?` matches one character except a path separator.
  - `**` matches any number of characters, including path separators.
  - `[]` matches one of the characters between the brackets. Inside `[...]`, a hyphen defines a
    locale-independent range, such as `a-z`, based on Unicode code points. A hyphen at the start or
    end matches a literal hyphen.
- The forward slash (`/`) separates path components. Patterns are relative to the given directory
  and cannot start with a slash.
- Patterns cannot contain parent-directory indicators (`..`).

These rules do not allow backslashes (`\`). This prevents conflicts with the Windows path separator.
