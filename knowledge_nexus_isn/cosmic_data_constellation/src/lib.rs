#![allow(unused_variables, dead_code, unused_imports)]
//! Cosmic Data Constellation: The core graph database of ISN.
use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use uuid;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsnNode {
    pub id: String,
    pub r#type: String,
    pub properties: HashMap<String, String>,
    pub created_at_block: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsnEdge {
    pub id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub relationship_type: String,
    pub properties: HashMap<String, String>,
    pub created_at_block: u64,
}

static ISN_MOCK_DB_NODES: Lazy<Mutex<HashMap<String, IsnNode>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static ISN_MOCK_DB_EDGES: Lazy<Mutex<Vec<IsnEdge>>> = Lazy::new(|| Mutex::new(Vec::new()));

fn find_node_id_by_did_property(did_string_to_find: &str, nodes_db: &HashMap<String, IsnNode>) -> Option<String> {
    println!("[ISN_CDC_Debug] find_node_id: Searching for DID property: '{}'", did_string_to_find);
    for (node_id_key, node_value) in nodes_db.iter() {
        if node_value.r#type == "CelestialIdentity" {
            if let Some(prop_did) = node_value.properties.get("did") {
                // println!("[ISN_CDC_Debug]   Node ID: {}, Prop DID: '{}'", node_id_key, prop_did);
                if prop_did == did_string_to_find {
                    println!("[ISN_CDC_Debug]   MATCH FOUND for DID '{}'! Node ID: {}", did_string_to_find, node_id_key);
                    return Some(node_id_key.clone());
                }
            }
        }
    }
    println!("[ISN_CDC_Debug]   No match found for DID property: '{}'", did_string_to_find);
    None
}

fn create_and_store_isn_node(
    base_id_prefix: &str, node_type: &str, block_height: u64,
    mut properties: HashMap<String, String>, main_subject_key: &str, main_subject_value: &str,
) -> Result<IsnNode, String> {
    let node_id = format!("{}_{}", base_id_prefix, uuid::Uuid::new_v4());
    properties.insert(main_subject_key.to_string(), main_subject_value.to_string());
    let new_node = IsnNode { id: node_id.clone(), r#type: node_type.to_string(), properties, created_at_block: block_height };
    ISN_MOCK_DB_NODES.lock().unwrap().insert(node_id.clone(), new_node.clone());
    println!("[ISN_CDC] Recorded {}. Node ID: {}, Subject Key ('{}'): '{}', Block: {}", node_type, new_node.id, main_subject_key, main_subject_value, block_height);
    Ok(new_node)
}
pub fn record_identity_creation(
    did: &str, blk_h: u64, mut details: HashMap<String,String>,
) -> Result<IsnNode,String> {
    details.insert("did".to_string(), did.to_string());
    create_and_store_isn_node("identity","CelestialIdentity",blk_h,details,"did",did)
}
pub fn record_confirmed_operation(op_type: &str, o_id: &str, tx_id: &str, blk_h: u64, mut dets: HashMap<String,String>) -> Result<IsnNode,String> { dets.insert("op_type".into(),op_type.into()); dets.insert("o_id".into(),o_id.into()); create_and_store_isn_node("op_record","ConfirmedOperation",blk_h,dets,"transaction_id",tx_id) }
pub fn record_governance_action(p_id: &str, out: &str, blk_h: u64, mut dets: HashMap<String,String>) -> Result<IsnNode,String> { dets.insert("outcome".into(),out.into()); create_and_store_isn_node("gov_action","GovernanceAction",blk_h,dets,"proposal_id",p_id) }
pub fn record_obligation_status(ob_id: &str, stat: &str, blk_h: u64, mut dets: HashMap<String,String>) -> Result<IsnNode,String> { dets.insert("status".into(),stat.into()); create_and_store_isn_node("obligation_status","VerifiableObligationStatus",blk_h,dets,"obligation_id",ob_id) }
pub fn record_ecocredit_minting(cr_id: &str, blk_h: u64, dets: HashMap<String,String>) -> Result<IsnNode,String> { create_and_store_isn_node("ecocredit_mint","EcoFluxCreditMinted",blk_h,dets,"credit_id",cr_id) }
pub fn record_ecoreward_distribution(rew_id: &str, blk_h: u64, dets: HashMap<String,String>) -> Result<IsnNode,String> { create_and_store_isn_node("ecoreward_dist","EcoRewardDistributed",blk_h,dets,"reward_record_id",rew_id) }
pub fn record_module_deployment(mod_id: &str, name: &str, blk_h: u64, mut dets: HashMap<String,String>) -> Result<IsnNode,String> { dets.insert("dapp_name".into(),name.into()); create_and_store_isn_node("module_deploy","ModuleDeployment",blk_h,dets,"module_id",mod_id) }
pub fn record_penalty_event(pen_id: &str, blk_h: u64, dets: HashMap<String,String>) -> Result<IsnNode,String> { create_and_store_isn_node("penalty","PenaltyEvent",blk_h,dets,"penalty_id",pen_id) }
pub fn record_integrity_report(rep_id: &str, blk_h: u64, dets: HashMap<String,String>) -> Result<IsnNode,String> { create_and_store_isn_node("nci_report","IntegrityStarReport",blk_h,dets,"report_id",rep_id) }
pub fn record_real_world_data_point(src_id: &str, d_type: &str, blk_h: u64, dets: HashMap<String,String>) -> Result<IsnNode,String> { let subj = format!("{}_{}",src_id,d_type); create_and_store_isn_node("rw_data","RealWorldDataPoint",blk_h,dets,"source_datatype",&subj) }
pub fn record_prediction_event(pred_id: &str, blk_h: u64, dets: HashMap<String,String>) -> Result<IsnNode,String> { create_and_store_isn_node("prediction","PredictionEvent",blk_h,dets,"prediction_id",pred_id) }

pub fn create_isn_edge(
    from_identifier: &str, to_identifier: &str, relationship_type: &str,
    properties: HashMap<String, String>, current_block_height: u64,
) -> Result<IsnEdge, String> {
    let nodes_db_guard = ISN_MOCK_DB_NODES.lock().unwrap();
    let actual_from_node_id = if nodes_db_guard.contains_key(from_identifier) { from_identifier.to_string() }
                            else if let Some(id) = find_node_id_by_did_property(from_identifier, &nodes_db_guard) { id }
                            else { return Err(format!("Source entity/node '{}' for edge not found in ISN DB.", from_identifier)); };
    
    // For 'to_identifier', it's likely a module_deploy_... ID which IS a primary key
    let actual_to_node_id = if nodes_db_guard.contains_key(to_identifier) { to_identifier.to_string() }
                          else if let Some(id) = find_node_id_by_did_property(to_identifier, &nodes_db_guard) { id } // Less likely for 'to' in this context
                          else { return Err(format!("Target node ID '{}' for edge not found in ISN DB.", to_identifier)); };
    drop(nodes_db_guard);

    let edge_id = format!("edge_{}", uuid::Uuid::new_v4());
    let new_edge = IsnEdge { id: edge_id.clone(), from_node_id: actual_from_node_id.clone(), to_node_id: actual_to_node_id.clone(), relationship_type: relationship_type.to_string(), properties, created_at_block: current_block_height };
    ISN_MOCK_DB_EDGES.lock().unwrap().push(new_edge.clone());
    println!("[ISN_CDC] Created Edge ID: '{}'. From Node ID: '{}', To Node ID: '{}', Type: '{}'", new_edge.id, new_edge.from_node_id, new_edge.to_node_id, new_edge.relationship_type);
    Ok(new_edge)
}

pub fn get_isn_node(node_id: &str) -> Option<IsnNode> {
    // println!("[ISN_CDC] Attempting to get node {} (mock)", node_id); // Can be noisy
    ISN_MOCK_DB_NODES.lock().unwrap().get(node_id).cloned()
}

pub fn get_edges_from_node(node_id_or_did_to_query: &str, relationship_filter: Option<&str>) -> Vec<IsnEdge> {
    let resolved_node_id;
    {
        let nodes_db_guard = ISN_MOCK_DB_NODES.lock().unwrap();
        if nodes_db_guard.contains_key(node_id_or_did_to_query) {
            resolved_node_id = node_id_or_did_to_query.to_string();
        } else if let Some(found_id) = find_node_id_by_did_property(node_id_or_did_to_query, &nodes_db_guard) {
            resolved_node_id = found_id;
        } else {
            println!("[ISN_CDC_Debug] get_edges_from_node: Could not resolve identifier '{}' to an ISN Node ID for edge query.", node_id_or_did_to_query);
            return Vec::new();
        }
    }
    println!("[ISN_CDC_Debug] get_edges_from_node: Querying edges for resolved Node ID '{}' with filter {:?}.", resolved_node_id, relationship_filter);
    let edges_db = ISN_MOCK_DB_EDGES.lock().unwrap();
    edges_db.iter()
        .filter(|edge| edge.from_node_id == resolved_node_id ) // Only outgoing for "deployed_module"
        .filter(|edge| {
            relationship_filter.map_or(true, |filter| edge.relationship_type == filter)
        })
        .cloned()
        .collect()
}

pub fn status() -> &'static str {
    let crate_name = "cosmic_data_constellation";
    println!("[{}] placeholder_function called (mock status)", crate_name);
    "skeleton operational (mock)"
}
