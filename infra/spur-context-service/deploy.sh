#!/usr/bin/env bash
# Build, package, and deploy spur-context-service Lambda, indexing worker, and source fetcher.
#
# Usage:
#   ./deploy.sh                    # build Lambda + worker, terraform apply
#   ./deploy.sh --env prod         # deploy with prod backend + var file
#   ./deploy.sh --backend-config backends/prod.s3.tfbackend --var-file env/prod.tfvars
#   ./deploy.sh --local-code-zip path --local-knowledge-zip path
#                                  # skip serving Lambda builds, use existing zips
#   ./deploy.sh --skip-worker      # skip worker image build/push
#   ./deploy.sh --skip-worker --package-only # build Lambda zip, skip terraform
#   ./deploy.sh --worker-image-only # build/push worker/fetcher images, print ECS image URI
#   ./deploy.sh --build-mode self-contained --no-push --worker-image-only
#
# Prerequisites:
#   - scripts/spur-cargo (remote mode) or docker buildx + QEMU
#     (self-contained mode); deploy builds force a Graviton2-safe arm64 CPU
#     baseline in both paths
#   - terraform >= 1.5
#   - docker (for worker container build)
#   - AWS credentials with Lambda/S3/IAM/ECR/ECS/SFN access
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
INFRA_DIR="$SCRIPT_DIR"
BUILD_DIR=$(mktemp -d)
trap 'rm -rf "$BUILD_DIR"' EXIT

DUCKDB_VERSION="1.5.4"
EXT_PLATFORM="linux_arm64"
# NOTE: the postgres backend extension is published on the DuckDB CDN as
# "postgres_scanner" (the bare "postgres" name 404s). `LOAD postgres` resolves
# the alias to the postgres_scanner file, so bundling postgres_scanner makes the
# offline `LOAD postgres` succeed.
# `lance` (vector sidecar reader) is bundled so the worker's `LOAD lance` finds
# it locally; without it, translate falls back to `INSTALL lance`, which the
# worker cannot download (no egress to extensions.duckdb.org from its VPC) and
# times out ~60s per sidecar, silently skipping embeddings/section vectors.
EXTENSIONS=("httpfs" "ducklake" "postgres_scanner" "sqlite_scanner" "aws" "parquet" "json" "lance")
# SHA-256 digests of the uncompressed DuckDB v1.5.4 linux_arm64 extension
# artifacts. Keep this list in the same order as EXTENSIONS.
EXTENSION_SHA256=(
    "6d30a487968cbe5553b7272fd5bad0e4485c4117db96e21fd1e5e2a225a5a538"
    "f73ec9ab68a6de5c3c190cd1ecbba553a5791bf5821a991d70ea65abc5e45562"
    "102f71e6d2e603b1056407410f981c7ec141375e1edd16f2cb61e901e9c6d617"
    "81135f8c3b4064bc5031c3bc08473a7224851461fa9a2b645ea8f851eba4a621"
    "8244ac8560925c3caf71f2087b45e3e37aa84a8a79712b9b8f3312524bd3f483"
    "d1aa523e5ae55731da2f67fca02ea1460e35fe4f8d824627fb565019e3eece1c"
    "10ce8205acd23bfbc98c934803d69f1e6768fe9bfcb4c7958406144a1ea898bd"
    "81a9f544f9f56be18db46060baf596ab973e17b50785a1eb1edd4885908b2158"
)
# shellcheck disable=SC2034
WORKER_DUCKDB_EXTENSION_DIR="/opt/duckdb/extensions"

resolve_image_tag() {
    local git_short_sha
    if git_short_sha="$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null)"; then
        local image_tag="$git_short_sha"
        if [[ -n "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=normal 2>/dev/null)" ]]; then
            image_tag="${image_tag}-dirty-$(date -u +%Y%m%d%H%M%S)"
        fi
        echo "$image_tag"
    else
        date -u +"nogit-%Y%m%d%H%M%S"
    fi
}

# Worker container config.  The image is built on the remote VM (x86_64 for
# Fargate compatibility) and pushed to ECR.
WORKER_ECR_REPO="spur-context-worker"
WORKER_LAMBDA_ECR_REPO="spur-context-worker-lambda"
SOURCE_FETCHER_LAMBDA_ECR_REPO="spur-context-source-fetcher"
IMAGE_TAG="${SPUR_CONTEXT_SERVICE_IMAGE_TAG:-$(resolve_image_tag)}"
WORKER_IMAGE_TAG="$IMAGE_TAG"
LATEST_IMAGE_TAG="latest"

resolve_terraform_aws_region() {
    local region="${SPUR_CONTEXT_SERVICE_AWS_REGION:-}"
    if [[ -z "$region" ]]; then
        region="$(cd "$INFRA_DIR" && terraform output -raw aws_region 2>/dev/null || true)"
    fi
    printf '%s\n' "${region:-ap-southeast-5}"
}

AWS_REGION_VAL="$(resolve_terraform_aws_region)"
WORKER_IMAGE_URI=""
WORKER_LAMBDA_IMAGE_URI=""
SOURCE_FETCHER_LAMBDA_IMAGE_URI=""
SELF_CONTAINED_EXPORT_DIR=""

log() { echo "[deploy] $*" >&2; }

# shellcheck source=infra/spur-context-service/graviton2-baseline.sh
source "$SCRIPT_DIR/graviton2-baseline.sh"

assert_ecr_exclusion_filter_support() {
    local capability_output

    if ! capability_output="$(aws ecr put-image-tag-mutability \
        --repository-name spur-capability-preflight \
        --image-tag-mutability IMMUTABLE_WITH_EXCLUSION \
        --image-tag-mutability-exclusion-filters \
            filterType=WILDCARD,filter="$LATEST_IMAGE_TAG" \
        --generate-cli-skeleton output 2>&1)"; then
        log "AWS CLI capability preflight failed: ECR immutable tag exclusions are unavailable; no ECR repository was changed"
        log "Upgrade AWS CLI v2 to a release that supports put-image-tag-mutability --image-tag-mutability-exclusion-filters"
        log "$capability_output"
        exit 2
    fi
}

assert_selected_aws_region() {
    local configured_region
    local selected_region

    configured_region="$(aws configure get region 2>/dev/null || true)"
    selected_region="${AWS_REGION:-${AWS_DEFAULT_REGION:-$configured_region}}"

    if [[ -z "$selected_region" ]]; then
        log "AWS region preflight failed: no CLI or environment region is selected"
        exit 2
    fi
    if [[ "$selected_region" != "$AWS_REGION_VAL" ]]; then
        log "AWS region preflight failed: selected '$selected_region', Terraform expects '$AWS_REGION_VAL'"
        exit 2
    fi

    # Child deployment scripts invoke the AWS CLI too. Keep their effective
    # region pinned to the Terraform-selected region after the assertion.
    export AWS_REGION="$AWS_REGION_VAL"
    export AWS_DEFAULT_REGION="$AWS_REGION_VAL"
    log "AWS region preflight passed: $AWS_REGION_VAL"
}

ensure_ecr_repository() {
    local repo="$1"
    local repository_exists="$2"

    if [[ "$repository_exists" != "true" ]]; then
        aws ecr create-repository \
            --repository-name "$repo" \
            --image-scanning-configuration scanOnPush=true \
            --image-tag-mutability IMMUTABLE_WITH_EXCLUSION \
            --image-tag-mutability-exclusion-filters \
                filterType=WILDCARD,filter="$LATEST_IMAGE_TAG" \
            --region "$AWS_REGION_VAL" >/dev/null
    fi

    # Reconcile existing repositories too; create-time settings alone would
    # leave older mutable/unscanned repositories unchanged.
    aws ecr put-image-scanning-configuration \
        --repository-name "$repo" \
        --image-scanning-configuration scanOnPush=true \
        --region "$AWS_REGION_VAL" >/dev/null
    aws ecr put-image-tag-mutability \
        --repository-name "$repo" \
        --image-tag-mutability IMMUTABLE_WITH_EXCLUSION \
        --image-tag-mutability-exclusion-filters \
            filterType=WILDCARD,filter="$LATEST_IMAGE_TAG" \
        --region "$AWS_REGION_VAL" >/dev/null
}

ensure_ecr_repositories() {
    local repository_exists

    repository_exists=false
    if aws ecr describe-repositories --repository-names "$WORKER_ECR_REPO" --region "$AWS_REGION_VAL" >/dev/null 2>&1; then
        repository_exists=true
    fi
    ensure_ecr_repository "$WORKER_ECR_REPO" "$repository_exists"

    repository_exists=false
    if aws ecr describe-repositories --repository-names "$WORKER_LAMBDA_ECR_REPO" --region "$AWS_REGION_VAL" >/dev/null 2>&1; then
        repository_exists=true
    fi
    ensure_ecr_repository "$WORKER_LAMBDA_ECR_REPO" "$repository_exists"

    repository_exists=false
    if aws ecr describe-repositories --repository-names "$SOURCE_FETCHER_LAMBDA_ECR_REPO" --region "$AWS_REGION_VAL" >/dev/null 2>&1; then
        repository_exists=true
    fi
    ensure_ecr_repository "$SOURCE_FETCHER_LAMBDA_ECR_REPO" "$repository_exists"
}

aws_account_id() {
    if [[ -z "${AWS_ACCOUNT_ID:-}" ]]; then
        AWS_ACCOUNT_ID="$(aws sts get-caller-identity --query Account --output text)"
    fi
    echo "$AWS_ACCOUNT_ID"
}

ecr_image_tag() {
    local repo="$1"
    local image_tag="${2:-$IMAGE_TAG}"
    echo "$(aws_account_id).dkr.ecr.${AWS_REGION_VAL}.amazonaws.com/${repo}:${image_tag}"
}

ecr_latest_image_tag() {
    local repo="$1"
    ecr_image_tag "$repo" "$LATEST_IMAGE_TAG"
}

ecr_image_digest() {
    local repo="$1"
    local image_tag="$2"
    aws ecr batch-get-image \
        --repository-name "$repo" \
        --image-ids "imageTag=$image_tag" \
        --region "$AWS_REGION_VAL" \
        --query 'images[0].imageId.imageDigest' \
        --output text
}

tag_ecr_image_as_latest() {
    local repo="$1"
    local image_tag="$2"
    local latest_tag
    local image_manifest
    local source_digest
    local latest_digest

    latest_tag="$(ecr_latest_image_tag "$repo")"
    image_manifest="$(aws ecr batch-get-image \
        --repository-name "$repo" \
        --image-ids "imageTag=$image_tag" \
        --region "$AWS_REGION_VAL" \
        --query 'images[0].imageManifest' \
        --output text)"

    if [[ -z "$image_manifest" || "$image_manifest" == "None" ]]; then
        log "failed to resolve ECR image manifest for $repo:$image_tag"
        exit 1
    fi

    source_digest="$(ecr_image_digest "$repo" "$image_tag")"
    if [[ -z "$source_digest" || "$source_digest" == "None" ]]; then
        log "failed to resolve ECR image digest for $repo:$image_tag"
        exit 1
    fi
    latest_digest="$(ecr_image_digest "$repo" "$LATEST_IMAGE_TAG")"
    if [[ "$latest_digest" == "$source_digest" ]]; then
        log "latest image pointer already current: $latest_tag"
        return 0
    fi

    aws ecr put-image \
        --repository-name "$repo" \
        --image-tag "$LATEST_IMAGE_TAG" \
        --image-manifest "$image_manifest" \
        --region "$AWS_REGION_VAL" >/dev/null
    log "latest image pointer updated: $latest_tag"
}

normalize_push_images() {
    case "${SPUR_CONTEXT_SERVICE_PUSH_IMAGES:-1}" in
        1 | true | TRUE | yes | YES | on | ON) echo true ;;
        0 | false | FALSE | no | NO | off | OFF) echo false ;;
        *)
            log "SPUR_CONTEXT_SERVICE_PUSH_IMAGES must be true/false or 1/0"
            exit 2
            ;;
    esac
}

validate_build_mode() {
    case "$BUILD_MODE" in
        remote | self-contained) ;;
        *)
            log "unknown build mode: $BUILD_MODE"
            log "expected --build-mode remote or --build-mode self-contained"
            exit 2
            ;;
    esac
}

write_self_contained_build_dockerfile() {
    local dockerfile="$1"
    cat > "$dockerfile" <<'DOCKERFILE'
# syntax=docker/dockerfile:1.7
FROM --platform=$TARGETPLATFORM rust:1.88-bookworm AS builder

ARG RUSTFLAGS
ARG CFLAGS
ARG CXXFLAGS

ENV CARGO_TERM_COLOR=always \
    CARGO_INCREMENTAL=0 \
    CI=true \
    SPUR_REMOTE=0 \
    SPUR_SCCACHE_S3=0 \
    AWS_RUSTFLAGS_DEFAULT="${RUSTFLAGS}" \
    RUSTFLAGS="${RUSTFLAGS}" \
    CFLAGS="${CFLAGS}" \
    CXXFLAGS="${CXXFLAGS}"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential pkg-config libssl-dev cmake clang lld protobuf-compiler \
        git ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /mnt/cargo/rust-lld-driver \
    && ln -sf "$(command -v ld.lld)" /mnt/cargo/rust-lld-driver/ld.lld

WORKDIR /workspace
COPY . .

RUN scripts/spur-cargo --workdir crates/spur-context-service build --no-default-features --features code-lambda --bin spur-context-code-lambda --release
RUN scripts/spur-cargo --workdir crates/spur-context-service build --no-default-features --features knowledge-lambda --bin spur-context-knowledge-lambda --release
RUN scripts/spur-cargo --workdir crates/spur-context-service build --features worker --release
RUN scripts/spur-cargo --workdir crates/spur-context-service build --features worker-lambda --release
RUN scripts/spur-cargo build -p spur-context-fetcher --release
RUN scripts/spur-cargo build -p spur-cli --release --no-default-features --features worker-no-embed,duckdb-bundled

RUN mkdir -p /out \
    && cp crates/spur-context-service/target/release/spur-context-code-lambda /out/spur-context-code-lambda \
    && cp crates/spur-context-service/target/release/spur-context-knowledge-lambda /out/spur-context-knowledge-lambda \
    && cp crates/spur-context-service/target/release/spur-context-worker /out/spur-context-worker \
    && cp crates/spur-context-service/target/release/spur-context-worker-lambda /out/spur-context-worker-lambda \
    && cp target/release/spur-context-fetcher-lambda /out/spur-context-fetcher-lambda \
    && cp target/release/spur /out/spur

FROM scratch AS artifacts
COPY --from=builder /out/ /
DOCKERFILE
}

prepare_self_contained_build_context() {
    local context_dir="$BUILD_DIR/self-contained-context"
    rm -rf "$context_dir"
    mkdir -p "$context_dir"

    # Keep generated target/ artifacts out of the docker build context. CI runs
    # this path from a checked-out commit, so a tracked-source archive is the
    # reproducible source of truth for the arm64 build container.
    git -C "$REPO_ROOT" archive --format=tar HEAD | tar -xf - -C "$context_dir"
    echo "$context_dir"
}

ensure_self_contained_artifacts() {
    if [[ -n "$SELF_CONTAINED_EXPORT_DIR" ]]; then
        return
    fi

    assert_graviton2_safe_flags "self-contained buildx artifacts" \
        "$SPUR_CONTEXT_GRAVITON2_RUSTFLAGS" \
        "$SPUR_CONTEXT_GRAVITON2_CFLAGS" \
        "$SPUR_CONTEXT_GRAVITON2_CXXFLAGS"

    local export_dir="$BUILD_DIR/self-contained-artifacts"
    local dockerfile="$BUILD_DIR/self-contained-build.Dockerfile"
    local context_dir
    mkdir -p "$export_dir"
    write_self_contained_build_dockerfile "$dockerfile"
    context_dir="$(prepare_self_contained_build_context)"

    log "building self-contained arm64 artifacts with docker buildx (neoverse-n1)..."
    RUSTFLAGS="$SPUR_CONTEXT_GRAVITON2_RUSTFLAGS" \
    CFLAGS="$SPUR_CONTEXT_GRAVITON2_CFLAGS" \
    CXXFLAGS="$SPUR_CONTEXT_GRAVITON2_CXXFLAGS" \
        docker buildx build \
            --platform linux/arm64 --provenance=false \
            --build-arg "RUSTFLAGS=$SPUR_CONTEXT_GRAVITON2_RUSTFLAGS" \
            --build-arg "CFLAGS=$SPUR_CONTEXT_GRAVITON2_CFLAGS" \
            --build-arg "CXXFLAGS=$SPUR_CONTEXT_GRAVITON2_CXXFLAGS" \
            --target artifacts \
            --output "type=local,dest=$export_dir" \
            -f "$dockerfile" \
            "$context_dir"

    SELF_CONTAINED_EXPORT_DIR="$export_dir"
}

build_self_contained_artifacts() {
    ensure_self_contained_artifacts
}

remote_worktree_key() {
    local git_toplevel worktree_key default_remote_namespace remote_namespace
    git_toplevel="$(git -C "$REPO_ROOT" rev-parse --show-toplevel)"
    if [[ "$git_toplevel" == *"/.spur/worktrees/"* ]]; then
        worktree_key="worktrees/$(basename "$git_toplevel")"
        default_remote_namespace="$(basename "$(dirname "$(dirname "$(dirname "$git_toplevel")")")")"
    else
        worktree_key="main"
        default_remote_namespace="$(basename "$git_toplevel")"
    fi
    remote_namespace="${SPUR_REMOTE_NAMESPACE:-$default_remote_namespace}"
    echo "$remote_namespace/$worktree_key"
}

remote_worktree_path() {
    local rel_path="$1"
    echo "/home/${AWS_SSH_USER:-admin}/$(remote_worktree_key)/$rel_path"
}

remote_target_path() {
    local rel_path="$1"
    rel_path="${rel_path#target/}"
    echo "/mnt/cargo/targets/$(remote_worktree_key)/$rel_path"
}

fetch_remote_file() {
    local remote_path="$1"
    local local_dest="$2"
    local cloud_dir="$REPO_ROOT/scripts/cloud-build"

    (
        SCRIPT_DIR="$cloud_dir"
        # shellcheck disable=SC1091
        source "$SCRIPT_DIR/config.env"
        # shellcheck disable=SC1090
        source "$SCRIPT_DIR/provider-${SPUR_CLOUD}.sh"
        provider_choose_transport
        provider_fetch "$remote_path" "$local_dest"
    )
}

fetch_remote_worktree_file() {
    local remote_rel_path="$1"
    local local_dest="$2"
    fetch_remote_file "$(remote_worktree_path "$remote_rel_path")" "$local_dest"
}

fetch_remote_target_file() {
    local remote_rel_path="$1"
    local local_dest="$2"
    fetch_remote_file "$(remote_target_path "$remote_rel_path")" "$local_dest"
}

sha256_file() {
    local path="$1"

    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$path" | awk '{print $1}'
    else
        log "SHA-256 verification requires sha256sum or shasum"
        exit 1
    fi
}

download_extensions() {
    local ext_dir="$BUILD_DIR/.duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}"
    local i
    mkdir -p "$ext_dir"
    for i in "${!EXTENSIONS[@]}"; do
        local ext="${EXTENSIONS[$i]}"
        local expected_sha="${EXTENSION_SHA256[$i]}"
        local url="https://extensions.duckdb.org/v${DUCKDB_VERSION}/${EXT_PLATFORM}/${ext}.duckdb_extension.gz"
        local dest="$ext_dir/${ext}.duckdb_extension"
        local candidate="$dest"
        local actual_sha

        if [[ ! -f "$dest" ]]; then
            log "downloading $ext..."
            candidate="${dest}.download"
            rm -f "$candidate"
            if ! curl --fail --silent --show-error --location "$url" | gunzip > "$candidate"; then
                rm -f "$candidate"
                log "failed to download DuckDB extension: $ext"
                exit 1
            fi
        fi

        actual_sha="$(sha256_file "$candidate")"
        if [[ "$actual_sha" != "$expected_sha" ]]; then
            if [[ "$candidate" != "$dest" ]]; then
                rm -f "$candidate"
            fi
            log "SHA-256 mismatch for DuckDB extension $ext: expected $expected_sha, got $actual_sha"
            exit 1
        fi

        if [[ "$candidate" != "$dest" ]]; then
            mv "$candidate" "$dest"
        fi
    done
}

copy_worker_extensions() {
    local context_dir="$1"
    local dest="$context_dir/duckdb-extensions"
    mkdir -p "$dest"
    cp -R "$BUILD_DIR/.duckdb/extensions/." "$dest/"
}

build_serving_binaries() {
    local code_dir="$BUILD_DIR/code-lambda"
    local knowledge_dir="$BUILD_DIR/knowledge-lambda"
    mkdir -p "$code_dir" "$knowledge_dir"

    if [[ "${BUILD_MODE:-remote}" == "self-contained" ]]; then
        ensure_self_contained_artifacts
        cp "$SELF_CONTAINED_EXPORT_DIR/spur-context-code-lambda" "$code_dir/bootstrap"
        cp "$SELF_CONTAINED_EXPORT_DIR/spur-context-knowledge-lambda" "$knowledge_dir/bootstrap"
        chmod +x "$code_dir/bootstrap" "$knowledge_dir/bootstrap"
        return
    fi

    log "building serving Lambdas on remote Graviton4 VM (portable arm64: neoverse-n1)..."
    cd "$REPO_ROOT"
    # spur-context-service is excluded from the workspace (standalone Cargo.toml
    # with duckdb 1.5.4). Build from the crate directory directly.
    run_graviton2_safe_cargo "Code Lambda bootstrap" \
        --workdir crates/spur-context-service build --no-default-features --features code-lambda --bin spur-context-code-lambda --release
    fetch_remote_target_file release/spur-context-code-lambda "$BUILD_DIR/code-lambda/bootstrap"
    run_graviton2_safe_cargo "Knowledge Lambda bootstrap" \
        --workdir crates/spur-context-service build --no-default-features --features knowledge-lambda --bin spur-context-knowledge-lambda --release
    fetch_remote_target_file release/spur-context-knowledge-lambda "$BUILD_DIR/knowledge-lambda/bootstrap"
}

build_spur_cli() {
    if [[ "${BUILD_MODE:-remote}" == "self-contained" ]]; then
        ensure_self_contained_artifacts
        return
    fi

    log "building spur CLI (portable arm64 neoverse-n1 for worker image)..."
    cd "$REPO_ROOT"
    run_graviton2_safe_cargo "spur CLI worker image dependency" \
        build -p spur-cli --release --no-default-features --features worker-no-embed,duckdb-bundled
}

copy_knowledge_extensions() {
    local knowledge_dir="$BUILD_DIR/knowledge-lambda"
    rm -rf "$knowledge_dir/.duckdb"
    cp -R "$BUILD_DIR/.duckdb" "$knowledge_dir/.duckdb"
}

package_code_zip() {
    local zip_path="$1"
    log "packaging Code Lambda zip..."
    cd "$BUILD_DIR/code-lambda"
    zip -r "$zip_path" bootstrap
    log "Code Lambda zip size: $(du -h "$zip_path" | cut -f1)"
}

package_knowledge_zip() {
    local zip_path="$1"
    log "packaging Knowledge Lambda zip..."
    cd "$BUILD_DIR/knowledge-lambda"
    # Exclude lance from the SERVING zip: serving reads gold DuckLake tables
    # (embeddings are native FLOAT[]) and never loads lance, and bundling it
    # pushes the zip past Lambda's 250 MB unzipped limit. lance stays bundled in
    # the worker images (copy_worker_extensions) where translate needs it.
    zip -r "$zip_path" bootstrap .duckdb/ -x "*.gz" -x "*/lance.duckdb_extension"
    log "Knowledge Lambda zip size: $(du -h "$zip_path" | cut -f1)"
}

build_worker() {
    if [[ "${BUILD_MODE:-remote}" == "self-contained" ]]; then
        ensure_self_contained_artifacts
        return
    fi

    log "building worker binary (--features worker, arm64 neoverse-n1 for Fargate)..."
    cd "$REPO_ROOT"
    # Fargate ARM64 runs on Graviton2 (neoverse-n1). The build VM's default
    # newest-CPU tuning can produce unsupported instructions. Match the Lambda
    # build flags.
    # The binary stays on the VM — build_and_push_worker_image() builds the
    # Docker image remotely using docker-build.sh, so no local fetch needed.
    run_graviton2_safe_cargo "Fargate worker binary" \
        --workdir crates/spur-context-service build --features worker --release
}

build_worker_lambda() {
    if [[ "${BUILD_MODE:-remote}" == "self-contained" ]]; then
        ensure_self_contained_artifacts
        return
    fi

    log "building Lambda worker binary (--features worker-lambda, arm64 neoverse-n1)..."
    cd "$REPO_ROOT"
    run_graviton2_safe_cargo "worker Lambda image binary" \
        --workdir crates/spur-context-service build --features worker-lambda --release
}

build_source_fetcher_lambda() {
    if [[ "${BUILD_MODE:-remote}" == "self-contained" ]]; then
        ensure_self_contained_artifacts
        return
    fi

    log "building source fetcher Lambda binary (arm64 neoverse-n1)..."
    cd "$REPO_ROOT"
    run_graviton2_safe_cargo "source fetcher Lambda image binary" \
        build -p spur-context-fetcher --release
}

write_worker_image_dockerfile() {
    local dockerfile="$1"
    cat > "$dockerfile" <<DOCKERFILE
FROM debian:bookworm-slim
LABEL io.spur.cpu-baseline="graviton2-safe"
RUN apt-get update && apt-get install -y --no-install-recommends git curl tar unzip ca-certificates && rm -rf /var/lib/apt/lists/*
ENV SPUR_CONTEXT_DUCKDB_EXTENSION_DIR=/opt/duckdb/extensions
WORKDIR /workspace
COPY duckdb-extensions/ /opt/duckdb/extensions/
COPY spur-context-worker /usr/local/bin/spur-context-worker
COPY spur /usr/local/bin/spur
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/httpfs.duckdb_extension
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/ducklake.duckdb_extension
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/postgres_scanner.duckdb_extension
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/sqlite_scanner.duckdb_extension
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/aws.duckdb_extension
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/lance.duckdb_extension
RUN /usr/local/bin/spur --version
RUN /usr/local/bin/spur-context-worker || true
CMD ["/usr/local/bin/spur-context-worker"]
DOCKERFILE
}

write_worker_lambda_image_dockerfile() {
    local dockerfile="$1"
    cat > "$dockerfile" <<DOCKERFILE
FROM debian:bookworm-slim
LABEL io.spur.cpu-baseline="graviton2-safe"
RUN apt-get update && apt-get install -y --no-install-recommends git curl tar unzip ca-certificates && rm -rf /var/lib/apt/lists/*
ENV SPUR_CONTEXT_DUCKDB_EXTENSION_DIR=/opt/duckdb/extensions
WORKDIR /workspace
COPY duckdb-extensions/ /opt/duckdb/extensions/
COPY spur-context-worker-lambda /usr/local/bin/spur-context-worker-lambda
COPY spur /usr/local/bin/spur
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/httpfs.duckdb_extension
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/ducklake.duckdb_extension
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/postgres_scanner.duckdb_extension
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/sqlite_scanner.duckdb_extension
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/aws.duckdb_extension
RUN test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/lance.duckdb_extension
RUN /usr/local/bin/spur --version
RUN /usr/local/bin/spur-context-worker-lambda --smoke
ENTRYPOINT ["/usr/local/bin/spur-context-worker-lambda"]
DOCKERFILE
}

write_source_fetcher_lambda_image_dockerfile() {
    local dockerfile="$1"
    cat > "$dockerfile" <<DOCKERFILE
FROM debian:bookworm-slim
LABEL io.spur.cpu-baseline="graviton2-safe"
RUN apt-get update && apt-get install -y --no-install-recommends git curl tar unzip ca-certificates && rm -rf /var/lib/apt/lists/*
ENV GIT_TERMINAL_PROMPT=0
WORKDIR /workspace
COPY spur-context-fetcher-lambda /usr/local/bin/spur-context-fetcher-lambda
RUN /usr/local/bin/spur-context-fetcher-lambda --smoke
ENTRYPOINT ["/usr/local/bin/spur-context-fetcher-lambda"]
DOCKERFILE
}

build_local_worker_images() {
    ensure_self_contained_artifacts

    local output_dir="$REPO_ROOT/target/lambda"
    local worker_context="$BUILD_DIR/worker-image-context"
    local worker_lambda_context="$BUILD_DIR/worker-lambda-image-context"
    local source_fetcher_context="$BUILD_DIR/source-fetcher-image-context"
    local worker_dockerfile="$worker_context/Dockerfile"
    local worker_lambda_dockerfile="$worker_lambda_context/Dockerfile"
    local source_fetcher_dockerfile="$source_fetcher_context/Dockerfile"
    mkdir -p "$output_dir" "$worker_context" "$worker_lambda_context" "$source_fetcher_context"

    cp "$SELF_CONTAINED_EXPORT_DIR/spur-context-worker" "$worker_context/spur-context-worker"
    cp "$SELF_CONTAINED_EXPORT_DIR/spur" "$worker_context/spur"
    cp "$SELF_CONTAINED_EXPORT_DIR/spur-context-worker-lambda" "$worker_lambda_context/spur-context-worker-lambda"
    cp "$SELF_CONTAINED_EXPORT_DIR/spur" "$worker_lambda_context/spur"
    cp "$SELF_CONTAINED_EXPORT_DIR/spur-context-fetcher-lambda" "$source_fetcher_context/spur-context-fetcher-lambda"
    copy_worker_extensions "$worker_context"
    copy_worker_extensions "$worker_lambda_context"
    chmod +x \
        "$worker_context/spur-context-worker" \
        "$worker_context/spur" \
        "$worker_lambda_context/spur-context-worker-lambda" \
        "$worker_lambda_context/spur" \
        "$source_fetcher_context/spur-context-fetcher-lambda"
    write_worker_image_dockerfile "$worker_dockerfile"
    write_worker_lambda_image_dockerfile "$worker_lambda_dockerfile"
    write_source_fetcher_lambda_image_dockerfile "$source_fetcher_dockerfile"

    if [[ "$PUSH_IMAGES" == "true" ]]; then
        WORKER_IMAGE_URI="$(ecr_image_tag "$WORKER_ECR_REPO")"
        WORKER_LAMBDA_IMAGE_URI="$(ecr_image_tag "$WORKER_LAMBDA_ECR_REPO")"
        SOURCE_FETCHER_LAMBDA_IMAGE_URI="$(ecr_image_tag "$SOURCE_FETCHER_LAMBDA_ECR_REPO")"
        local worker_latest_image_uri
        local worker_lambda_latest_image_uri
        local source_fetcher_latest_image_uri
        worker_latest_image_uri="$(ecr_latest_image_tag "$WORKER_ECR_REPO")"
        worker_lambda_latest_image_uri="$(ecr_latest_image_tag "$WORKER_LAMBDA_ECR_REPO")"
        source_fetcher_latest_image_uri="$(ecr_latest_image_tag "$SOURCE_FETCHER_LAMBDA_ECR_REPO")"

        log "building and pushing self-contained worker image: $WORKER_IMAGE_URI"
        docker buildx build \
            --platform linux/arm64 --provenance=false \
            --push \
            --tag "$WORKER_IMAGE_URI" \
            --tag "$worker_latest_image_uri" \
            "$worker_context"

        log "building and pushing self-contained worker Lambda image: $WORKER_LAMBDA_IMAGE_URI"
        docker buildx build \
            --platform linux/arm64 --provenance=false \
            --push \
            --tag "$WORKER_LAMBDA_IMAGE_URI" \
            --tag "$worker_lambda_latest_image_uri" \
            "$worker_lambda_context"

        log "building and pushing self-contained source fetcher Lambda image: $SOURCE_FETCHER_LAMBDA_IMAGE_URI"
        docker buildx build \
            --platform linux/arm64 --provenance=false \
            --push \
            --tag "$SOURCE_FETCHER_LAMBDA_IMAGE_URI" \
            --tag "$source_fetcher_latest_image_uri" \
            "$source_fetcher_context"
    else
        WORKER_IMAGE_URI="${WORKER_ECR_REPO}:${IMAGE_TAG}"
        WORKER_LAMBDA_IMAGE_URI="${WORKER_LAMBDA_ECR_REPO}:${IMAGE_TAG}"
        SOURCE_FETCHER_LAMBDA_IMAGE_URI="${SOURCE_FETCHER_LAMBDA_ECR_REPO}:${IMAGE_TAG}"

        log "building self-contained worker image tar: $output_dir/spur-context-worker-image.tar"
        docker buildx build \
            --platform linux/arm64 --provenance=false \
            --tag "$WORKER_IMAGE_URI" \
            --tag "${WORKER_ECR_REPO}:${LATEST_IMAGE_TAG}" \
            --output "type=docker,dest=$output_dir/spur-context-worker-image.tar" \
            "$worker_context"

        log "building self-contained worker Lambda image tar: $output_dir/spur-context-worker-lambda-image.tar"
        docker buildx build \
            --platform linux/arm64 --provenance=false \
            --tag "$WORKER_LAMBDA_IMAGE_URI" \
            --tag "${WORKER_LAMBDA_ECR_REPO}:${LATEST_IMAGE_TAG}" \
            --output "type=docker,dest=$output_dir/spur-context-worker-lambda-image.tar" \
            "$worker_lambda_context"

        log "building self-contained source fetcher Lambda image tar: $output_dir/spur-context-source-fetcher-image.tar"
        docker buildx build \
            --platform linux/arm64 --provenance=false \
            --tag "$SOURCE_FETCHER_LAMBDA_IMAGE_URI" \
            --tag "${SOURCE_FETCHER_LAMBDA_ECR_REPO}:${LATEST_IMAGE_TAG}" \
            --output "type=docker,dest=$output_dir/spur-context-source-fetcher-image.tar" \
            "$source_fetcher_context"
    fi
}

build_and_push_worker_image() {
    log "building Docker image for spur-context-worker (remote Docker build on VM)..."

    local full_tag
    full_tag="$(ecr_image_tag "$WORKER_ECR_REPO")"
    local worker_context="$BUILD_DIR/worker-image-context"
    local worker_dockerfile="$worker_context/Dockerfile"
    mkdir -p "$worker_context"
    copy_worker_extensions "$worker_context"
    write_worker_image_dockerfile "$worker_dockerfile"

    # Build the Docker image entirely on the remote VM and push to ECR — no
    # local Docker needed. The VM has Docker installed via startup-aws.sh and
    # ECR push permissions via the spur-ecr-push IAM policy.
    # The --remote-binary flags point at binaries already built on the VM: the
    # standalone worker and the workspace spur CLI used by `spur graph build`.
    cd "$REPO_ROOT"
    scripts/cloud-build/docker-build.sh \
        --remote-binary "$(remote_target_path release/spur-context-worker)" \
        --remote-binary "$(remote_target_path target/release/spur)" \
        --context-dir "$worker_context" \
        --dockerfile Dockerfile \
        --tag "$full_tag"

    smoke_worker_image "$full_tag"
    tag_ecr_image_as_latest "$WORKER_ECR_REPO" "$IMAGE_TAG"

    log "worker image pushed: $full_tag"
    WORKER_IMAGE_URI="$full_tag"
}

build_and_push_worker_lambda_image() {
    log "building Docker image for spur-context-worker Lambda (remote Docker build on VM)..."

    local full_tag
    full_tag="$(ecr_image_tag "$WORKER_LAMBDA_ECR_REPO")"
    local worker_lambda_context="$BUILD_DIR/worker-lambda-image-context"
    local worker_lambda_dockerfile="$worker_lambda_context/Dockerfile"
    mkdir -p "$worker_lambda_context"
    copy_worker_extensions "$worker_lambda_context"
    write_worker_lambda_image_dockerfile "$worker_lambda_dockerfile"

    cd "$REPO_ROOT"
    scripts/cloud-build/docker-build.sh \
        --remote-binary "$(remote_target_path release/spur-context-worker-lambda)" \
        --remote-binary "$(remote_target_path target/release/spur)" \
        --context-dir "$worker_lambda_context" \
        --dockerfile Dockerfile \
        --tag "$full_tag"

    tag_ecr_image_as_latest "$WORKER_LAMBDA_ECR_REPO" "$IMAGE_TAG"

    log "worker Lambda image pushed: $full_tag"
    WORKER_LAMBDA_IMAGE_URI="$full_tag"
}

build_and_push_source_fetcher_lambda_image() {
    log "building Docker image for spur-context-source-fetcher Lambda (remote Docker build on VM)..."

    local full_tag
    full_tag="$(ecr_image_tag "$SOURCE_FETCHER_LAMBDA_ECR_REPO")"
    local source_fetcher_context="$BUILD_DIR/source-fetcher-image-context"
    local source_fetcher_dockerfile="$source_fetcher_context/Dockerfile"
    mkdir -p "$source_fetcher_context"
    write_source_fetcher_lambda_image_dockerfile "$source_fetcher_dockerfile"

    cd "$REPO_ROOT"
    scripts/cloud-build/docker-build.sh \
        --remote-binary "$(remote_target_path target/release/spur-context-fetcher-lambda)" \
        --context-dir "$source_fetcher_context" \
        --dockerfile Dockerfile \
        --tag "$full_tag"

    smoke_source_fetcher_image "$full_tag"
    tag_ecr_image_as_latest "$SOURCE_FETCHER_LAMBDA_ECR_REPO" "$IMAGE_TAG"

    log "source fetcher Lambda image pushed: $full_tag"
    SOURCE_FETCHER_LAMBDA_IMAGE_URI="$full_tag"
}

smoke_worker_image() {
    local full_tag="$1"
    local cloud_dir="$REPO_ROOT/scripts/cloud-build"
    local smoke_command
    smoke_command=$(cat <<EOF
docker run --rm "$full_tag" /usr/local/bin/spur --version
docker run --rm "$full_tag" /usr/local/bin/spur-context-worker || true
EOF
)

    log "running worker image smoke checks on remote VM..."
    (
        SCRIPT_DIR="$cloud_dir"
        # shellcheck disable=SC1091
        source "$SCRIPT_DIR/config.env"
        # shellcheck disable=SC1090
        source "$SCRIPT_DIR/provider-${SPUR_CLOUD}.sh"
        provider_choose_transport
        provider_remote_ssh --command="$smoke_command"
    )
}

smoke_source_fetcher_image() {
    local full_tag="$1"
    local cloud_dir="$REPO_ROOT/scripts/cloud-build"
    local smoke_command
    smoke_command=$(cat <<EOF
docker run --rm "$full_tag" /usr/local/bin/spur-context-fetcher-lambda --smoke
EOF
)

    log "running source fetcher image smoke check on remote VM..."
    (
        SCRIPT_DIR="$cloud_dir"
        # shellcheck disable=SC1091
        source "$SCRIPT_DIR/config.env"
        # shellcheck disable=SC1090
        source "$SCRIPT_DIR/provider-${SPUR_CLOUD}.sh"
        provider_choose_transport
        provider_remote_ssh --command="$smoke_command"
    )
}

main() {
    local code_zip_path="$REPO_ROOT/target/lambda/spur-context-code-lambda.zip"
    local knowledge_zip_path="$REPO_ROOT/target/lambda/spur-context-knowledge-lambda.zip"
    local tf_code_zip_path="../../target/lambda/spur-context-code-lambda.zip"
    local tf_knowledge_zip_path="../../target/lambda/spur-context-knowledge-lambda.zip"
    local local_code_zip=""
    local local_knowledge_zip=""
    local skip_worker=false
    local worker_image_only=false
    local package_only=false
    local BUILD_MODE="${SPUR_CONTEXT_SERVICE_BUILD_MODE:-remote}"
    local PUSH_IMAGES
    local environment="staging"
    local backend_config=""
    local var_file=""
    local worker_image_uri=""
    local worker_lambda_image_uri=""
    local source_fetcher_lambda_image_uri=""

    PUSH_IMAGES="$(normalize_push_images)"

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --local-code-zip) local_code_zip="$2"; shift 2 ;;
            --local-knowledge-zip|--local-zip) local_knowledge_zip="$2"; shift 2 ;;
            --skip-worker) skip_worker=true; shift ;;
            --worker-image-only) worker_image_only=true; shift ;;
            --package-only) package_only=true; shift ;;
            --build-mode) BUILD_MODE="$2"; shift 2 ;;
            --no-push) PUSH_IMAGES=false; shift ;;
            --push) PUSH_IMAGES=true; shift ;;
            --env) environment="$2"; shift 2 ;;
            --backend-config|-backend-config) backend_config="$2"; shift 2 ;;
            --var-file|-var-file) var_file="$2"; shift 2 ;;
            *) break ;;
        esac
    done

    validate_build_mode

    backend_config="${backend_config:-backends/${environment}.s3.tfbackend}"
    var_file="${var_file:-env/${environment}.tfvars}"

    if [[ "$worker_image_only" == "true" && "$package_only" == "true" ]]; then
        log "--worker-image-only and --package-only are mutually exclusive"
        exit 2
    fi

    if [[ -n "$local_code_zip" || -n "$local_knowledge_zip" ]]; then
        if [[ -z "$local_code_zip" || -z "$local_knowledge_zip" ]]; then
            log "--local-code-zip and --local-knowledge-zip must be provided together"
            exit 2
        fi
        if [[ "$local_code_zip" == "$local_knowledge_zip" ]]; then
            log "Code and Knowledge Lambda zip paths must be distinct"
            exit 2
        fi
    fi

    if [[ "$BUILD_MODE" == "remote" && "$PUSH_IMAGES" != "true" && "$skip_worker" == "false" ]]; then
        log "--no-push requires --build-mode self-contained for worker image builds"
        exit 2
    fi

    if [[ "$PUSH_IMAGES" == "true" && "$skip_worker" == "false" ]] \
        || [[ "$worker_image_only" == "false" && "$package_only" == "false" ]]; then
        assert_ecr_exclusion_filter_support
        assert_selected_aws_region
    fi

    # Build + push worker container image (unless --skip-worker).
    if [[ "$skip_worker" == "false" ]]; then
        if [[ "$PUSH_IMAGES" == "true" ]]; then
            ensure_ecr_repositories
        fi
        download_extensions
        build_spur_cli
        build_worker
        build_worker_lambda
        build_source_fetcher_lambda
        if [[ "$BUILD_MODE" == "self-contained" ]]; then
            build_local_worker_images
        else
            build_and_push_worker_image
            build_and_push_worker_lambda_image
            build_and_push_source_fetcher_lambda_image
        fi
        worker_image_uri="$WORKER_IMAGE_URI"
        worker_lambda_image_uri="$WORKER_LAMBDA_IMAGE_URI"
        source_fetcher_lambda_image_uri="$SOURCE_FETCHER_LAMBDA_IMAGE_URI"
        log "worker ECS image URI: $worker_image_uri"
        log "worker Lambda image URI: $worker_lambda_image_uri"
        log "source fetcher Lambda image URI: $source_fetcher_lambda_image_uri"
    fi

    if [[ "$worker_image_only" == "true" ]]; then
        if [[ -z "$worker_image_uri" || -z "$worker_lambda_image_uri" || -z "$source_fetcher_lambda_image_uri" ]]; then
            log "--worker-image-only requires worker image builds; do not combine it with --skip-worker"
            exit 2
        fi
        echo "$worker_image_uri"
        exit 0
    fi

    # Build or reuse the two serving Lambda zips.
    if [[ -n "$local_code_zip" ]]; then
        code_zip_path="$local_code_zip"
        knowledge_zip_path="$local_knowledge_zip"
        tf_code_zip_path="$local_code_zip"
        tf_knowledge_zip_path="$local_knowledge_zip"
    else
        mkdir -p "$(dirname "$code_zip_path")"
        rm -f "$code_zip_path" "$knowledge_zip_path"
        download_extensions
        build_serving_binaries
        copy_knowledge_extensions
        package_code_zip "$code_zip_path"
        package_knowledge_zip "$knowledge_zip_path"
    fi

    if [[ "$package_only" == "true" ]]; then
        log "package-only requested; skipping terraform"
        printf '%s\n%s\n' "$code_zip_path" "$knowledge_zip_path"
        exit 0
    fi

    log "running terraform..."
    cd "$INFRA_DIR"
    if [[ ! -f "$backend_config" ]]; then
        log "backend config not found: $backend_config"
        exit 2
    fi
    if [[ ! -f "$var_file" ]]; then
        log "variable file not found: $var_file"
        exit 2
    fi

    terraform init -upgrade -backend-config="$backend_config"

    local tf_vars=(
        -var-file="$var_file"
        -var "lambda_zip_path=$tf_knowledge_zip_path"
        -var "code_lambda_zip_path=$tf_code_zip_path"
        -var "knowledge_lambda_zip_path=$tf_knowledge_zip_path"
    )
    if [[ -n "$worker_image_uri" ]]; then
        tf_vars+=(-var "worker_ecr_image=$worker_image_uri")
    else
        worker_image_uri="$(terraform output -raw worker_image_uri 2>/dev/null || true)"
        if [[ -n "$worker_image_uri" ]]; then
            tf_vars+=(-var "worker_ecr_image=$worker_image_uri")
        fi
    fi
    if [[ -n "$worker_lambda_image_uri" ]]; then
        tf_vars+=(-var "worker_lambda_image=$worker_lambda_image_uri")
    else
        worker_lambda_image_uri="$(terraform output -raw worker_lambda_image_uri 2>/dev/null || true)"
        if [[ -n "$worker_lambda_image_uri" ]]; then
            tf_vars+=(-var "worker_lambda_image=$worker_lambda_image_uri")
        fi
    fi
    if [[ -n "$source_fetcher_lambda_image_uri" ]]; then
        tf_vars+=(-var "source_fetcher_lambda_image=$source_fetcher_lambda_image_uri")
    else
        source_fetcher_lambda_image_uri="$(terraform output -raw source_fetcher_lambda_image_uri 2>/dev/null || true)"
        if [[ -n "$source_fetcher_lambda_image_uri" ]]; then
            tf_vars+=(-var "source_fetcher_lambda_image=$source_fetcher_lambda_image_uri")
        fi
    fi

    terraform plan "${tf_vars[@]}"
    terraform apply "${tf_vars[@]}" -auto-approve

    log "deployed. API URL:"
    terraform output -raw api_url
    echo ""
}

main "$@"
