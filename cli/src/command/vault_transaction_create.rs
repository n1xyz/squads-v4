use std::str::FromStr;
use std::time::Duration;

use clap::{Args, builder::TypedValueParser};
use colored::Colorize;
use dialoguer::Confirm;
use indicatif::ProgressBar;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::instruction::Instruction;
use solana_sdk::message::v0::Message;
use solana_sdk::message::VersionedMessage;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::VersionedTransaction;
use solana_sdk::bs58;
use serde_json::Value;
use squads_multisig::anchor_lang::AnchorSerialize;
use squads_multisig::squads_multisig_program::TransactionMessage;
use squads_multisig::squads_multisig_program::CompiledInstruction;
use squads_multisig::squads_multisig_program::MessageAddressTableLookup;

use squads_multisig::anchor_lang::InstructionData;
use squads_multisig::client::get_multisig;
use squads_multisig::pda::{get_proposal_pda, get_transaction_pda};
use squads_multisig::solana_client::nonblocking::rpc_client::RpcClient;
use squads_multisig::squads_multisig_program::accounts::ProposalCreate as ProposalCreateAccounts;
use squads_multisig::squads_multisig_program::accounts::VaultTransactionCreate as VaultTransactionCreateAccounts;
use squads_multisig::squads_multisig_program::anchor_lang::ToAccountMetas;
use squads_multisig::squads_multisig_program::instruction::ProposalCreate as ProposalCreateData;
use squads_multisig::squads_multisig_program::instruction::VaultTransactionCreate as VaultTransactionCreateData;
use squads_multisig::squads_multisig_program::ProposalCreateArgs;
use squads_multisig::squads_multisig_program::VaultTransactionCreateArgs;

use crate::utils::{create_signer_from_path, send_and_confirm_transaction};

#[derive(Clone)]
struct Base58Parser;

impl TypedValueParser for Base58Parser {
    type Value = Vec<u8>;

    fn parse_ref(&self, _cmd: &clap::Command, _arg: Option<&clap::Arg>, value: &std::ffi::OsStr) -> Result<Self::Value, clap::Error> {
        let s = value.to_str().ok_or_else(|| {
            clap::Error::raw(clap::error::ErrorKind::InvalidValue, "Invalid UTF-8 in base58 string")
        })?;

        bs58::decode(s).into_vec().map_err(|e| {
            clap::Error::raw(clap::error::ErrorKind::InvalidValue, format!("Invalid base58: {}", e))
        })
    }
}

// Helper struct for organizing transaction info
#[derive(Debug)]
struct TransactionInfo {
    instruction_type: String,
    mint: String,
    min_deposit: u64,
    asset_config: String,
    crumb_authority: String,
}

#[derive(Args)]
pub struct VaultTransactionCreate {
    /// RPC URL
    #[arg(long)]
    rpc_url: Option<String>,

    /// Multisig Program ID
    #[arg(long)]
    program_id: Option<String>,

    /// Path to the Program Config Initializer Keypair
    #[arg(long)]
    keypair: String,

    /// The multisig where the transaction has been proposed
    #[arg(long)]
    multisig_pubkey: String,

    #[arg(long)]
    vault_index: u8,

    /// Base58 encoded transaction message
    #[arg(long, value_parser = Base58Parser)]
    transaction_message: Option<Vec<u8>>,

    /// Optional JSON file containing transaction details (for rollman integration)
    #[arg(long)]
    transaction_json: Option<String>,

    /// Memo to be included in the transaction
    #[arg(long)]
    memo: Option<String>,

    #[arg(long)]
    priority_fee_lamports: Option<u64>,

    #[arg(long)]
    compute_unit_limit: Option<u32>,

    /// Skip confirmation prompt
    #[arg(long)]
    no_confirm: bool,
}

impl VaultTransactionCreate {
    pub async fn execute(self) -> eyre::Result<()> {
        let Self {
            rpc_url,
            program_id,
            keypair,
            multisig_pubkey,
            memo,
            mut transaction_message,
            transaction_json,
            vault_index,
            priority_fee_lamports,
            compute_unit_limit: _,
            no_confirm,
        } = self;

        // Get transaction message either from direct input or JSON file
        let (transaction_message, additional_info, ephemeral_signers_count) = if let Some(json_path) = transaction_json {
            let json_content = std::fs::read_to_string(json_path)?;
            let json: Value = serde_json::from_str(&json_content)?;

            // Extract transaction message
            let tx_message = json["transaction_message"]
                .as_str()
                .ok_or_else(|| eyre::eyre!("transaction_message not found in JSON"))?;

            println!("Debug: Transaction message length (base58): {}", tx_message.len());
            let decoded = bs58::decode(tx_message).into_vec()?;
            println!("Debug: Decoded message length (bytes): {}", decoded.len());

            // Extract ephemeral signers from JSON if available
            let ephemeral_signers_count = if let Some(ephemeral_signers) = json.get("ephemeral_signers") {
                if let Some(ephemeral_signers_array) = ephemeral_signers.as_array() {
                    println!("Debug: Found {} ephemeral signers:", ephemeral_signers_array.len());
                    for (i, signer) in ephemeral_signers_array.iter().enumerate() {
                        if let Some(signer_str) = signer.as_str() {
                            println!("  {}: {}", i, signer_str);
                        }
                    }
                    ephemeral_signers_array.len() as u8
                } else {
                    0
                }
            } else {
                // Look for PDAs that are marked as signers in asset_config or other fields
                let pda_signers_count = if let Some(pdas) = json.get("pdas") {
                    if let Some(asset_config) = pdas.get("asset_config") {
                        if asset_config.get("is_signer").is_some() || 
                           json.get("ephemeral_signers").is_some() {
                            1
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                } else {
                    0
                };
                pda_signers_count
            };

            // Debug message structure
            if let Ok(msg) = bincode::deserialize::<solana_sdk::message::Message>(&decoded) {
                println!("Debug: Legacy message format");
                println!("Debug: Number of accounts: {}", msg.account_keys.len());
                println!("Debug: Number of instructions: {}", msg.instructions.len());
                println!("Debug: First instruction program ID: {}", msg.account_keys[msg.instructions[0].program_id_index as usize]);
                println!("Debug: Account keys:");
                for (i, key) in msg.account_keys.iter().enumerate() {
                    println!("  {}: {}", i, key);
                }
                println!("Debug: Instruction accounts:");
                for (i, acc) in msg.instructions[0].accounts.iter().enumerate() {
                    println!("  {}: {} (index: {})", i, msg.account_keys[*acc as usize], acc);
                }

                // Special account check for asset_config PDA
                if let Some(pdas) = json.get("pdas") {
                    if let Some(asset_config) = pdas.get("asset_config") {
                        if let Some(address) = asset_config["address"].as_str() {
                            println!("Special account check: {}", address);
                            // Check if this account exists in the message keys
                            for (i, key) in msg.account_keys.iter().enumerate() {
                                if key.to_string() == address {
                                    println!("Special account found: {}", address);
                                    break;
                                }
                            }
                        }
                    }
                }

                // Convert Solana instructions to Squads instructions
                let squads_instructions: Vec<CompiledInstruction> = msg.instructions.iter().map(|ix| {
                    CompiledInstruction {
                        program_id_index: ix.program_id_index,
                        account_indexes: ix.accounts.clone().into(),
                        data: ix.data.clone().into(),
                    }
                }).collect();

                // Create Squads TransactionMessage
                let squads_msg = TransactionMessage {
                    num_signers: 1,
                    num_writable_signers: 1,
                    num_writable_non_signers: msg.header.num_required_signatures as u8 - 1,
                    account_keys: msg.account_keys.clone().into(),
                    instructions: squads_instructions.into(),
                    address_table_lookups: Vec::<MessageAddressTableLookup>::new().into(),
                };

                let msg_bytes = squads_msg.try_to_vec().unwrap();
                println!("Debug: Squads message length: {}", msg_bytes.len());
                transaction_message = Some(msg_bytes);
            } else {
                println!("Debug: Failed to deserialize message");
                println!("Debug: Raw bytes: {:?}", decoded);
            }

            // Get additional info for display
            let additional_info = Some(TransactionInfo {
                instruction_type: json["instruction"]["instruction_type"].as_str().unwrap_or("Unknown").to_string(),
                mint: json["instruction"]["parameters"]["mint"].as_str().unwrap_or("Unknown").to_string(),
                min_deposit: json["instruction"]["parameters"]["min_deposit"]
                    .as_u64()
                    .unwrap_or(0),
                asset_config: json["accounts"]["asset_config"].as_str().unwrap_or("Unknown").to_string(),
                crumb_authority: json["accounts"]["crumb_authority"].as_str().unwrap_or("Unknown").to_string(),
            });

            (transaction_message, additional_info, ephemeral_signers_count)
        } else if let Some(msg) = transaction_message {
            (Some(msg), None, 0)
        } else {
            return Err(eyre::eyre!("Either transaction_message or transaction_json must be provided"));
        };

        let program_id =
            program_id.unwrap_or_else(|| "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf".to_string());

        let program_id = Pubkey::from_str(&program_id).expect("Invalid program ID");

        let transaction_creator_keypair = create_signer_from_path(keypair).unwrap();

        let transaction_creator = transaction_creator_keypair.pubkey();

        let rpc_url = rpc_url.unwrap_or_else(|| "https://api.mainnet-beta.solana.com".to_string());
        let rpc_url_clone = rpc_url.clone();
        let rpc_client = &RpcClient::new(rpc_url);

        let multisig = Pubkey::from_str(&multisig_pubkey).expect("Invalid multisig address");

        let multisig_data = get_multisig(rpc_client, &multisig).await?;

        let transaction_index = multisig_data.transaction_index + 1;

        let transaction_pda = get_transaction_pda(&multisig, transaction_index, Some(&program_id));
        let proposal_pda = get_proposal_pda(&multisig, transaction_index, Some(&program_id));

        println!();
        println!(
            "{}",
            "👀 You're about to create a vault transaction, please review the details:".yellow()
        );
        println!();
        println!("RPC Cluster URL:   {}", rpc_url_clone);
        println!("Program ID:        {}", program_id);
        println!("Your Public Key:   {}", transaction_creator);
        println!();
        println!("⚙️ Config Parameters");
        println!("Multisig Key:      {}", multisig_pubkey);
        println!("Transaction Index: {}", transaction_index);
        println!("Vault Index:       {}", vault_index);

        // Display additional info if available
        if let Some(info) = additional_info {
            println!();
            println!("📝 Transaction Details");
            println!("Instruction Type:   {}", info.instruction_type);
            println!("Token Mint:         {}", info.mint);
            println!("Min Deposit:        {}", info.min_deposit);
            println!("Asset Config PDA:   {}", info.asset_config);
            println!("Crumb Authority:    {}", info.crumb_authority);
        }
        println!();

        let proceed = if no_confirm {
            true
        } else {
            Confirm::new()
            .with_prompt("Do you want to proceed?")
            .default(false)
                .interact()?
        };

        if !proceed {
            println!("OK, aborting.");
            return Ok(());
        }
        println!();

        let progress = ProgressBar::new_spinner().with_message("Sending transaction...");
        progress.enable_steady_tick(Duration::from_millis(100));

        let blockhash = rpc_client
            .get_latest_blockhash()
            .await
            .expect("Failed to get blockhash");

        let message = Message::try_compile(
            &transaction_creator,
            &[
                ComputeBudgetInstruction::set_compute_unit_limit(400_000),
                ComputeBudgetInstruction::set_compute_unit_price(
                    priority_fee_lamports.unwrap_or(5000),
                ),
                Instruction {
                    accounts: VaultTransactionCreateAccounts {
                        creator: transaction_creator,
                        rent_payer: transaction_creator,
                        transaction: transaction_pda.0,
                        multisig,
                        system_program: solana_sdk::system_program::id(),
                    }
                    .to_account_metas(Some(false)),
                    data: VaultTransactionCreateData {
                        args: VaultTransactionCreateArgs {
                            ephemeral_signers: ephemeral_signers_count,
                            vault_index,
                            memo,
                            transaction_message: transaction_message.expect("Transaction message must be set"),
                        },
                    }
                    .data(),
                    program_id,
                },
                Instruction {
                    accounts: ProposalCreateAccounts {
                        creator: transaction_creator,
                        rent_payer: transaction_creator,
                        proposal: proposal_pda.0,
                        multisig,
                        system_program: solana_sdk::system_program::id(),
                    }
                    .to_account_metas(Some(false)),
                    data: ProposalCreateData {
                        args: ProposalCreateArgs {
                            draft: false,
                            transaction_index,
                        },
                    }
                    .data(),
                    program_id,
                },
            ],
            &[],
            blockhash,
        )
        .unwrap();

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[&*transaction_creator_keypair],
        )
        .expect("Failed to create transaction");

        let signature = send_and_confirm_transaction(&transaction, &rpc_client).await?;

        println!(
            "Transaction confirmed: {}\n\n",
            signature.to_string().green()
        );

        // Print transaction index for script integration
        println!("Transaction Index: {}", transaction_index);

        Ok(())
    }
}
