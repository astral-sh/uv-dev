"""Handwritten filesystem, TOML, and ZIP adapters for the generated core."""

import base64
import hashlib
import os
import re
import sys
import tempfile
import tomllib
import zipfile
from pathlib import Path
from types import SimpleNamespace

from . import _generated as g
from ._runtime import _NONE, RustError, _Some


class UnsupportedOperation(ValueError):
    """The input is outside the explicitly supported PoC domain."""


_PROJECT_FIELDS = {
    "name",
    "version",
    "description",
    "readme",
    "requires-python",
    "license",
    "authors",
    "maintainers",
    "keywords",
    "urls",
    "scripts",
    "gui-scripts",
    "entry-points",
    "dependencies",
    "optional-dependencies",
    "dynamic",
    "classifiers",
}
_RELEASE = r"(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*))*"


def _string(value, field):
    if not isinstance(value, str):
        raise UnsupportedOperation(f"{field} must be a string")
    value.encode("utf-8")
    return value


def _strings(value, field):
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise UnsupportedOperation(f"{field} must be an array of strings")
    for item in value:
        item.encode("utf-8")
    return value


def _mapping(value, field):
    if not isinstance(value, dict):
        raise UnsupportedOperation(f"{field} must be a table")
    return value


def _string_map(value, field, *, ordered=False):
    value = _mapping(value, field)
    return {
        key: _string(value[key], field) for key in (value if ordered else sorted(value))
    }


def _option(value):
    return _NONE if value is None else _Some(value)


def _read_utf8(path):
    # Path.read_text performs universal-newline translation; fs::read_to_string does not.
    return path.read_bytes().decode("utf-8")


def _inside(root, relative, field):
    relative = Path(_string(relative, field))
    if relative.is_absolute() or ".." in relative.parts:
        raise UnsupportedOperation(f"{field} must be a relative path without '..'")
    path = root / relative
    if path.is_symlink() or not path.resolve().is_relative_to(root.resolve()):
        raise UnsupportedOperation(f"Symlinks are outside the PoC domain: {field}")
    return path


def _contacts(entries, field):
    if entries is None:
        return _NONE, _NONE
    if not isinstance(entries, list):
        raise UnsupportedOperation(f"{field} must be an array")
    names, emails = [], []
    for contact in entries:
        contact = _mapping(contact, field)
        if contact.keys() - {"name", "email"} or not contact:
            raise UnsupportedOperation(f"Unsupported contact in {field}")
        name = _string(contact["name"], field) if "name" in contact else None
        email = _string(contact["email"], field) if "email" in contact else None
        if name is not None:
            names.append(name)
        if email is not None:
            emails.append(f"{name} <{email}>" if name is not None else email)
    joined_names, joined_emails = ", ".join(names), ", ".join(emails)
    return _option(joined_names or None), _option(joined_emails or None)


def _readme(root, value):
    if value is None:
        return _NONE, _NONE
    supported = {".txt": "text/plain", ".md": "text/markdown", ".rst": "text/x-rst"}
    if isinstance(value, str):
        path = _inside(root, value, "project.readme")
        if path.suffix not in supported:
            raise UnsupportedOperation("Only .txt, .md, and .rst readmes are supported")
        return _Some(_read_utf8(path)), _Some(supported[path.suffix])
    value = _mapping(value, "project.readme")
    if value.keys() - {"file", "text", "content-type", "charset"}:
        raise UnsupportedOperation("Unsupported readme fields")
    if ("file" in value) == ("text" in value):
        raise UnsupportedOperation("Readme must contain exactly one of file or text")
    if (
        value.get("content-type") not in supported.values()
        or value.get("charset", "UTF-8") != "UTF-8"
    ):
        raise UnsupportedOperation("Unsupported readme content type or charset")
    content = (
        _read_utf8(_inside(root, value["file"], "project.readme.file"))
        if "file" in value
        else _string(value["text"], "project.readme.text")
    )
    return _Some(content), _Some(value["content-type"])


def _load(root):
    if os.name != "posix":
        raise UnsupportedOperation(
            "The PoC filesystem adapter currently supports POSIX"
        )
    if os.environ.get("UV_PREVIEW") or os.environ.get("UV_PREVIEW_FEATURES"):
        raise UnsupportedOperation("Preview features are outside the PoC domain")
    document = tomllib.loads(_read_utf8(root / "pyproject.toml"))
    project = _mapping(document.get("project"), "project")
    unknown = project.keys() - _PROJECT_FIELDS
    if unknown:
        raise UnsupportedOperation(
            f"Untranslated project fields: {', '.join(sorted(unknown))}"
        )
    build_system = _mapping(document.get("build-system"), "build-system")
    _strings(build_system.get("requires"), "build-system.requires")
    _string(build_system.get("build-backend"), "build-system.build-backend")
    for field in ("dependencies", "dynamic", "classifiers"):
        if field in project and _strings(project[field], f"project.{field}"):
            raise UnsupportedOperation(f"project.{field} is not translated yet")
    if "optional-dependencies" in project and _mapping(
        project["optional-dependencies"], "project.optional-dependencies"
    ):
        raise UnsupportedOperation("Optional dependencies are not translated yet")
    name = _string(project.get("name"), "project.name")
    normalized = str(g._rs_validate_and_normalize_ref(name))
    dist_name = str(
        g._rs_PackageName__as_dist_info_name(SimpleNamespace(_0=normalized))
    )
    version = _string(project.get("version"), "project.version")
    if not re.fullmatch(_RELEASE, version) or any(
        int(part) > 2**64 - 1 for part in version.split(".")
    ):
        raise UnsupportedOperation(
            "Only canonical numeric release versions are supported"
        )
    requires_python = project.get("requires-python")
    if requires_python is not None:
        requires_python = _string(requires_python, "project.requires-python")
        if not re.fullmatch(rf"(?:>=|>|<=|<|==|!=){_RELEASE}", requires_python):
            raise UnsupportedOperation(
                "Only a single canonical Python version comparison is supported"
            )
        if any(
            int(part) > 2**64 - 1 for part in requires_python.lstrip("<>=!").split(".")
        ):
            raise UnsupportedOperation(
                "Python version components must fit in a Rust u64"
            )
    summary = project.get("description")
    if summary is not None:
        summary = _string(summary, "project.description")
        if "\n" in summary:
            raise UnsupportedOperation(
                "Project descriptions containing LF are not supported"
            )
    description, content_type = _readme(root, project.get("readme"))
    author, author_email = _contacts(project.get("authors"), "project.authors")
    maintainer, maintainer_email = _contacts(
        project.get("maintainers"), "project.maintainers"
    )
    license_value = project.get("license")
    license_text = None
    if license_value is not None:
        license_value = _mapping(license_value, "project.license")
        if set(license_value) == {"text"}:
            license_text = _string(license_value["text"], "project.license.text")
        elif set(license_value) == {"file"}:
            license_text = _read_utf8(
                _inside(root, license_value["file"], "project.license.file")
            )
        else:
            raise UnsupportedOperation(
                "Only legacy text or file licenses are supported"
            )
    keywords = _NONE
    if "keywords" in project:
        keywords = _Some(g._Keywords(_strings(project["keywords"], "project.keywords")))
    urls = _string_map(project.get("urls", {}), "project.urls", ordered=True)
    metadata = g._Metadata23(
        metadata_version="2.3",
        name=name,
        version=version,
        summary=_option(summary),
        description=description,
        description_content_type=content_type,
        author=author,
        author_email=author_email,
        maintainer=maintainer,
        maintainer_email=maintainer_email,
        license=_option(license_text),
        keywords=keywords,
        project_urls=g._ProjectUrls(urls),
        requires_python=_option(requires_python),
    )
    entries = {}
    for field in ("scripts", "gui-scripts"):
        entries[field.replace("-", "_")] = (
            _Some(_string_map(project[field], field)) if field in project else _NONE
        )
    if "entry-points" in project:
        groups = _mapping(project["entry-points"], "project.entry-points")
        entries["entry_points"] = _Some(
            {group: _string_map(groups[group], group) for group in sorted(groups)}
        )
    else:
        entries["entry_points"] = _NONE
    entry_points = g._rs_PyProjectToml__to_entry_points(
        SimpleNamespace(project=SimpleNamespace(**entries))
    ).unwrap()
    defaults = g._rs_BuildBackendSettings__default()
    tool = _mapping(document.get("tool", {}), "tool")
    uv_tool = _mapping(tool.get("uv", {}), "tool.uv")
    settings = _mapping(uv_tool.get("build-backend", {}), "tool.uv.build-backend")
    if settings.keys() - {"module-root", "module-name", "default-excludes"}:
        raise UnsupportedOperation(
            "Only module-root, module-name, and default-excludes are supported"
        )
    module_root = settings.get("module-root", defaults.module_root)
    source_root = _inside(root, module_root, "module-root")
    default_excludes = settings.get("default-excludes", defaults.default_excludes)
    if type(default_excludes) is not bool:
        raise UnsupportedOperation("default-excludes must be a boolean")
    explicit_module = settings.get("module-name")
    if explicit_module is None:
        module = (
            dist_name[:-6] + "-stubs" if normalized.endswith("-stubs") else dist_name
        )
    else:
        module = _string(explicit_module, "module-name")
    components = module.split(".")
    for index, component in enumerate(components):
        stem = (
            component[:-6] if index == 0 and component.endswith("-stubs") else component
        )
        if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", stem):
            raise UnsupportedOperation("Only ASCII module identifiers are supported")
    stub = components[0].endswith("-stubs")
    module_path = source_root.joinpath(*components)
    required_init = "__init__.pyi" if stub else "__init__.py"
    if not (module_path / required_init).is_file():
        raise RustError(f"Expected a Python module at: {module_path / required_init}")
    for index in range(1, len(components)):
        parent = source_root.joinpath(*components[:index])
        if (parent / "__init__.py").exists() or (parent / "__init__.pyi").exists():
            raise UnsupportedOperation(
                "Namespace parents must not contain __init__.py[i]"
            )
    return SimpleNamespace(
        name=dist_name,
        version=version,
        metadata=metadata,
        entry_points=entry_points,
        root=root,
        source_root=source_root,
        module_path=module_path,
        default_excludes=default_excludes,
    )


def _metadata(project):
    filename = SimpleNamespace(
        python_tags=["py3"], abi_tags=["none"], platform_tags=["any"]
    )
    info = g._rs_WheelInfo__new(filename, g.UPSTREAM_VERSION)
    files = {"WHEEL": str(info).encode("utf-8")}
    if isinstance(project.entry_points, _Some):
        files["entry_points.txt"] = str(project.entry_points.value).encode("utf-8")
    files["METADATA"] = str(
        g._rs_Metadata23__core_metadata_format(project.metadata)
    ).encode("utf-8")
    return files


def _warn(config_settings):
    if config_settings:
        print("Warning: Config settings are not supported", file=sys.stderr)


def get_requires_for_build_wheel(config_settings=None):
    _warn(config_settings)
    return []


get_requires_for_build_editable = get_requires_for_build_wheel
get_requires_for_build_sdist = get_requires_for_build_wheel


def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    _warn(config_settings)
    project = _load(Path.cwd())
    name = f"{project.name}-{project.version}.dist-info"
    destination = Path(metadata_directory) / name
    destination.mkdir(parents=True, exist_ok=True)
    files = _metadata(project)
    for filename, contents in files.items():
        (destination / filename).write_bytes(contents)
    (destination / "RECORD").write_bytes(
        _record_bytes(
            [(f"{name}/{filename}", contents) for filename, contents in files.items()],
            name,
        )
    )
    return name


prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel


def _record_bytes(files, dist_info):
    def quote(value):
        return (
            '"' + value.replace('"', '""') + '"'
            if any(character in value for character in ',"\r\n')
            else value
        )

    rows = []
    for path, data in files:
        digest = (
            base64.urlsafe_b64encode(hashlib.sha256(data).digest())
            .rstrip(b"=")
            .decode("ascii")
        )
        rows.append((path, f"sha256={digest}", str(len(data))))
    rows.append((f"{dist_info}/RECORD", "", ""))
    return "".join(",".join(map(quote, row)) + "\n" for row in rows).encode("utf-8")


def _walk(project):
    def visit(path):
        if path.is_symlink():
            raise UnsupportedOperation(f"Symlink is outside the PoC domain: {path}")
        if project.default_excludes and (
            path.name == "__pycache__" or path.name.endswith((".pyc", ".pyo"))
        ):
            return
        if path.name == "pyvenv.cfg":
            raise UnsupportedOperation("Virtual environments cannot be packaged")
        if path.is_dir():
            for child in sorted(path.iterdir(), key=lambda child: child.name):
                yield from visit(child)
        elif path.is_file():
            yield path
        else:
            raise UnsupportedOperation(f"Unsupported file type: {path}")

    yield from visit(project.module_path)


def _build(wheel_directory, config_settings, metadata_directory, editable):
    _warn(config_settings)
    project = _load(Path.cwd())
    metadata = _metadata(project)
    if metadata_directory:
        previous = Path(metadata_directory)
        for name in ("METADATA", "entry_points.txt"):
            if name in metadata:
                matches = (previous / name).read_bytes() == metadata[name]
            else:
                matches = not (previous / name).is_file()
            if not matches:
                raise RustError(
                    f"Inconsistent metadata between prepare and build step: {name}"
                )
    name = f"{project.name}-{project.version}-py3-none-any.whl"
    dist_info = f"{project.name}-{project.version}.dist-info"
    output = Path(wheel_directory)
    compression = zipfile.ZIP_STORED if editable else zipfile.ZIP_DEFLATED
    # Temporary output lives beside the destination, so publication is atomic.
    descriptor, temporary = tempfile.mkstemp(dir=output, prefix=".uv-build-slim-")
    os.close(descriptor)
    try:
        with zipfile.ZipFile(temporary, "w", compression=compression) as archive:
            record, directories = [], set()

            def add(path, data, mode=0o100644, directory=False):
                entry = zipfile.ZipInfo(path, date_time=(1980, 1, 1, 0, 0, 0))
                entry.create_system = 3
                entry.external_attr = mode << 16
                entry.compress_type = zipfile.ZIP_STORED if directory else compression
                archive.writestr(entry, data)
                if not directory:
                    record.append((path, data))

            def directory(path):
                if path not in directories:
                    directories.add(path)
                    add(path + "/", b"", 0o040755, directory=True)

            if editable:
                add(f"{project.name}.pth", os.fsencode(project.source_root))
            else:
                for path in _walk(project):
                    relative = path.relative_to(project.source_root)
                    for parent in reversed(relative.parents):
                        if parent != Path("."):
                            directory(parent.as_posix())
                    mode = 0o100755 if path.stat().st_mode & 0o111 else 0o100644
                    add(relative.as_posix(), path.read_bytes(), mode)
            directory(dist_info)
            for filename, data in metadata.items():
                add(f"{dist_info}/{filename}", data)
            record_path = f"{dist_info}/RECORD"
            add(record_path, _record_bytes(record, dist_info))
        os.replace(temporary, output / name)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
    return name


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    return _build(wheel_directory, config_settings, metadata_directory, False)


def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
    return _build(wheel_directory, config_settings, metadata_directory, True)


def build_sdist(sdist_directory, config_settings=None):
    raise UnsupportedOperation(
        "sdist generation requires translating the Rust TOML 1.0 serializer and file selection"
    )
