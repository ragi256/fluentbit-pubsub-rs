#!/usr/bin/env python3
import json
import os
import sys
import urllib.error
import urllib.request


def call(method: str, path: str, payload: dict | None = None) -> dict:
    host = os.environ.get("PUBSUB_EMULATOR_HOST", "pubsub-emulator:8681")
    url = f"http://{host}{path}"
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"

    req = urllib.request.Request(url=url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            raw = resp.read().decode("utf-8")
            if not raw:
                return {}
            return json.loads(raw)
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {e.code} {e.reason}: {body}") from e


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: pubsub_emulator_api.py <create-topic|create-subscription|pull> ...", file=sys.stderr)
        return 2

    cmd = sys.argv[1]
    try:
        if cmd == "ping":
            host = os.environ.get("PUBSUB_EMULATOR_HOST", "pubsub-emulator:8681")
            url = f"http://{host}/"
            req = urllib.request.Request(url=url, method="GET")
            try:
                with urllib.request.urlopen(req, timeout=5):
                    pass
            except urllib.error.HTTPError:
                # Any HTTP response means emulator is reachable.
                pass
            return 0

        if cmd == "create-topic":
            if len(sys.argv) != 4:
                print("usage: create-topic <project> <topic>", file=sys.stderr)
                return 2
            project, topic = sys.argv[2], sys.argv[3]
            call("PUT", f"/v1/projects/{project}/topics/{topic}")
            return 0

        if cmd == "create-subscription":
            if len(sys.argv) != 5:
                print("usage: create-subscription <project> <subscription> <topic>", file=sys.stderr)
                return 2
            project, sub, topic = sys.argv[2], sys.argv[3], sys.argv[4]
            call(
                "PUT",
                f"/v1/projects/{project}/subscriptions/{sub}",
                {"topic": f"projects/{project}/topics/{topic}"},
            )
            return 0

        if cmd == "pull":
            if len(sys.argv) != 4:
                print("usage: pull <project> <subscription>", file=sys.stderr)
                return 2
            project, sub = sys.argv[2], sys.argv[3]
            body = call(
                "POST",
                f"/v1/projects/{project}/subscriptions/{sub}:pull",
                {"maxMessages": 1, "returnImmediately": True},
            )
            msgs = body.get("receivedMessages", [])
            if not msgs:
                print("")
                return 0
            print(msgs[0].get("message", {}).get("data", ""))
            return 0

        print(f"unknown command: {cmd}", file=sys.stderr)
        return 2
    except Exception as e:
        print(str(e), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
