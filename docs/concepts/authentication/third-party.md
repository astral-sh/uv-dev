# Third-party services

## Authentication with alternative package indexes

See these guides to authenticate with alternative Python package indexes:

- [Azure Artifacts](../../guides/integration/azure.md)
- [Google Artifact Registry](../../guides/integration/google.md)
- [AWS CodeArtifact](../../guides/integration/aws.md)
- [JFrog Artifactory](../../guides/integration/jfrog.md)

## Hugging Face support

uv automatically authenticates requests to the Hugging Face Hub. If the `HF_TOKEN` environment
variable is set, uv includes its value in requests to `huggingface.co`.

For example, run this command to execute `main.py` from a private Hugging Face dataset:

```console
$ HF_TOKEN=hf_... uv run https://huggingface.co/datasets/<user>/<name>/resolve/<branch>/main.py
```

`UV_NO_HF_TOKEN=1` disables automatic Hugging Face authentication.
