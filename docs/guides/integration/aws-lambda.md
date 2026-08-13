---
title: Using uv with AWS Lambda
description:
  Use uv with AWS Lambda to manage Python dependencies and deploy serverless functions with Docker
  containers or zip archives.
---

# Using uv with AWS Lambda

[AWS Lambda](https://aws.amazon.com/lambda/) is a serverless computing service. It runs code without
requiring you to provision or manage servers.

Use uv to manage Python dependencies, build deployment packages, and deploy Lambda functions.

!!! tip

    See the [`uv-aws-lambda-example`](https://github.com/astral-sh/uv-aws-lambda-example) project for
    recommended practices for deploying an application to AWS Lambda with uv.

## Getting started

Start with a minimal FastAPI application that has this structure:

```plaintext
project
├── pyproject.toml
└── app
    ├── __init__.py
    └── main.py
```

The `pyproject.toml` file contains:

```toml title="pyproject.toml"
[project]
name = "uv-aws-lambda-example"
version = "0.1.0"
requires-python = ">=3.13"
dependencies = [
    # FastAPI is a modern web framework for building APIs with Python.
    "fastapi",
    # Mangum is a library that adapts ASGI applications to AWS Lambda and API Gateway.
    "mangum",
]

[dependency-groups]
dev = [
    # In development mode, include the FastAPI development server.
    "fastapi[standard]>=0.115",
]
```

The `main.py` file contains:

```python title="app/main.py"
import logging

from fastapi import FastAPI
from mangum import Mangum

logger = logging.getLogger()
logger.setLevel(logging.INFO)

app = FastAPI()
handler = Mangum(app)


@app.get("/")
async def root() -> str:
    return "Hello, world!"
```

Run the application locally:

```console
$ uv run fastapi dev
```

Open http://127.0.0.1:8000/ in a web browser to show "Hello, world!"

## Deploying a Docker image

To deploy to AWS Lambda, build a container image that includes the application code and dependencies
in one output directory.

Use a multi-stage build, as described in the [Docker guide](./docker.md), to keep the final image
small and reuse cached layers.

In the first stage, put the application code and dependencies in one directory. In the second stage,
copy that directory into the final image. Exclude build tools and other unnecessary files.

```dockerfile title="Dockerfile"
FROM ghcr.io/astral-sh/uv:0.12.10 AS uv

# First, bundle the dependencies into the task root.
FROM public.ecr.aws/lambda/python:3.13 AS builder

# Enable bytecode compilation, to improve cold-start performance.
ENV UV_COMPILE_BYTECODE=1

# Disable installer metadata, to create a deterministic layer.
ENV UV_NO_INSTALLER_METADATA=1

# Enable copy mode to support bind mount caching.
ENV UV_LINK_MODE=copy

# Bundle the dependencies into the Lambda task root via `uv pip install --target`.
#
# Omit any local packages (`--no-emit-workspace`) and development dependencies (`--no-dev`).
# This ensures that the Docker layer cache is only invalidated when the `pyproject.toml` or `uv.lock`
# files change, but remains robust to changes in the application code.
RUN --mount=from=uv,source=/uv,target=/bin/uv \
    --mount=type=cache,target=/root/.cache/uv \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    uv export --frozen --no-emit-workspace --no-dev --no-editable -o requirements.txt && \
    uv pip install -r requirements.txt --target "${LAMBDA_TASK_ROOT}"

FROM public.ecr.aws/lambda/python:3.13

# Copy the runtime dependencies from the builder stage.
COPY --from=builder ${LAMBDA_TASK_ROOT} ${LAMBDA_TASK_ROOT}

# Copy the application code.
COPY ./app ${LAMBDA_TASK_ROOT}/app

# Set the AWS Lambda handler.
CMD ["app.main.handler"]
```

!!! tip

    To deploy to ARM-based AWS Lambda runtimes, replace `public.ecr.aws/lambda/python:3.13` with
    `public.ecr.aws/lambda/python:3.13-arm64`.

Build the image:

```console
$ uv lock
$ docker build -t fastapi-app .
```

This Dockerfile structure has two main benefits:

1. **Minimal image size.** The multi-stage build includes only the application code and dependencies
   in the final image. For example, the final image does not include the uv binary.
2. **Maximal cache reuse.** Install dependencies separately from the application code. Docker
   invalidates the dependency layer only when the dependencies change.

If you change the application source code and rebuild the image, Docker reuses cached layers. The
rebuild can finish in milliseconds:

```console
 => [internal] load build definition from Dockerfile                                                                 0.0s
 => => transferring dockerfile: 1.31kB                                                                               0.0s
 => [internal] load metadata for public.ecr.aws/lambda/python:3.13                                                   0.3s
 => [internal] load metadata for ghcr.io/astral-sh/uv:latest                                                         0.3s
 => [internal] load .dockerignore                                                                                    0.0s
 => => transferring context: 106B                                                                                    0.0s
 => [uv 1/1] FROM ghcr.io/astral-sh/uv:latest@sha256:ea61e006cfec0e8d81fae901ad703e09d2c6cf1aa58abcb6507d124b50286f  0.0s
 => [builder 1/2] FROM public.ecr.aws/lambda/python:3.13@sha256:f5b51b377b80bd303fe8055084e2763336ea8920d12955b23ef  0.0s
 => [internal] load build context                                                                                    0.0s
 => => transferring context: 185B                                                                                    0.0s
 => CACHED [builder 2/2] RUN --mount=from=uv,source=/uv,target=/bin/uv     --mount=type=cache,target=/root/.cache/u  0.0s
 => CACHED [stage-2 2/3] COPY --from=builder /var/task /var/task                                                     0.0s
 => CACHED [stage-2 3/3] COPY ./app /var/task                                                                        0.0s
 => exporting to image                                                                                               0.0s
 => => exporting layers                                                                                              0.0s
 => => writing image sha256:6f8f9ef715a7cda466b677a9df4046ebbb90c8e88595242ade3b4771f547652d                         0.0
```

After you build the image, push it to
[Elastic Container Registry (ECR)](https://aws.amazon.com/ecr/):

```console
$ aws ecr get-login-password --region region | docker login --username AWS --password-stdin aws_account_id.dkr.ecr.region.amazonaws.com
$ docker tag fastapi-app:latest aws_account_id.dkr.ecr.region.amazonaws.com/fastapi-app:latest
$ docker push aws_account_id.dkr.ecr.region.amazonaws.com/fastapi-app:latest
```

Deploy the image to AWS Lambda with the AWS Management Console or the AWS CLI:

```console
$ aws lambda create-function \
   --function-name myFunction \
   --package-type Image \
   --code ImageUri=aws_account_id.dkr.ecr.region.amazonaws.com/fastapi-app:latest \
   --role arn:aws:iam::111122223333:role/my-lambda-role
```

Create the
[execution role](https://docs.aws.amazon.com/lambda/latest/dg/lambda-intro-execution-role.html#permissions-executionrole-api)
with:

```console
$ aws iam create-role \
   --role-name my-lambda-role \
   --assume-role-policy-document '{"Version": "2012-10-17", "Statement": [{ "Effect": "Allow", "Principal": {"Service": "lambda.amazonaws.com"}, "Action": "sts:AssumeRole"}]}'
```

To update an existing function, run:

```console
$ aws lambda update-function-code \
   --function-name myFunction \
   --image-uri aws_account_id.dkr.ecr.region.amazonaws.com/fastapi-app:latest \
   --publish
```

To test the Lambda function, use the AWS Management Console or the AWS CLI:

```console
$ aws lambda invoke \
   --function-name myFunction \
   --payload file://event.json \
   --cli-binary-format raw-in-base64-out \
   response.json
{
  "StatusCode": 200,
  "ExecutedVersion": "$LATEST"
}
```

The `event.json` file contains the event payload for the Lambda function:

```json title="event.json"
{
  "httpMethod": "GET",
  "path": "/",
  "requestContext": {},
  "version": "1.0"
}
```

The `response.json` file contains the response from the Lambda function:

```json title="response.json"
{
  "statusCode": 200,
  "headers": {
    "content-length": "14",
    "content-type": "application/json"
  },
  "multiValueHeaders": {},
  "body": "\"Hello, world!\"",
  "isBase64Encoded": false
}
```

For details, see the
[AWS Lambda documentation](https://docs.aws.amazon.com/lambda/latest/dg/python-image.html).

### Workspace support

If a project includes local dependencies, include them in the deployment package. For example, a
project can include local dependencies through [Workspaces](../../concepts/projects/workspaces.md).

Extend the example with a locally developed library named `library`.

First, create the library:

```console
$ uv init --lib library
$ uv add ./library
```

When you run `uv init` in the `project` directory, uv converts `project` to a workspace and adds
`library` as a workspace member:

```toml title="pyproject.toml"
[project]
name = "uv-aws-lambda-example"
version = "0.1.0"
requires-python = ">=3.13"
dependencies = [
    # FastAPI is a modern web framework for building APIs with Python.
    "fastapi",
    # A local library.
    "library",
    # Mangum is a library that adapts ASGI applications to AWS Lambda and API Gateway.
    "mangum",
]

[dependency-groups]
dev = [
    # In development mode, include the FastAPI development server.
    "fastapi[standard]",
]

[tool.uv.workspace]
members = ["library"]

[tool.uv.sources]
lib = { workspace = true }
```

By default, `uv init --lib` creates a package that exports a `hello` function. Update the
application source code to call that function:

```python title="app/main.py"
import logging

from fastapi import FastAPI
from mangum import Mangum

from library import hello

logger = logging.getLogger()
logger.setLevel(logging.INFO)

app = FastAPI()
handler = Mangum(app)


@app.get("/")
async def root() -> str:
    return hello()
```

Run the modified application locally:

```console
$ uv run fastapi dev
```

Open http://127.0.0.1:8000/ in a web browser. Check that the browser shows "Hello from library!"
instead of "Hello, World!"

Update the Dockerfile to include the local library in the deployment package:

```dockerfile title="Dockerfile"
FROM ghcr.io/astral-sh/uv:0.12.10 AS uv

# First, bundle the dependencies into the task root.
FROM public.ecr.aws/lambda/python:3.13 AS builder

# Enable bytecode compilation, to improve cold-start performance.
ENV UV_COMPILE_BYTECODE=1

# Disable installer metadata, to create a deterministic layer.
ENV UV_NO_INSTALLER_METADATA=1

# Enable copy mode to support bind mount caching.
ENV UV_LINK_MODE=copy

# Bundle the dependencies into the Lambda task root via `uv pip install --target`.
#
# Omit any local packages (`--no-emit-workspace`) and development dependencies (`--no-dev`).
# This ensures that the Docker layer cache is only invalidated when the `pyproject.toml` or `uv.lock`
# files change, but remains robust to changes in the application code.
RUN --mount=from=uv,source=/uv,target=/bin/uv \
    --mount=type=cache,target=/root/.cache/uv \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    uv export --frozen --no-emit-workspace --no-dev --no-editable -o requirements.txt && \
    uv pip install -r requirements.txt --target "${LAMBDA_TASK_ROOT}"

# If you have a workspace, copy it over and install it too.
#
# By omitting `--no-emit-workspace`, `library` will be copied into the task root. Using a separate
# `RUN` command ensures that all third-party dependencies are cached separately and remain
# robust to changes in the workspace.
RUN --mount=from=uv,source=/uv,target=/bin/uv \
    --mount=type=cache,target=/root/.cache/uv \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=library,target=library \
    uv export --frozen --no-dev --no-editable -o requirements.txt && \
    uv pip install -r requirements.txt --target "${LAMBDA_TASK_ROOT}"

FROM public.ecr.aws/lambda/python:3.13

# Copy the runtime dependencies from the builder stage.
COPY --from=builder ${LAMBDA_TASK_ROOT} ${LAMBDA_TASK_ROOT}

# Copy the application code.
COPY ./app ${LAMBDA_TASK_ROOT}/app

# Set the AWS Lambda handler.
CMD ["app.main.handler"]
```

!!! tip

    To deploy to ARM-based AWS Lambda runtimes, replace `public.ecr.aws/lambda/python:3.13` with
    `public.ecr.aws/lambda/python:3.13-arm64`.

Build and deploy the updated image as described above.

## Deploying a zip archive

AWS Lambda also supports deployment with zip archives. For simple applications, a zip archive can be
simpler and more efficient than a Docker image. However, zip archives cannot exceed
[250 MB](https://docs.aws.amazon.com/lambda/latest/dg/python-package.html#python-package-create-update).

To continue the FastAPI example, install the application dependencies in a local directory for AWS
Lambda:

```console
$ uv export --frozen --no-dev --no-editable -o requirements.txt
$ uv pip install \
   --no-installer-metadata \
   --no-compile-bytecode \
   --python-platform x86_64-manylinux2014 \
   --python 3.13 \
   --target packages \
   -r requirements.txt
```

!!! tip

    To deploy to ARM-based AWS Lambda runtimes, replace `x86_64-manylinux2014` with
    `aarch64-manylinux2014`.

Follow the
[AWS Lambda documentation](https://docs.aws.amazon.com/lambda/latest/dg/python-package.html) to add
the dependencies to a zip archive:

```console
$ cd packages
$ zip -r ../package.zip .
$ cd ..
```

Add the application code to the zip archive:

```console
$ zip -r package.zip app
```

Deploy the zip archive to AWS Lambda with the AWS Management Console or the AWS CLI:

```console
$ aws lambda create-function \
   --function-name myFunction \
   --runtime python3.13 \
   --zip-file fileb://package.zip \
   --handler app.main.handler \
   --role arn:aws:iam::111122223333:role/service-role/my-lambda-role
```

Create the
[execution role](https://docs.aws.amazon.com/lambda/latest/dg/lambda-intro-execution-role.html#permissions-executionrole-api)
with:

```console
$ aws iam create-role \
   --role-name my-lambda-role \
   --assume-role-policy-document '{"Version": "2012-10-17", "Statement": [{ "Effect": "Allow", "Principal": {"Service": "lambda.amazonaws.com"}, "Action": "sts:AssumeRole"}]}'
```

To update an existing function, run:

```console
$ aws lambda update-function-code \
   --function-name myFunction \
   --zip-file fileb://package.zip
```

!!! note

    The AWS Management Console uses `lambda_function.lambda_handler` as the default Lambda entrypoint.
    If your application uses a different entrypoint, change it in the AWS Management Console. The
    FastAPI application above uses `app.main.handler`.

To test the Lambda function, use the AWS Management Console or the AWS CLI:

```console
$ aws lambda invoke \
   --function-name myFunction \
   --payload file://event.json \
   --cli-binary-format raw-in-base64-out \
   response.json
{
  "StatusCode": 200,
  "ExecutedVersion": "$LATEST"
}
```

The `event.json` file contains the event payload for the Lambda function:

```json title="event.json"
{
  "httpMethod": "GET",
  "path": "/",
  "requestContext": {},
  "version": "1.0"
}
```

The `response.json` file contains the response from the Lambda function:

```json title="response.json"
{
  "statusCode": 200,
  "headers": {
    "content-length": "14",
    "content-type": "application/json"
  },
  "multiValueHeaders": {},
  "body": "\"Hello, world!\"",
  "isBase64Encoded": false
}
```

### Using a Lambda layer

AWS Lambda also supports multiple composed
[Lambda layers](https://docs.aws.amazon.com/lambda/latest/dg/python-layers.html) for zip archives.
These layers work like Docker image layers: they separate application code from dependencies.

Create a Lambda layer for application dependencies and attach it to the Lambda function. Keep the
layer separate from the application code. Because deployments can reuse the dependency layer, this
setup can improve cold-start performance after application updates.

To create a Lambda layer, make two zip archives: one for the application code and one for the
application dependencies.

First, create the dependency layer. Lambda layers require a different directory structure, so use
`--prefix` instead of `--target`:

```console
$ uv export --frozen --no-dev --no-editable -o requirements.txt
$ uv pip install \
   --no-installer-metadata \
   --no-compile-bytecode \
   --python-platform x86_64-manylinux2014 \
   --python 3.13 \
   --prefix packages \
   -r requirements.txt
```

Add the dependencies to a zip archive with the required Lambda layer layout:

```console
$ mkdir python
$ cp -r packages/lib python/
$ zip -r layer_content.zip python
```

!!! tip

    For deterministic zip archives, consider the `-X` flag for `zip`. It excludes extended attributes
    and file system metadata.

Publish the Lambda layer:

```console
$ aws lambda publish-layer-version --layer-name dependencies-layer \
   --zip-file fileb://layer_content.zip \
   --compatible-runtimes python3.13 \
   --compatible-architectures "x86_64"
```

Create the Lambda function as in the previous example, but omit the dependencies:

```console
$ # Zip the application code.
$ zip -r app.zip app

$ # Create the Lambda function.
$ aws lambda create-function \
   --function-name myFunction \
   --runtime python3.13 \
   --zip-file fileb://app.zip \
   --handler app.main.handler \
   --role arn:aws:iam::111122223333:role/service-role/my-lambda-role
```

Attach the dependency layer to the Lambda function. Use the ARN that `publish-layer-version`
returns:

```console
$ aws lambda update-function-configuration --function-name myFunction \
    --cli-binary-format raw-in-base64-out \
    --layers "arn:aws:lambda:region:111122223333:layer:dependencies-layer:1"
```

If the application dependencies change, republish the layer and update the Lambda function
configuration. This updates the layer without changing the application:

```console
$ # Update the dependencies in the layer.
$ aws lambda publish-layer-version --layer-name dependencies-layer \
   --zip-file fileb://layer_content.zip \
   --compatible-runtimes python3.13 \
   --compatible-architectures "x86_64"

$ # Update the Lambda function configuration.
$ aws lambda update-function-configuration --function-name myFunction \
    --cli-binary-format raw-in-base64-out \
    --layers "arn:aws:lambda:region:111122223333:layer:dependencies-layer:2"
```
