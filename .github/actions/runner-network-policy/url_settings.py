"""Compile a reviewed URL profile and explicit GitHub runner-service rules."""

from urllib.parse import urlsplit, urlunsplit

from policy import hostname, private_origins
from url_policy import URLPolicy, profile_config

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


def service_url(value):
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
    return urlunsplit(("https", name, (parsed.path or "/").rstrip("/") + "/", "", ""))


def runtime_services(environment):
    """Pass endpoint metadata to the root installer, never credentials or queries."""
    runtime = service_url(environment["ACTIONS_RUNTIME_URL"])
    results = environment.get("ACTIONS_RESULTS_URL", RESULTS)
    if results.startswith("http://"):
        if (
            private_origins(environment).get(urlsplit(results).netloc)
            != urlsplit(RESULTS).hostname
        ):
            raise ValueError("unexpected private results service")
        # The validated Depot mapping forwards to this public origin.
        results = RESULTS
    return {"runtime": runtime, "results": service_url(results)}


def rule(url, methods=METHODS, *, prefix=True):
    return {
        "url": url,
        "methods": list(methods),
        "match": "prefix" if prefix else "exact",
        "query": "any",
    }


def compile_policy(path, name, domain_policy, services):
    selected = profile_config(path, name)
    rules = selected["rules"]
    if selected["runner_services"]:
        if set(services) != {"runtime", "results"}:
            raise ValueError("runner service URLs are required")
        runtime = service_url(services["runtime"])
        results = service_url(services["results"])
        rules.extend(rule(runtime + suffix) for suffix in RUNTIME_PATHS)
        rules.append(rule(runtime + "_apis/connectionData", prefix=False))
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
