use std::{env, error::Error, io, time::Duration};

use reqwest::redirect::Policy;
use serde_json::{json, Value};

const MAX_TRANSACTIONS: usize = 5;

fn main() -> Result<(), Box<dyn Error>> {
    let request = send_bundle_request(env::args().skip(1).collect()).map_err(io::Error::other)?;
    let url =
        env::var("BAM_BUNDLE_URL").map_err(|_| io::Error::other("BAM_BUNDLE_URL is required"))?;
    let credential =
        env::var("JITO_AUTH_UUID").map_err(|_| io::Error::other("JITO_AUTH_UUID is required"))?;

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

    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(io::Error::other(format!("BAM JSON-RPC error: {error}")).into());
    }
    let bundle_id = response
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| io::Error::other("BAM response did not contain a bundle ID"))?;

    println!("{bundle_id}");
    Ok(())
}

fn send_bundle_request(transactions: Vec<String>) -> Result<Value, &'static str> {
    if !(1..=MAX_TRANSACTIONS).contains(&transactions.len()) {
        return Err("expected 1 to 5 base64-encoded transactions");
    }

    Ok(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendBundle",
        "params": [transactions, {"encoding": "base64"}],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use solana_hash::Hash;
    use solana_keypair::{Keypair, Signer};
    use solana_message::{v1, VersionedMessage};
    use solana_system_interface::instruction::transfer;
    use solana_transaction::versioned::VersionedTransaction;

    #[test]
    fn builds_request_with_v1_transaction() {
        let payer = Keypair::new();
        let message = v1::Message::try_compile(
            &payer.pubkey(),
            &[transfer(&payer.pubkey(), &payer.pubkey(), 1)],
            Hash::new_from_array([7; 32]),
        )
        .unwrap();
        let transaction =
            VersionedTransaction::try_new(VersionedMessage::V1(message), &[&payer]).unwrap();
        let encoded_transaction = STANDARD.encode(wincode::serialize(&transaction).unwrap());

        assert_eq!(
            send_bundle_request(vec![encoded_transaction.clone()]).unwrap(),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "sendBundle",
                "params": [[encoded_transaction], {"encoding": "base64"}],
            })
        );
        assert!(send_bundle_request(Vec::new()).is_err());
        assert!(send_bundle_request(vec!["AQ==".to_owned(); 6]).is_err());
    }
}
