// astral_hyper_engines/cluster_alpha_economic_sovereign/novavault_flux_finance/src/lib.rs
#![allow(unused_variables, dead_code, unused_imports)]
//! NovaVault Flux: Omni-Financial Continuum. Basic Public AUC Transfers.

use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use uuid::Uuid;

use cosmic_data_constellation::{IsnNode, record_confirmed_operation as isn_record_op};
use ecliptic_concordance::TransferAucPayload; 

static PUBLIC_AUC_BALANCES: Lazy<Mutex<HashMap<String, u64>>> = Lazy::new(|| {
    let balances = HashMap::new(); // No mut needed here for Lazy initialization
    Mutex::new(balances)
});

const INITIAL_BALANCE_FOR_NEW_ACCOUNT: u64 = 1000; 

pub fn ensure_account_exists_with_initial_funds(account_pk_hex: &str) {
    let mut balances = PUBLIC_AUC_BALANCES.lock().unwrap();
    balances.entry(account_pk_hex.to_string()).or_insert_with(|| {
        println!("[NovaVault] Initializing PK {} with {} AUC.", account_pk_hex, INITIAL_BALANCE_FOR_NEW_ACCOUNT);
        INITIAL_BALANCE_FOR_NEW_ACCOUNT
    });
}

pub fn process_public_auc_transfer(
    transfer_payload: &TransferAucPayload,
    current_block_height: u64, 
) -> Result<String, String> {
    let operation_id = format!("nv_pub_tx_{}", Uuid::new_v4());
    println!(
        "[NovaVault] Processing Public AUC Transfer ID: {}. From: {:.8} To: {:.8}, Amount: {}",
        operation_id, transfer_payload.sender_pk_hex, transfer_payload.recipient_pk_hex, transfer_payload.amount
    );

    ensure_account_exists_with_initial_funds(&transfer_payload.sender_pk_hex);
    ensure_account_exists_with_initial_funds(&transfer_payload.recipient_pk_hex);

    let mut balances = PUBLIC_AUC_BALANCES.lock().unwrap(); // Lock once

    // --- Sender Operation ---
    let sender_sufficient_funds;
    let current_sender_balance;
    { // Scope for sender_balance mutable borrow
        let sender_balance_mut_ref = balances.entry(transfer_payload.sender_pk_hex.clone()).or_insert(0);
        current_sender_balance = *sender_balance_mut_ref; // Get current value
        if *sender_balance_mut_ref < transfer_payload.amount {
            sender_sufficient_funds = false;
        } else {
            *sender_balance_mut_ref -= transfer_payload.amount;
            sender_sufficient_funds = true;
        }
    } // sender_balance_mut_ref borrow ends here

    if !sender_sufficient_funds {
        let err_msg = format!(
            "Insufficient funds for sender {:.8}. Has: {}, Needs: {}",
            transfer_payload.sender_pk_hex, current_sender_balance, transfer_payload.amount
        );
        eprintln!("[NovaVault] {}", err_msg);
        return Err(err_msg);
    }

    // --- Recipient Operation ---
    { // Scope for recipient_balance mutable borrow
        let recipient_balance_mut_ref = balances.entry(transfer_payload.recipient_pk_hex.clone()).or_insert(0);
        *recipient_balance_mut_ref += transfer_payload.amount;
    } // recipient_balance_mut_ref borrow ends here

    // For logging, re-fetch balances (or store them before modification if needed for complex logic)
    let final_sender_balance = *balances.get(&transfer_payload.sender_pk_hex).unwrap_or(&0);
    let final_recipient_balance = *balances.get(&transfer_payload.recipient_pk_hex).unwrap_or(&0);

    println!(
        "[NovaVault] Transfer successful. Sender {:.8} new balance: {}. Recipient {:.8} new balance: {}",
        transfer_payload.sender_pk_hex, final_sender_balance,
        transfer_payload.recipient_pk_hex, final_recipient_balance
    );
    
    // ISN Recording (no change here)
    let mut isn_details = HashMap::new();
    isn_details.insert("sender_pk_hex".to_string(), transfer_payload.sender_pk_hex.clone());
    isn_details.insert("recipient_pk_hex".to_string(), transfer_payload.recipient_pk_hex.clone());
    isn_details.insert("amount".to_string(), transfer_payload.amount.to_string());
    isn_details.insert("nonce".to_string(), transfer_payload.nonce.to_string());
    isn_details.insert("status".to_string(), "confirmed_public_transfer".to_string());

    match isn_record_op(
        "PublicTransferAUC",
        &transfer_payload.sender_pk_hex, 
        &operation_id, 
        current_block_height,
        isn_details,
    ) {
        Ok(isn_node) => {
            println!("[NovaVault] Public AUC transfer op recorded in ISN (Node ID: {}).", isn_node.id);
        }
        Err(e) => {
            eprintln!("[NovaVault] Error recording public AUC transfer in ISN: {}", e);
        }
    }
    Ok(operation_id)
}

pub fn get_account_balance(account_pk_hex: &str) -> Result<u64, String> {
    ensure_account_exists_with_initial_funds(account_pk_hex); 
    let balances = PUBLIC_AUC_BALANCES.lock().unwrap();
    match balances.get(account_pk_hex) {
        Some(balance) => {
            println!("[NovaVault] Balance for PK_hex {}: {}", account_pk_hex, *balance);
            Ok(*balance)
        }
        None => { 
            Err(format!("Account {} not found in NovaVault.", account_pk_hex))
        }
    }
}

pub fn status() -> &'static str {
    "NovaVault Flux Operational (Mock Public Balances)"
}