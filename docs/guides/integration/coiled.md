---
title: Using uv with Coiled
description: Use uv with Coiled to manage Python dependencies and deploy serverless scripts.
---

# Using uv with Coiled

[Coiled](https://coiled.io?utm_source=uv-docs) is a serverless cloud computing platform that focuses
on user experience. It runs code on cloud hardware from AWS, GCP, and Azure.

Use this guide to run Python scripts in the cloud. uv manages the dependencies, and Coiled deploys
the scripts.

## Managing script dependencies with uv

!!! note

    This guide uses one example, but uv and Coiled work with any Python script.

Use the following script as an example:

```python title="process.py" hl_lines="1-8"
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "pandas",
#   "pyarrow",
#   "s3fs",
# ]
# ///

import pandas as pd

df = pd.read_parquet(
    "s3://coiled-data/uber/part.0.parquet",
    storage_options={"anon": True},
)
print(df.head())
```

The script uses [`pandas`](https://pandas.pydata.org/docs/) to load a Parquet file from a public S3
bucket. It then prints the first few rows. The script uses
[inline script metadata](https://peps.python.org/pep-0723/) to list its dependencies.

Run the script locally:

```bash
$ uv run process.py
```

uv automatically creates a virtual environment and installs the dependencies.

To learn more about using inline script metadata with uv, see the
[script guide](../scripts.md#declaring-script-dependencies).

## Running scripts on the cloud with Coiled

Inline script metadata makes the script self-contained. The script includes the information that it
needs to run on other machines, including cloud machines.

Some tasks need more resources than a local workstation provides. Examples include:

- Process large amounts of cloud-hosted data.
- Use accelerated hardware, such as a GPU, or a machine with more memory.
- Run one script with hundreds or thousands of different inputs in parallel.

Coiled runs code on cloud hardware.

First, authenticate with Coiled using
[`coiled login`](https://docs.coiled.io/user_guide/api.html?utm_source=uv-docs#coiled-login) :

```bash
$ uvx coiled login
```

If you do not have a Coiled account, Coiled prompts you to create one. You can start using Coiled
for free.

To run the script on an AWS virtual machine, add two comments to the top:

```python title="process.py" hl_lines="1-2"
# COILED container ghcr.io/astral-sh/uv:debian-slim
# COILED region us-east-2

# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "pandas",
#   "pyarrow",
#   "s3fs",
# ]
# ///

import pandas as pd

df = pd.read_parquet(
    "s3://coiled-data/uber/part.0.parquet",
    storage_options={"anon": True},
)
print(df.head())
```

!!! tip

    Coiled supports AWS, GCP, and Azure. This example uses AWS, as the `region` option shows. New
    Coiled users automatically receive a free account that runs on AWS. If you use another cloud
    provider, set a valid `region` or remove the `region` line.

The comments tell Coiled to use the official [uv Docker image](../integration/docker.md). This makes
uv available when the script runs. The comments also set the AWS region to `us-east-2`, where the
example data file is stored, to avoid data egress.

To submit a batch job, use
[`coiled batch run`](https://docs.coiled.io/user_guide/api.html?utm_source=uv-docs#coiled-batch-run)
to run the `uv run` command in the cloud:

```bash hl_lines="1"
$ uvx coiled batch run \
    uv run process.py
```

The same process now runs on a remote AWS virtual machine.

Monitor the batch job in the UI at [cloud.coiled.io](https://cloud.coiled.io). You can also use the
`coiled batch status`, `coiled batch wait`, and `coiled batch logs` commands in the terminal.

![Coiled UI](https://docs.coiled.io/_images/uv-coiled.png)

You can also configure the instance type, disk size, and use of spot instances. The default instance
is a four-core virtual machine with 16 GiB of memory. See the
[Coiled Batch documentation](https://docs.coiled.io/user_guide/batch.html?utm_source=uv-docs) for
details.

For more information and other use cases, see the
[Coiled documentation](https://docs.coiled.io?utm_source=uv-docs).
