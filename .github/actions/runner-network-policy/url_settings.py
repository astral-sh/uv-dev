"""Compile a reviewed URL profile and explicit GitHub runner-service rules."""

from urllib.parse import urljoin, urlsplit, urlunsplit

from policy import hostname, private_origins
from url_policy import URLPolicy, parse_url, profile_config

METHODS = ("GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS")
RESULTS = "https://results-receiver.actions.githubusercontent.com/"
RESULTS_PATHS = (
    "twirp/results.services.receiver.Receiver/",
    "twirp/github.actions.results.api.v1.WorkflowStepUpdateService/",
    "twirp/github.actions.results.api.v1.ArtifactService/",
)
RUNTIME_PATHS = (
    "_apis/distributedtask/",
    "_apis/pipelines/",
    "_apis/actions/",
    "api/v3/workflow/",
)
RUN_SERVICE_PATHS = ("renewjob", "completejob")
RUN_SERVICE_ORIGINS = tuple(
    f"https://run-actions-{index}-azure-eastus.actions.githubusercontent.com/"
    for index in range(1, 4)
)


def service_url(value, *, directory=True):
    """Accept only the exact, public service origins supplied by the runner."""
    if not isinstance(value, str) or any(not 32 < ord(item) < 127 for item in value):
        raise ValueError("invalid runner service URL")
    parsed = urlsplit(value)
    name = hostname(parsed.hostname)
    if (
        parsed.scheme != "https"
        or not name.endswith(".actions.githubusercontent.com")
        or parsed.port not in {None, 443}
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
    ):
        raise ValueError("unexpected runner service URL")
    path = parsed.path or "/"
    if directory:
        path = path.rstrip("/") + "/"
    return urlunsplit(("https", name, path, "", ""))


def oidc_url(value):
    """Keep the runner's exact identity-token route, without its query data."""
    if not isinstance(value, str) or any(not 32 < ord(item) < 127 for item in value):
        raise ValueError("invalid OIDC service URL")
    parsed = urlsplit(value)
    if "#" in value:
        raise ValueError("unexpected OIDC service URL")
    endpoint = service_url(
        urlunsplit((parsed.scheme, parsed.netloc, parsed.path, "", "")),
        directory=False,
    )
    # Some runner-issued routes contain repeated slashes. Keep that exact path,
    # but reject every other spelling the URL matcher considers ambiguous.
    parse_url(endpoint, allow_repeated_slashes=True)
    return endpoint


def runtime_services(environment):
    """Pass endpoint metadata to the root installer, never credentials or queries."""
    runtime = service_url(environment["ACTIONS_RUNTIME_URL"], directory=False)
    results = environment.get("ACTIONS_RESULTS_URL", RESULTS)
    if results.startswith("http://"):
        if (
            private_origins(environment).get(urlsplit(results).netloc)
            != urlsplit(RESULTS).hostname
        ):
            raise ValueError("unexpected private results service")
        # The validated Depot mapping forwards to this public origin.
        results = RESULTS
    services = {"runtime": runtime, "results": service_url(results)}
    if endpoint := environment.get("ACTIONS_ID_TOKEN_REQUEST_URL"):
        services["oidc"] = oidc_url(endpoint)
    return services


def rule(url, methods=METHODS, *, match="prefix", query="any"):
    return {
        "url": url,
        "methods": list(methods),
        "match": match,
        "query": query,
    }


def compile_policy(path, name, domain_policy, services):
    selected = profile_config(path, name)
    rules = selected["rules"]
    if selected["runner_services"]:
        if (
            not isinstance(services, dict)
            or not {"runtime", "results"} <= services.keys()
            or not services.keys() <= {"runtime", "results", "oidc"}
        ):
            raise ValueError("runner service URLs are required")
        runtime_base = service_url(services["runtime"], directory=False)
        runtime = service_url(runtime_base)
        results = service_url(services["results"])
        rules.extend(rule(runtime + suffix) for suffix in RUNTIME_PATHS)
        rules.append(rule(runtime + "_apis/connectionData", match="exact"))
        # The runner can expose a different PipelinesServiceUrl to actions.
        # These exact Run Service hosts are published in GET /meta. Its opaque
        # per-job base is not exposed to actions; numeric shard prefixes are an
        # explicit compatibility exception for the two post-bootstrap routes.
        rules.extend(
            rule(urljoin(origin, suffix), ("POST",), match="exact", query="exact")
            for origin in dict.fromkeys((runtime_base, *RUN_SERVICE_ORIGINS))
            for suffix in RUN_SERVICE_PATHS
        )
        rules.extend(
            rule(
                origin + "{integer}/" + suffix,
                ("POST",),
                match="template",
                query="exact",
            )
            for origin in RUN_SERVICE_ORIGINS
            for suffix in RUN_SERVICE_PATHS
        )
        if "oidc" in services:
            # The original query is deliberately not persisted; clients append
            # their audience to this one exact, authenticated request path.
            endpoint = oidc_url(services["oidc"])
            identity_rule = rule(endpoint, ("GET",), match="exact")
            if "//" in urlsplit(endpoint).path:
                identity_rule["allow_repeated_slashes"] = True
            rules.append(identity_rule)
        for origin in dict.fromkeys((results, RESULTS)):
            rules.extend(rule(origin + suffix, ("POST",)) for suffix in RESULTS_PATHS)
        # These exact GitHub-published storage accounts carry signed log/artifact
        # uploads. They are explicit coarse exceptions, not arbitrary Azure hosts.
        rules.extend(
            rule(
                f"https://productionresultssa{index}.blob.core.windows.net/",
                ("GET", "HEAD", "PUT"),
            )
            for index in range(20)
        )
        rules.extend(
            rule(
                f"https://hosted-compute-{service}-prod-{region}-{number}.githubapp.com/"
            )
            for service in ("request-orchestrator", "watchdog")
            for region in ("eus", "iad")
            for number in ("01", "02")
        )
    policy = URLPolicy.from_dict({"rules": rules})
    if any(not domain_policy.permits(host) for host in policy.hosts):
        raise ValueError("URL profile exceeds its domain policy")
    if len(policy.hosts) > 128:
        raise ValueError("too many URL policy hosts")
    return policy
