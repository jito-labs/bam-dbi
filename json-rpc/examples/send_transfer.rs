use std::{env, error::Error, io};

use reqwest::blocking::Client;
use serde_json::{json, Value};
use solana_hash::Hash;
use solana_keypair::{read_keypair_file, Signer};
use solana_message::{v0, VersionedMessage};
use solana_system_interface::instruction::transfer;
use solana_transaction::versioned::VersionedTransaction;

fn main() -> Result<(), Box<dyn Error>> {
    let payer_path = required("PAYER_KEYPAIR")?;
    let payer = read_keypair_file(&payer_path)
        .map_err(|error| io::Error::other(format!("read payer keypair {payer_path}: {error}")))?;
    let recipient = match env::var("RECIPIENT") {
        Ok(value) if !value.is_empty() => value.parse()?,
        _ => payer.pubkey(),
    };
    let lamports = env::var("LAMPORTS")
        .unwrap_or_else(|_| "1".to_owned())
        .parse()?;

    let rpc_url = required("SOLANA_RPC_URL")?;
    let blockhash = latest_blockhash(&Client::new(), &rpc_url)?;
    let message = v0::Message::try_compile(
        &payer.pubkey(),
        &[transfer(&payer.pubkey(), &recipient, lamports)],
        &[],
        blockhash,
    )?;
    let transaction = VersionedTransaction::try_new(VersionedMessage::V0(message), &[&payer])?;
    let wire_transaction = wincode::serialize(&transaction)?;
    let signature = transaction.signatures[0].to_string();

    println!("transaction_signature={signature}");

    let bundle_id = bam_json_rpc::send_bundle(
        &required("BAM_BUNDLE_URL")?,
        &required("JITO_AUTH_UUID")?,
        &[wire_transaction],
    )?;
    println!("bundle_id={bundle_id}");
    println!("bundle_id_hex={}", bam_json_rpc::bundle_id_hex(&bundle_id)?);
    Ok(())
}

fn latest_blockhash(client: &Client, url: &str) -> Result<Hash, Box<dyn Error>> {
    let response: Value = client
        .post(url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getLatestBlockhash",
            "params": []
        }))
        .send()?
        .error_for_status()?
        .json()?;
    if let Some(error) = response.get("error") {
        return Err(io::Error::other(format!("Solana RPC error: {error}")).into());
    }
    response
        .pointer("/result/value/blockhash")
        .and_then(Value::as_str)
        .ok_or_else(|| io::Error::other("Solana RPC response did not contain a blockhash"))?
        .parse()
        .map_err(Into::into)
}

fn required(name: &str) -> Result<String, io::Error> {
    env::var(name).map_err(|_| io::Error::other(format!("{name} is required")))
}
