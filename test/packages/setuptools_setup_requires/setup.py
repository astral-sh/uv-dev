from importlib.metadata import version

from setuptools import setup
from setuptools.command.egg_info import egg_info


class SetupRequiresEggInfo(egg_info):
    def run(self):
        # Metadata generation needs a dependency supplied only by `setup_requires`.
        installed_version = version("iniconfig")
        if installed_version != "2.0.0":
            raise RuntimeError(f"Unexpected iniconfig version: {installed_version}")
        super().run()


# Intentionally omit `pyproject.toml` to exercise the implicit legacy backend.
setup(
    name="setuptools-setup-requires",
    version="0.1.0",
    packages=["setuptools_setup_requires"],
    setup_requires=["iniconfig==2.0.0"],
    cmdclass={"egg_info": SetupRequiresEggInfo},
)
