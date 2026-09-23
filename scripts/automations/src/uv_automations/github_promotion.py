"""Bounded GitHub reads and provenance checks shared by promotion workflows."""

import json
import re
import subprocess
from collections.abc import Callable, Sequence
from functools import wraps
from typing import Protocol, assert_never, override
from urllib.parse import quote, urlencode

from uv_automations.github import decode_pull_request_details
from uv_automations.github_actions import ActionsGitHub
from uv_automations.json import (
    as_array,
    as_boolean,
    as_object,
    as_positive_integer,
    as_string,
    loads,
)
from uv_automations.models import (
    ActorKind,
    CommitSha,
    PullRequestState,
    RepositoryIdentity,
    RepositoryName,
    Timestamp,
)
from uv_automations.promotion_models import (
    AUTOMATIONS_BOT_ID,
    MAX_PROMOTION_PAGE_SIZE,
    MAX_PROMOTION_PAGES,
    AmbiguousPromotionRecord,
    BranchRevision,
    ClosedPromotedParent,
    CommitComparison,
    ComparisonStatus,
    ConvertedToDraftEvent,
    GitHubAppIdentity,
    HeadForcePush,
    LabelAddedEvent,
    LabelRemovedEvent,
    MergedPromotedParent,
    OpenPromotedParent,
    PromotionActor,
    PromotionComment,
    PromotionEvent,
    PromotionEventKind,
    PromotionPullRequest,
    PromotionScope,
    PullRequestMerge,
    PullRequestSelection,
    ReadyForReviewEvent,
    RepositoryPermission,
    SynchronizedParent,
    UneditedPromotionComment,
    UnsynchronizedParent,
    VerifiedPromotedParent,
    check_promotion_branch,
    require_promotion_repository,
    require_promotion_source,
    unique_promotion_record,
)


class PromotionReadError(RuntimeError):
    """A promotion read failed without exposing private request or response data."""


def _redacted_read[**Parameters, Result](
    function: Callable[Parameters, Result],
) -> Callable[Parameters, Result]:
    @wraps(function)
    def read(*args: Parameters.args, **kwargs: Parameters.kwargs) -> Result:
        try:
            return function(*args, **kwargs)
        except KeyError, TypeError, ValueError:
            raise PromotionReadError("Invalid GitHub promotion response") from None

    return read


def _repository(value: object) -> RepositoryIdentity:
    data = as_object(value)
    return RepositoryIdentity(
        RepositoryName(as_string(data["full_name"])),
        as_positive_integer(data["id"]),
    )


def _graphql_repository(value: object) -> RepositoryIdentity:
    data = as_object(value)
    return RepositoryIdentity(
        RepositoryName(as_string(data["nameWithOwner"])),
        as_positive_integer(data["databaseId"]),
    )


def _graphql_database_id(value: object) -> int:
    # GitHub's GraphQL BigInt is a decimal string, not its deprecated Int ID.
    identifier = as_string(value)
    if re.fullmatch(r"[1-9][0-9]*", identifier) is None:
        raise ValueError("Expected a positive GitHub database ID")
    return int(identifier)


def _actor(value: object) -> PromotionActor | None:
    if value is None:
        return None
    data = as_object(value)
    return PromotionActor(
        as_string(data["login"]),
        as_positive_integer(data["id"]),
        ActorKind(as_string(data["type"])),
    )


def _app(value: object) -> GitHubAppIdentity | None:
    if value is None:
        return None
    data = as_object(value)
    return GitHubAppIdentity(as_positive_integer(data["id"]), as_string(data["slug"]))


def decode_promotion_pull_request(
    value: object, scope: PromotionScope
) -> PromotionPullRequest:
    data = as_object(value)
    merged_at = data["merged_at"]
    merge = (
        PullRequestMerge(
            CommitSha(as_string(data["merge_commit_sha"])),
            Timestamp.parse(as_string(merged_at)),
        )
        if merged_at is not None
        else None
    )
    body = data["body"]
    return PromotionPullRequest(
        scope,
        decode_pull_request_details(data, scope.reference),
        as_boolean(data["draft"]),
        _actor(data["user"]),
        as_string(data["title"]),
        as_string(body) if body is not None else "",
        merge,
    )


def _event(value: object) -> PromotionEvent | None:
    data = as_object(value)
    event = as_string(data["event"])
    if event not in PromotionEventKind:
        return None
    kind = PromotionEventKind(event)
    identifier = as_positive_integer(data["id"])
    actor = _actor(data["actor"])
    created_at = Timestamp.parse(as_string(data["created_at"]))
    match kind:
        case PromotionEventKind.READY_FOR_REVIEW:
            return ReadyForReviewEvent(identifier, actor, created_at)
        case PromotionEventKind.CONVERT_TO_DRAFT:
            return ConvertedToDraftEvent(identifier, actor, created_at)
        case PromotionEventKind.LABELED:
            return LabelAddedEvent(
                identifier,
                actor,
                created_at,
                as_string(as_object(data["label"])["name"]),
            )
        case PromotionEventKind.UNLABELED:
            return LabelRemovedEvent(
                identifier,
                actor,
                created_at,
                as_string(as_object(data["label"])["name"]),
            )
    assert_never(kind)


def decode_promotion_comment(value: object, scope: PromotionScope) -> PromotionComment:
    data = as_object(value)
    identifier = as_positive_integer(data["id"])
    if as_string(data["issue_url"]) != (
        f"https://api.github.com/repos/{scope.repository.name}/issues/{scope.number}"
    ):
        raise ValueError("Promotion comment belongs to a different pull request")
    return PromotionComment(
        scope,
        identifier,
        _actor(data["user"]),
        _app(data["performed_via_github_app"]),
        as_string(data["body"]),
        Timestamp.parse(as_string(data["created_at"])),
        Timestamp.parse(as_string(data["updated_at"])),
    )


def _check_graphql_pull_request(
    scope: PromotionScope, value: object
) -> dict[str, object]:
    data = as_object(value)
    if (
        as_positive_integer(data["number"]) != scope.number
        or _graphql_repository(data["repository"]) != scope.repository
    ):
        raise ValueError("Unexpected promotion pull request identity")
    return data


def _nullable_commit(value: object) -> CommitSha | None:
    return CommitSha(as_string(as_object(value)["oid"])) if value is not None else None


def _selected(pull_request: PromotionPullRequest, state: PullRequestSelection) -> bool:
    match state:
        case PullRequestSelection.OPEN:
            return pull_request.is_open
        case PullRequestSelection.CLOSED:
            return not pull_request.is_open
        case PullRequestSelection.ALL:
            return True
    assert_never(state)


class PromotedParentReader(Protocol):
    def get_promotion_pull_request(
        self, scope: PromotionScope
    ) -> PromotionPullRequest: ...

    def list_promotion_comments(
        self, scope: PromotionScope
    ) -> tuple[PromotionComment, ...]: ...

    def list_head_force_pushes(
        self, scope: PromotionScope
    ) -> tuple[HeadForcePush, ...]: ...


class PromotionComparisonReader(Protocol):
    def compare_commits(
        self, repository: RepositoryIdentity, base: CommitSha, head: CommitSha
    ) -> CommitComparison: ...


class PromotionRevisionReader(PromotionComparisonReader, Protocol):
    def get_ref(self, repository: RepositoryIdentity, ref: str) -> CommitSha | None: ...


class PromotionReceiptReader(Protocol):
    def get_unedited_promotion_comment(
        self, scope: PromotionScope, identifier: int
    ) -> UneditedPromotionComment | None: ...


class PromotionReader(PromotedParentReader, PromotionRevisionReader, Protocol):
    def list_pull_requests(
        self,
        repository: RepositoryIdentity,
        *,
        state: PullRequestSelection = PullRequestSelection.OPEN,
        head: str | None = None,
        base: str | None = None,
    ) -> tuple[PromotionPullRequest, ...]: ...

    def list_promotion_events(
        self, scope: PromotionScope
    ) -> tuple[PromotionEvent, ...]: ...

    def get_repository_permission(
        self, repository: RepositoryIdentity, actor: PromotionActor
    ) -> RepositoryPermission: ...


class PromotionGitHub(ActionsGitHub):
    """Promotion transport that is also safe in public post-sync job logs."""

    @override
    def _command(
        self, arguments: Sequence[str], *, payload: object | None = None
    ) -> object:
        try:
            result = subprocess.run(
                [self.executable, *arguments],
                input=(
                    json.dumps(payload, allow_nan=False)
                    if payload is not None
                    else None
                ),
                check=False,
                text=True,
                capture_output=True,
                env=self._environment(),
                timeout=60,
            )
        except OSError, subprocess.TimeoutExpired:
            raise PromotionReadError("GitHub promotion request failed") from None
        if result.returncode != 0:
            raise PromotionReadError("GitHub promotion request failed")
        try:
            return loads(result.stdout) if result.stdout.strip() else None
        except TypeError, ValueError:
            raise PromotionReadError("Invalid GitHub promotion response") from None

    def _graphql(self, query: str, **variables: object) -> dict[str, object]:
        result = as_object(
            self._api(
                "POST", "graphql", payload={"query": query, "variables": variables}
            )
        )
        if result.get("errors"):
            raise PromotionReadError("GitHub promotion request failed")
        return as_object(result["data"])

    def _pages(self, path: str, **parameters: str) -> tuple[object, ...]:
        values: list[object] = []
        for page in range(1, MAX_PROMOTION_PAGES + 1):
            query = urlencode(
                {**parameters, "per_page": MAX_PROMOTION_PAGE_SIZE, "page": page}
            )
            batch = as_array(self._api("GET", f"{path}?{query}"))
            if len(batch) > MAX_PROMOTION_PAGE_SIZE:
                raise ValueError("GitHub returned an oversized promotion page")
            values.extend(batch)
            if len(batch) < MAX_PROMOTION_PAGE_SIZE:
                return tuple(values)
        raise ValueError("Promotion history exceeds the bounded collection budget")

    @_redacted_read
    def get_repository(self, repository: RepositoryIdentity) -> RepositoryIdentity:
        require_promotion_repository(repository)
        if _repository(self._api("GET", f"repos/{repository.name}")) != repository:
            raise ValueError("Unexpected promotion repository identity")
        return repository

    @_redacted_read
    def get_promotion_pull_request(self, scope: PromotionScope) -> PromotionPullRequest:
        return decode_promotion_pull_request(
            self._api("GET", f"repos/{scope.repository.name}/pulls/{scope.number}"),
            scope,
        )

    @_redacted_read
    def list_pull_requests(
        self,
        repository: RepositoryIdentity,
        *,
        state: PullRequestSelection = PullRequestSelection.OPEN,
        head: str | None = None,
        base: str | None = None,
    ) -> tuple[PromotionPullRequest, ...]:
        self.get_repository(repository)
        parameters = {
            "state": state.value,
            "sort": "created",
            "direction": "asc",
        }
        if head is not None:
            check_promotion_branch(head)
            owner = str(repository.name).split("/", 1)[0]
            parameters["head"] = f"{owner}:{head}"
        if base is not None:
            check_promotion_branch(base)
            parameters["base"] = base
        result: list[PromotionPullRequest] = []
        for value in self._pages(f"repos/{repository.name}/pulls", **parameters):
            scope = PromotionScope(
                repository, as_positive_integer(as_object(value)["number"])
            )
            pull_request = decode_promotion_pull_request(value, scope)
            if head is not None and pull_request.details.head.ref != head:
                raise ValueError("GitHub returned a different head branch")
            if base is not None and pull_request.details.base.ref != base:
                raise ValueError("GitHub returned a different base branch")
            if not _selected(pull_request, state):
                raise ValueError("GitHub returned a different pull request state")
            result.append(pull_request)
        if len({pull_request.scope for pull_request in result}) != len(result):
            raise ValueError("GitHub returned duplicate promotion pull requests")
        return tuple(result)

    @_redacted_read
    def list_promotion_events(
        self, scope: PromotionScope
    ) -> tuple[PromotionEvent, ...]:
        self.get_promotion_pull_request(scope)
        identifiers: set[int] = set()
        result: list[PromotionEvent] = []
        for value in self._pages(
            f"repos/{scope.repository.name}/issues/{scope.number}/events"
        ):
            data = as_object(value)
            identifier = as_positive_integer(data["id"])
            if identifier in identifiers:
                raise ValueError("GitHub returned duplicate promotion events")
            identifiers.add(identifier)
            event = _event(data)
            if event is not None:
                result.append(event)
        return tuple(sorted(result, key=lambda event: event.identifier))

    @_redacted_read
    def list_promotion_comments(
        self, scope: PromotionScope
    ) -> tuple[PromotionComment, ...]:
        self.get_promotion_pull_request(scope)
        result = tuple(
            decode_promotion_comment(value, scope)
            for value in self._pages(
                f"repos/{scope.repository.name}/issues/{scope.number}/comments"
            )
        )
        if len({comment.identifier for comment in result}) != len(result):
            raise ValueError("GitHub returned duplicate promotion comments")
        return result

    @_redacted_read
    def get_promotion_comment(
        self, scope: PromotionScope, identifier: int
    ) -> PromotionComment:
        as_positive_integer(identifier)
        self.get_promotion_pull_request(scope)
        result = decode_promotion_comment(
            self._api(
                "GET", f"repos/{scope.repository.name}/issues/comments/{identifier}"
            ),
            scope,
        )
        if result.identifier != identifier:
            raise ValueError("GitHub returned a different promotion comment")
        return result

    @_redacted_read
    def get_unedited_promotion_comment(
        self, scope: PromotionScope, identifier: int
    ) -> UneditedPromotionComment | None:
        """Bind a new authoritative receipt to its exact, never-edited body.

        REST preserves the original author/App even when a repository writer
        edits someone else's comment. Its timestamps alone cannot establish the
        absence of same-second edits, so check the GraphQL edit metadata too.
        """
        as_positive_integer(identifier)
        self.get_promotion_pull_request(scope)
        data = as_object(
            self._api(
                "GET", f"repos/{scope.repository.name}/issues/comments/{identifier}"
            )
        )
        direct = decode_promotion_comment(data, scope)
        if direct.identifier != identifier:
            raise ValueError("GitHub returned a different promotion comment")
        # The exact-comment endpoint can omit performed_via_github_app even
        # when the issue-comment list contains it. Bind that list entry to the
        # direct response instead of treating missing App metadata as proof.
        matches = tuple(
            comment
            for comment in self.list_promotion_comments(scope)
            if comment.identifier == identifier
        )
        if len(matches) != 1:
            return None
        comment = matches[0]
        if (
            not comment.is_automation
            or direct.author != comment.author
            or direct.body != comment.body
            or direct.created_at != comment.created_at
            or direct.updated_at != comment.updated_at
            or (direct.app is not None and direct.app != comment.app)
        ):
            return None
        node_id = as_string(data["node_id"])
        if not node_id or len(node_id) > 200:
            raise ValueError("Invalid promotion comment node identity")
        graph = self._graphql(
            """
                query($id: ID!) {
                    node(id: $id) {
                        __typename
                        ... on IssueComment {
                            id fullDatabaseId body lastEditedAt
                            editor { __typename }
                            repository { nameWithOwner databaseId }
                            pullRequest {
                                number repository { nameWithOwner databaseId }
                            }
                        }
                    }
                }
            """,
            id=node_id,
        )
        value = graph["node"]
        if value is None:
            return None
        node = as_object(value)
        if (
            as_string(node["__typename"]) != "IssueComment"
            or as_string(node["id"]) != node_id
            or _graphql_database_id(node["fullDatabaseId"]) != identifier
            or _graphql_repository(node["repository"]) != scope.repository
        ):
            raise ValueError("GraphQL returned a different promotion comment")
        _check_graphql_pull_request(scope, node["pullRequest"])
        if (
            node["lastEditedAt"] is not None
            or node["editor"] is not None
            or as_string(node["body"]) != comment.body
        ):
            return None
        return UneditedPromotionComment(comment)

    @_redacted_read
    def get_repository_permission(
        self, repository: RepositoryIdentity, actor: PromotionActor
    ) -> RepositoryPermission:
        self.get_repository(repository)
        data = as_object(
            self._api(
                "GET",
                f"repos/{repository.name}/collaborators/"
                f"{quote(actor.login, safe='')}/permission",
            )
        )
        if _actor(data["user"]) != actor:
            raise ValueError("GitHub returned another collaborator's permission")
        return RepositoryPermission(as_string(data["permission"]))

    @_redacted_read
    def get_ref(self, repository: RepositoryIdentity, ref: str) -> CommitSha | None:
        require_promotion_repository(repository)
        check_promotion_branch(ref)
        owner, name = str(repository.name).split("/", 1)
        data = self._graphql(
            """
                query($owner: String!, $name: String!, $ref: String!) {
                    repository(owner: $owner, name: $name) {
                        nameWithOwner databaseId
                        ref(qualifiedName: $ref) {
                            name prefix target { __typename oid }
                        }
                    }
                }
            """,
            owner=owner,
            name=name,
            ref=f"refs/heads/{ref}",
        )
        result = as_object(data["repository"])
        if _graphql_repository(result) != repository:
            raise ValueError("Unexpected promotion repository identity")
        value = result["ref"]
        if value is None:
            return None
        branch = as_object(value)
        target = as_object(branch["target"])
        if (
            as_string(branch["prefix"]) != "refs/heads/"
            or as_string(branch["name"]) != ref
            or as_string(target["__typename"]) != "Commit"
        ):
            raise ValueError("GitHub returned a different branch reference")
        return CommitSha(as_string(target["oid"]))

    @_redacted_read
    def compare_commits(
        self, repository: RepositoryIdentity, base: CommitSha, head: CommitSha
    ) -> CommitComparison:
        self.get_repository(repository)
        data = as_object(
            self._api(
                "GET",
                f"repos/{repository.name}/compare/{base}...{head}?per_page=1&page=1",
            )
        )
        if CommitSha(as_string(as_object(data["base_commit"])["sha"])) != base:
            raise ValueError("GitHub compared a different base commit")
        return CommitComparison(
            repository,
            base,
            head,
            ComparisonStatus(as_string(data["status"])),
            CommitSha(as_string(as_object(data["merge_base_commit"])["sha"])),
        )

    @_redacted_read
    def list_head_force_pushes(
        self, scope: PromotionScope
    ) -> tuple[HeadForcePush, ...]:
        owner, name = str(scope.repository.name).split("/", 1)
        cursor: str | None = None
        seen_cursors: set[str] = set()
        seen_identifiers: set[str] = set()
        result: list[HeadForcePush] = []
        query = f"""
            query($owner: String!, $name: String!, $number: Int!, $cursor: String) {{
                repository(owner: $owner, name: $name) {{
                    nameWithOwner databaseId
                    pullRequest(number: $number) {{
                        number repository {{ nameWithOwner databaseId }}
                        timelineItems(first: {MAX_PROMOTION_PAGE_SIZE}, after: $cursor,
                                      itemTypes: [HEAD_REF_FORCE_PUSHED_EVENT]) {{
                            nodes {{
                                __typename
                                ... on HeadRefForcePushedEvent {{
                                    id createdAt
                                    beforeCommit {{ oid }} afterCommit {{ oid }}
                                    pullRequest {{
                                        number repository {{ nameWithOwner databaseId }}
                                    }}
                                }}
                            }}
                            pageInfo {{ hasNextPage endCursor }}
                        }}
                    }}
                }}
            }}
        """
        for _ in range(MAX_PROMOTION_PAGES):
            data = self._graphql(
                query, owner=owner, name=name, number=scope.number, cursor=cursor
            )
            repository = as_object(data["repository"])
            if _graphql_repository(repository) != scope.repository:
                raise ValueError("Unexpected promotion repository identity")
            pull_request = _check_graphql_pull_request(scope, repository["pullRequest"])
            connection = as_object(pull_request["timelineItems"])
            batch = as_array(connection["nodes"])
            if len(batch) > MAX_PROMOTION_PAGE_SIZE:
                raise ValueError("GitHub returned an oversized force-push page")
            for value in batch:
                event = as_object(value)
                if as_string(event["__typename"]) != "HeadRefForcePushedEvent":
                    raise ValueError("GitHub returned an unexpected timeline event")
                _check_graphql_pull_request(scope, event["pullRequest"])
                force_push = HeadForcePush(
                    as_string(event["id"]),
                    _nullable_commit(event["beforeCommit"]),
                    _nullable_commit(event["afterCommit"]),
                    Timestamp.parse(as_string(event["createdAt"])),
                )
                if force_push.identifier in seen_identifiers:
                    raise ValueError("GitHub returned duplicate force-push events")
                seen_identifiers.add(force_push.identifier)
                result.append(force_push)
            page = as_object(connection["pageInfo"])
            if not as_boolean(page["hasNextPage"]):
                return tuple(result)
            next_cursor = as_string(page["endCursor"])
            if not next_cursor or next_cursor in seen_cursors:
                raise ValueError("GitHub returned a non-advancing force-push cursor")
            seen_cursors.add(next_cursor)
            cursor = next_cursor
        raise ValueError("Force-push history exceeds the bounded collection budget")


def verified_promoted_parent(
    source_reader: PromotedParentReader,
    source_parent: PromotionPullRequest,
    *,
    upstream_reader: PromotedParentReader | None = None,
) -> VerifiedPromotedParent | None:
    """Resolve a unique bot-issued source-to-upstream parent relationship.

    Missing or contradictory provenance is not a parent. Transport failures and
    malformed/incomplete histories remain errors, rather than a partial proof.
    """
    require_promotion_source(source_parent.scope.repository)
    if source_parent.is_open or not source_parent.same_repository:
        return None
    try:
        record = unique_promotion_record(
            source_parent.scope,
            source_reader.list_promotion_comments(source_parent.scope),
        )
    except AmbiguousPromotionRecord:
        return None
    if record is None:
        return None
    upstream_reader = upstream_reader or source_reader
    upstream = upstream_reader.get_promotion_pull_request(record.upstream)
    author = upstream.author
    if (
        not upstream.same_repository
        or upstream.details.head.ref != source_parent.details.head.ref
        or author is None
        or author.database_id != AUTOMATIONS_BOT_ID
        or author.kind != ActorKind.BOT
    ):
        return None
    original_head = source_parent.details.head.sha
    if upstream.details.head.sha != original_head and not any(
        event.contains(original_head)
        for event in upstream_reader.list_head_force_pushes(upstream.scope)
    ):
        return None
    match upstream.details.state:
        case PullRequestState.OPEN:
            return OpenPromotedParent(source_parent, upstream, record)
        case PullRequestState.CLOSED:
            if upstream.merge is None:
                return ClosedPromotedParent(source_parent, upstream, record)
            return MergedPromotedParent(source_parent, upstream, record, upstream.merge)
    assert_never(upstream.details.state)


def parent_sync(
    reader: PromotionComparisonReader,
    parent: MergedPromotedParent,
    main: BranchRevision,
) -> SynchronizedParent | UnsynchronizedParent:
    if main.repository != parent.source.scope.repository or main.ref != "main":
        raise ValueError("Parent synchronization requires the source main branch")
    comparison = reader.compare_commits(main.repository, parent.merge.sha, main.sha)
    if (
        comparison.repository != main.repository
        or comparison.base != parent.merge.sha
        or comparison.head != main.sha
    ):
        raise ValueError("Parent synchronization compared different commits")
    if comparison.is_ancestor:
        return SynchronizedParent(parent, main)
    return UnsynchronizedParent(parent, main)
