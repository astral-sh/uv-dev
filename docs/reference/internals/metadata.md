# Workspace metadata

`uv workspace metadata` exports workspace or PEP 723 script information as JSON for other tools.
This command is the preferred way to access information from `uv.lock` or a script lockfile because
lockfile formats have no stability guarantees. The `--script path/to/script.py` option requests
metadata for a script.

Pass `--sync` to install the selected packages before collecting module ownership information.
Synchronization preserves unrelated installed packages by default; add `--exact` to remove them.

The main structure is the `resolution` field. It contains the dependency graph and exact package
versions from a `uv.lock` file.

Each node defines `dependencies`, which form the edges of the graph. Installing a node also requires
installing its dependencies and their dependencies. Cycles are valid. Each dependency includes the
`id` of its target node and an optional `marker` that
[specifies which platforms require the dependency](https://packaging.python.org/en/latest/specifications/dependency-specifiers/#dependency-specifiers).
Without a marker, the dependency is always required.

Package `name`, `version`, `source`, and `kind` uniquely identify package-derived nodes. Paths
identify script and workspace nodes. The group name and workspace path identify workspace-root
dependency group nodes. Node identifiers are opaque.

The graph contains five kinds of node:

- `"script"` -- a PEP 723 script and its direct dependencies
- `"workspace"` -- a workspace root and its workspace-exclusive dependency groups
- `"package"` -- the package itself
- `{ "extra": "extraname" }` -- an extra the package defines
- `{ "group": "groupname" }` -- a dependency group a package or workspace root defines

Future versions will add `build` nodes for the dependencies of
[build environments](../../concepts/projects/config.md#build-isolation).

The `"kind": "package"` node for `mypackage` represents its installation. This node also includes
its source distribution, wheels, extras (`optional_dependencies`), and dependency groups
(`dependency_groups`).

The node with `"kind": { "extra": "myextra" }` represents `mypackage[myextra]`. This node always
depends on `mypackage`. The installation of `mypackage[extra1, extra2]` uses the separate nodes for
`mypackage[extra1]` and `mypackage[extra2]`.

The node with `"kind": { "group": "mygroup" }` represents the dependency group `mypackage:mygroup`.
This node does _not_ depend on `mypackage` because dependency groups only list packages used when
working on a project.

If the workspace root defines dependency groups but is not itself a package, its `"workspace"` node
provides the corresponding group node ids through `dependency_groups`.

## Handling multiple versions of a package

One Python environment cannot contain two versions of the same package. However, the dependency
graph can include multiple versions for two reasons.

First,
[different platforms](https://packaging.python.org/en/latest/specifications/dependency-specifiers/#dependency-specifiers)
can have conflicting requirements that need different package versions.

Second, a workspace can have [conflicts](../../concepts/resolution.md#conflicting-dependencies).
Conflicting workspace members or extras cannot be installed together. The top-level `conflicts`
field describes these conflicts.

uv guarantees that **for any concrete choice of
[markers](https://packaging.python.org/en/latest/specifications/dependency-specifiers/#dependency-specifiers),
any set of packages without [conflicts](../../concepts/resolution.md#conflicting-dependencies)
contains at most one version of each package**.

Collecting every version of a package, such as `pydantic`, only requires iterating through the
nodes. Analyzing valid resolutions also requires checking `conflicts` and evaluating `markers` for a
specific platform.

The workspace root, workspace members, and requested script are the natural entry points to the
dependency graph. Starting from these nodes prevents errors when multiple versions of a package are
present. These entry points represent operations such as "install the workspace `dev` group",
"install `member1` and `member2[extra]`", or "install this script's declared dependencies".

When possible, graph analysis _avoids iterating over the `resolution` object to find a node_. It
accesses `resolution` as a map using identifiers from another part of the metadata. For a workspace,
`workspace.id` identifies the root and the `members` array lists package entry points. For a script,
`script.id` identifies the starting node. Following dependency edges discovers other packages.

For example, analysis of `anyio` starts from the workspace members being installed instead of
finding an `anyio` node directly. Traversing their `dependencies` identifies the applicable `anyio`
node. If the traversal reaches multiple versions of `anyio`, the selected packages conflict and uv
would not install them together.

The following example analyzes installation of the `dev` dependency group for workspace member
`mypackage`:

```python
member = find_by_name(metadata.members, "mypackage")
member_node = metadata.resolution[member.id]
group = find_by_name(member_node.dependency_groups, "dev")
group_node = metadata.resolution[group.id]
visit(metadata, [group_node])
```

The following example finds a workspace-root dependency group through the workspace node:

```python
workspace_node = metadata.resolution[metadata.workspace.id]
group = find_by_name(workspace_node.dependency_groups, "dev")
group_node = metadata.resolution[group.id]
visit(metadata, [group_node])
```

The following example analyzes two workspace members installed together:

```python
to_analyze = []
for member_name in ["package1", "package2"]:
    member = find_by_name(metadata.members, member_name)
    member_node = metadata.resolution[member.id]
    to_analyze.append(member_node)
visit(metadata, to_analyze)
```

For a script, the analysis starts from its resolution node:

```python
script_node = metadata.resolution[metadata.script.id]
visit(metadata, [script_node])
```

Here, `visit` uses a graph traversal algorithm such as depth-first search:

```python
def visit(metadata: UvMetadata, to_analyze: list[Node]):
    visited = set()
    while len(to_analyze) > 0:
        node = to_analyze.pop()

        # Handle cycles by avoiding revisiting nodes
        if node.id in visited:
            continue
        visited.add(node.id)

        # We also need to analyze its dependencies
        for dependency in node.dependencies:
            # Only follow edges if they satisfy the desired platform's markers
            if dependency.marker and not satisfies(platform, dependency.marker):
                continue
            to_analyze.append(metadata.resolution[dependency.id])

        # Analyze any package node we encounter
        if node.kind == "package":
            print(node.name, node.version, node.source)
```

## Schema

A full JSON schema will be available when the format is final.

The following annotated example describes the format:

```js
{
  // Information about the schema of this output
  "schema": {
    // The version of this output, currently "preview"
    "version": "preview"
  },
  // The directory the uv.lock can be found in
  "workspace_root": "/workspace",
  // Information about the environment, available when an environment exists or `--sync` is used
  "environment": {
    // The absolute path to the environment root
    "root": "/workspace/.venv",
    // Information about the Python interpreter in the environment
    "python": {
      // The absolute path to the Python executable
      "path": "/workspace/.venv/bin/python",
      // The full Python version
      "version": "3.12.12",
      // The Python implementation name
      "implementation": "cpython"
    }
  },
  // Information about the script target, only present with `--script`.
  // Workspace metadata uses `workspace` and `members` below as graph entry-points instead.
  "script": {
    // The absolute path to the script
    "path": "/workspace/script.py",
    // The id of the script's node in the `resolution` map below
    "id": "script+/workspace/script.py"
  },
  // Information about the workspace target, omitted when `--script` is used.
  "workspace": {
    // The absolute path to the workspace root
    "path": "/workspace",
    // The id of the workspace's node in the `resolution` map below
    "id": "workspace+/workspace"
  },
  // Any requirements on the python version this workspace has
  //
  // `marker` fields all have this as an implicit constraint that is omitted for cleanliness
  "requires_python": ">=3.12",
  // A list of workspace members
  "members": [
    {
      // The name of the package
      "name": "mypackage",
      // The directory that contains its pyproject.toml
      "path": "/workspace/packages/mypackage",
      // The id of this package's info in the `resolution` map below
      "id": "mypackage==0.1.0@editable+/workspace/packages/mypackage"
    },
  ],
  // A list-of-sets of workspace items that are mutually-exclusive to install,
  // presumably because they need to install different versions of the same package.
  //
  // Any attempt to install two things that belong to the same set must be rejected.
  //
  // There are 3 kinds of item:
  //
  // * Project -- "kind": "project"
  // * Extra   -- "kind": { "extra": "extraname" }
  // * Group   -- "kind": { "group": "groupname" }
  "conflicts": {
    "sets": [
      {
        "items": [
          {
            "package": "mypackage",
            "kind": { "extra": "myextra" }
            "id": "mypackage[myextra]==0.1.0@editable+/workspace/packages/mypackage",
          }
          {
            "package": "mypackage",
            "kind": { "group": "mygroup" }
            "id": "mypackage:mygroup==0.1.0@editable+/workspace/packages/mypackage",
          }
        ]
      }
    ]
  }
  // Resolved information about packages and dependencies.
  //
  // Each entry in this map is a node in the dependency graph. There are currently
  // 5 kinds of node in the dependency graph, although more are planned in the future.
  //
  // * Scripts  -- "kind": "script"
  // * Workspaces -- "kind": "workspace"
  // * Packages -- "kind": "package"
  // * Extras   -- "kind": { "extra": "extraname" }
  // * Groups   -- "kind": { "group": "groupname" }
  //
  // Package nodes contain most of the metadata, while other nodes are mostly just a list
  // of dependencies. The different kinds of node are included like this to encourage correct
  // analysis of the graph. For instance, a node for `mypackage[someextra]` always depends on
  // `mypackage`, while `mypackage:somegroup` does not (because dependency-groups are just a
  // list of packages you might want to install while working on `mypackage`). Sugars like
  // `mypackage[extra1, extra2]` are decomposed into separate dependencies on `mypackage[extra1]`
  // and `mypackage[extra2]`.
  //
  // The ids used here are human-readable but should be handled as opaque (the nodes contain
  // the same information in a more convenient form).
  "resolution": {

    // The script node is present when metadata was requested with `--script`. Its dependencies
    // are the direct requirements declared by the script.
    "script+/workspace/script.py": {
      "kind": "script",
      "path": "/workspace/script.py",
      "dependencies": [
        {
          "id": "iniconfig==2.0.0@registry+https://pypi.org/simple"
        }
      ]
    },

    // The workspace node owns metadata defined directly on the workspace root.
    "workspace+/workspace": {
      "kind": "workspace",
      "path": "/workspace",
      "dependencies": [],
      "dependency_groups": [
        {
          "name": "dev",
          "id": "workspace+/workspace:dev"
        }
      ]
    },

    // This node is a dependency group defined on the non-package workspace root.
    "workspace+/workspace:dev": {
      "kind": { "group": "dev" },
      "path": "/workspace",
      "dependencies": [
        {
          "id": "iniconfig==2.0.0@registry+https://pypi.org/simple"
        }
      ]
    },

    // This node is a workspace member
    "mypackage==0.1.0@editable+/workspace/packages/mypackage": {
      // The name of the package
      "name": "mypackage",
      // The version of the package (this may be missing, as source trees do not need versions)
      "version": "0.1.0",
      // The source of the package, in this case it's an editable whose path relative to the
      // `workspace_root` is `./packages/mypackage`
      "source": {
        "editable": "/workspace/packages/mypackage"
      },
      // The kind of the node, in this case "package" (see the docs on `resolution` above for details)
      "kind": "package",
      // The dependencies that must be installed to also install this node into an environment
      "dependencies": [
        {
          // The id of the node to lookup for details
          "id": "iniconfig==2.0.0@registry+https://pypi.org/simple"
          "marker": "marker": "sys_platform == 'linux'"
        }
      ],
      // The extras that this package defines
      "optional_dependencies": [
        {
          "name": "myextra",
          "id": "mypackage[myextra]==0.1.0@editable+/workspace/packages/mypackage"
        }
      ],
      // The dependency groups this package defines
      "dependency_groups": [
        {
          "name": "mygroup",
          "id": "mypackage:mygroup==0.1.0@editable+/workspace/packages/mypackage"
        }
      ]
    },

    // This node is an extra on a workspace member
    "mypackage[myextra]==0.1.0@editable+/workspace/packages/mypackage": {
      // These fields will match the package node above
      "name": "mypackage",
      "version": "0.1.0",
      "source": {
        "editable": "/workspace/packages/mypackage"
      },
      // But these two will differ from the package node above
      "kind": { "extra": "myextra" },
      "dependencies": [
        {
          "id": "mypackage==0.1.0@editable+/workspace/packages/mypackage"
        }
        {
          "id": "anyio==2.0.0@registry+https://pypi.org/simple"
        }
      ]
    },

    // This node is a dependency-group on a workspace member
    "mypackage:mygroup==0.1.0@editable+/workspace/packages/mypackage": {
      // These fields will match the package node above
      "name": "mypackage",
      "version": "0.1.0",
      "source": {
        "editable": "/workspace/packages/mypackage"
      },
      // But these two will differ from the package node above
      "kind": { "extra": "myextra" },
      "dependencies": [
        {
          "id": "anyio==1.0.0@registry+https://pypi.org/simple"
        }
      ]
    },

    // This node is a package on pypi
    "iniconfig==2.0.0@registry+https://pypi.org/simple": {
      "name": "iniconfig",
      "version": "2.0.0",
      // registry sources look like this
      "source": {
        "registry": {
          "url": "https://pypi.org/simple"
        }
      },
      "kind": "package",
      "dependencies": [],
      // Details on the package's source distribution
      "sdist": {
        // May alternatively be `path`
        "url": "https://files.pythonhosted.org/packages/d7/4b/cbd8e699e64a6f16ca3a8220661b5f83792b3017d0f79807cb8708d33913/iniconfig-2.0.0.tar.gz",
        "hashes": {
          "sha256": "2d91e135bf72d31a410b17c16da610a82cb55f6b0477d1a902134b24a455b8b3"
        },
        "size": 4646,
        "upload_time": "2023-01-07T11:08:11.254Z"
      },
      // The wheels we found for this package
      "wheels": [
        {
          // May alternatively be `path`
          "url": "https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl",
          "hashes": {
            "sha256": "b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374"
          },
          "size": 5892,
          "upload_time": "2023-01-07T11:08:09.864Z",
          // Parsing this name is how you know what platform a wheel supports
          "filename": "iniconfig-2.0.0-py3-none-any.whl"
        }
      ]
    }

    // ...and so on
    "anyio==1.0.0@registry+https://pypi.org/simple": { ... }
    "anyio==2.0.0@registry+https://pypi.org/simple": { ... }
  }
}
```
