use std::{
    collections::BTreeMap,
    path::{Component, Components, Path, PathBuf},
};

/// Check if a component of the path looks like it may be a glob pattern.
///
/// Note: this function is being used when splitting a glob pattern into a long possible
/// base and the glob remainder (scanning through components until we hit the first component
/// for which this function returns true). It is acceptable for this function to return
/// false positives (e.g. patterns like 'foo[bar' or 'foo{bar') in which case correctness
/// will not be affected but efficiency might be (because we'll traverse more than we should),
/// however it should not return false negatives.
fn is_glob_like(part: Component) -> bool {
    matches!(part, Component::Normal(_))
        && part.as_os_str().to_str().is_some_and(|part| {
            ["*", "{", "}", "?", "[", "]"]
                .into_iter()
                .any(|c| part.contains(c))
        })
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct GlobParts {
    base: PathBuf,
    pattern: PathBuf,
}

/// Split a glob into longest possible base + shortest possible glob pattern.
fn split_glob(pattern: impl AsRef<str>) -> GlobParts {
    let pattern: &Path = pattern.as_ref().as_ref();

    let mut glob = GlobParts::default();
    let mut globbing = false;
    let mut last = None;

    for part in pattern.components() {
        if let Some(last) = last {
            if last != Component::CurDir {
                if globbing {
                    glob.pattern.push(last);
                } else {
                    glob.base.push(last);
                }
            }
        }
        if !globbing {
            globbing = is_glob_like(part);
        }
        // we don't know if this part is the last one, defer handling it by one iteration
        last = Some(part);
    }

    if let Some(last) = last {
        // defer handling the last component to prevent draining entire pattern into base
        if globbing || matches!(last, Component::Normal(_)) {
            glob.pattern.push(last);
        } else {
            glob.base.push(last);
        }
    }
    glob
}

/// Classic trie with edges being path components and values being glob patterns.
#[derive(Default)]
struct TrieNode<'a> {
    children: BTreeMap<Component<'a>, usize>,
    patterns: Vec<&'a Path>,
}

/// Store nodes in an arena so inserting, traversing, and dropping a deep path are not recursive.
struct Trie<'a> {
    nodes: Vec<TrieNode<'a>>,
}

impl Default for Trie<'_> {
    fn default() -> Self {
        Self {
            nodes: vec![TrieNode::default()],
        }
    }
}

enum Visit {
    Group {
        node: usize,
        prefix: PathBuf,
    },
    Patterns {
        node: usize,
        pattern_prefix: PathBuf,
        group_prefix: PathBuf,
        group: usize,
    },
    FinishGroup {
        prefix: PathBuf,
        group: usize,
    },
}

impl<'a> Trie<'a> {
    fn insert(&mut self, components: Components<'a>, pattern: &'a Path) {
        let mut node = 0;
        for part in components {
            let next = self.nodes.len();
            let child = *self.nodes[node].children.entry(part).or_insert(next);
            if child == next {
                self.nodes.push(TrieNode::default());
            }
            node = child;
        }
        self.nodes[node].patterns.push(pattern);
    }

    fn collect_groups(&self) -> Vec<(PathBuf, Vec<PathBuf>)> {
        let mut groups = Vec::new();
        let mut patterns: Vec<Vec<PathBuf>> = Vec::new();
        let mut pending = vec![Visit::Group {
            node: 0,
            prefix: PathBuf::new(),
        }];

        while let Some(visit) = pending.pop() {
            match visit {
                Visit::Group { node, prefix } => {
                    let trie_node = &self.nodes[node];
                    if trie_node.patterns.is_empty() {
                        // Child nodes can form independent groups. Reverse insertion preserves
                        // the original depth-first, component-sorted traversal order.
                        for (part, child) in trie_node.children.iter().rev() {
                            pending.push(Visit::Group {
                                node: *child,
                                prefix: prefix.join(part),
                            });
                        }
                    } else {
                        // This pattern node is a pivot. Finish its group after any nested groups
                        // reached through non-normal components.
                        let group = patterns.len();
                        patterns.push(Vec::new());
                        pending.push(Visit::FinishGroup {
                            prefix: prefix.clone(),
                            group,
                        });
                        pending.push(Visit::Patterns {
                            node,
                            pattern_prefix: PathBuf::new(),
                            group_prefix: prefix,
                            group,
                        });
                    }
                }
                Visit::Patterns {
                    node,
                    pattern_prefix,
                    group_prefix,
                    group,
                } => {
                    let node = &self.nodes[node];
                    patterns[group].extend(
                        node.patterns
                            .iter()
                            .map(|pattern| pattern_prefix.join(pattern)),
                    );
                    for (part, child) in node.children.iter().rev() {
                        if let Component::Normal(_) = part {
                            pending.push(Visit::Patterns {
                                node: *child,
                                pattern_prefix: pattern_prefix.join(part),
                                group_prefix: group_prefix.join(part),
                                group,
                            });
                        } else {
                            pending.push(Visit::Group {
                                node: *child,
                                prefix: group_prefix.join(part),
                            });
                        }
                    }
                }
                Visit::FinishGroup { prefix, group } => {
                    groups.push((prefix, std::mem::take(&mut patterns[group])));
                }
            }
        }
        groups
    }
}

/// Given a collection of globs, cluster them into (base, globs) groups so that:
/// - base doesn't contain any glob symbols
/// - each directory would only be walked at most once
/// - base of each group is the longest common prefix of globs in the group
pub(crate) fn cluster_globs(patterns: &[impl AsRef<str>]) -> Vec<(PathBuf, Vec<String>)> {
    // split all globs into base/pattern
    let globs: Vec<_> = patterns.iter().map(split_glob).collect();

    // construct a path trie out of all split globs
    let mut trie = Trie::default();
    for glob in &globs {
        trie.insert(glob.base.components(), &glob.pattern);
    }

    // run LCP-style aggregation of patterns in the trie into groups
    let groups = trie.collect_groups();

    // finally, convert resulting patterns to strings
    groups
        .into_iter()
        .map(|(base, patterns)| {
            (
                base,
                patterns
                    .iter()
                    // NOTE: this unwrap is ok because input patterns are valid utf-8
                    .map(|p| p.to_str().unwrap().to_owned())
                    .collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{GlobParts, cluster_globs, split_glob};

    fn windowsify(path: &str) -> String {
        if cfg!(windows) {
            path.replace('/', "\\")
        } else {
            path.to_owned()
        }
    }

    #[test]
    fn test_split_glob() {
        #[track_caller]
        fn check(input: &str, base: &str, pattern: &str) {
            let result = split_glob(input);
            let expected = GlobParts {
                base: base.into(),
                pattern: pattern.into(),
            };
            assert_eq!(result, expected, "{input:?} != {base:?} + {pattern:?}");
        }

        check("", "", "");
        check("a", "", "a");
        check("a/b", "a", "b");
        check("a/b/", "a", "b");
        check("a/.//b/", "a", "b");
        check("./a/b/c", "a/b", "c");
        check("c/d/*", "c/d", "*");
        check("c/d/*/../*", "c/d", "*/../*");
        check("a/?b/c", "a", "?b/c");
        check("/a/b/*", "/a/b", "*");
        check("../x/*", "../x", "*");
        check("a/{b,c}/d", "a", "{b,c}/d");
        check("a/[bc]/d", "a", "[bc]/d");
        check("*", "", "*");
        check("*/*", "", "*/*");
        check("..", "..", "");
        check("/", "/", "");
    }

    #[test]
    fn test_cluster_globs() {
        #[track_caller]
        fn check(input: &[&str], expected: &[(&str, &[&str])]) {
            let input = input.iter().map(|s| windowsify(s)).collect::<Vec<_>>();

            let mut result_sorted = cluster_globs(&input);
            for (_, patterns) in &mut result_sorted {
                patterns.sort_unstable();
            }
            result_sorted.sort_unstable();

            let mut expected_sorted = Vec::new();
            for (base, patterns) in expected {
                let mut patterns_sorted = Vec::new();
                for pattern in *patterns {
                    patterns_sorted.push(windowsify(pattern));
                }
                patterns_sorted.sort_unstable();
                expected_sorted.push((windowsify(base).into(), patterns_sorted));
            }
            expected_sorted.sort_unstable();

            assert_eq!(
                result_sorted, expected_sorted,
                "{input:?} != {expected_sorted:?} (got: {result_sorted:?})"
            );
        }

        check(&["a/b/*", "a/c/*"], &[("a/b", &["*"]), ("a/c", &["*"])]);
        check(&["./a/b/*", "a/c/*"], &[("a/b", &["*"]), ("a/c", &["*"])]);
        check(&["/a/b/*", "/a/c/*"], &[("/a/b", &["*"]), ("/a/c", &["*"])]);
        check(
            &["../a/b/*", "../a/c/*"],
            &[("../a/b", &["*"]), ("../a/c", &["*"])],
        );
        check(&["x/*", "y/*"], &[("x", &["*"]), ("y", &["*"])]);
        check(&[], &[]);
        check(
            &["./*", "a/*", "../foo/*.png"],
            &[("", &["*", "a/*"]), ("../foo", &["*.png"])],
        );
        check(
            &[
                "?",
                "/foo/?",
                "/foo/bar/*",
                "../bar/*.png",
                "../bar/../baz/*.jpg",
            ],
            &[
                ("", &["?"]),
                ("/foo", &["?", "bar/*"]),
                ("../bar", &["*.png"]),
                ("../bar/../baz", &["*.jpg"]),
            ],
        );
        check(&["/abs/path/*"], &[("/abs/path", &["*"])]);
        check(&["/abs/*", "rel/*"], &[("/abs", &["*"]), ("rel", &["*"])]);
        check(&["a/{b,c}/*", "a/d?/*"], &[("a", &["{b,c}/*", "d?/*"])]);
        check(
            &[
                "../shared/a/[abc].png",
                "../shared/a/b/*",
                "../shared/b/c/?x/d",
                "docs/important/*.{doc,xls}",
                "docs/important/very/*",
            ],
            &[
                ("../shared/a", &["[abc].png", "b/*"]),
                ("../shared/b/c", &["?x/d"]),
                ("docs/important", &["*.{doc,xls}", "very/*"]),
            ],
        );
        check(&["file.txt"], &[("", &["file.txt"])]);
        check(&["/"], &[("/", &[""])]);
        check(&[".."], &[("..", &[""])]);
        check(
            &["file1.txt", "file2.txt"],
            &[("", &["file1.txt", "file2.txt"])],
        );
        check(
            &["a/file1.txt", "a/file2.txt"],
            &[("a", &["file1.txt", "file2.txt"])],
        );
        check(
            &["*", "a/b/*", "a/../c/*.jpg", "a/../c/*.png", "/a/*", "/b/*"],
            &[
                ("", &["*", "a/b/*"]),
                ("a/../c", &["*.jpg", "*.png"]),
                ("/a", &["*"]),
                ("/b", &["*"]),
            ],
        );

        if cfg!(windows) {
            check(
                &[
                    r"\\foo\bar\shared/a/[abc].png",
                    r"\\foo\bar\shared/a/b/*",
                    r"\\foo\bar/shared/b/c/?x/d",
                    r"D:\docs\important/*.{doc,xls}",
                    r"D:\docs/important/very/*",
                ],
                &[
                    (r"\\foo\bar\shared\a", &["[abc].png", r"b\*"]),
                    (r"\\foo\bar\shared\b\c", &[r"?x\d"]),
                    (r"D:\docs\important", &["*.{doc,xls}", r"very\*"]),
                ],
            );
        }
    }

    #[test]
    fn test_cluster_globs_preserves_order() {
        let patterns = [
            "*",
            "a/../z/*.rs",
            "a/file.txt",
            "a/nested/*.py",
            "b/file.txt",
        ];
        let expected = vec![
            (windowsify("a/../z").into(), vec![windowsify("*.rs")]),
            (
                "".into(),
                vec![
                    windowsify("*"),
                    windowsify("a/file.txt"),
                    windowsify("a/nested/*.py"),
                    windowsify("b/file.txt"),
                ],
            ),
        ];

        assert_eq!(cluster_globs(&patterns), expected);
    }
}
