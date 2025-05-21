# ~/aurora_project/core_modules/hnk/tests.py
import torch
# Removed unused imports from typing (Dict, List, Tuple) for tidiness if not directly used in the test script
from typing import Dict, List, Tuple # Keep these if they are used in type hints within this file's functions

from .sheaf_hypergraph_network import SheafHypergraph, SheafHypergraphNetwork, DEVICE

if __name__ == "__main__":
    print("--- Running HNK Module Basic Tests (PyTorch-Native) ---")

    # 1. Test CUDA availability and PyTorch setup (should be fine as CESS Mesh passed)
    if not torch.cuda.is_available():
        print("WARNING: CUDA not available. Running HNK tests on CPU.")
    else:
        print(f"CUDA available: {torch.cuda.get_device_name(0)}")
        try:
            test_tensor: torch.Tensor = torch.randn(2, 2, device=DEVICE)
            print(f"Test tensor on {DEVICE}: {test_tensor.sum().item()}")
            assert test_tensor.device.type == DEVICE.type # Corrected assertion
            print("GPU tensor creation verified.")
        except Exception as e:
            print(f"ERROR: Failed to create GPU tensor for HNK: {e}")
            print("Please ensure your CUDA installation and PyTorch setup are correct.")
            exit(1) # Exit if GPU is expected but fails


    # 2. Initialize a Sheaf Hypergraph
    num_nodes: int = 5
    # Example hyperedges: Node 0,1,2 form one; Node 1,3 form another; Node 2,3,4 form another
    hyperedges_data: List[List[int]] = [
        [0, 1, 2],
        [1, 3],
        [2, 3, 4]
    ]
    feature_dim: int = 8 # Feature dimension for nodes and stalks

    hypergraph: SheafHypergraph = SheafHypergraph(num_nodes=num_nodes, hyperedges_data=hyperedges_data, feature_dim=feature_dim)
    print(f"\nHypergraph Initialized: {len(hypergraph.node_features)} nodes, {len(hypergraph.hyperedges)} hyperedges.")
    
    # Verify features and stalks are on DEVICE
    for nid, f in hypergraph.node_features.items():
        assert f.device.type == DEVICE.type, f"Node {nid} features not on {DEVICE.type}" # Corrected assertion
    for he, s in hypergraph.hyperedge_stalks.items():
        assert s.device.type == DEVICE.type, f"Hyperedge {he} stalk not on {DEVICE.type}" # Corrected assertion
    print(f"All features and stalks confirmed on {DEVICE.type}.")

    # 3. Test Hypergraph Operations
    print("\nTesting hypergraph operations:")
    # First, update stalks for existing hyperedges
    for he_tuple in hypergraph.hyperedges:
        hypergraph.update_stalk_from_nodes(he_tuple)
    
    # Add a new hyperedge
    hypergraph.add_hyperedge([0, 4]) 
    hypergraph.update_stalk_from_nodes((0, 4)) # Update the new hyperedge's stalk

    # 4. Initialize and run a conceptual SHN layer
    print("\nRunning Sheaf Hypergraph Network layer:")
    shn_layer: SheafHypergraphNetwork = SheafHypergraphNetwork(in_features=feature_dim, out_features=feature_dim)

    # Perform a forward pass
    output_node_features: Dict[int, torch.Tensor] = shn_layer(hypergraph)
    print(f"Output node features for node 0 (first 4 dims): {output_node_features[0][:4].tolist()}")
    
    # Verify output node features are on DEVICE
    for nid, f in output_node_features.items():
        assert f.device.type == DEVICE.type, f"Output node {nid} feature not on {DEVICE.type}" # Corrected assertion
    print(f"All output node features confirmed on {DEVICE.type}.")

    print("\n--- HNK Module Basic Tests Complete ---")