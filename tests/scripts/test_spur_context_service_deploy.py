import json
import os
import re
import subprocess
import zipfile
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[2]
DEPLOY_SH = ROOT / "infra" / "spur-context-service" / "deploy.sh"
REMOTE_BUILD_SH = ROOT / "infra" / "spur-context-service" / "build-and-push-remote.sh"
INFRA_DIR = ROOT / "infra" / "spur-context-service"
VERSIONS_TF = INFRA_DIR / "versions.tf"
GRAVITON_BASELINE_GUARD = INFRA_DIR / "test-graviton2-baseline.sh"
CONTEXT_SERVICE_WORKFLOW = ROOT / ".github" / "workflows" / "context-service.yml"
STAGING_SMOKE = INFRA_DIR / "smoke-staging-e2e.py"
STAGING_SMOKE_ENTRYPOINT = INFRA_DIR / "smoke-staging-e2e.sh"
REMOVED_ROUTE_ADDRESS = re.compile(
    r"\baws_apigatewayv2_route\.(?:default|oauth)(?![A-Za-z0-9_])"
)
REMOVED_ROUTE_RESOURCE = re.compile(
    r'^\s*resource\s+"aws_apigatewayv2_route"\s+"(?:default|oauth)"'
)


def terraform_resource_block(text, resource_type, resource_name):
    marker = f'resource "{resource_type}" "{resource_name}"'
    assert marker in text, f"missing Terraform resource: {marker}"
    start = text.index(marker)
    brace = text.index("{", start)
    depth = 0
    for offset, character in enumerate(text[brace:], start=brace):
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return text[start : offset + 1]
    raise AssertionError(f"unterminated Terraform resource: {marker}")


def terraform_resource_blocks(text, resource_type):
    return {
        resource_name: terraform_resource_block(text, resource_type, resource_name)
        for resource_name in re.findall(
            rf'resource\s+"{re.escape(resource_type)}"\s+"([^"]+)"', text
        )
    }


def terraform_output_block(text, output_name):
    marker = f'output "{output_name}"'
    start = text.index(marker)
    brace = text.index("{", start)
    depth = 0
    for offset, character in enumerate(text[brace:], start=brace):
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return text[start : offset + 1]
    raise AssertionError(f"unterminated Terraform output: {marker}")


def terraform_assignment(block, name):
    match = re.search(rf"^\s*{re.escape(name)}\s*=\s*(.+)$", block, re.MULTILINE)
    return match.group(1).strip() if match else None


def terraform_module_contract_files():
    return sorted(
        path
        for pattern in ("*.tf", "*.tftest.hcl", "*.md")
        for path in INFRA_DIR.rglob(pattern)
        if ".terraform" not in path.parts
    )


def shell_function(source, name):
    match = re.search(
        rf"(?ms)^{re.escape(name)}\(\) \{{\n(.*?)(?=^[a-zA-Z_][a-zA-Z0-9_]*\(\) \{{|\Z)",
        source,
    )
    assert match is not None, f"missing shell function: {name}"
    return match.group(1)


def write_executable(path, source):
    path.write_text(source)
    path.chmod(0o755)


def test_main_module_route_contracts_drop_removed_addresses_and_unsuffixed_paths():
    contract_files = sorted(
        {
            *INFRA_DIR.glob("*.tf"),
            *INFRA_DIR.glob("tests/*.tftest.hcl"),
            INFRA_DIR / "env" / "default.tfvars",
            INFRA_DIR / "README.md",
        }
    )
    assert all("poc" not in path.relative_to(INFRA_DIR).parts for path in contract_files)

    unsuffixed_serving_route = re.compile(
        r'^\s*route_key\s*=\s*"POST /mcp/(?:oauth|api-key)"\s*$'
    )
    valid_default_stage_name = re.compile(r'^\s*name\s*=\s*"\$default"\s*$')
    stale_contracts = []

    for path in contract_files:
        for line_number, line in enumerate(path.read_text().splitlines(), start=1):
            reasons = []
            if REMOVED_ROUTE_RESOURCE.search(line):
                reasons.append("removed route resource")
            if unsuffixed_serving_route.search(line):
                reasons.append("unsuffixed serving route")
            if "$default" in line and not (
                path == INFRA_DIR / "main.tf" and valid_default_stage_name.fullmatch(line)
            ):
                reasons.append("removed catch-all route contract")
            if reasons:
                stale_contracts.append(
                    f"{path.relative_to(ROOT)}:{line_number}: "
                    f"{', '.join(reasons)}: {line.strip()}"
                )

    assert not stale_contracts, "\n".join(stale_contracts)


def test_second_route_correction_restores_direct_and_exact_output_contracts():
    api_key_static = (INFRA_DIR / "tests" / "api_key_static.tftest.hcl").read_text()
    cognito_static = (INFRA_DIR / "tests" / "cognito_static.tftest.hcl").read_text()
    outputs_tf = (INFRA_DIR / "outputs.tf").read_text()
    readme = (INFRA_DIR / "README.md").read_text()
    problems = []

    for address in (
        "aws_apigatewayv2_route.api_key_mcp",
        "aws_apigatewayv2_route.oauth_code",
        "aws_apigatewayv2_route.oauth_knowledge",
        "aws_apigatewayv2_route.api_key_mcp_knowledge",
    ):
        if REMOVED_ROUTE_ADDRESS.search(address):
            problems.append(f"stale-address guard rejects valid route {address}")
    for address in (
        "aws_apigatewayv2_route.default",
        "aws_apigatewayv2_route.oauth",
    ):
        if not REMOVED_ROUTE_ADDRESS.search(address):
            problems.append(f"stale-address guard allows removed route {address}")

    direct_api_key_assertions = (
        "length(aws_apigatewayv2_route.api_key_mcp) == 0",
        'aws_apigatewayv2_route.api_key_mcp[0].route_key == "POST /mcp/api-key/code"',
        'aws_apigatewayv2_route.api_key_mcp[0].authorization_type == "CUSTOM"',
        "aws_apigatewayv2_route.api_key_mcp[0].authorizer_id == aws_apigatewayv2_authorizer.api_key[0].id",
        'aws_apigatewayv2_route.api_key_mcp[0].target == "integrations/${aws_apigatewayv2_integration.api_key[0].id}"',
    )
    for assertion in direct_api_key_assertions:
        if assertion not in api_key_static:
            problems.append(f"API-key Terraform test lacks direct assertion: {assertion}")
    if 'strcontains(file("${path.module}/api_keys.tf")' in api_key_static:
        problems.append("API-key Terraform test still uses source-text route assertions")

    output_contracts = {
        "oauth_api_urls": (
            "var.cognito_auth_enabled",
            "/mcp/oauth/code",
            "/mcp/oauth/knowledge",
        ),
        "api_key_mcp_urls": (
            "var.api_key_auth_enabled",
            "/mcp/api-key/code",
            "/mcp/api-key/knowledge",
        ),
    }
    for output_name, required_fragments in output_contracts.items():
        try:
            block = terraform_output_block(outputs_tf, output_name)
        except ValueError:
            problems.append(f'outputs.tf lacks output "{output_name}"')
            continue
        for fragment in required_fragments:
            if fragment not in block:
                problems.append(f"{output_name} lacks {fragment}")
        if ": null" not in block:
            problems.append(f"{output_name} does not become null when disabled")

    for output_name in ("oauth_api_url", "api_key_mcp_url"):
        block = terraform_output_block(outputs_tf, output_name).lower()
        if "deprecated" not in block or "not directly callable" not in block:
            problems.append(
                f"{output_name} is not described as a deprecated, non-callable prefix"
            )

    terraform_output_assertions = (
        (cognito_static, "oauth_api_url", 2),
        (cognito_static, "oauth_api_urls", 2),
        (api_key_static, "api_key_mcp_url", 2),
        (api_key_static, "api_key_mcp_urls", 2),
    )
    for test_source, output_name, minimum in terraform_output_assertions:
        count = len(re.findall(rf"\boutput\.{output_name}\b", test_source))
        if count < minimum:
            problems.append(
                f"Terraform tests assert output.{output_name} {count} time(s), "
                f"need execute-api and custom-domain coverage"
            )

    for output_name in ("oauth_api_urls", "api_key_mcp_urls"):
        if output_name not in readme:
            problems.append(f"README does not direct operators to {output_name}")
    for output_name in ("oauth_api_url", "api_key_mcp_url"):
        if not re.search(
            rf"(?is)(?:deprecated.{{0,200}}{output_name}|{output_name}.{{0,200}}deprecated)",
            readme,
        ):
            problems.append(f"README does not mark {output_name} deprecated")

    assert not problems, "\n".join(problems)


def test_direct_routes_have_expected_auth_and_unique_terminal_integrations():
    main_tf = (INFRA_DIR / "main.tf").read_text()
    api_keys_tf = (INFRA_DIR / "api_keys.tf").read_text()
    iam_tf = (INFRA_DIR / "iam.tf").read_text()
    problems = []

    integration_blocks = {
        **terraform_resource_blocks(main_tf, "aws_apigatewayv2_integration"),
        **terraform_resource_blocks(api_keys_tf, "aws_apigatewayv2_integration"),
    }
    expected_integrations = {
        "code": ("code", "aws_lambda_function.code.invoke_arn"),
        "lambda": ("knowledge", "aws_lambda_alias.knowledge_live.invoke_arn"),
        "api_key": ("code", "aws_lambda_function.code.invoke_arn"),
        "api_key_knowledge": (
            "knowledge",
            "aws_lambda_alias.knowledge_live.invoke_arn",
        ),
    }
    if set(integration_blocks) != set(expected_integrations):
        problems.append(
            "serving integrations differ: "
            f"expected {sorted(expected_integrations)}, got {sorted(integration_blocks)}"
        )

    integration_backends = {}
    for integration_name, (backend, expected_uri) in expected_integrations.items():
        block = integration_blocks.get(integration_name)
        if block is None:
            continue
        terminal_uris = re.findall(
            r"integration_uri\s*=\s*"
            r"(aws_lambda_(?:function|alias)\.[A-Za-z0-9_]+(?:\[0\])?\.invoke_arn)",
            block,
        )
        if terminal_uris != [expected_uri]:
            problems.append(
                f"{integration_name} must have one terminal URI {expected_uri}; "
                f"got {terminal_uris}"
            )
        integration_backends[integration_name] = backend

    for integration_name in ("api_key", "api_key_knowledge"):
        block = integration_blocks.get(integration_name)
        if block is not None and (
            '"remove:header.X-SPUR-API-Key" = "\'\'"' not in block
        ):
            problems.append(
                f"{integration_name} does not remove X-SPUR-API-Key before serving"
            )

    expected_routes = {
        "POST /mcp/code": ("code", "var.api_authorization_type", None, None),
        "POST /mcp/knowledge": (
            "knowledge",
            "var.api_authorization_type",
            None,
            None,
        ),
        "POST /mcp/oauth/code": (
            "code",
            '"JWT"',
            "aws_apigatewayv2_authorizer.cognito[0].id",
            "local.cognito_custom_scopes",
        ),
        "POST /mcp/oauth/knowledge": (
            "knowledge",
            '"JWT"',
            "aws_apigatewayv2_authorizer.cognito[0].id",
            "local.cognito_custom_scopes",
        ),
        "POST /mcp/api-key/code": (
            "code",
            '"CUSTOM"',
            "aws_apigatewayv2_authorizer.api_key[0].id",
            None,
        ),
        "POST /mcp/api-key/knowledge": (
            "knowledge",
            '"CUSTOM"',
            "aws_apigatewayv2_authorizer.api_key[0].id",
            None,
        ),
        "GET /.well-known/spur-context-service": ("code", '"NONE"', None, None),
        "GET /auth/login": ("code", '"NONE"', None, None),
        "POST /auth/api-keys": (
            "code",
            '"JWT"',
            "aws_apigatewayv2_authorizer.cognito[0].id",
            "[local.api_key_management_scope]",
        ),
        "GET /auth/api-keys": (
            "code",
            '"JWT"',
            "aws_apigatewayv2_authorizer.cognito[0].id",
            "[local.api_key_management_scope]",
        ),
        "DELETE /auth/api-keys/{key_id}": (
            "code",
            '"JWT"',
            "aws_apigatewayv2_authorizer.cognito[0].id",
            "[local.api_key_management_scope]",
        ),
    }

    route_entries = []
    route_blocks = {
        **terraform_resource_blocks(main_tf, "aws_apigatewayv2_route"),
        **terraform_resource_blocks(api_keys_tf, "aws_apigatewayv2_route"),
    }
    for resource_name, block in route_blocks.items():
        route_key_assignment = terraform_assignment(block, "route_key")
        if route_key_assignment == "each.value":
            for_each = re.search(r"for_each\s*=.*?toset\(\[(.*?)\]\)", block, re.DOTALL)
            route_keys = (
                re.findall(r'"((?:GET|POST|DELETE) [^"]+)"', for_each.group(1))
                if for_each
                else []
            )
        elif route_key_assignment and route_key_assignment.startswith('"'):
            route_keys = [route_key_assignment.strip('"')]
        else:
            route_keys = []

        target = terraform_assignment(block, "target") or ""
        target_integrations = re.findall(
            r"aws_apigatewayv2_integration\.([A-Za-z0-9_]+)"
            r"(?:\[0\])?\.id",
            target,
        )
        if len(target_integrations) != 1:
            problems.append(
                f"route resource {resource_name} must name one integration; "
                f"got {target_integrations}"
            )
            integration_name = None
        else:
            integration_name = target_integrations[0]

        for route_key in route_keys:
            route_entries.append(
                (
                    route_key,
                    integration_name,
                    terraform_assignment(block, "authorization_type"),
                    terraform_assignment(block, "authorizer_id"),
                    terraform_assignment(block, "authorization_scopes"),
                )
            )

    route_keys = [entry[0] for entry in route_entries]
    duplicate_route_keys = sorted(
        route_key for route_key in set(route_keys) if route_keys.count(route_key) != 1
    )
    if duplicate_route_keys:
        problems.append(f"duplicate route keys: {duplicate_route_keys}")
    if set(route_keys) != set(expected_routes):
        problems.append(
            "route table differs: "
            f"missing {sorted(set(expected_routes) - set(route_keys))}; "
            f"unexpected {sorted(set(route_keys) - set(expected_routes))}"
        )

    for route_key, integration_name, auth_type, authorizer_id, scopes in route_entries:
        expected = expected_routes.get(route_key)
        if expected is None:
            continue
        expected_backend, expected_auth, expected_authorizer, expected_scopes = expected
        actual_backend = integration_backends.get(integration_name)
        actual = (actual_backend, auth_type, authorizer_id, scopes)
        if actual != expected:
            problems.append(f"{route_key}: expected {expected}, got {actual}")

    serving_role_policies = [
        block
        for block in terraform_resource_blocks(
            iam_tf, "aws_iam_role_policy"
        ).values()
        if re.search(
            r"role\s*=\s*aws_iam_role\.(?:code|knowledge)_lambda\.id", block
        )
    ]
    if any("lambda:InvokeFunction" in block for block in serving_role_policies):
        problems.append("a serving Lambda role can invoke another Lambda")

    knowledge_permission = terraform_resource_block(
        main_tf, "aws_lambda_permission", "apigw_knowledge"
    )
    code_permission = terraform_resource_block(
        main_tf, "aws_lambda_permission", "apigw_code"
    )
    if "function_name = aws_lambda_function.knowledge.function_name" not in knowledge_permission:
        problems.append("Knowledge API Gateway permission changed function")
    if "qualifier     = aws_lambda_alias.knowledge_live.name" not in knowledge_permission:
        problems.append("Knowledge API Gateway permission lost alias qualifier")
    if "function_name = aws_lambda_function.code.function_name" not in code_permission:
        problems.append("Code API Gateway permission changed function")
    if any(
        'principal     = "apigateway.amazonaws.com"' not in permission
        for permission in (knowledge_permission, code_permission)
    ):
        problems.append("serving Lambda permission principal changed")

    assert not problems, "\n".join(problems)


def test_serving_compute_correction_retains_legacy_service_without_routing_to_it():
    main_tf = (INFRA_DIR / "main.tf").read_text()
    api_keys_tf = (INFRA_DIR / "api_keys.tf").read_text()
    compatibility_path = INFRA_DIR / "legacy_compatibility.tf"
    assert compatibility_path.exists(), "missing legacy compatibility Terraform"
    compatibility_tf = compatibility_path.read_text()
    problems = []

    legacy_service_marker = 'resource "aws_lambda_function" "service"'
    if legacy_service_marker not in compatibility_tf:
        problems.append("missing retained legacy serving resource")
    else:
        legacy_service = terraform_resource_block(
            compatibility_tf, "aws_lambda_function", "service"
        )
        if "prevent_destroy = true" not in legacy_service:
            problems.append("legacy serving resource is not protected from destroy")
        if "ignore_changes  = all" not in legacy_service:
            problems.append("legacy serving resource is not frozen for compatibility")

    code_integration_marker = 'resource "aws_apigatewayv2_integration" "code"'
    if code_integration_marker not in main_tf:
        problems.append("missing stable Code compatibility integration")
    else:
        code_integration = terraform_resource_block(
            main_tf, "aws_apigatewayv2_integration", "code"
        )
        if "integration_uri        = aws_lambda_function.code.invoke_arn" not in code_integration:
            problems.append("Code compatibility integration does not invoke Code")

    for text, resource_name in (
        (api_keys_tf, "api_key_discovery"),
        (api_keys_tf, "api_key_management"),
        (main_tf, "login_redirect"),
    ):
        route = terraform_resource_block(text, "aws_apigatewayv2_route", resource_name)
        if "aws_apigatewayv2_integration.code.id" not in route:
            problems.append(f"{resource_name} does not target the Code integration")

    api_key_integration = terraform_resource_block(
        api_keys_tf, "aws_apigatewayv2_integration", "api_key"
    )
    if "integration_uri        = aws_lambda_function.code.invoke_arn" not in api_key_integration:
        problems.append("API-key integration does not invoke Code")

    if "aws_lambda_function.service" in main_tf + api_keys_tf:
        problems.append("active split routes or integrations still target legacy serving")

    code_permission_marker = 'resource "aws_lambda_permission" "apigw_code"'
    if code_permission_marker not in main_tf:
        problems.append("missing API Gateway permission for the Code integration")
    else:
        code_permission = terraform_resource_block(
            main_tf, "aws_lambda_permission", "apigw_code"
        )
        if "function_name = aws_lambda_function.code.function_name" not in code_permission:
            problems.append("Code API permission names a different function")

    assert not problems, "\n".join(problems)


def test_serving_compute_correction_attaches_knowledge_warm_pool_to_alias_traffic():
    main_tf = (INFRA_DIR / "main.tf").read_text()
    knowledge = terraform_resource_block(main_tf, "aws_lambda_function", "knowledge")
    warm = terraform_resource_block(
        main_tf,
        "aws_lambda_provisioned_concurrency_config",
        "knowledge_warm",
    )
    integration = terraform_resource_block(
        main_tf, "aws_apigatewayv2_integration", "lambda"
    )
    permission = terraform_resource_block(
        main_tf, "aws_lambda_permission", "apigw_knowledge"
    )
    problems = []

    if not re.search(r"\bpublish\s*=\s*true\b", knowledge):
        problems.append("Knowledge does not publish immutable versions")

    alias_marker = 'resource "aws_lambda_alias" "knowledge_live"'
    if alias_marker not in main_tf:
        problems.append("missing stable Knowledge alias")
    else:
        alias = terraform_resource_block(main_tf, "aws_lambda_alias", "knowledge_live")
        if "function_name    = aws_lambda_function.knowledge.function_name" not in alias:
            problems.append("Knowledge alias does not name the Knowledge function")
        if "function_version = aws_lambda_function.knowledge.version" not in alias:
            problems.append("Knowledge alias does not track the published version")

    if "qualifier                         = aws_lambda_alias.knowledge_live.name" not in warm:
        problems.append("Knowledge provisioned concurrency does not qualify the alias")
    if 'qualifier                         = "$LATEST"' in warm:
        problems.append("Knowledge provisioned concurrency still uses $LATEST")
    if "integration_uri        = aws_lambda_alias.knowledge_live.invoke_arn" not in integration:
        problems.append("Knowledge API integration does not invoke the alias")
    if "function_name = aws_lambda_function.knowledge.function_name" not in permission:
        problems.append("Knowledge API permission names a different function")
    if "qualifier     = aws_lambda_alias.knowledge_live.name" not in permission:
        problems.append("Knowledge API permission does not match the alias qualifier")

    assert not problems, "\n".join(problems)


def test_serving_compute_correction_removes_unused_eni_permissions_without_vpc_config():
    main_tf = (INFRA_DIR / "main.tf").read_text()
    iam_tf = (INFRA_DIR / "iam.tf").read_text()
    problems = []

    for function_name in ("code", "knowledge"):
        function = terraform_resource_block(
            main_tf, "aws_lambda_function", function_name
        )
        runtime = terraform_resource_block(
            iam_tf, "aws_iam_role_policy", f"{function_name}_lambda_runtime"
        )
        if "vpc_config" in function:
            problems.append(f"{function_name} unexpectedly has vpc_config")
        eni_actions = sorted(set(re.findall(r'"(ec2:[^"]+)"', runtime)))
        if eni_actions:
            problems.append(
                f"{function_name} has unused EC2 ENI permissions: {', '.join(eni_actions)}"
            )

    assert not problems, "\n".join(problems)


def test_serving_compute_correction_removes_unused_knowledge_catalog_secret_contract():
    main_tf = (INFRA_DIR / "main.tf").read_text()
    iam_tf = (INFRA_DIR / "iam.tf").read_text()
    knowledge = terraform_resource_block(main_tf, "aws_lambda_function", "knowledge")
    problems = []

    if 'resource "aws_iam_role_policy" "knowledge_catalog_secret"' in iam_tf:
        problems.append("unused knowledge_catalog_secret IAM policy remains")
    if "aws_iam_role_policy.knowledge_catalog_secret" in knowledge:
        problems.append("Knowledge still depends on knowledge_catalog_secret")
    if "SPUR_CATALOG_SECRET_ARN" in knowledge:
        problems.append("Knowledge unexpectedly declares SPUR_CATALOG_SECRET_ARN")
    if "secretsmanager:" in knowledge:
        problems.append("Knowledge environment unexpectedly embeds secret access")

    assert not problems, "\n".join(problems)


def test_serving_compute_defines_exactly_code_and_knowledge_with_isolated_envs():
    main_tf = (INFRA_DIR / "main.tf").read_text()
    variables_tf = (INFRA_DIR / "variables.tf").read_text()

    serving_resources = set(
        re.findall(r'resource\s+"aws_lambda_function"\s+"([^"]+)"', main_tf)
    )
    assert serving_resources == {"code", "knowledge"}

    code = terraform_resource_block(main_tf, "aws_lambda_function", "code")
    knowledge = terraform_resource_block(
        main_tf, "aws_lambda_function", "knowledge"
    )
    assert 'function_name = "spur-context-code"' in code
    assert "role        = aws_iam_role.code_lambda.arn" in code
    assert 'function_name = "spur-context-knowledge"' in knowledge
    assert "role        = aws_iam_role.knowledge_lambda.arn" in knowledge

    assert "SPUR_CONTEXT_CODE_CACHE_BYTES" in code
    assert (
        "tostring(local.code_lambda_ephemeral_storage_bytes)" in code
    )
    assert "size = var.code_lambda_ephemeral_storage_mb" in code
    assert "SPUR_CONTEXT_DUCKDB_EXTENSION_DIR" not in code
    assert "SPUR_CATALOG_DSN" not in code
    assert "HOME" not in code

    assert "SPUR_CATALOG_S3_URI" in knowledge
    assert "SPUR_CONTEXT_DUCKDB_EXTENSION_DIR" in knowledge
    assert re.search(r'\bHOME\s*=\s*"/tmp"', knowledge)
    for code_only_env in (
        "SPUR_CONTEXT_CODE_CACHE_BYTES",
        "SPUR_INDEX_STATE_MACHINE_ARN",
        "SPUR_INDEX_JOBS_TABLE",
        "SPUR_CONTEXT_API_KEYS_TABLE",
    ):
        assert code_only_env not in knowledge

    ephemeral_variable = variables_tf.split(
        'variable "code_lambda_ephemeral_storage_mb"', 1
    )[1].split("\n}", 1)[0]
    assert "default     = 512" in ephemeral_variable
    assert "var.code_lambda_ephemeral_storage_mb == 512" in ephemeral_variable
    assert (
        "code_lambda_ephemeral_storage_bytes = "
        "var.code_lambda_ephemeral_storage_mb * 1024 * 1024"
    ) in main_tf


def test_serving_compute_roles_are_least_privilege_and_backend_specific():
    iam_tf = (INFRA_DIR / "iam.tf").read_text()

    assert 'resource "aws_iam_role" "code_lambda"' in iam_tf
    assert 'resource "aws_iam_role" "knowledge_lambda"' in iam_tf

    code_s3 = terraform_resource_block(
        iam_tf, "aws_iam_role_policy", "code_s3_read"
    )
    assert "role = aws_iam_role.code_lambda.id" in code_s3
    assert '"s3:GetObject"' in code_s3
    assert '"${aws_s3_bucket.data.arn}/${local.catalog_s3_key}"' in code_s3
    assert "serving-registry.json" in code_s3
    assert '"${aws_s3_bucket.data.arn}/silver/*"' in code_s3
    for forbidden in (
        "s3:PutObject",
        "s3:DeleteObject",
        "s3:AbortMultipartUpload",
        "secretsmanager:",
    ):
        assert forbidden not in code_s3

    knowledge_s3 = terraform_resource_block(
        iam_tf, "aws_iam_role_policy", "knowledge_s3_read"
    )
    assert "role = aws_iam_role.knowledge_lambda.id" in knowledge_s3
    assert '"s3:GetObject"' in knowledge_s3
    assert '"s3:GetObjectVersion"' in knowledge_s3
    assert '"s3:ListBucket"' in knowledge_s3
    assert "s3:PutObject" not in knowledge_s3
    assert "s3:DeleteObject" not in knowledge_s3

    assert 'resource "aws_iam_role_policy" "knowledge_catalog_secret"' not in iam_tf

    code_role_resources = "\n".join(
        terraform_resource_block(iam_tf, resource_type, resource_name)
        for resource_type, resource_name in re.findall(
            r'resource\s+"(aws_iam_[^"]+)"\s+"([^"]+)"', iam_tf
        )
        if "role = aws_iam_role.code_lambda" in terraform_resource_block(
            iam_tf, resource_type, resource_name
        )
    )
    assert "secretsmanager:" not in code_role_resources
    for required in (
        "dynamodb:",
        "states:",
        "logs:",
        "xray:",
    ):
        assert required in code_role_resources

    api_key_management = terraform_resource_block(
        iam_tf, "aws_iam_role_policy", "code_api_key_management"
    )
    assert "role = aws_iam_role.code_lambda.id" in api_key_management


def test_serving_compute_drainer_and_warm_pool_target_the_owning_backends():
    main_tf = (INFRA_DIR / "main.tf").read_text()
    variables_tf = (INFRA_DIR / "variables.tf").read_text()

    drainer = terraform_resource_block(
        main_tf, "aws_cloudwatch_event_target", "index_queue_drainer"
    )
    assert "arn       = aws_lambda_function.code.arn" in drainer
    assert 'operation = "drain_queued_jobs"' in drainer
    drainer_permission = terraform_resource_block(
        main_tf, "aws_lambda_permission", "eventbridge_code"
    )
    assert (
        "function_name = aws_lambda_function.code.function_name"
        in drainer_permission
    )

    warm_resources = re.findall(
        r'resource\s+"aws_lambda_provisioned_concurrency_config"\s+"([^"]+)"',
        main_tf,
    )
    assert warm_resources == ["knowledge_warm"]
    knowledge_warm = terraform_resource_block(
        main_tf,
        "aws_lambda_provisioned_concurrency_config",
        "knowledge_warm",
    )
    assert re.search(
        r"function_name\s*=\s*aws_lambda_function\.knowledge\.function_name",
        knowledge_warm,
    )
    assert "qualifier                         = aws_lambda_alias.knowledge_live.name" in knowledge_warm
    assert (
        "provisioned_concurrent_executions = var.concurrent_warm_instances"
        in knowledge_warm
    )
    assert "local.code_warm_instances == 0" in knowledge_warm
    assert (
        "local.total_serving_warm_instances == var.concurrent_warm_instances"
        in knowledge_warm
    )

    serving_tf = main_tf + variables_tf
    assert "reserved_concurrent_executions" not in serving_tf
    assert 'variable "serving_reserved_concurrency"' not in serving_tf
    # Reserved concurrency only bounds capacity. Provisioned concurrency is the
    # billable pre-initialized pool, so the existing warm budget stays on
    # Knowledge and Code deliberately has none.
    assert "Reserved concurrency bounds capacity" in main_tf
    assert "provisioned concurrency is the billable warm pool" in main_tf


def test_serving_compute_outputs_and_preconditions_fail_closed():
    main_tf = (INFRA_DIR / "main.tf").read_text()
    outputs_tf = (INFRA_DIR / "outputs.tf").read_text()

    code = terraform_resource_block(main_tf, "aws_lambda_function", "code")
    knowledge = terraform_resource_block(
        main_tf, "aws_lambda_function", "knowledge"
    )
    assert "local.serving_assignment_count == 2" in code
    assert "local.serving_zip_paths_are_consistent" in code
    assert "local.catalog_s3_path_is_consistent" in code
    assert "local.catalog_s3_path_is_consistent" in knowledge

    for output_name, resource_name in (
        ("code_lambda_function_name", "code"),
        ("code_lambda_function_arn", "code"),
        ("knowledge_lambda_function_name", "knowledge"),
        ("knowledge_lambda_function_arn", "knowledge"),
    ):
        output = outputs_tf.split(f'output "{output_name}"', 1)[1].split(
            "\n}", 1
        )[0]
        assert f"aws_lambda_function.{resource_name}." in output

    legacy_name = outputs_tf.split('output "lambda_function_name"', 1)[1].split(
        "\n}", 1
    )[0]
    legacy_arn = outputs_tf.split('output "lambda_function_arn"', 1)[1].split(
        "\n}", 1
    )[0]
    assert "aws_lambda_function.knowledge.function_name" in legacy_name
    assert "aws_lambda_function.knowledge.arn" in legacy_arn


def render_index_build_asl():
    template = (INFRA_DIR / "index_build_asl.json").read_text()
    values = {
        "cluster_arn": "arn:aws:ecs:ap-southeast-5:123456789012:cluster/spur-context",
        "worker_taskdef_arn": (
            "arn:aws:ecs:ap-southeast-5:123456789012:"
            "task-definition/spur-context-worker:1"
        ),
        "worker_lambda_arn": (
            "arn:aws:lambda:ap-southeast-5:123456789012:"
            "function:spur-context-worker:live"
        ),
        "source_fetch_lambda_arn": (
            "arn:aws:lambda:ap-southeast-5:123456789012:"
            "function:spur-context-source-fetcher:live"
        ),
        "worker_lambda_timeout_sec": "900",
        "source_fetcher_lambda_timeout_sec": "900",
        "worker_ecs_timeout_sec": "2700",
        "index_jobs_table_name": "spur-context-index-jobs",
        "catalog_leases_table_name": "spur-context-catalog-leases",
        "catalog_dsn": (
            "postgres:host=writer.example.com port=5432 dbname=spur_context "
            "user=spur_context sslmode=require"
        ),
        "context_ducklake_data_path": "s3://spur-context/gold/data/",
        "worker_checkpoint_uri_template": (
            "s3://spur-context/jobs/{}/checkpoint.json"
        ),
        "subnets_json": json.dumps(["subnet-123"]),
        "security_groups_json": json.dumps(["sg-123"]),
    }

    rendered = re.sub(r"\${([A-Za-z0-9_]+)}", lambda m: values[m.group(1)], template)
    return json.loads(rendered)


def test_deploy_builds_named_serving_lambdas_from_exact_feature_closures():
    script = DEPLOY_SH.read_text()
    cargo_toml = (ROOT / "crates" / "spur-context-service" / "Cargo.toml").read_text()

    code_command = (
        "--workdir crates/spur-context-service build --no-default-features "
        "--features code-lambda --bin spur-context-code-lambda --release"
    )
    knowledge_command = (
        "--workdir crates/spur-context-service build --no-default-features "
        "--features knowledge-lambda --bin spur-context-knowledge-lambda --release"
    )

    assert 'name = "spur-context-code-lambda"' in cargo_toml
    assert 'required-features = ["code-lambda"]' in cargo_toml
    assert 'name = "spur-context-knowledge-lambda"' in cargo_toml
    assert 'required-features = ["knowledge-lambda"]' in cargo_toml
    assert script.count(code_command) == 2
    assert script.count(knowledge_command) == 2
    assert 'run_graviton2_safe_cargo "Code Lambda bootstrap"' in script
    assert 'run_graviton2_safe_cargo "Knowledge Lambda bootstrap"' in script
    assert (
        'fetch_remote_target_file release/spur-context-code-lambda '
        '"$BUILD_DIR/code-lambda/bootstrap"'
    ) in script
    assert (
        'fetch_remote_target_file release/spur-context-knowledge-lambda '
        '"$BUILD_DIR/knowledge-lambda/bootstrap"'
    ) in script
    assert "build --features lambda --release" not in script


def test_graviton_guard_tracks_both_split_serving_lambdas():
    guard = GRAVITON_BASELINE_GUARD.read_text()

    assert 'run_graviton2_safe_cargo "Code Lambda bootstrap"' in guard
    assert 'run_graviton2_safe_cargo "Knowledge Lambda bootstrap"' in guard
    assert 'run_graviton2_safe_cargo "serving Lambda bootstrap"' not in guard


def test_code_lambda_defaults_to_independent_256_mb_memory():
    variables_tf = (INFRA_DIR / "variables.tf").read_text()
    main_tf = (INFRA_DIR / "main.tf").read_text()

    assert 'variable "code_lambda_memory_mb"' in variables_tf
    code_memory = variables_tf.split(
        'variable "code_lambda_memory_mb"', 1
    )[1].split('variable "', 1)[0]
    shared_memory = variables_tf.split(
        'variable "lambda_memory_mb"', 1
    )[1].split('variable "', 1)[0]
    code_lambda = main_tf.split(
        'resource "aws_lambda_function" "code"', 1
    )[1].split('resource "aws_lambda_function" "knowledge"', 1)[0]
    knowledge_lambda = main_tf.split(
        'resource "aws_lambda_function" "knowledge"', 1
    )[1]

    assert "default     = 256" in code_memory
    assert "default     = 1024" in shared_memory
    assert "memory_size = var.code_lambda_memory_mb" in code_lambda
    assert "memory_size = var.lambda_memory_mb" in knowledge_lambda


def test_deploy_preserves_worker_builds_while_splitting_serving_lambdas():
    script = DEPLOY_SH.read_text()

    assert 'run_graviton2_safe_cargo "Fargate worker binary"' in script
    assert "--workdir crates/spur-context-service build --features worker --release" in script
    assert 'run_graviton2_safe_cargo "spur CLI worker image dependency"' in script
    assert "build -p spur-cli --release" in script
    assert (
        '--remote-binary "$(remote_target_path release/spur-context-worker)"'
    ) in script
    assert (
        '--remote-binary "$(remote_target_path '
        'release/spur-context-worker-lambda)"'
    ) in script
    assert '--remote-binary "$(remote_target_path target/release/spur)"' in script
    assert "fetch_remote_target_file target/release/spur" not in script
    assert '--context-dir "$worker_context"' in script
    assert '--context-dir "$worker_lambda_context"' in script
    assert 'worker_image_uri="$(build_and_push_worker_image)"' not in script
    assert "scripts/spur-cargo run --workdir crates/spur-context-service" not in script
    assert "scripts/spur-cargo build -p spur-context-service" not in script


def test_serving_zip_packagers_isolate_code_and_verify_knowledge_before_copy():
    script = DEPLOY_SH.read_text()
    code_package = shell_function(script, "package_code_zip")
    knowledge_copy = shell_function(script, "copy_knowledge_extensions")
    knowledge_package = shell_function(script, "package_knowledge_zip")
    main = shell_function(script, "main")

    assert 'zip -r "$zip_path" bootstrap' in code_package
    for forbidden in (
        ".duckdb",
        "duckdb_extension",
        "ducklake",
        "httpfs",
        "aws.duckdb_extension",
        "catalog",
        "spur-context-knowledge-lambda",
    ):
        assert forbidden not in code_package

    assert 'cp -R "$BUILD_DIR/.duckdb" "$knowledge_dir/.duckdb"' in knowledge_copy
    assert 'zip -r "$zip_path" bootstrap .duckdb/' in knowledge_package
    assert '"*/lance.duckdb_extension"' in knowledge_package
    assert main.index("download_extensions") < main.index(
        "copy_knowledge_extensions"
    )


def test_deploy_has_selectable_self_contained_buildx_path_with_baseline_flags():
    script = DEPLOY_SH.read_text()

    assert 'BUILD_MODE="${SPUR_CONTEXT_SERVICE_BUILD_MODE:-remote}"' in script
    assert "--build-mode) BUILD_MODE=\"$2\"" in script
    assert "--no-push) PUSH_IMAGES=false" in script
    assert "build_self_contained_artifacts()" in script
    assert "prepare_self_contained_build_context()" in script
    assert 'git -C "$REPO_ROOT" archive --format=tar HEAD' in script
    assert "buildx build" in script
    assert "--platform linux/arm64 --provenance=false" in script
    assert "--output \"type=local,dest=$export_dir\"" in script
    assert 'assert_graviton2_safe_flags "self-contained buildx artifacts"' in script
    assert 'RUSTFLAGS="$SPUR_CONTEXT_GRAVITON2_RUSTFLAGS"' in script
    assert 'CFLAGS="$SPUR_CONTEXT_GRAVITON2_CFLAGS"' in script
    assert 'CXXFLAGS="$SPUR_CONTEXT_GRAVITON2_CXXFLAGS"' in script
    assert "SPUR_CONTEXT_GRAVITON2_RUSTFLAGS" in script
    assert "SPUR_CONTEXT_GRAVITON2_CFLAGS" in script
    assert "SPUR_CONTEXT_GRAVITON2_CXXFLAGS" in script
    assert "fetch_remote_worktree_file" in script


def test_self_contained_worker_images_build_locally_without_ecr_mutations_by_default():
    script = DEPLOY_SH.read_text()

    assert "build_local_worker_images()" in script
    assert 'local output_dir="$REPO_ROOT/target/lambda"' in script
    assert '--output "type=docker,dest=$output_dir/spur-context-worker-image.tar"' in script
    assert '--output "type=docker,dest=$output_dir/spur-context-worker-lambda-image.tar"' in script
    assert '--output "type=docker,dest=$output_dir/spur-context-source-fetcher-image.tar"' in script
    assert 'if [[ "$PUSH_IMAGES" == "true" ]]' in script
    assert "build_and_push_worker_image" in script
    assert "build_and_push_worker_lambda_image" in script
    assert "build_and_push_source_fetcher_lambda_image" in script
    assert 'aws ecr describe-repositories --repository-names "$WORKER_ECR_REPO"' in script
    assert 'aws ecr describe-repositories --repository-names "$WORKER_LAMBDA_ECR_REPO"' in script
    assert 'aws ecr describe-repositories --repository-names "$SOURCE_FETCHER_LAMBDA_ECR_REPO"' in script


def test_deploy_tags_worker_images_with_immutable_tag_and_latest_pointer():
    script = DEPLOY_SH.read_text()

    assert 'IMAGE_TAG="${SPUR_CONTEXT_SERVICE_IMAGE_TAG:-$(resolve_image_tag)}"' in script
    assert 'WORKER_IMAGE_TAG="$IMAGE_TAG"' in script
    assert 'LATEST_IMAGE_TAG="latest"' in script
    assert 'git -C "$REPO_ROOT" rev-parse --short HEAD' in script
    assert 'git -C "$REPO_ROOT" status --porcelain --untracked-files=normal' in script
    assert 'ecr_latest_image_tag "$WORKER_ECR_REPO"' in script
    assert 'ecr_latest_image_tag "$WORKER_LAMBDA_ECR_REPO"' in script
    assert 'ecr_latest_image_tag "$SOURCE_FETCHER_LAMBDA_ECR_REPO"' in script
    assert 'tag_ecr_image_as_latest "$WORKER_ECR_REPO" "$IMAGE_TAG"' in script
    assert 'tag_ecr_image_as_latest "$WORKER_LAMBDA_ECR_REPO" "$IMAGE_TAG"' in script
    assert 'tag_ecr_image_as_latest "$SOURCE_FETCHER_LAMBDA_ECR_REPO" "$IMAGE_TAG"' in script


def test_deploy_worker_image_contains_worker_and_spur_binaries():
    deploy_sh = DEPLOY_SH.read_text()

    assert "COPY spur-context-worker /usr/local/bin/spur-context-worker" in deploy_sh
    assert "COPY spur /usr/local/bin/spur" in deploy_sh
    assert "spur --version" in deploy_sh
    assert "spur-context-worker" in deploy_sh


def test_worker_images_bundle_duckdb_extensions_for_offline_loads():
    script = DEPLOY_SH.read_text()

    assert 'WORKER_DUCKDB_EXTENSION_DIR="/opt/duckdb/extensions"' in script
    assert (
        'EXTENSIONS=("httpfs" "ducklake" "postgres_scanner" "sqlite_scanner" "aws" "parquet" "json" "lance")'
        in script
    )
    assert 'copy_worker_extensions "$worker_context"' in script
    assert 'copy_worker_extensions "$worker_lambda_context"' in script
    assert "COPY duckdb-extensions/ /opt/duckdb/extensions/" in script
    assert (
        "ENV SPUR_CONTEXT_DUCKDB_EXTENSION_DIR=/opt/duckdb/extensions"
        in script
    )
    assert (
        "test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/httpfs.duckdb_extension"
        in script
    )
    assert (
        "test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/ducklake.duckdb_extension"
        in script
    )
    assert (
        "test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/postgres_scanner.duckdb_extension"
        in script
    )
    assert (
        "test -f /opt/duckdb/extensions/v${DUCKDB_VERSION}/${EXT_PLATFORM}/sqlite_scanner.duckdb_extension"
        in script
    )
    assert script.index("download_extensions") < script.index("build_and_push_worker_image")


def test_deploy_builds_lambda_worker_image_for_fast_start():
    script = DEPLOY_SH.read_text()
    cargo_toml = (ROOT / "crates" / "spur-context-service" / "Cargo.toml").read_text()

    assert "worker-lambda" in cargo_toml
    assert "spur-context-worker-lambda" in cargo_toml
    assert 'run_graviton2_safe_cargo "worker Lambda image binary"' in script
    assert "--workdir crates/spur-context-service build --features worker-lambda --release" in script
    assert "WORKER_LAMBDA_ECR_REPO" in script
    assert "COPY spur-context-worker-lambda /usr/local/bin/spur-context-worker-lambda" in script
    assert 'ENTRYPOINT ["/usr/local/bin/spur-context-worker-lambda"]' in script
    assert '-var "worker_lambda_image=' in script


def test_deploy_builds_source_fetcher_lambda_image_and_passes_to_terraform():
    script = DEPLOY_SH.read_text()
    cargo_toml = (ROOT / "crates" / "spur-context-fetcher" / "Cargo.toml").read_text()
    outputs_tf = (INFRA_DIR / "outputs.tf").read_text()

    assert "spur-context-fetcher-lambda" in cargo_toml
    assert 'SOURCE_FETCHER_LAMBDA_ECR_REPO="spur-context-source-fetcher"' in script
    assert "SOURCE_FETCHER_LAMBDA_IMAGE_URI" in script
    assert 'run_graviton2_safe_cargo "source fetcher Lambda image binary"' in script
    assert "build -p spur-context-fetcher --release" in script
    assert "build_and_push_source_fetcher_lambda_image" in script
    assert 'aws ecr describe-repositories --repository-names "$SOURCE_FETCHER_LAMBDA_ECR_REPO"' in script
    assert "write_source_fetcher_lambda_image_dockerfile" in script
    assert "COPY spur-context-fetcher-lambda /usr/local/bin/spur-context-fetcher-lambda" in script
    assert "/usr/local/bin/spur-context-fetcher-lambda --smoke" in script
    assert 'ENTRYPOINT ["/usr/local/bin/spur-context-fetcher-lambda"]' in script
    assert "spur-context-source-fetcher-image.tar" in script
    assert '--remote-binary "$(remote_target_path target/release/spur-context-fetcher-lambda)"' in script
    assert '-var "source_fetcher_lambda_image=' in script
    assert "terraform output -raw source_fetcher_lambda_image_uri" in script
    assert "source fetcher Lambda image URI:" in script
    assert 'output "source_fetcher_lambda_image_uri"' in outputs_tf


def test_deploy_rebuilds_distinct_serving_lambda_zips_by_default():
    script = DEPLOY_SH.read_text()
    main_tf = (INFRA_DIR / "main.tf").read_text()

    code_zip = "../../target/lambda/spur-context-code-lambda.zip"
    knowledge_zip = "../../target/lambda/spur-context-knowledge-lambda.zip"
    assert code_zip != knowledge_zip
    assert f'local tf_code_zip_path="{code_zip}"' in script
    assert f'local tf_knowledge_zip_path="{knowledge_zip}"' in script
    assert 'tf_code_zip_path="$local_code_zip"' in script
    assert 'tf_knowledge_zip_path="$local_knowledge_zip"' in script
    assert '-var "code_lambda_zip_path=$tf_code_zip_path"' in script
    assert '-var "knowledge_lambda_zip_path=$tf_knowledge_zip_path"' in script
    assert '-var "lambda_zip_path=$tf_knowledge_zip_path"' in script
    assert 'elif [[ ! -f "$code_zip_path" ]]' not in script
    assert 'elif [[ ! -f "$knowledge_zip_path" ]]' not in script
    assert 'rm -f "$code_zip_path" "$knowledge_zip_path"' in script
    lambda_zip = main_tf.split('resource "aws_s3_object" "lambda_zip"', 1)[1]
    assert "source_hash = filemd5(local.knowledge_lambda_zip_path)" in lambda_zip
    assert "source_code_hash = filebase64sha256(local.knowledge_lambda_zip_path)" in main_tf
    assert "etag   = filemd5(var.lambda_zip_path)" not in lambda_zip
    code_lambda_zip = terraform_resource_block(
        main_tf, "aws_s3_object", "code_lambda_zip"
    )
    assert "source_hash = filemd5(local.code_lambda_zip_path)" in code_lambda_zip
    assert "source_code_hash = filebase64sha256(local.code_lambda_zip_path)" in main_tf
    assert (
        "knowledge_lambda_zip_path = coalesce("
        "var.knowledge_lambda_zip_path, var.lambda_zip_path)"
    ) in main_tf
    assert (
        "code_lambda_zip_path      = coalesce("
        "var.code_lambda_zip_path, var.lambda_zip_path)"
    ) in main_tf


def test_deploy_can_package_lambda_without_terraform_apply():
    script = DEPLOY_SH.read_text()

    assert "./deploy.sh --skip-worker --package-only" in script
    assert "package_only=false" in script
    assert "--package-only) package_only=true" in script
    assert 'if [[ "$package_only" == "true" ]]' in script
    assert script.index('if [[ "$package_only" == "true" ]]') < script.index("terraform init")


def test_package_only_builds_two_isolated_zip_manifests_with_stubbed_tools(tmp_path):
    repo_root = tmp_path / "repo"
    script_dir = repo_root / "infra" / "spur-context-service"
    script_dir.mkdir(parents=True)
    deploy = script_dir / "deploy.sh"
    deploy.write_text(DEPLOY_SH.read_text())
    deploy.chmod(0o755)
    (script_dir / "graviton2-baseline.sh").write_text(
        (DEPLOY_SH.parent / "graviton2-baseline.sh").read_text()
    )

    tool_dir = tmp_path / "bin"
    tool_dir.mkdir()
    aws_marker = tmp_path / "aws-called"
    terraform_marker = tmp_path / "terraform-mutations"

    write_executable(
        tool_dir / "terraform",
        """#!/usr/bin/env bash
if [[ "$*" == "output -raw aws_region" ]]; then
    echo ap-southeast-5
else
    printf '%s\n' "$*" >> "$TERRAFORM_MARKER"
    exit 97
fi
""",
    )
    write_executable(
        tool_dir / "aws",
        """#!/usr/bin/env bash
printf '%s\n' "$*" >> "$AWS_MARKER"
exit 98
""",
    )
    write_executable(
        tool_dir / "git",
        """#!/usr/bin/env bash
case "$*" in
    *"rev-parse --short HEAD"*) echo deadbeef ;;
    *"status --porcelain"*) ;;
    *"archive --format=tar HEAD"*) /usr/bin/tar -cf - --files-from /dev/null ;;
    *) echo "unexpected git call: $*" >&2; exit 96 ;;
esac
""",
    )
    write_executable(
        tool_dir / "docker",
        """#!/usr/bin/env bash
output=""
while [[ $# -gt 0 ]]; do
    if [[ "$1" == "--output" ]]; then
        output="$2"
        shift 2
    else
        shift
    fi
done
dest="${output#type=local,dest=}"
mkdir -p "$dest"
printf code-binary > "$dest/spur-context-code-lambda"
printf knowledge-binary > "$dest/spur-context-knowledge-lambda"
printf worker > "$dest/spur-context-worker"
printf worker-lambda > "$dest/spur-context-worker-lambda"
printf fetcher > "$dest/spur-context-fetcher-lambda"
printf spur > "$dest/spur"
""",
    )
    write_executable(
        tool_dir / "curl",
        """#!/usr/bin/env bash
printf extension-payload
""",
    )
    write_executable(tool_dir / "gunzip", "#!/usr/bin/env bash\ncat\n")
    write_executable(
        tool_dir / "sha256sum",
        """#!/usr/bin/env bash
case "$1" in
    *httpfs.duckdb_extension*) sha=6d30a487968cbe5553b7272fd5bad0e4485c4117db96e21fd1e5e2a225a5a538 ;;
    *ducklake.duckdb_extension*) sha=f73ec9ab68a6de5c3c190cd1ecbba553a5791bf5821a991d70ea65abc5e45562 ;;
    *postgres_scanner.duckdb_extension*) sha=102f71e6d2e603b1056407410f981c7ec141375e1edd16f2cb61e901e9c6d617 ;;
    *sqlite_scanner.duckdb_extension*) sha=81135f8c3b4064bc5031c3bc08473a7224851461fa9a2b645ea8f851eba4a621 ;;
    *aws.duckdb_extension*) sha=8244ac8560925c3caf71f2087b45e3e37aa84a8a79712b9b8f3312524bd3f483 ;;
    *parquet.duckdb_extension*) sha=d1aa523e5ae55731da2f67fca02ea1460e35fe4f8d824627fb565019e3eece1c ;;
    *json.duckdb_extension*) sha=10ce8205acd23bfbc98c934803d69f1e6768fe9bfcb4c7958406144a1ea898bd ;;
    *lance.duckdb_extension*) sha=81a9f544f9f56be18db46060baf596ab973e17b50785a1eb1edd4885908b2158 ;;
    *) exit 95 ;;
esac
printf '%s  %s\n' "$sha" "$1"
""",
    )

    result = subprocess.run(
        [
            "bash",
            str(deploy),
            "--skip-worker",
            "--package-only",
            "--build-mode",
            "self-contained",
            "--no-push",
        ],
        cwd=repo_root,
        env={
            **os.environ,
            "PATH": f"{tool_dir}:{os.environ['PATH']}",
            "AWS_MARKER": str(aws_marker),
            "TERRAFORM_MARKER": str(terraform_marker),
        },
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert not aws_marker.exists()
    assert not terraform_marker.exists()

    code_zip = repo_root / "target" / "lambda" / "spur-context-code-lambda.zip"
    knowledge_zip = (
        repo_root / "target" / "lambda" / "spur-context-knowledge-lambda.zip"
    )
    assert code_zip != knowledge_zip
    assert code_zip.read_bytes() != knowledge_zip.read_bytes()

    with zipfile.ZipFile(code_zip) as archive:
        assert archive.namelist() == ["bootstrap"]
        assert archive.read("bootstrap") == b"code-binary"
        assert all(
            forbidden not in name.lower()
            for name in archive.namelist()
            for forbidden in ("duckdb", "ducklake", "httpfs", "aws", "catalog", "knowledge")
        )

    with zipfile.ZipFile(knowledge_zip) as archive:
        files = {name for name in archive.namelist() if not name.endswith("/")}
        extension_root = ".duckdb/extensions/v1.5.4/linux_arm64"
        assert files == {
            "bootstrap",
            *(f"{extension_root}/{name}.duckdb_extension" for name in (
                "httpfs",
                "ducklake",
                "postgres_scanner",
                "sqlite_scanner",
                "aws",
                "parquet",
                "json",
            )),
        }
        assert archive.read("bootstrap") == b"knowledge-binary"


def test_terraform_uses_partial_s3_backend_and_environment_files():
    versions_tf = VERSIONS_TF.read_text()

    assert 'backend "s3" {}' in versions_tf

    for environment in ("staging", "prod"):
        backend_config = INFRA_DIR / "backends" / f"{environment}.s3.tfbackend"
        var_file = INFRA_DIR / "env" / f"{environment}.tfvars"

        assert backend_config.exists()
        backend_text = backend_config.read_text()
        assert "bucket" in backend_text
        assert "key" in backend_text
        assert "region" in backend_text
        assert "dynamodb_table" in backend_text

        assert var_file.exists()
        assert "vpc_id" in var_file.read_text()


def test_terraform_requires_cross_variable_validation_compatible_cli():
    versions_tf = VERSIONS_TF.read_text()
    required_version = re.search(
        r'required_version\s*=\s*"([^"]+)"', versions_tf
    )

    assert required_version is not None
    assert required_version.group(1) == ">= 1.9, < 2.0"


def test_deploy_passes_backend_config_and_var_file_to_terraform():
    script = DEPLOY_SH.read_text()

    assert 'local environment="staging"' in script
    assert "--env)" in script
    assert "--backend-config|-backend-config)" in script
    assert "--var-file|-var-file)" in script
    assert 'backend_config="${backend_config:-backends/${environment}.s3.tfbackend}"' in script
    assert 'var_file="${var_file:-env/${environment}.tfvars}"' in script
    assert 'terraform init -upgrade -backend-config="$backend_config"' in script
    assert '-var "code_lambda_zip_path=$tf_code_zip_path"' in script
    assert '-var "knowledge_lambda_zip_path=$tf_knowledge_zip_path"' in script
    assert 'terraform plan "${tf_vars[@]}"' in script
    assert 'terraform apply "${tf_vars[@]}" -auto-approve' in script


def test_remote_worker_image_script_delegates_to_canonical_deploy_path():
    script = REMOTE_BUILD_SH.read_text()

    assert 'SPUR_CONTEXT_SERVICE_BUILD_MODE="${SPUR_CONTEXT_SERVICE_BUILD_MODE:-remote}"' in script
    assert "export SPUR_CONTEXT_SERVICE_BUILD_MODE" in script
    assert 'exec "$SCRIPT_DIR/deploy.sh"' in script
    assert "COPY spur-context-worker /usr/local/bin/spur-context-worker" not in script
    assert "COPY spur /usr/local/bin/spur" not in script
    assert "docker build" not in script


def test_remote_docker_build_accepts_multiple_remote_binaries():
    docker_build = ROOT / "scripts" / "cloud-build" / "docker-build.sh"
    if not docker_build.exists():
        # scripts/cloud-build is a symlink into the private sibling
        # spur-notebook checkout; on a bare CI checkout it dangles. The
        # invariant stays enforced on workstations and anywhere the
        # cloud-build bundle is restored.
        pytest.skip("scripts/cloud-build sibling checkout not present")
    script = docker_build.read_text()

    assert "REMOTE_BINARIES=()" in script
    assert 'REMOTE_BINARIES+=("$2")' in script
    assert 'for remote_binary in "${REMOTE_BINARIES[@]}"' in script
    assert "docker build --platform linux/arm64 --provenance=false" in script


def test_context_service_workflow_runs_tests_and_gated_aws_artifacts():
    workflow = CONTEXT_SERVICE_WORKFLOW.read_text()
    assert "crate-serving-features:" in workflow
    crate_job = workflow.split("crate-serving-features:", 1)[1].split(
        "script-guards:", 1
    )[0]
    code_test = (
        "scripts/spur-cargo --workdir crates/spur-context-service test "
        "--no-default-features --features code-lambda --lib"
    )
    knowledge_test = (
        "scripts/spur-cargo --workdir crates/spur-context-service test "
        "--no-default-features --features knowledge-lambda"
    )

    assert "workflow_dispatch:" in workflow
    assert "build_aws_artifacts:" in workflow
    assert "run_staging_smoke:" in workflow
    assert code_test in crate_job
    assert knowledge_test in crate_job
    assert "--test mcp_test" not in crate_job
    assert "test --all-features" not in crate_job
    assert 'rustflags: ""' in crate_job
    assert 'SPUR_CONTEXT_SERVICE_BUILD_MODE: "self-contained"' in workflow
    assert 'SPUR_CONTEXT_SERVICE_PUSH_IMAGES: "0"' in workflow
    assert 'SPUR_REMOTE: "1"' not in workflow.split("build-aws-artifacts:", 1)[1].split("staging-smoke:", 1)[0]
    assert "docker/setup-qemu-action@v3" in workflow
    assert "docker/setup-buildx-action@v3" in workflow
    assert "infra/spur-context-service/build-and-push-remote.sh" in workflow
    assert "infra/spur-context-service/deploy.sh --skip-worker --package-only" in workflow
    assert "spur-context-service-worker-images" in workflow
    assert "target/lambda/*worker*image.tar" in workflow
    assert "spur-context-service-lambda-zips" in workflow
    assert "target/lambda/spur-context-*-lambda.zip" in workflow
    assert "infra/spur-context-service/smoke-staging-e2e.py" in workflow
    assert "CONTEXT_SERVICE_AWS_ROLE_ARN" in workflow
    assert "context-service-staging" in workflow
    assert "terraform apply" not in workflow


def test_context_service_workflow_releases_serving_lambda_on_main_push():
    workflow = CONTEXT_SERVICE_WORKFLOW.read_text()

    # The release job exists and sits at the end of the file so this slice is
    # the job body only.
    assert "release-staging:" in workflow
    release = workflow.split("release-staging:", 1)[1]

    # Gating: only main pushes (already paths-filtered to the service) release,
    # after the test and guardrail jobs, behind the staging environment.
    assert "github.event_name == 'push'" in release
    assert "github.ref == 'refs/heads/main'" in release
    assert "crate-serving-features" in release
    assert "script-guards" in release
    assert "environment: context-service-staging" in release
    assert "id-token: write" in release

    # Releases serialize instead of cancelling mid-rollout; PR runs keep
    # cancel-in-progress.
    assert "group: context-service-release" in release
    assert "cancel-in-progress: false" in release
    assert (
        "cancel-in-progress: ${{ github.event_name == 'pull_request' }}"
        in workflow
    )

    # Release builds use the provider-approved shared AWS Spot pool
    # (release-dist.yml pattern), not the runner: restore the cloud-build
    # bundle, compile the Lambda zips through deploy.sh's remote mode with
    # Graviton2-safe flags, and leave pool shutdown to the VM-side idle service.
    assert 'SPUR_CONTEXT_SERVICE_BUILD_MODE: "remote"' in release
    assert 'SPUR_CONTEXT_SERVICE_PUSH_IMAGES: "0"' in release
    assert 'SPUR_NO_LOCAL_FALLBACK: "1"' in release
    assert "VM_NAME: spur-builder" in release
    assert "SPUR_BUILD_POOL_NAME: spur-builder" in release
    assert 'SPUR_BUILD_MAX_BUILDERS: "3"' in release
    assert 'SPUR_BUILD_SLOTS_PER_VM: "3"' in release
    assert "spur-ctx-release-${{ github.run_id }}" not in release
    assert "ci/cloud-build/bundle.tar.gz" in release
    assert "session-manager-plugin" in release
    assert "scripts/cloud-build/spin.sh" in release
    assert "scripts/cloud-build/teardown.sh" not in release
    assert "AWS_RELEASE_ROLE_ARN" in release
    assert "CONTEXT_SERVICE_RELEASE_ROLE_ARN" in release

    # Packaging and deployment deliberately use separate least-privilege
    # credentials: the shared-builder role owns SSM/EC2, while the context
    # release role owns the staging S3/Lambda mutation and preflight reads.
    builder_auth = release.index(
        "role-to-assume: ${{ vars.AWS_RELEASE_ROLE_ARN }}"
    )
    package = release.index("Package serving Lambda artifacts")
    deploy_auth = release.index(
        "role-to-assume: ${{ vars.CONTEXT_SERVICE_RELEASE_ROLE_ARN }}"
    )
    code_release = release.index("Release Code Lambda")
    assert builder_auth < package < deploy_auth < code_release
    assert release.count("aws-actions/configure-aws-credentials@") == 2

    # The runner never cross-compiles or pushes images itself.
    assert "amazon-ecr-login" not in release
    assert "setup-qemu-action" not in release
    assert "setup-buildx-action" not in release

    # Code-only rollout through the canonical deploy path: worker images stay
    # pinned, split serving zips are packaged, then update-function-code deploys
    # only the staging Code Lambda. Knowledge also stays pinned because this
    # release is for the Arrow-native code backend.
    assert "infra/spur-context-service/build-and-push-remote.sh" not in release
    assert "--worker-image-only" not in release
    assert (
        "infra/spur-context-service/deploy.sh --skip-worker --package-only"
        in release
    )
    assert release.count("aws lambda update-function-code") == 1
    # The zip exceeds UpdateFunctionCode's ~70 MB inline payload cap, so the
    # rollout stages it in S3 first, reusing terraform's content-addressed
    # key shape (aws_s3_object.code_lambda_zip in main.tf).
    assert "--zip-file" not in release
    assert "zip_path=target/lambda/spur-context-code-lambda.zip" in release
    assert 'aws s3 cp "$zip_path"' in release
    assert "lambda/spur-context-code-" in release
    assert "--s3-bucket" in release
    assert "--s3-key" in release
    assert "aws lambda wait function-updated-v2" in release
    assert "CONTEXT_SERVICE_STAGING_CODE_LAMBDA" in release
    assert "CONTEXT_SERVICE_STAGING_KNOWLEDGE_LAMBDA" in release
    assert '--function-name "$SPUR_CONTEXT_SMOKE_CODE_LAMBDA"' in release
    assert '--function-name "$SPUR_CONTEXT_SMOKE_KNOWLEDGE_LAMBDA"' not in release
    assert "SPUR_CONTEXT_SMOKE_LAMBDA" not in release

    # Worker/source-fetcher live aliases and the ECS taskdef stay pinned:
    # no alias repoint and no terraform from the release job.
    assert "update-alias" not in release
    assert "terraform" not in release

    # Post-release verification is the non-ingesting preflight.
    assert "smoke-staging-e2e.py --preflight" in release


def test_staging_smoke_codifies_e1_real_worker_and_frozen_serving():
    script = STAGING_SMOKE.read_text()
    readme = (INFRA_DIR / "README.md").read_text()

    assert "CONTEXT_SERVICE_STAGING_CODE_LAMBDA" in readme
    assert "CONTEXT_SERVICE_STAGING_KNOWLEDGE_LAMBDA" in readme
    assert "SPUR_CONTEXT_SMOKE_CODE_LAMBDA" in readme
    assert "SPUR_CONTEXT_SMOKE_KNOWLEDGE_LAMBDA" in readme
    assert "CONTEXT_SERVICE_STAGING_LAMBDA" not in readme
    assert "SPUR_CONTEXT_SMOKE_LAMBDA" not in readme

    assert "argparse" in script
    assert "--preflight" in script
    assert "run_preflight" in script
    assert "external_index" in script
    assert "external_index_status" in script
    assert "external_code_search" in script
    assert "external_code_read" in script
    assert "external_knowledge_context" in script
    assert "symbol_embeddings" in script
    assert "bronze/{source}/{package}/{revision}/source.tar.gz" in script
    assert "silver/{source}/{package}/{revision}/{builder_version}/manifest.json" in script
    assert "gold/catalog-snapshot/current.json" in script
    assert "get-function-configuration" in script
    assert "sts" in script
    assert "get-caller-identity" in script
    assert "SPUR_CATALOG_DSN" in script
    assert "postgres" in script
    assert "aws lambda invoke" in script
    assert "aws s3 presign" in script
    assert "prefetch_source=false" in script
    assert "SPUR_CONTEXT_SMOKE_SOURCE_BUCKET" in readme
    assert "smoke-staging-e2e.sh --preflight" in readme
    assert "smoke-staging-e2e.sh" in readme


def test_staging_smoke_codifies_github_fetch_source_path():
    script = STAGING_SMOKE.read_text()
    readme = (INFRA_DIR / "README.md").read_text()

    assert "--github-source" in script
    assert "SPUR_CONTEXT_SMOKE_GITHUB_URL" in script
    assert "git+https://github.com/" in script
    assert '"source_kind": "git"' in script
    assert "assert_stepfunctions_visited_state" in script
    assert "get-execution-history" in script
    assert "FetchSource" in script
    assert "SPUR_CONTEXT_SMOKE_GITHUB_SYMBOL_QUERY" in script
    assert "smoke-staging-e2e.sh --github-source" in readme
    assert "FetchSource" in readme
    assert "presigned HTTPS" in readme


def test_staging_smoke_entrypoint_runs_python_script():
    script = STAGING_SMOKE_ENTRYPOINT.read_text()

    assert "set -euo pipefail" in script
    assert 'exec python3 "$SCRIPT_DIR/smoke-staging-e2e.py" "$@"' in script


def test_staging_smoke_preflight_does_not_ingest():
    script = STAGING_SMOKE.read_text()

    preflight = script.split("def run_preflight", 1)[1].split("def ", 1)[0]
    assert "caller_identity_arn" in preflight
    assert "verify_serving_uses_frozen_s3_catalog" in preflight
    assert "upload_fixture_source" not in preflight
    assert "presign_fixture_source" not in preflight
    assert "external_index" not in preflight
    assert "LambdaInvoker" not in preflight


def test_worker_checkpoint_uri_is_per_job_object_from_state_machine():
    ecs_tf = (INFRA_DIR / "ecs.tf").read_text()
    state_machine_tf = (INFRA_DIR / "state_machine.tf").read_text()
    variables_tf = (INFRA_DIR / "variables.tf").read_text()
    asl = (INFRA_DIR / "index_build_asl.json").read_text()

    assert "SPUR_CONTEXT_WORKER_CHECKPOINT_URI" not in ecs_tf
    assert "worker_checkpoint_uri_template" in state_machine_tf
    assert '"Name": "SPUR_CONTEXT_WORKER_CHECKPOINT_URI"' in asl
    assert (
        '"Value.$": "States.Format('
        "'${worker_checkpoint_uri_template}', $.workerInput.job_id"
        ')"'
    ) in asl
    assert "/jobs/{}/checkpoint.json" in variables_tf


def test_state_machine_does_not_retry_worker_reported_failures():
    asl = (INFRA_DIR / "index_build_asl.json").read_text()

    assert '"States.TaskFailed"' not in asl
    assert '"States.ALL"' not in asl


def test_state_machine_invokes_lambda_worker_before_ecs_fallback():
    asl = (INFRA_DIR / "index_build_asl.json").read_text()
    rendered = render_index_build_asl()
    state_machine_tf = (INFRA_DIR / "state_machine.tf").read_text()
    iam_tf = (INFRA_DIR / "iam.tf").read_text()

    assert rendered["StartAt"] == "RouteSource"
    assert '"StartAt": "RouteSource"' in asl
    assert rendered["States"]["RouteSource"]["Default"] == "PrepareOriginalWorkerInput"
    assert (
        rendered["States"]["RouteSource"]["Choices"][0]["Variable"]
        == "$.prefetch_source"
    )
    assert rendered["States"]["RouteSource"]["Choices"][0]["BooleanEquals"] is True
    assert rendered["States"]["RouteSource"]["Choices"][0]["Next"] == "FetchSource"
    assert rendered["States"]["PrepareOriginalWorkerInput"]["Next"] == "RunLambdaBuild"
    assert '"Resource": "arn:aws:states:::lambda:invoke"' in asl
    assert '"FunctionName": "${worker_lambda_arn}"' in asl
    assert '"Next": "CheckLambdaBuild"' in asl
    assert '"Next": "RunBuild"' in asl
    assert '"ErrorEquals": ["States.Timeout"' in asl
    assert '"Lambda.Unknown"' in asl
    assert '"Sandbox.Timedout"' in asl
    assert "worker_lambda_arn" in state_machine_tf
    assert "source_fetch_lambda_arn" in state_machine_tf
    assert 'Action = ["lambda:InvokeFunction"]' in iam_tf


def test_state_machine_fetches_source_and_normalizes_worker_input():
    rendered = render_index_build_asl()
    states = rendered["States"]

    fetch_source = states["FetchSource"]
    assert fetch_source["Resource"] == "arn:aws:states:::lambda:invoke"
    assert (
        fetch_source["Parameters"]["FunctionName"]
        == "arn:aws:lambda:ap-southeast-5:123456789012:"
        "function:spur-context-source-fetcher:live"
    )
    assert fetch_source["ResultPath"] == "$.fetchResult"
    # No Catch: a deterministic fetcher failure (it throws) fails the execution
    # and does NOT route to the VPC worker. The fetcher's success payload has no
    # `status` field, so FetchSource goes straight to PrepareFetchedWorkerInput
    # (a Choice on a missing path would be a runtime error).
    assert "Catch" not in fetch_source
    assert fetch_source["Next"] == "PrepareFetchedWorkerInput"
    assert fetch_source["Retry"][0]["ErrorEquals"] == [
        "Lambda.ServiceException",
        "Lambda.AWSLambdaException",
        "Lambda.SdkClientException",
        "Lambda.TooManyRequestsException",
    ]
    assert "CheckFetchSource" not in states
    assert "FetchSourceFailed" not in states

    original_worker_input = states["PrepareOriginalWorkerInput"]["Parameters"]
    assert original_worker_input["source_url.$"] == "$.source_url"
    assert original_worker_input["source_kind.$"] == "$.source_kind"

    fetched_worker_input = states["PrepareFetchedWorkerInput"]["Parameters"]
    assert fetched_worker_input["job_id.$"] == "$.job_id"
    assert fetched_worker_input["source_url.$"] == "$.fetchResult.Payload.source_url"
    assert fetched_worker_input["source_kind.$"] == "$.fetchResult.Payload.source_kind"
    assert states["PrepareFetchedWorkerInput"]["ResultPath"] == "$.workerInput"

    lambda_payload = states["RunLambdaBuild"]["Parameters"]["Payload"]
    assert lambda_payload["source_url.$"] == "$.workerInput.source_url"
    assert lambda_payload["source_kind.$"] == "$.workerInput.source_kind"

    for state_name in ("RunBuild", "FallbackBuild"):
        env = {
            item["Name"]: item
            for item in states[state_name]["Parameters"]["Overrides"][
                "ContainerOverrides"
            ][0]["Environment"]
        }
        assert env["SOURCE_URL"]["Value.$"] == "$.workerInput.source_url"
        assert env["SOURCE_KIND"]["Value.$"] == "$.workerInput.source_kind"


def test_source_fetcher_lambda_is_non_vpc_and_least_privilege():
    source_fetcher_tf = (INFRA_DIR / "source_fetcher_lambda.tf").read_text()
    variables_tf = (INFRA_DIR / "variables.tf").read_text()
    state_machine_tf = (INFRA_DIR / "state_machine.tf").read_text()
    iam_tf = (INFRA_DIR / "iam.tf").read_text()
    main_tf = (INFRA_DIR / "main.tf").read_text()

    assert 'resource "aws_lambda_function" "source_fetcher"' in source_fetcher_tf
    assert 'resource "aws_lambda_alias" "source_fetcher_live"' in source_fetcher_tf
    assert 'resource "aws_cloudwatch_log_group" "source_fetcher_lambda"' in source_fetcher_tf
    assert "image_uri     = var.source_fetcher_lambda_image" in source_fetcher_tf
    assert "timeout       = var.source_fetcher_lambda_timeout_sec" in source_fetcher_tf
    assert "memory_size   = var.source_fetcher_lambda_memory_mb" in source_fetcher_tf
    assert "source_fetcher_lambda_ephemeral_storage_mb" in source_fetcher_tf
    assert "vpc_config" not in source_fetcher_tf
    for env_name in (
        "SPUR_CONTEXT_FETCH_BUCKET",
        "SPUR_CONTEXT_FETCH_PREFIX",
        "SPUR_CONTEXT_MAX_TARBALL_BYTES",
        "SPUR_CONTEXT_MAX_GIT_BYTES",
        "SPUR_CONTEXT_ALLOWED_SOURCE_DOMAINS",
        "SPUR_CONTEXT_FETCH_PRESIGN_SECONDS",
    ):
        assert env_name in source_fetcher_tf

    for variable_name in (
        "source_fetcher_lambda_image",
        "source_fetcher_lambda_timeout_sec",
        "source_fetcher_lambda_memory_mb",
        "source_fetcher_lambda_ephemeral_storage_mb",
        "source_fetch_presign_seconds",
        "fetch_artifact_retention_days",
    ):
        assert f'variable "{variable_name}"' in variables_tf
    assert "default     = 900" in variables_tf
    assert "default     = 1024" in variables_tf
    assert "default     = 10240" in variables_tf
    assert "default     = 21600" in variables_tf
    assert "default     = 7" in variables_tf

    assert "source_fetch_lambda_arn" in state_machine_tf
    assert "aws_lambda_alias.source_fetcher_live.arn" in state_machine_tf

    assert 'resource "aws_iam_role" "source_fetcher_lambda"' in iam_tf
    source_fetcher_policy = iam_tf.split(
        'resource "aws_iam_role_policy" "source_fetcher_lambda"', 1
    )[1].split('resource "aws_iam_role_policy" "lambda_catalog_secret"', 1)[0]
    assert '"logs:CreateLogStream"' in source_fetcher_policy
    assert '"logs:PutLogEvents"' in source_fetcher_policy
    assert '"s3:PutObject"' in source_fetcher_policy
    assert '"s3:GetObject"' in source_fetcher_policy
    assert '"s3:AbortMultipartUpload"' in source_fetcher_policy
    assert '"s3:ListBucket"' in source_fetcher_policy
    assert '"${aws_s3_bucket.data.arn}/fetch/*"' in source_fetcher_policy
    assert '"s3:prefix"' in source_fetcher_policy
    for forbidden in (
        "secretsmanager:",
        "dynamodb:",
        "states:",
        "ec2:",
        "AWSLambdaVPCAccessExecutionRole",
    ):
        assert forbidden not in source_fetcher_policy

    assert 'resource "aws_iam_role_policy" "sfn_source_fetcher_lambda"' in iam_tf
    sfn_fetcher_policy = iam_tf.split(
        'resource "aws_iam_role_policy" "sfn_source_fetcher_lambda"', 1
    )[1]
    assert "aws_lambda_function.source_fetcher.arn" in sfn_fetcher_policy
    assert "aws_lambda_alias.source_fetcher_live.arn" in sfn_fetcher_policy

    assert 'resource "aws_s3_bucket_lifecycle_configuration" "data"' in main_tf
    assert 'prefix = "fetch/"' in main_tf
    assert "days = var.fetch_artifact_retention_days" in main_tf
    assert "noncurrent_version_expiration" in main_tf


def test_tfvars_example_documents_source_fetcher_image_and_tuning_knobs():
    tfvars = (INFRA_DIR / "terraform.tfvars.example").read_text()

    assert "source_fetcher_lambda_image" in tfvars
    assert "source_fetcher_lambda_timeout_sec" in tfvars
    assert "source_fetcher_lambda_memory_mb" in tfvars
    assert "source_fetcher_lambda_ephemeral_storage_mb" in tfvars
    assert "source_fetch_presign_seconds" in tfvars
    assert "fetch_artifact_retention_days" in tfvars


def test_lambda_worker_resource_is_configured_for_fast_start_mvp():
    lambda_tf = (INFRA_DIR / "lambda_worker.tf").read_text()
    variables_tf = (INFRA_DIR / "variables.tf").read_text()
    outputs_tf = (INFRA_DIR / "outputs.tf").read_text()
    iam_tf = "\n".join(path.read_text() for path in sorted(INFRA_DIR.glob("*.tf")))

    worker = terraform_resource_block(lambda_tf, "aws_lambda_function", "worker")
    assert 'package_type  = "Image"' in worker
    assert "image_uri     = var.worker_lambda_image" in worker
    assert "timeout       = var.worker_lambda_timeout_sec" in worker
    assert "memory_size   = var.worker_lambda_memory_mb" in worker
    assert "ephemeral_storage" in worker
    assert terraform_assignment(worker, "role") == "aws_iam_role.worker_lambda.arn"
    assert "aws_iam_role_policy.worker_lambda_runtime" in worker
    assert "AWS_REGION" not in lambda_tf
    assert "worker_lambda_memory_mb" in variables_tf
    assert "default     = 3008" in variables_tf
    assert "worker_lambda_ephemeral_storage_mb" in variables_tf
    assert 'output "worker_image_uri"' in outputs_tf
    assert 'output "worker_lambda_image_uri"' in outputs_tf
    assert 'output "worker_lambda_function_name"' in outputs_tf
    worker_s3_policy = terraform_resource_block(
        iam_tf, "aws_iam_role_policy", "worker_lambda_s3"
    )
    assert terraform_assignment(worker_s3_policy, "role") == "aws_iam_role.worker_lambda.id"
    assert "prevent_destroy = true" in worker_s3_policy
    assert "ignore_changes  = all" in worker_s3_policy

    worker_runtime = terraform_resource_block(
        iam_tf, "aws_iam_role_policy", "worker_lambda_runtime"
    )
    assert terraform_assignment(worker_runtime, "role") == "aws_iam_role.worker_lambda.id"
    for required_action in (
        "logs:PutLogEvents",
        "xray:PutTraceSegments",
        "ec2:CreateNetworkInterface",
        "ec2:DescribeSubnets",
        "s3:PutObject",
        "s3:DeleteObject",
        "dynamodb:UpdateItem",
    ):
        assert f'"{required_action}"' in worker_runtime


def test_worker_backends_use_the_deployed_bucket_for_medallion_objects():
    lambda_tf = (INFRA_DIR / "lambda_worker.tf").read_text()
    ecs_tf = (INFRA_DIR / "ecs.tf").read_text()

    for variable_name in (
        "SPUR_CONTEXT_BRONZE_BUCKET",
        "SPUR_CONTEXT_SILVER_BUCKET",
    ):
        assert re.search(
            rf"{variable_name}\s*=\s*aws_s3_bucket\.data\.bucket", lambda_tf
        )
        assert f'name  = "{variable_name}"' in ecs_tf
    assert ecs_tf.count("value = aws_s3_bucket.data.bucket") >= 2


def test_nat_free_worker_vpc_endpoints_are_declared():
    variables_tf = (INFRA_DIR / "variables.tf").read_text()
    default_tfvars = (INFRA_DIR / "env" / "default.tfvars").read_text()
    staging_tfvars = (INFRA_DIR / "env" / "staging.tfvars").read_text()
    prod_tfvars = (INFRA_DIR / "env" / "prod.tfvars").read_text()
    state_machine_tf = (INFRA_DIR / "state_machine.tf").read_text()
    endpoints_tf = (INFRA_DIR / "vpc_endpoints.tf").read_text()
    outputs_tf = (INFRA_DIR / "outputs.tf").read_text()

    assert 'variable "create_vpc_endpoints"' in variables_tf
    assert "default     = true" in variables_tf
    assert 'variable "worker_route_table_ids"' in variables_tf
    assert 'variable "interface_vpc_endpoint_subnet_ids"' in variables_tf
    assert 'variable "interface_vpc_endpoint_service_keys"' in variables_tf
    assert 'variable "vpc_endpoint_region"' in variables_tf

    for service in (
        "s3",
        "dynamodb",
        "states",
        "secretsmanager",
        "ecr.api",
        "ecr.dkr",
        "logs",
        "sts",
    ):
        assert f'com.amazonaws.${{local.vpc_endpoint_region}}.{service}' in endpoints_tf

    assert "all_interface_vpc_endpoint_services" in endpoints_tf
    assert "var.interface_vpc_endpoint_service_keys" in endpoints_tf

    gateway_endpoint = endpoints_tf.split('resource "aws_vpc_endpoint" "gateway"', 1)[1]
    assert 'vpc_endpoint_type = "Gateway"' in gateway_endpoint
    assert "route_table_ids   = local.net_route_table_ids" in gateway_endpoint

    interface_endpoint = endpoints_tf.split('resource "aws_vpc_endpoint" "interface"', 1)[1]
    assert 'vpc_endpoint_type   = "Interface"' in interface_endpoint
    assert "subnet_ids          = local.interface_vpc_endpoint_subnet_ids" in interface_endpoint
    assert "private_dns_enabled = true" in interface_endpoint
    assert "security_group_ids  = [aws_security_group.vpc_endpoints[0].id]" in interface_endpoint

    assert "interface_vpc_endpoint_subnet_ids =" in default_tfvars
    default_endpoint_subnets = re.findall(r'"(subnet-[A-Za-z0-9]+)"', default_tfvars)
    assert default_endpoint_subnets == ["subnet-0e57004af78597f73"]

    for tfvars in (default_tfvars, staging_tfvars, prod_tfvars):
        endpoint_services = re.findall(
            r'"([a-z_]+)"',
            tfvars.split("interface_vpc_endpoint_service_keys", 1)[1].split("]", 1)[0],
        )
        assert endpoint_services == ["states", "secretsmanager"]

    endpoint_sg = state_machine_tf.split('resource "aws_security_group" "vpc_endpoints"', 1)[1]
    assert "count = var.create_vpc_endpoints ? 1 : 0" in endpoint_sg
    assert "from_port       = 443" in endpoint_sg
    assert "to_port         = 443" in endpoint_sg
    assert 'protocol        = "tcp"' in endpoint_sg
    assert "security_groups = [aws_security_group.worker.id]" in endpoint_sg

    assert 'output "gateway_vpc_endpoint_ids"' in outputs_tf
    assert 'output "interface_vpc_endpoint_ids"' in outputs_tf


def test_ecr_lifecycle_policies_prune_context_service_images():
    variables_tf = (INFRA_DIR / "variables.tf").read_text()
    ecr_lifecycle_tf = (INFRA_DIR / "ecr_lifecycle.tf").read_text()
    tfvars_example = (INFRA_DIR / "terraform.tfvars.example").read_text()

    for variable_name in (
        "manage_ecr_lifecycle_policies",
        "ecr_lifecycle_repository_names",
        "ecr_lifecycle_keep_tagged_images",
        "ecr_lifecycle_untagged_image_days",
    ):
        assert f'variable "{variable_name}"' in variables_tf
        assert variable_name in tfvars_example

    for repository in (
        "spur-context-worker",
        "spur-context-worker-lambda",
        "spur-context-source-fetcher",
    ):
        assert repository in variables_tf

    assert 'resource "aws_ecr_lifecycle_policy" "context_service_images"' in ecr_lifecycle_tf
    assert "for_each = var.manage_ecr_lifecycle_policies" in ecr_lifecycle_tf
    assert "repository = each.value" in ecr_lifecycle_tf
    assert re.search(r'tagStatus\s+= "untagged"', ecr_lifecycle_tf)
    assert re.search(r'countType\s+= "sinceImagePushed"', ecr_lifecycle_tf)
    assert re.search(r'countUnit\s+= "days"', ecr_lifecycle_tf)
    assert re.search(
        r"countNumber\s+= var\.ecr_lifecycle_untagged_image_days",
        ecr_lifecycle_tf,
    )
    assert re.search(r'tagStatus\s+= "tagged"', ecr_lifecycle_tf)
    assert re.search(r'tagPatternList\s+= \["\*"\]', ecr_lifecycle_tf)
    assert re.search(r'countType\s+= "imageCountMoreThan"', ecr_lifecycle_tf)
    assert re.search(
        r"countNumber\s+= var\.ecr_lifecycle_keep_tagged_images",
        ecr_lifecycle_tf,
    )


def test_catalog_lease_ttl_uses_worker_expiry_field():
    main_tf = (INFRA_DIR / "main.tf").read_text()

    catalog_leases = main_tf.split('resource "aws_dynamodb_table" "catalog_leases"', 1)[1]

    assert 'attribute_name = "expires_at_unix_secs"' in catalog_leases
