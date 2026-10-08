# spur-context-fetcher

Non-VPC AWS Lambda source fetcher for context indexing.

Lambda entrypoint:

```text
spur-context-fetcher-lambda
```

Input JSON:

```json
{
  "job_id": "job-123",
  "package": "getspur/SpurCode",
  "revision": "abc123",
  "source": "github",
  "source_url": "git+https://github.com/getspur/SpurCode.git",
  "source_kind": "git",
  "limits": {
    "max_source_bytes": 2147483648,
    "max_build_seconds": 900
  }
}
```

Success output JSON:

```json
{
  "source_url": "https://presigned-s3-url",
  "source_kind": "tarball",
  "source_archive_s3_uri": "s3://bucket/prefix/job-123/source.tar.gz",
  "original_source_url": "git+https://github.com/getspur/SpurCode.git",
  "original_source_kind": "git",
  "content_sha256": "hex-sha256",
  "bytes": 1234
}
```

Environment:

- `SPUR_CONTEXT_FETCH_BUCKET`: destination S3 bucket. Required.
- `SPUR_CONTEXT_FETCH_PREFIX`: destination key prefix. Defaults to `fetch`.
- `SPUR_CONTEXT_FETCH_PRESIGN_SECONDS`: presigned GET URL TTL. Defaults to `21600`.
- `SPUR_CONTEXT_MAX_TARBALL_BYTES`: tarball/zip download, normalized archive, and unpacked-tree cap.
- `SPUR_CONTEXT_MAX_GIT_BYTES`: git checkout unpacked-tree cap.
- `SPUR_CONTEXT_ALLOWED_SOURCE_DOMAINS`: comma-separated public domain allowlist. Empty allows all public domains.

The fetcher accepts public `https` and `git+https` sources only. It rejects
URL-embedded credentials, private/loopback/link-local DNS targets, and
redirects that fail the same validation.

`job_id` must match `[A-Za-z0-9_-]{1,128}` because it is part of the
deterministic S3 object key.
