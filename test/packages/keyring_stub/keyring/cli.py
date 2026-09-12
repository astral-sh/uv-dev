import json
import os
import sys


def credentials():
    return json.loads(os.environ.get("KEYRING_TEST_CREDENTIALS", "{}"))


def get_password(service, username):
    print(f"Keyring request for {username}@{service}", file=sys.stderr)
    password = credentials().get(service, {}).get(username)
    if password is None:
        return 1
    print(password)
    return 0


def get_credential(service):
    if os.environ.get("KEYRING_TEST_UNSUPPORTED_CREDENTIALS_MODE"):
        print("unrecognized arguments: --mode creds", file=sys.stderr)
        return 2
    print(f"Keyring request for {service}", file=sys.stderr)
    service_credentials = credentials().get(service, {})
    if not service_credentials:
        return 1
    username, password = next(iter(service_credentials.items()))
    print(username)
    print(password)
    return 0


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "get":
        return get_password(sys.argv[2], sys.argv[3])
    if len(sys.argv) == 5 and sys.argv[1] == "get" and sys.argv[3:] == [
        "--mode",
        "creds",
    ]:
        return get_credential(sys.argv[2])
    return 2
