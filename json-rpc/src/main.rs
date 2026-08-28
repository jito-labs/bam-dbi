use std::{env, error::Error, io};

use base64::{engine::general_purpose::STANDARD, Engine};

fn main() -> Result<(), Box<dyn Error>> {
    let transactions = env::args()
        .skip(1)
        .enumerate()
        .map(|(index, transaction)| {
            STANDARD.decode(transaction).map_err(|error| {
                io::Error::other(format!(
                    "transaction {} is not valid base64: {error}",
                    index + 1
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let url =
        env::var("BAM_BUNDLE_URL").map_err(|_| io::Error::other("BAM_BUNDLE_URL is required"))?;
    let credential =
        env::var("JITO_AUTH_UUID").map_err(|_| io::Error::other("JITO_AUTH_UUID is required"))?;

    let bundle_id = bam_json_rpc::send_bundle(&url, &credential, &transactions)?;
    println!("bundle_id={bundle_id}");
    println!("bundle_id_hex={}", bam_json_rpc::bundle_id_hex(&bundle_id)?);
    Ok(())
}
