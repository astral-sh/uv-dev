"""The bounded GitHub reads and narrow writes needed for feedback handling."""

import json
import re
from dataclasses import dataclass
from urllib.parse import urlencode

from uv_automations.comment_models import (
    MAX_COLLECTION_PAGES,
    MAX_PAGE_SIZE,
    MAX_SELECTED_THREADS,
    MAX_THREAD_COMMENTS,
    ActorKind,
    AuthorAssociation,
    CommentAuthor,
    CommentScope,
    ConversationComment,
    InlineComment,
    ReviewState,
    ReviewThread,
    SubmittedReview,
    ThreadComment,
    ThreadPage,
    ThreadRoot,
)
from uv_automations.github import decode_pull_request_details
from uv_automations.github_actions import ActionsGitHub
from uv_automations.json import (
    as_array,
    as_boolean,
    as_object,
    as_positive_integer,
    as_string,
)
from uv_automations.models import (
    PullRequestDetails,
    RepositoryIdentity,
    RepositoryName,
    Timestamp,
)

REVIEW_FIELDS = """
    fullDatabaseId author { login __typename } authorAssociation body updatedAt state
"""
THREAD_FIELDS = f"""
    id isResolved isOutdated path
    repository {{ nameWithOwner databaseId }}
    pullRequest {{ number repository {{ nameWithOwner databaseId }} }}
    comments(first: {MAX_THREAD_COMMENTS}) {{
        pageInfo {{ hasNextPage }}
        nodes {{ fullDatabaseId author {{ login __typename }} authorAssociation body updatedAt }}
    }}
"""


@dataclass(frozen=True, slots=True)
class CommentPullRequest:
    details: PullRequestDetails
    draft: bool
    event_json: str


def _rest_author(data: dict[str, object]) -> CommentAuthor:
    user = data["user"]
    author = as_object(user) if user is not None else None
    return CommentAuthor(
        as_string(author["login"]) if author is not None else None,
        ActorKind(as_string(author["type"])) if author is not None else None,
        AuthorAssociation(as_string(data["author_association"])),
    )


def _graphql_author(data: dict[str, object]) -> CommentAuthor:
    value = data["author"]
    author = as_object(value) if value is not None else None
    return CommentAuthor(
        as_string(author["login"]) if author is not None else None,
        ActorKind(as_string(author["__typename"])) if author is not None else None,
        AuthorAssociation(as_string(data["authorAssociation"])),
    )


def _database_id(value: object) -> int:
    # GraphQL's BigInt scalar is a decimal string, not the deprecated 32-bit Int.
    identifier = as_string(value)
    if re.fullmatch(r"[1-9][0-9]*", identifier) is None:
        raise ValueError("Expected a positive GitHub database ID")
    return int(identifier)


def _conversation_comment(value: object) -> ConversationComment:
    data = as_object(value)
    return ConversationComment(
        as_positive_integer(data["id"]),
        _rest_author(data),
        as_string(data["body"]),
        Timestamp.parse(as_string(data["updated_at"])),
    )


def _inline_comment(value: object) -> InlineComment:
    data = as_object(value)
    identifier = as_positive_integer(data["id"])
    root = data.get("in_reply_to_id")
    return InlineComment(
        identifier,
        as_positive_integer(root) if root is not None else identifier,
        _rest_author(data),
        as_string(data["body"]),
        Timestamp.parse(as_string(data["updated_at"])),
        as_string(data["path"]),
        as_string(data["diff_hunk"]),
    )


def _submitted_review(value: object) -> SubmittedReview:
    data = as_object(value)
    return SubmittedReview(
        _database_id(data["fullDatabaseId"]),
        _graphql_author(data),
        as_string(data["body"]),
        Timestamp.parse(as_string(data["updatedAt"])),
        ReviewState(as_string(data["state"])),
    )


def _graphql_repository(value: object) -> RepositoryIdentity:
    data = as_object(value)
    return RepositoryIdentity(
        RepositoryName(as_string(data["nameWithOwner"])),
        as_positive_integer(data["databaseId"]),
    )


def _check_graphql_pull_request(
    scope: CommentScope, value: object
) -> dict[str, object]:
    pull_request = as_object(value)
    if (
        as_positive_integer(pull_request["number"]) != scope.number
        or _graphql_repository(pull_request["repository"]) != scope.repository
    ):
        raise ValueError("GitHub returned a different pull request")
    return pull_request


def _review_thread(scope: CommentScope, value: object) -> ReviewThread:
    data = as_object(value)
    if _graphql_repository(data["repository"]) != scope.repository:
        raise ValueError("The review thread belongs to a different repository")
    _check_graphql_pull_request(scope, data["pullRequest"])
    comments = as_object(data["comments"])
    if as_boolean(as_object(comments["pageInfo"])["hasNextPage"]):
        raise ValueError("The review thread exceeds the complete-context limit")
    result: list[ThreadComment] = []
    for item in as_array(comments["nodes"]):
        comment = as_object(item)
        result.append(
            ThreadComment(
                _database_id(comment["fullDatabaseId"]),
                _graphql_author(comment),
                as_string(comment["body"]),
                Timestamp.parse(as_string(comment["updatedAt"])),
            )
        )
    return ReviewThread(
        as_string(data["id"]),
        as_boolean(data["isResolved"]),
        as_boolean(data["isOutdated"]),
        as_string(data["path"]),
        tuple(result),
    )


class CommentGitHub(ActionsGitHub):
    def _graphql(self, query: str, **variables: object) -> dict[str, object]:
        result = as_object(
            self._api(
                "POST", "graphql", payload={"query": query, "variables": variables}
            )
        )
        if result.get("errors"):
            raise ValueError("The GitHub GraphQL request was not successful")
        return as_object(result["data"])

    def _pages(self, path: str, **parameters: str) -> tuple[object, ...]:
        values: list[object] = []
        for page in range(1, MAX_COLLECTION_PAGES + 1):
            query = urlencode({**parameters, "per_page": MAX_PAGE_SIZE, "page": page})
            batch = as_array(self._api("GET", f"{path}?{query}"))
            if len(batch) > MAX_PAGE_SIZE:
                raise ValueError("GitHub returned an oversized comment page")
            values.extend(batch)
            if len(batch) < MAX_PAGE_SIZE:
                return tuple(values)
        raise ValueError("Comment history exceeds the bounded collection budget")

    def get_comment_pull_request(self, scope: CommentScope) -> CommentPullRequest:
        data = as_object(
            self._api("GET", f"repos/{scope.repository.name}/pulls/{scope.number}")
        )
        return CommentPullRequest(
            decode_pull_request_details(data, scope.reference),
            as_boolean(data["draft"]),
            json.dumps(data, allow_nan=False),
        )

    def list_conversation_comments(
        self, scope: CommentScope, since: Timestamp | None
    ) -> tuple[ConversationComment, ...]:
        parameters = {"since": str(since)} if since is not None else {}
        return tuple(
            _conversation_comment(value)
            for value in self._pages(
                f"repos/{scope.repository.name}/issues/{scope.number}/comments",
                **parameters,
            )
        )

    def list_inline_comments(
        self, scope: CommentScope, since: Timestamp | None
    ) -> tuple[InlineComment, ...]:
        parameters = {"sort": "updated", "direction": "asc"}
        if since is not None:
            parameters["since"] = str(since)
        return tuple(
            _inline_comment(value)
            for value in self._pages(
                f"repos/{scope.repository.name}/pulls/{scope.number}/comments",
                **parameters,
            )
        )

    def list_reviews(self, scope: CommentScope) -> tuple[SubmittedReview, ...]:
        owner, name = str(scope.repository.name).split("/", 1)
        cursor: str | None = None
        reviews: list[SubmittedReview] = []
        query = f"""
            query($owner: String!, $name: String!, $number: Int!, $cursor: String) {{
                repository(owner: $owner, name: $name) {{
                    pullRequest(number: $number) {{
                        number repository {{ nameWithOwner databaseId }}
                        reviews(first: {MAX_PAGE_SIZE}, after: $cursor) {{
                            pageInfo {{ hasNextPage endCursor }}
                            nodes {{ {REVIEW_FIELDS} }}
                        }}
                    }}
                }}
            }}
        """
        for _ in range(MAX_COLLECTION_PAGES):
            data = self._graphql(
                query, owner=owner, name=name, number=scope.number, cursor=cursor
            )
            pull_request = _check_graphql_pull_request(
                scope, as_object(data["repository"])["pullRequest"]
            )
            connection = as_object(pull_request["reviews"])
            batch = as_array(connection["nodes"])
            if len(batch) > MAX_PAGE_SIZE:
                raise ValueError("GitHub returned an oversized review page")
            reviews.extend(_submitted_review(value) for value in batch)
            if len({review.identifier for review in reviews}) != len(reviews):
                raise ValueError("GitHub returned duplicate pull request reviews")
            page = as_object(connection["pageInfo"])
            if not as_boolean(page["hasNextPage"]):
                return tuple(reviews)
            next_cursor = as_string(page["endCursor"])
            if not next_cursor or next_cursor == cursor:
                raise ValueError("GitHub returned a non-advancing review cursor")
            cursor = next_cursor
        raise ValueError("Pull request reviews exceed the bounded collection budget")

    def list_thread_roots(self, scope: CommentScope, cursor: str | None) -> ThreadPage:
        owner, name = str(scope.repository.name).split("/", 1)
        data = self._graphql(
            f"""
                query($owner: String!, $name: String!, $number: Int!, $cursor: String) {{
                    repository(owner: $owner, name: $name) {{
                        pullRequest(number: $number) {{
                            number repository {{ nameWithOwner databaseId }}
                            reviewThreads(first: {MAX_PAGE_SIZE}, after: $cursor) {{
                                pageInfo {{ hasNextPage endCursor }}
                                nodes {{ id comments(first: 1) {{ nodes {{ fullDatabaseId }} }} }}
                            }}
                        }}
                    }}
                }}
            """,
            owner=owner,
            name=name,
            number=scope.number,
            cursor=cursor,
        )
        pull_request = _check_graphql_pull_request(
            scope, as_object(data["repository"])["pullRequest"]
        )
        connection = as_object(pull_request["reviewThreads"])
        roots: list[ThreadRoot] = []
        for value in as_array(connection["nodes"]):
            thread = as_object(value)
            comments = as_array(as_object(thread["comments"])["nodes"])
            if len(comments) != 1:
                raise ValueError("Expected one root comment per review thread")
            roots.append(
                ThreadRoot(
                    _database_id(as_object(comments[0])["fullDatabaseId"]),
                    as_string(thread["id"]),
                )
            )
        page = as_object(connection["pageInfo"])
        end_cursor = page["endCursor"]
        return ThreadPage(
            tuple(roots),
            as_string(end_cursor) if end_cursor is not None else None,
            as_boolean(page["hasNextPage"]),
        )

    def get_review_threads(
        self, scope: CommentScope, identifiers: tuple[str, ...]
    ) -> tuple[ReviewThread, ...]:
        if len(identifiers) > MAX_SELECTED_THREADS or len(set(identifiers)) != len(
            identifiers
        ):
            raise ValueError("Too many or duplicate selected review threads")
        if not identifiers:
            return ()
        data = self._graphql(
            f"""
                query($identifiers: [ID!]!) {{
                    nodes(ids: $identifiers) {{
                        ... on PullRequestReviewThread {{ {THREAD_FIELDS} }}
                    }}
                }}
            """,
            identifiers=identifiers,
        )
        threads = tuple(
            _review_thread(scope, value) for value in as_array(data["nodes"])
        )
        if tuple(thread.identifier for thread in threads) != identifiers:
            raise ValueError("GitHub returned different review thread nodes")
        return threads

    def get_conversation_comment(
        self, scope: CommentScope, identifier: int
    ) -> ConversationComment:
        as_positive_integer(identifier)
        data = as_object(
            self._api(
                "GET", f"repos/{scope.repository.name}/issues/comments/{identifier}"
            )
        )
        if (
            as_positive_integer(data["id"]) != identifier
            or as_string(data["issue_url"])
            != f"https://api.github.com/repos/{scope.repository.name}/issues/{scope.number}"
        ):
            raise ValueError(
                "The conversation comment belongs to a different pull request"
            )
        return _conversation_comment(data)

    def get_review(self, scope: CommentScope, identifier: int) -> SubmittedReview:
        as_positive_integer(identifier)
        review = as_object(
            self._api(
                "GET",
                f"repos/{scope.repository.name}/pulls/{scope.number}/reviews/{identifier}",
            )
        )
        if (
            as_positive_integer(review["id"]) != identifier
            or as_string(review["pull_request_url"])
            != f"https://api.github.com/repos/{scope.repository.name}/pulls/{scope.number}"
        ):
            raise ValueError("The review belongs to a different pull request")
        data = self._graphql(
            f"""
                query($identifier: ID!) {{
                    node(id: $identifier) {{
                        ... on PullRequestReview {{
                            {REVIEW_FIELDS}
                            pullRequest {{ number repository {{ nameWithOwner databaseId }} }}
                        }}
                    }}
                }}
            """,
            identifier=as_string(review["node_id"]),
        )
        node = as_object(data["node"])
        _check_graphql_pull_request(scope, node["pullRequest"])
        result = _submitted_review(node)
        if result.identifier != identifier:
            raise ValueError("GitHub returned a different review")
        return result

    def post_conversation_comment(self, scope: CommentScope, body: str) -> None:
        self._api(
            "POST",
            f"repos/{scope.repository.name}/issues/{scope.number}/comments",
            payload={"body": body},
        )

    def reply_to_review_thread(self, identifier: str, body: str) -> None:
        self._graphql(
            """
                mutation($identifier: ID!, $body: String!) {
                    addPullRequestReviewThreadReply(input: {
                        pullRequestReviewThreadId: $identifier, body: $body
                    }) { comment { id } }
                }
            """,
            identifier=identifier,
            body=body,
        )

    def resolve_review_thread(self, identifier: str) -> None:
        data = self._graphql(
            """
                mutation($identifier: ID!) {
                    resolveReviewThread(input: {threadId: $identifier}) {
                        thread { id isResolved }
                    }
                }
            """,
            identifier=identifier,
        )
        thread = as_object(as_object(data["resolveReviewThread"])["thread"])
        if as_string(thread["id"]) != identifier or thread["isResolved"] is not True:
            raise ValueError("GitHub did not resolve the expected review thread")
