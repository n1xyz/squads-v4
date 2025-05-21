# Squads v4 CLI Multisig Improvements

This update implements significant improvements to multisig transaction handling in the Squads v4 CLI, particularly focusing on PDA signatures. These changes resolve issues with operations like asset whitelisting where Program Derived Addresses need to sign during account creation.

## Key Changes

### Dependency Updates
- Added support for dirs, openssl, reqwest, and other utility libraries
- Updated from Solana 1.18.3 to 1.16.19 dependencies
- Replaced solana-clap-v3-utils with solana-cli-config for better configuration handling

### Enhanced PDA Signature Support
- Added `ephemeral_signers` field to transaction create process
- Implemented specific handling for asset_config PDA signatures
- Added support for proper bump seed management for PDAs

### Transaction Execution Improvements
- Added robust handling for transactions requiring multiple signatures
- Implemented fallback mechanism using direct RPC calls with skipPreflight=true
- Improved debugging output for transaction composition and signing
- Increased default compute unit limit from 200,000 to 1,000,000

### Keypair Management
- Replaced the remote wallet system with a more flexible keypair loading mechanism
- Added support for loading keypairs from config files or default locations

### Debug Enhancements
- Added extensive transaction debugging output
- Improved error handling and reporting for multisig operations
- Added detailed logging for PDA derivation and signature requirements

## Technical Implementation

The primary improvements focus on handling ephemeral signers correctly:

1. When creating transactions via `vault_transaction_create.rs`:
   - JSON input now supports an `ephemeral_signers` array
   - PDAs that need to sign are properly identified and tracked
   - Bump seeds are correctly passed to the transaction execution

2. In `vault_transaction_execute.rs`:
   - Added special handling for transactions requiring multiple signatures 
   - Implemented direct RPC fallback method for complex transactions
   - Enhanced debugging to show PDA signing requirements

These changes fix "missing required signature" errors that occur when working with PDAs in multisig contexts, particularly during account creation operations. 