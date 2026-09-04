#!/usr/bin/env python3
"""End-to-end privacy regression checks against an isolated LumiChat instance."""

import json
import sys
import urllib.error
import urllib.request


BASE = sys.argv[1].rstrip("/") if len(sys.argv) > 1 else "http://127.0.0.1:18084"


def request(method, path, token=None, payload=None, expected=200, raw=None, content_type=None):
    headers = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    data = raw
    if payload is not None:
        data = json.dumps(payload).encode()
        headers["Content-Type"] = "application/json"
    if content_type:
        headers["Content-Type"] = content_type
    try:
        with urllib.request.urlopen(
            urllib.request.Request(BASE + "/api" + path, data=data, headers=headers, method=method)
        ) as response:
            status = response.status
            body = response.read()
    except urllib.error.HTTPError as error:
        status = error.code
        body = error.read()
    assert status == expected, f"{method} {path}: expected {expected}, got {status}: {body!r}"
    return json.loads(body) if body else None


def register(username, display):
    return request("POST", "/register", payload={"username": username, "password": "test-pass-123", "display_name": display})


def multipart_file(filename, data):
    boundary = "----lumichat-regression-boundary"
    body = (
        f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n"
        "Content-Type: image/png\r\n\r\n"
    ).encode() + data + f"\r\n--{boundary}--\r\n".encode()
    return body, f"multipart/form-data; boundary={boundary}"


def main():
    register("test_admin", "Test Admin")
    a, b, c = register("alice_exact", "Alice"), register("bob_exact", "Bob"), register("carol_exact", "Carol")
    ta, tb, tc = a["token"], b["token"], c["token"]
    aid, bid, cid = a["user"]["id"], b["user"]["id"], c["user"]["id"]

    assert request("GET", "/friends", ta) == []
    request("GET", "/users", tb, expected=404)
    request("POST", "/friends/lookup", ta, {"query": "bob"}, expected=404)
    exact = request("POST", "/friends/lookup", ta, {"query": "bob_exact"})
    assert exact["user"]["id"] == bid and exact["relationship"] == "none"
    assert request("POST", "/friends/lookup", ta, {"query": str(bid)})["user"]["id"] == bid
    request("POST", "/friends/lookup", ta, {"query": "alice_exact"}, expected=404)

    created = request("POST", "/friend-requests", ta, {"identifier": "bob_exact"})
    request("POST", "/friend-requests", ta, {"identifier": "bob_exact"}, expected=400)
    incoming = request("GET", "/friend-requests", tb)["incoming"]
    assert incoming[0]["id"] == created["id"] and incoming[0]["user"]["id"] == aid
    request("POST", f"/friend-requests/{created['id']}/accept", tb)
    assert [u["id"] for u in request("GET", "/friends", ta)] == [bid]
    assert [u["id"] for u in request("GET", "/friends", tb)] == [aid]

    request("POST", f"/dm/{bid}/messages", ta, {"body": "private hello", "file_url": None})
    upload_body, upload_type = multipart_file("pixel.png", b"\x89PNG\r\n\x1a\n")
    uploaded = request("POST", "/upload", ta, raw=upload_body, content_type=upload_type)
    request("POST", f"/dm/{bid}/messages", ta, {"body": "private image", "file_url": uploaded["url"]})
    history = request("GET", f"/dm/{bid}/messages", ta)
    assert len(history) == 2 and any(row["file_url"] for row in history)

    bc = request("POST", "/friend-requests", tb, {"identifier": "carol_exact"})
    request("POST", f"/friend-requests/{bc['id']}/accept", tc)
    assert [u["id"] for u in request("GET", "/friends", ta)] == [bid]
    request("GET", f"/users/{cid}", ta, expected=404)
    request("POST", f"/dm/{cid}/messages", ta, {"body": "must fail", "file_url": None}, expected=404)

    invite1 = request("GET", "/friend-invite", tc)
    assert len(invite1["token"]) == 48 and invite1["path"] == f"/invite/{invite1['token']}"
    assert request("POST", "/friends/lookup", ta, {"invite_token": invite1["token"]})["user"]["id"] == cid
    invite2 = request("POST", "/friend-invite/regenerate", tc)
    assert invite1["token"] != invite2["token"]
    request("POST", "/friends/lookup", ta, {"invite_token": invite1["token"]}, expected=404)

    request("DELETE", f"/friends/{bid}", ta)
    assert request("GET", "/friends", ta) == []
    assert len(request("GET", f"/dm/{bid}/messages", ta)) == 2
    request("POST", f"/dm/{bid}/messages", ta, {"body": "must fail after removal", "file_url": None}, expected=404)
    request("GET", f"/users/{bid}", ta, expected=404)

    request("POST", f"/friends/{cid}", tb)
    assert request("GET", "/friends", tb) == []
    request("POST", "/friends/lookup", tc, {"query": "bob_exact"}, expected=404)
    request("POST", f"/dm/{bid}/messages", tc, {"body": "blocked", "file_url": None}, expected=404)

    print(json.dumps({"status": "ok", "users": [aid, bid, cid], "checks": 29}))


if __name__ == "__main__":
    main()
