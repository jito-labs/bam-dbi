import json
import tempfile
import unittest
from http.client import BadStatusLine, IncompleteRead, RemoteDisconnected
from io import BytesIO
from pathlib import Path
from unittest.mock import MagicMock, patch

import send_bam_json_rpc_bundle as json_rpc
import send_bam_quic_bundle as quic


class ExampleTests(unittest.TestCase):
    def test_json_rpc_send_bundle_is_base64_only_and_authenticated(self):
        with patch.object(
            json_rpc.HTTP_OPENER,
            "open",
            return_value=BytesIO(
                b'{"jsonrpc":"2.0","id":1,"result":"example-bundle-id"}'
            ),
        ) as mocked_urlopen:
            bundle_id = json_rpc.submit_bundle(
                "http://example.test:9090/api/v1/bundles", "example-uuid", ["AQ=="], 1
            )

        request = mocked_urlopen.call_args.args[0]
        body = json.loads(request.data)
        self.assertEqual(bundle_id, "example-bundle-id")
        self.assertEqual(request.get_header("X-jito-auth"), "example-uuid")
        self.assertEqual(body["method"], "sendBundle")
        self.assertEqual(body["params"], [["AQ=="], {"encoding": "base64"}])

    def test_json_rpc_rejects_non_json_responses(self):
        with patch.object(
            json_rpc.HTTP_OPENER,
            "open",
            return_value=BytesIO(b"\xff"),
        ):
            with self.assertRaisesRegex(RuntimeError, "non-JSON response"):
                json_rpc.submit_bundle(
                    "http://example.test:9090/api/v1/bundles",
                    "example-uuid",
                    ["AQ=="],
                    1,
                )

    def test_json_rpc_rejects_redirects_and_handles_read_timeouts(self):
        handler = json_rpc.NoRedirectHandler()
        self.assertIsNone(
            handler.redirect_request(None, None, 302, "Found", {}, "http://other")
        )

        response = MagicMock()
        response.__enter__.return_value.read.side_effect = TimeoutError("timed out")
        with patch.object(
            json_rpc.HTTP_OPENER,
            "open",
            return_value=response,
        ):
            with self.assertRaisesRegex(RuntimeError, "timed out"):
                json_rpc.submit_bundle(
                    "http://example.test:9090/api/v1/bundles",
                    "example-uuid",
                    ["AQ=="],
                    1,
                )

    def test_json_rpc_normalizes_response_read_failures(self):
        response = MagicMock()
        response.__enter__.return_value.read.side_effect = IncompleteRead(b"{")
        with patch.object(
            json_rpc.HTTP_OPENER,
            "open",
            return_value=response,
        ):
            with self.assertRaisesRegex(RuntimeError, "could not read BAM response"):
                json_rpc.submit_bundle(
                    "http://example.test:9090/api/v1/bundles",
                    "example-uuid",
                    ["AQ=="],
                    1,
                )

        error_body = MagicMock()
        error_body.read.side_effect = TimeoutError("timed out")
        http_error = json_rpc.HTTPError(
            "http://example.test:9090/api/v1/bundles",
            503,
            "Service Unavailable",
            {},
            error_body,
        )
        with patch.object(
            json_rpc.HTTP_OPENER,
            "open",
            side_effect=http_error,
        ):
            with self.assertRaisesRegex(RuntimeError, "HTTP 503: Service Unavailable"):
                json_rpc.submit_bundle(
                    "http://example.test:9090/api/v1/bundles",
                    "example-uuid",
                    ["AQ=="],
                    1,
                )

    def test_json_rpc_normalizes_response_header_failures(self):
        for error in (BadStatusLine("bad status"), RemoteDisconnected("closed")):
            with self.subTest(error=error), patch.object(
                json_rpc.HTTP_OPENER,
                "open",
                side_effect=error,
            ):
                with self.assertRaisesRegex(RuntimeError, "could not reach BAM"):
                    json_rpc.submit_bundle(
                        "http://example.test:9090/api/v1/bundles",
                        "example-uuid",
                        ["AQ=="],
                        1,
                    )

    def test_bamb_v0_frame_layout(self):
        frame = quic.encode_bamb_v0([b"\x01\x02", b"\x03"])
        self.assertEqual(frame[:4], b"BAMB")
        self.assertEqual(frame[4:8], bytes([0, 0, 2, 26]))
        self.assertEqual(frame[8:16], bytes(8))
        self.assertEqual(frame[16:26], b"\x02\x00\x01\x00" + bytes(6))
        self.assertEqual(frame[26:], b"\x01\x02\x03")

    def test_quic_keypair_builds_a_loadable_certificate(self):
        private_key = quic.ed25519.Ed25519PrivateKey.generate()
        seed = private_key.private_bytes(
            quic.serialization.Encoding.Raw,
            quic.serialization.PrivateFormat.Raw,
            quic.serialization.NoEncryption(),
        )
        public_key = private_key.public_key().public_bytes(
            quic.serialization.Encoding.Raw,
            quic.serialization.PublicFormat.Raw,
        )
        with tempfile.TemporaryDirectory() as directory:
            keypair_path = Path(directory, "keypair.json")
            keypair_path.write_text(
                json.dumps(list(seed + public_key)),
                encoding="utf-8",
            )
            loaded_key = quic.read_quic_private_key(str(keypair_path))
            cert_path, key_path = quic.write_cert_chain(directory, loaded_key)
            configuration = quic.QuicConfiguration(is_client=True)
            configuration.load_cert_chain(cert_path, key_path)

    def test_rejects_invalid_input_before_network_io(self):
        for transactions in ([], ["AQ=="] * 6, ["not base64"]):
            with self.subTest(transactions=transactions), self.assertRaises(ValueError):
                json_rpc.build_request(transactions)


if __name__ == "__main__":
    unittest.main()
