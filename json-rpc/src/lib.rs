use std::{error::Error, fmt::Write as _, io, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::redirect::Policy;
use serde_json::{json, Value};

const MAX_TRANSACTIONS: usize = 5;
const MAX_TRANSACTION_SIZE: usize = 4_096;

/// Submit one bundle of signed wire-format transactions and return its bundle ID.
pub fn send_bundle(
    url: &str,
    credential: &str,
    transactions: &[Vec<u8>],
) -> Result<String, Box<dyn Error>> {
    let request = send_bundle_request(transactions).map_err(io::Error::other)?;
    let response: Value = reqwest::blocking::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(30))
        .build()?
        .post(url)
        .header("x-jito-auth", credential)
        .json(&request)
        .send()?
        .error_for_status()?
        .json()?;

    bundle_id(response)
}

pub fn bundle_id_hex(bundle_id: &str) -> Result<String, Box<dyn Error>> {
    let raw_bundle_id = STANDARD
        .decode(bundle_id)
        .map_err(|error| io::Error::other(format!("BAM bundle ID is not valid base64: {error}")))?;
    if raw_bundle_id.len() != 32 {
        return Err(io::Error::other("BAM bundle ID must decode to 32 bytes").into());
    }

    let mut hex_bundle_id = String::with_capacity(64);
    for byte in raw_bundle_id {
        write!(&mut hex_bundle_id, "{byte:02x}")?;
    }
    Ok(hex_bundle_id)
}

fn send_bundle_request(transactions: &[Vec<u8>]) -> Result<Value, &'static str> {
    if !(1..=MAX_TRANSACTIONS).contains(&transactions.len()) {
        return Err("expected 1 to 5 transactions");
    }
    if transactions
        .iter()
        .any(|transaction| !(1..=MAX_TRANSACTION_SIZE).contains(&transaction.len()))
    {
        return Err("transactions must contain 1 to 4096 bytes");
    }

    Ok(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendBundle",
        "params": [
            transactions
                .iter()
                .map(|transaction| STANDARD.encode(transaction))
                .collect::<Vec<_>>(),
            {"encoding": "base64"}
        ],
    }))
}

fn bundle_id(response: Value) -> Result<String, Box<dyn Error>> {
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(io::Error::other(format!("BAM JSON-RPC error: {error}")).into());
    }
    response
        .get("result")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other("BAM response did not contain a bundle ID").into())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        thread,
    };

    use solana_hash::Hash;
    use solana_keypair::{Keypair, Signer};
    use solana_message::{v0, v1, VersionedMessage};
    use solana_system_interface::instruction::transfer;
    use solana_transaction::versioned::VersionedTransaction;

    use super::*;

    fn transaction(version: u8) -> Vec<u8> {
        let payer = Keypair::new();
        let instruction = transfer(&payer.pubkey(), &payer.pubkey(), 1);
        let message = match version {
            0 => VersionedMessage::V0(
                v0::Message::try_compile(
                    &payer.pubkey(),
                    &[instruction],
                    &[],
                    Hash::new_from_array([7; 32]),
                )
                .unwrap(),
            ),
            1 => VersionedMessage::V1(
                v1::Message::try_compile(
                    &payer.pubkey(),
                    &[instruction],
                    Hash::new_from_array([7; 32]),
                )
                .unwrap(),
            ),
            _ => unreachable!(),
        };
        let transaction = VersionedTransaction::try_new(message, &[&payer]).unwrap();
        wincode::serialize(&transaction).unwrap()
    }

    #[test]
    fn builds_requests_with_v0_and_v1_transactions() {
        let transactions = vec![transaction(0), transaction(1)];
        let request = send_bundle_request(&transactions).unwrap();

        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["method"], "sendBundle");
        assert_eq!(request["params"][1]["encoding"], "base64");
        assert_eq!(
            request["params"][0]
                .as_array()
                .unwrap()
                .iter()
                .map(|encoded| STANDARD.decode(encoded.as_str().unwrap()).unwrap())
                .collect::<Vec<_>>(),
            transactions
        );

        assert!(send_bundle_request(&[]).is_err());
        assert!(send_bundle_request(&vec![vec![1]; 6]).is_err());
        assert!(send_bundle_request(&[vec![]]).is_err());
        assert!(send_bundle_request(&[vec![0; 4_097]]).is_err());
    }

    #[test]
    fn sends_authenticated_bundle_and_reads_id() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let transaction = transaction(0);
        let expected_transaction = transaction.clone();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (headers, body) = read_request(&mut stream);
            let headers = headers.to_ascii_lowercase();
            assert!(headers.starts_with("post /api/v1/bundles http/1.1\r\n"));
            assert!(headers.contains("\r\nx-jito-auth: test-credential\r\n"));

            let body: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["method"], "sendBundle");
            assert_eq!(
                STANDARD
                    .decode(body["params"][0][0].as_str().unwrap())
                    .unwrap(),
                expected_transaction
            );

            let body = r#"{"jsonrpc":"2.0","id":1,"result":"bundle-id"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let id = send_bundle(
            &format!("http://{address}/api/v1/bundles"),
            "test-credential",
            &[transaction],
        )
        .unwrap();
        assert_eq!(id, "bundle-id");
        server.join().unwrap();
    }

    #[test]
    fn surfaces_json_rpc_errors() {
        let error = bundle_id(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -32602, "message": "invalid params"}
        }))
        .unwrap_err();

        assert!(error.to_string().contains("invalid params"));
    }

    #[test]
    fn converts_bundle_id_to_hex() {
        let raw_bundle_id = (0u8..32).collect::<Vec<_>>();
        let encoded_bundle_id = STANDARD.encode(&raw_bundle_id);

        assert_eq!(
            bundle_id_hex(&encoded_bundle_id).unwrap(),
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
        );
        assert!(bundle_id_hex("not-base64").is_err());
        assert!(bundle_id_hex(&STANDARD.encode([0; 31])).is_err());
    }

    fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
        let mut request = Vec::new();
        let mut buffer = [0; 1_024];
        let header_end = loop {
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0, "connection closed before request headers");
            request.extend_from_slice(&buffer[..read]);
            if let Some(index) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                break index;
            }
        };
        let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        let body_start = header_end + 4;
        while request.len() < body_start + content_length {
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0, "connection closed before request body");
            request.extend_from_slice(&buffer[..read]);
        }
        (
            headers,
            request[body_start..body_start + content_length].to_vec(),
        )
    }
}
