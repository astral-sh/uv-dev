"""Explicit boundary for dependencies outside the direct AST experiment."""


class UntranslatedDependency(NotImplementedError):
    """A generated function needs a dependency that has not been translated."""


def external(name):
    raise UntranslatedDependency(
        f"Direct AST translation has no implementation for {name}; "
        "the complete compiled-program experiment is in uv-build-slim-wasm"
    )
