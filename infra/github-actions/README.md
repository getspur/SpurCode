# GitHub Actions on AWS

Build infrastructure uses AWS account `591950085580` (the current local AWS CLI
account), primarily `ap-southeast-5`. Local wrappers default to AWS Malaysia,
with AWS Tokyo as the regional fallback. GCP build dispatch and GCS compiler
caching have been retired.

The versioned policies in this directory configure three GitHub OIDC roles:

| Policy prefix | Role | Purpose |
| --- | --- | --- |
| `cache` | `spur-github-release-sccache` | Read/write compiler caches under `spur/` and `spur-notebook/` |
| `builder` | `spur-github-release-dist` | Control the three ARM builder pool members and transfer artifacts |
| `context` | `spur-github-context-service` | Release the existing Code Lambda and run context-service smoke checks |

The cache bucket is `spurlab-591950085580-spur-sccache-apse5`. The context data
bucket is `spur-context-591950085580`. The builder EC2 instance profile remains
`spur-builder`. CI checks the returned AWS account before using credentials.

GitHub's OIDC provider is `https://token.actions.githubusercontent.com`, with
audience `sts.amazonaws.com`. SpurCode uses the immutable subject prefix
`repo:getspur@276235072/SpurCode@1211291099`, as returned by:

```sh
gh api repos/getspur/SpurCode/actions/oidc/customization/sub
```

Cache and builder access is restricted to main and `v*` release tags. The
context-service jobs use the `context-service-staging` environment subject; its GitHub deployment branch
policy allows only `main`.
Pull requests use disk cache and cannot assume the shared-cache role. The
notebook's main branch can assume the cache role using its existing legacy
subject format.

The cache role may list the bucket so missing objects return `NoSuchKey`, which
sccache needs for cache misses. Object access remains limited to the two cache
prefixes; the role cannot read or overwrite the build-script bundle.

Each `*-trust.json` is an assume-role policy; each `*-policy.json` is the
corresponding inline `SpurCodeActions` policy. Existing roles can be updated
with `aws iam update-assume-role-policy` and `aws iam put-role-policy`. Their
maximum session duration is 10800 seconds for the release build workflow.

Repository variables:

| Variable | Value |
| --- | --- |
| `AWS_SCCACHE_ROLE_ARN` | `arn:aws:iam::591950085580:role/spur-github-release-sccache` |
| `AWS_RELEASE_ROLE_ARN` | `arn:aws:iam::591950085580:role/spur-github-release-dist` |
| `AWS_SCCACHE_BUCKET` | `spurlab-591950085580-spur-sccache-apse5` |
| `SPUR_BUILDER_AMI_ID` | `ami-0804f4b1748f19229` |
| `CONTEXT_SERVICE_AWS_ROLE_ARN` | `arn:aws:iam::591950085580:role/spur-github-context-service` |
| `CONTEXT_SERVICE_RELEASE_ROLE_ARN` | `arn:aws:iam::591950085580:role/spur-github-context-service` |
| `CONTEXT_SERVICE_AWS_REGION` | `ap-southeast-5` |
| `CONTEXT_SERVICE_STAGING_SOURCE_BUCKET` | `spur-context-591950085580` |
| `CONTEXT_SERVICE_STAGING_DATA_BUCKET` | `spur-context-591950085580` |

The old `GCP_SA_EMAIL` and `GCP_WIF_PROVIDER` repository variables are removed.

After changes to the shared build scripts, publish their bundle from a checkout
with the `spur-notebook` sibling present:

```sh
scripts/cloud-build-publish-bundle.sh
gh workflow run aws-infrastructure.yml --repo getspur/SpurCode --ref main
```

The manual infrastructure check verifies all three OIDC roles, performs an S3
cache round trip with cleanup, checks the AMI and builder discovery, downloads
the bundle, and reads the serving Lambda configuration. It does not launch
builders, update Lambda code, or publish a release.
