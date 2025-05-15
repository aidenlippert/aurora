// governance_economy/econova_incentives/src/lib.rs
#![allow(unused_variables, dead_code, unused_imports)]
//! EcoNova Incentives: Regenerative Economic Harmony.
use std::collections::HashMap;
use novavault_flux_finance::process_public_auc_transfer; // Use the main transfer function
use cosmic_data_constellation::{IsnNode, record_ecoreward_distribution};
use ecliptic_concordance::TransferAucPayload; // Import the payload type
use uuid::Uuid; // For reward record ID

// Mock GreenStarOracle check
fn mock_greenstar_oracle_check(validator_did: &str, operation_id: &str) -> bool {
    println!("[EcoNovaIncentives/GreenStarOracleMock] Checking if operation '{}' by DID '{}' was green...",
        operation_id, validator_did);
    validator_did.to_lowercase().contains("green") || validator_did.to_lowercase().contains("eco")
}

pub fn calculate_and_distribute_fluxboost_reward(
    validator_did: &str, // This is the recipient_pk_hex for the transfer
    base_block_reward: u64, // Not directly used for transfer amount, but for calculation
    operation_id_being_rewarded: &str, 
    current_block_height: u64,
    // We need a "sender" for the reward, e.g., a system treasury PK
    reward_funder_pk_hex: &str, 
    next_reward_nonce: u64, // Nonce for the funder account
) -> Result<Option<u64>, String> {
    println!(
        "[EcoNovaIncentives] Calculating FluxBoost Reward for DID '{}', Base Reward: {}, Operation: {}",
        validator_did, base_block_reward, operation_id_being_rewarded
    );

    if mock_greenstar_oracle_check(validator_did, operation_id_being_rewarded) {
        let eco_multiplier = 0.2; 
        let flux_boost_amount = (base_block_reward as f64 * eco_multiplier).round() as u64;

        if flux_boost_amount == 0 {
            println!("[EcoNovaIncentives] Calculated FluxBoost is 0 for DID '{}'. No reward distributed.", validator_did);
            return Ok(None);
        }

        println!("[EcoNovaIncentives] DID '{}' qualifies for FluxBoost! Amount: {} AUC.",
            validator_did, flux_boost_amount);

        let transfer_payload = TransferAucPayload {
            sender_pk_hex: reward_funder_pk_hex.to_string(),
            recipient_pk_hex: validator_did.to_string(),
            amount: flux_boost_amount,
            nonce: next_reward_nonce, // This nonce must be managed for the reward_funder_pk_hex
        };

        // Call NovaVault to process this as a standard transfer
        // The block height here is for ISN recording within NovaVault
        match process_public_auc_transfer(&transfer_payload, current_block_height) {
            Ok(novavault_op_id) => {
                println!("[EcoNovaIncentives] FluxBoost Reward of {} AUC (NovaVault Op ID: {}) distributed to DID '{}'.",
                flux_boost_amount, novavault_op_id, validator_did);
                 // Record specific ecoreward in ISN
                let mut details = HashMap::new();
                details.insert("recipient_did".to_string(), validator_did.to_string());
                details.insert("reward_amount_auc".to_string(), flux_boost_amount.to_string());
                details.insert("base_reward_auc".to_string(), base_block_reward.to_string());
                details.insert("operation_id_rewarded".to_string(), operation_id_being_rewarded.to_string());
                details.insert("novavault_op_id".to_string(), novavault_op_id);
                let reward_record_id = format!("ecoreward_{}", Uuid::new_v4());

                match record_ecoreward_distribution(&reward_record_id, current_block_height, details) {
                    Ok(isn_node) => println!("[EcoNovaIncentives] FluxBoost Reward for DID '{}' recorded in ISN. Node ID: {}", validator_did, isn_node.id),
                    Err(e) => eprintln!("[EcoNovaIncentives] Error recording FluxBoost Reward for DID '{}' in ISN: {}", validator_did, e),
                }
                Ok(Some(flux_boost_amount))
            }
            Err(e) => {
                eprintln!("[EcoNovaIncentives] Failed to distribute FluxBoost Reward to DID '{}' via NovaVault: {}",
                    validator_did, e);
                Err(format!("NovaVault distribution failed: {}", e))
            }
        }
    } else {
        println!("[EcoNovaIncentives] DID '{}' does not qualify for FluxBoost for operation '{}'.",
            validator_did, operation_id_being_rewarded);
        Ok(None)
    }
}

pub fn status() -> &'static str {
    "econova_incentives operational (mock)"
}