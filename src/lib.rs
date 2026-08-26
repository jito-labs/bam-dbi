use anyhow::{ensure, Context};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};

pub const MAX_TXS_PER_BUNDLE: usize = 5;
pub const STANDARD_PACKET_SIZE: usize = 1232;

pub fn decode_transactions(encoded_transactions: &[String]) -> anyhow::Result<Vec<Vec<u8>>> {
    ensure!(
        (1..=MAX_TXS_PER_BUNDLE).contains(&encoded_transactions.len()),
        "a bundle must contain 1 to 5 transactions"
    );

    encoded_transactions
        .iter()
        .enumerate()
        .map(|(index, encoded)| {
            let transaction = STANDARD
                .decode(encoded)
                .with_context(|| format!("transaction {} must be valid base64", index + 1))?;
            ensure!(
                !transaction.is_empty(),
                "transaction {} must be non-empty",
                index + 1
            );
            ensure!(
                transaction.len() <= STANDARD_PACKET_SIZE,
                "transaction {} exceeds the standard 1232-byte packet size",
                index + 1
            );
            Ok(transaction)
        })
        .collect()
}

pub fn send_bundle_request(encoded_transactions: &[String]) -> anyhow::Result<Value> {
    decode_transactions(encoded_transactions)?;
    Ok(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendBundle",
        "params": [encoded_transactions, {"encoding": "base64"}],
    }))
}

pub fn encode_bamb_v0(signed_transactions: &[Vec<u8>]) -> anyhow::Result<Vec<u8>> {
    ensure!(
        (1..=MAX_TXS_PER_BUNDLE).contains(&signed_transactions.len()),
        "a bundle must contain 1 to 5 transactions"
    );

    let mut frame =
        Vec::with_capacity(26 + signed_transactions.iter().map(Vec::len).sum::<usize>());
    frame.extend_from_slice(b"BAMB");
    frame.extend_from_slice(&[0, 0, signed_transactions.len() as u8, 26]);
    frame.extend_from_slice(&0u64.to_le_bytes());

    for transaction in signed_transactions {
        ensure!(!transaction.is_empty(), "transactions must be non-empty");
        ensure!(
            transaction.len() <= STANDARD_PACKET_SIZE,
            "serialized transaction exceeds the standard 1232-byte packet size"
        );
        frame.extend_from_slice(&(transaction.len() as u16).to_le_bytes());
    }
    for _ in signed_transactions.len()..MAX_TXS_PER_BUNDLE {
        frame.extend_from_slice(&0u16.to_le_bytes());
    }
    for transaction in signed_transactions {
        frame.extend_from_slice(transaction);
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rpc_request_is_base64_only() {
        let request = send_bundle_request(&["AQ==".to_owned()]).unwrap();
        assert_eq!(request["method"], "sendBundle");
        assert_eq!(request["params"], json!([["AQ=="], {"encoding": "base64"}]));
    }

    #[test]
    fn bamb_v0_frame_matches_wire_layout() {
        let frame = encode_bamb_v0(&[vec![1, 2], vec![3]]).unwrap();
        assert_eq!(&frame[..4], b"BAMB");
        assert_eq!(&frame[4..8], &[0, 0, 2, 26]);
        assert_eq!(&frame[8..16], &[0; 8]);
        assert_eq!(&frame[16..26], &[2, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&frame[26..], &[1, 2, 3]);
    }

    #[test]
    fn invalid_transactions_are_rejected() {
        for transactions in [
            vec![],
            vec!["AQ==".to_owned(); 6],
            vec!["not base64".to_owned()],
        ] {
            assert!(decode_transactions(&transactions).is_err());
        }
    }
}
