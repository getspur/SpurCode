# External indexing failure and deployment — bd-12o

Date: 2026-10-27. AWS account: 591950085580; region: ap-southeast-5.
Original job: `22d4a629-84f3-4adc-9579-691b13529ce8`.
Package: `codex-acp`, revision `v2.3.0`, source `git:github.com/agentclientprotocol/codex-acp`.

## Findings

1. **Lambda readiness was treated as terminal too quickly.** The original FetchSource invocation received HTTP 409 `CodeArtifactUserPendingException`, surfaced by Step Functions as `Lambda.AWSLambdaException`. The old policy allowed only two retries, after 2 and 4 seconds. Subsequent worker invocations failed with `Lambda.ResourceNotReadyException`, which was missing from Retry and Catch. The VPC worker version had been inactive after 17 days without use. AWS documents VPC resource reclamation after 14 idle days; this is consistent with the observed `Inactive`/`Idle` state. The exact internal cause of the earlier artifact-initialization delay is not exposed by the available AWS logs.

2. **The deployed ECS task lacked bucket configuration.** Revision 1 omitted `SPUR_CONTEXT_BRONZE_BUCKET` and `SPUR_CONTEXT_SILVER_BUCKET`. The worker defaulted to `spur-context`, while its IAM role correctly allowed only `spur-context-591950085580`. IAM simulation denied the default bucket and allowed the intended bucket. CloudWatch showed source archive upload failure. Correct variables already existed in Terraform source but had not reached the live task definition.

3. **The catalog exporter issued unsigned S3 requests on ECS.** After fixing buckets, forced ECS verification reached catalog publication and reported a supposedly missing object. The exact S3 object exists (4,607 bytes, created September 24). The exporter constructed an S3 client with environment credentials only. ECS provides task-role credentials through a metadata endpoint; earlier worker stages resolved them separately, but the catalog exporter did not. The regression test demonstrated that the ECS request lacked `X-Amz-Credential`, while the Lambda environment case passed. Both cases pass with the SDK default credential provider chain.

## Changes

- Both Lambda tasks retry readiness failures with waits of 5, 10, 20, 40, 60, 60, 60, and 60 seconds: 315 seconds total, capped at 60 seconds per wait.
- `Lambda.ResourceNotReadyException` is explicitly retryable and enters ECS fallback after worker retry exhaustion. Deterministic application errors are not added to the fallback policy.
- ECS revisions 2 and 3 set both medallion buckets to `spur-context-591950085580`; revision 3 additionally carries the catalog credential fix.
- Catalog S3 helpers load the SDK default credential chain, preserving region precedence and supporting credential refresh. S3 inspection errors no longer falsely assert that an object is missing.

## Verification

- Deployment-script tests: 53 passed. AWS ASL validation: OK, zero diagnostics.
- Readiness tests failed before the fix; the new credential regression failed for ECS before the fix and passed for Lambda. After the credential fix, both tests passed remotely through `scripts/spur-cargo`.
- Production Lambda verification `e3a1969c-7c83-4b07-abbc-7aaf9d612841` succeeded and published snapshot 4424: 1,631 files, 10,460 nodes, 25,140 edges.
- First isolated ECS verification `cc210383-cc88-4a20-94f4-859a747268dc` confirmed bucket repair, then exposed the unsigned catalog request. Its CloudWatch stream is `indexing/worker/b8452c15d0694bb2ac18bc2f9565e891` in `/ecs/spur-context-worker`.
- Corrected ECS verification `4fdc84a0-45f2-4dc0-a2da-e2740a43894d` succeeded at 10:05:56 UTC+7, publishing snapshot **4646** with the same file/node/edge counts. CloudWatch stream `indexing/worker/d60e33f314c44b5796216c337d768a44` confirms snapshot export completed in 19.633 seconds and translate in 25.467 seconds.
- Production now uses ECS task **revision 3**, image digest `sha256:4f4c998f74d833bd362a3addb32de7bda2c51134d8fea65be73e819ccafaa9cd`, and state-machine revision **6372a9d7-2c09-4140-8882-e4b3babd5329**. Live definition was read back and matched the prepared definition exactly.
- Temporary verification workflow was deleted after both executions finished. No Lambda image was replaced. The optimized worker build and container startup checks passed.

## Reproduction and rollback

The notebook `external-index-investigation-22d4a629.ipynb` in the sibling `tcinvest_rest` directory contains the CloudWatch, execution-history, S3, IAM, and deployment evidence. AWS CLI commands ran through DuckDB `shellfs` using notebook MCP.

Private operational artifacts are retained under `.spur/tmp/deployments/bd-12o/` (ignored by Git). Definitions, task registration payloads, validation receipts, and rollback snapshots are included. The original state-machine revision was `119fb821-e794-4add-88b2-42d11f3dadb0`; the readiness/bucket deployment produced `bc916f7e-37c5-4b56-9de7-ce9245312afc`. Rollback uses a retained definition with its pinned prior task revision; no IAM broadening was required.

## Follow-up recommendations

- Reconcile Terraform deployment configuration before the next full apply. Checked-in backend defaults point to the retired account, while the live state is `s3://spur-context-tfstate-591950085580-apse5/spur-context-service/staging/terraform.tfstate`. Staging variable placeholders must not be applied to the live environment. This incident used bounded CLI updates and did not rewrite Terraform state.
- Add deployment smoke coverage for both Lambda and forced ECS fallback; a successful Lambda run alone concealed the ECS defects.
- Validate required production bucket variables rather than relying on a development bucket default.
- Enable Step Functions error logging and alert on failed executions. Lambda invocation rejection may occur before handler logs exist, so execution history is essential alongside CloudWatch application logs.

## Source commits

- `b4e2a64d7`: readiness regressions.
- `4d709ea28`: Lambda readiness retry/fallback fix.
- `194c06c05`: ECS/Lambda credential regressions.
- `a65976a52`: catalog credential-chain fix.

The latest issue progress comment was blocked by automatic approval review because it regarded the project-management destination as unverified. Local notebook and deployment artifacts retain the evidence.

## AWS references

- [Lambda VPC lifecycle and idle functions](https://docs.aws.amazon.com/lambda/latest/dg/configuration-vpc.html)
- [Rust SDK credential provider chain](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/credproviders.html)
