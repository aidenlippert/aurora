#![allow(unused_variables, dead_code, unused_imports)]
//! StarSenate Collectives: Galactic Governance Matrix.
use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;

use oraclesync_futarchy::get_futarchy_prediction_for_proposal;
use cosmic_data_constellation::{record_governance_action, IsnNode};

#[derive(Debug, Clone)]
pub enum ProposalStatus {
    Pending,
    VotingOpen,
    Approved,
    Rejected,
    Executed,
}

#[derive(Debug, Clone)]
pub struct Proposal {
    pub id: String,
    pub proposer_id: String,
    pub title: String,
    pub description: String,
    pub target_module: Option<String>,
    pub proposed_changes_hash: String,
    pub status: ProposalStatus,
    pub futarchy_prediction_score: Option<f64>,
    pub votes_for: u64,
    pub votes_against: u64,
}

#[derive(Debug)]
pub struct Vote {
    pub proposal_id: String,
    pub voter_id: String,
    pub in_favor: bool,
    pub weight: u64,
}

// Mock in-memory store for proposals
static PROPOSALS_DB: Lazy<Mutex<HashMap<String, Proposal>>> = Lazy::new(|| {
    Mutex::new(HashMap::new())
});

pub fn submit_proposal(
    proposer_id: &str,
    title: &str,
    description: &str,
    target_module: Option<String>,
    proposed_changes_hash: &str,
) -> Result<Proposal, String> {
    let proposal_id = format!("prop_{}", uuid::Uuid::new_v4());
    println!(
        "[StarSenate] New proposal submitted by '{}': ID '{}', Title '{}'",
        proposer_id, proposal_id, title
    );

    let prediction_score = get_futarchy_prediction_for_proposal(&proposal_id, description).ok();

    let new_proposal = Proposal {
        id: proposal_id.clone(),
        proposer_id: proposer_id.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        target_module,
        proposed_changes_hash: proposed_changes_hash.to_string(),
        status: ProposalStatus::VotingOpen,
        futarchy_prediction_score: prediction_score,
        votes_for: 0,
        votes_against: 0,
    };

    PROPOSALS_DB.lock().unwrap().insert(proposal_id.clone(), new_proposal.clone());
    println!("[StarSenate] Proposal '{}' Futarchy Prediction Score: {:?}", title, prediction_score.unwrap_or(-1.0));
    Ok(new_proposal)
}

pub fn cast_vote_on_proposal(
    proposal_id: &str,
    voter_id: &str,
    in_favor: bool,
    weight: u64,
) -> Result<(), String> {
    let mut db_lock = PROPOSALS_DB.lock().unwrap();
    if let Some(proposal) = db_lock.get_mut(proposal_id) {
        if !matches!(proposal.status, ProposalStatus::VotingOpen) {
            return Err(format!("Voting is not open for proposal {}", proposal_id));
        }
        if in_favor {
            proposal.votes_for += weight;
        } else {
            proposal.votes_against += weight;
        }
        println!("[StarSenate] Vote cast by '{}' on proposal '{}'. In Favor: {}, Weight: {}. Current Tally: For {}, Against {}",
            voter_id, proposal_id, in_favor, weight, proposal.votes_for, proposal.votes_against);
        Ok(())
    } else {
        Err(format!("Proposal {} not found", proposal_id))
    }
}

pub fn tally_votes_and_decide(proposal_id: &str, block_height_for_isn: u64) -> Result<ProposalStatus, String> {
    let mut db_lock = PROPOSALS_DB.lock().unwrap();
    if let Some(proposal) = db_lock.get_mut(proposal_id) {
        if !matches!(proposal.status, ProposalStatus::VotingOpen) {
            return Err(format!("Proposal {} is not in VotingOpen state.", proposal_id));
        }

        let decision = if proposal.votes_for > proposal.votes_against {
            ProposalStatus::Approved
        } else {
            ProposalStatus::Rejected
        };
        proposal.status = decision.clone();

        println!("[StarSenate] Proposal '{}' voting concluded. Result: {:?}. For: {}, Against: {}",
            proposal_id, proposal.status, proposal.votes_for, proposal.votes_against);

        let mut details = HashMap::new();
        details.insert("title".to_string(), proposal.title.clone());
        details.insert("proposer".to_string(), proposal.proposer_id.clone());
        details.insert("votes_for".to_string(), proposal.votes_for.to_string());
        details.insert("votes_against".to_string(), proposal.votes_against.to_string());
        details.insert("futarchy_score".to_string(), proposal.futarchy_prediction_score.map_or("N/A".to_string(), |s| s.to_string()));
        if let Some(ref target) = proposal.target_module {
            details.insert("target_module".to_string(), target.clone());
        }
        // Drop the lock before calling another function that might also try to lock ISN_MOCK_DB (though in this case record_governance_action uses its own Lazy static)
        // This is good practice to avoid potential deadlocks if they shared the same Mutex.
        // Here, they use different Lazy<Mutex>, so it's less critical but still good form.
        let proposal_title_clone = proposal.title.clone(); // Clone data needed after lock release
        let proposal_proposer_clone = proposal.proposer_id.clone();
        let proposal_status_clone = proposal.status.clone();
        drop(db_lock);


        match record_governance_action(
            proposal_id,
            &format!("{:?}", proposal_status_clone),
            block_height_for_isn,
            details, // This HashMap was created with cloned data
        ) {
            Ok(isn_node) => println!("[StarSenate] Governance action for proposal '{}' recorded in ISN. Node ID: {}", proposal_id, isn_node.id),
            Err(e) => eprintln!("[StarSenate] Error recording governance action for proposal '{}' in ISN: {}", proposal_id, e),
        }

        Ok(proposal_status_clone)
    } else {
        Err(format!("Proposal {} not found for tallying.", proposal_id))
    }
}

pub fn status() -> &'static str {
    let crate_name = "starsenate_collectives_governance";
    println!("[{}] placeholder_function called (mock status)", crate_name);
    "skeleton operational (mock)"
}
