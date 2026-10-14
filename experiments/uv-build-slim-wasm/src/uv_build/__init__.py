"""PEP 517/660 compatibility entry point for the generated Python backend."""

from uv_build_slim_wasm import (
    build_editable,
    build_sdist,
    build_wheel,
    call,
    get_requires_for_build_editable,
    get_requires_for_build_sdist,
    get_requires_for_build_wheel,
    prepare_metadata_for_build_editable,
    prepare_metadata_for_build_wheel,
    warn_config_settings,
)

__all__ = [
    "build_editable",
    "build_sdist",
    "build_wheel",
    "call",
    "get_requires_for_build_editable",
    "get_requires_for_build_sdist",
    "get_requires_for_build_wheel",
    "prepare_metadata_for_build_editable",
    "prepare_metadata_for_build_wheel",
    "warn_config_settings",
]
