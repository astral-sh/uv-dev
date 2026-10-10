# NOTE this is not a real shell-script, it's parsed by `smoke-test/__main__.py` and executed
# serially via Python for cross-platform support.

# Show the uv version
uv --version

# Use any Python 3.13 version
uv python pin 3.13

# Create a virtual environment and install a package with `uv pip`
uv venv -v
uv pip install ruff==0.16.2 -v

# Install and import an extension module with wheels for every smoke-test platform
uv pip install cffi==2.0.0 -v
uv run --no-project python -c "import _cffi_backend; import cffi; print(cffi.__version__)"

# Show the `uvx` version
uvx --version

# Run a package via `uvx`
uvx -v ruff@0.16.2 --version
