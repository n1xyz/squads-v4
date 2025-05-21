use clap::ArgMatches;
use colored::Colorize;
use eyre::eyre;
use solana_cli_config::Config;
use solana_sdk::{signature::{read_keypair_file, Keypair}, signer::Signer, transaction::VersionedTransaction};
use squads_multisig::solana_client::nonblocking::rpc_client::RpcClient;
use squads_multisig::solana_client::{
    client_error::ClientErrorKind,
    rpc_request::{RpcError, RpcResponseErrorData},
    rpc_response::RpcSimulateTransactionResult,
};
use std::path::Path;

pub fn create_signer_from_path(
    keypair_path: String,
) -> Result<Box<dyn Signer>, Box<dyn std::error::Error>> {
    // Check if it's a path to a keypair file
    if let Ok(keypair) = read_keypair_file(&keypair_path) {
        return Ok(Box::new(keypair));
    }

    // If it's a config path
    if keypair_path == "config" {
        let config_file = solana_cli_config::CONFIG_FILE.as_ref()
            .ok_or("Failed to find default config file")?;
        let config = Config::load(config_file).unwrap_or_default();
        let path = Path::new(&config.keypair_path);
        let keypair = read_keypair_file(path).map_err(|_| format!("Failed to read keypair from {}", config.keypair_path))?;
        return Ok(Box::new(keypair));
    }

    // Default location
    if keypair_path == "default" {
        let home_dir = dirs::home_dir().ok_or("Failed to find home directory")?;
        let path = home_dir.join(".config/solana/id.json");
        let keypair = read_keypair_file(&path).map_err(|_| format!("Failed to read keypair from {}", path.display()))?;
        return Ok(Box::new(keypair));
    }

    Err("Failed to load keypair".into())
}

pub async fn send_and_confirm_transaction(
    transaction: &VersionedTransaction,
    rpc_client: &RpcClient,
) -> eyre::Result<String> {
    // Try to send and confirm the transaction
    match rpc_client.send_and_confirm_transaction(transaction).await {
        Ok(signature) => {
            println!(
                "Transaction confirmed: {}\n\n",
                signature.to_string().green()
            );
            Ok(signature.to_string())
        }
        Err(err) => {
            if let ClientErrorKind::RpcError(RpcError::RpcResponseError {
                data:
                    RpcResponseErrorData::SendTransactionPreflightFailure(
                        RpcSimulateTransactionResult {
                            logs: Some(logs), ..
                        },
                    ),
                ..
            }) = &err.kind
            {
                println!("Simulation logs:\n\n{}\n", logs.join("\n").yellow());
            }

            Err(eyre!("Transaction failed: {}", err.to_string().red()))
        }
    }
}
