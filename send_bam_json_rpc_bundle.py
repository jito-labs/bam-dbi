#!/usr/bin/env python3
"""Submit one bundle of base64-encoded signed transactions to Jito BAM."""

import argparse
import base64
import binascii
import json
import os
from http.client import HTTPException
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

MAX_TXS_PER_BUNDLE = 5
STANDARD_PACKET_SIZE = 1232


class NoRedirectHandler(HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, url):
        return None


HTTP_OPENER = build_opener(NoRedirectHandler)


def read_response_body(response) -> bytes:
    try:
        return response.read()
    except (HTTPException, OSError) as error:
        raise RuntimeError(f"could not read BAM response: {error}") from error


def validate_transactions(transactions: list[str]) -> list[str]:
    if not 1 <= len(transactions) <= MAX_TXS_PER_BUNDLE:
        raise ValueError("a bundle must contain 1 to 5 transactions")

    for index, transaction in enumerate(transactions, start=1):
        try:
            decoded = base64.b64decode(transaction, validate=True)
        except (binascii.Error, ValueError) as error:
            raise ValueError(f"transaction {index} must be valid base64") from error
        if not decoded:
            raise ValueError(f"transaction {index} must be non-empty")
        if len(decoded) > STANDARD_PACKET_SIZE:
            raise ValueError(
                f"transaction {index} exceeds the standard 1232-byte packet size"
            )
    return transactions


def build_request(transactions: list[str]) -> dict:
    return {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendBundle",
        "params": [validate_transactions(transactions), {"encoding": "base64"}],
    }


def submit_bundle(
    url: str,
    auth_uuid: str,
    transactions: list[str],
    timeout: float,
) -> str:
    parsed_url = urlsplit(url)
    if parsed_url.scheme != "http" or not parsed_url.netloc:
        raise ValueError("BAM JSON-RPC URL must be an http:// URL")
    if not auth_uuid:
        raise ValueError("JITO_AUTH_UUID must not be empty")

    request = Request(
        url,
        data=json.dumps(build_request(transactions), separators=(",", ":")).encode(),
        headers={"content-type": "application/json", "x-jito-auth": auth_uuid},
        method="POST",
    )
    try:
        with HTTP_OPENER.open(request, timeout=timeout) as response:
            body = read_response_body(response)
    except HTTPError as error:
        try:
            detail = read_response_body(error).decode(errors="replace").strip()
        except RuntimeError:
            detail = ""
        message = detail or error.reason
        raise RuntimeError(f"BAM returned HTTP {error.code}: {message}") from error
    except (HTTPException, OSError) as error:
        reason = getattr(error, "reason", error)
        raise RuntimeError(f"could not reach BAM: {reason}") from error

    try:
        payload = json.loads(body)
    except ValueError as error:
        raise RuntimeError("BAM returned a non-JSON response") from error
    if not isinstance(payload, dict):
        raise RuntimeError("BAM returned an invalid JSON-RPC response")
    if payload.get("error") is not None:
        error = payload["error"]
        if isinstance(error, dict):
            raise RuntimeError(
                f"BAM JSON-RPC error {error.get('code')}: {error.get('message')}"
            )
        raise RuntimeError(f"BAM JSON-RPC error: {error}")
    bundle_id = payload.get("result")
    if not isinstance(bundle_id, str) or not bundle_id:
        raise RuntimeError("BAM response did not contain a bundle ID")
    return bundle_id


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Submit 1-5 base64 signed transactions as one Jito BAM bundle."
    )
    parser.add_argument(
        "--url",
        default=os.environ.get("BAM_BUNDLE_URL"),
        help="BAM bundle URL (or set BAM_BUNDLE_URL)",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=10.0,
        help="request timeout in seconds",
    )
    parser.add_argument(
        "transaction",
        nargs="+",
        help="base64 signed transaction, 1 to 5 values",
    )
    args = parser.parse_args()

    if not args.url:
        parser.error("--url or BAM_BUNDLE_URL is required")
    try:
        bundle_id = submit_bundle(
            args.url,
            os.environ.get("JITO_AUTH_UUID", ""),
            args.transaction,
            args.timeout,
        )
    except (RuntimeError, ValueError) as error:
        parser.error(str(error))

    print(f"bundle_id={bundle_id}")
    print("accepted_by_ingress=true (this does not guarantee the bundle will land)")


if __name__ == "__main__":
    main()
