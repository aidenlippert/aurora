# ~/aurora_project/core_modules/hnk/sheaf_hypergraph_network.py
import torch
import torch.nn as nn
from typing import Dict, List, Set, Any, Tuple, Optional
import random

# Ensure CUDA is available (DEVICE is already defined in cess_mesh, but we'll re-check here for this module's context)
if torch.cuda.is_available():
    DEVICE = torch.device("cuda")
    print(f"HNK Module will use GPU: {torch.cuda.get_device_name(0)}")
else:
    DEVICE = torch.device("cpu")
    print("WARNING: CUDA not available. HNK Module will run on CPU. Performance will be severely limited.")

class SheafHypergraph:
    """
    Represents a simplified Sheaf Hypergraph using PyTorch tensors and standard Python structures.
    - Nodes are basic entities (represented by integer IDs).
    - Hyperedges connect arbitrary subsets of nodes.
    - Each hyperedge has a 'stalk' (a tensor of features).
    - Each node has a 'feature' (a tensor).
    """
    def __init__(self, num_nodes: int, hyperedges_data: List[List[int]], feature_dim: int = 8, seed: Optional[int] = None):
        if seed is not None:
            random.seed(seed)
            torch.manual_seed(seed)
            if torch.cuda.is_available():
                torch.cuda.manual_seed_all(seed)

        self.num_nodes = num_nodes
        self.feature_dim = feature_dim
        
        # Node features: A dictionary mapping node ID to its feature tensor
        self.node_features: Dict[int, torch.Tensor] = {
            i: torch.randn(feature_dim, device=DEVICE) for i in range(num_nodes)
        }
        
        # Hyperedges: A list of tuples, where each tuple is a sorted collection of node IDs
        self.hyperedges: List[Tuple[int, ...]] = []
        # Hyperedge stalks: A dictionary mapping hyperedge tuple to its stalk tensor
        self.hyperedge_stalks: Dict[Tuple[int, ...], torch.Tensor] = {}

        # Add initial hyperedges and their stalks
        for he_nodes in hyperedges_data:
            self._add_hyperedge_internal(he_nodes)

        print(f"Initialized Sheaf Hypergraph with {num_nodes} nodes and {len(self.hyperedges)} hyperedges.")

    def _add_hyperedge_internal(self, nodes: List[int]) -> None:
        """Helper to add a hyperedge and initialize its stalk."""
        # Ensure node IDs are valid
        if any(node_id >= self.num_nodes or node_id < 0 for node_id in nodes):
            raise ValueError("Hyperedge contains invalid node IDs.")
        
        he_tuple = tuple(sorted(nodes))
        if he_tuple not in self.hyperedge_stalks:
            self.hyperedges.append(he_tuple)
            self.hyperedge_stalks[he_tuple] = torch.randn(self.feature_dim, device=DEVICE)
        else:
            print(f"Hyperedge {he_tuple} already exists, skipping.")

    def add_hyperedge(self, nodes: List[int]) -> None:
        """Public method to add a new hyperedge and initialize its stalk."""
        self._add_hyperedge_internal(nodes)
        print(f"Added new hyperedge: {tuple(sorted(nodes))}")


    def get_incident_hyperedges(self, node_id: int) -> List[Tuple[int, ...]]:
        """Returns hyperedges incident to a given node."""
        if node_id >= self.num_nodes or node_id < 0:
            raise ValueError(f"Node ID {node_id} is out of bounds.")
        return [he for he in self.hyperedges if node_id in he]

    def update_stalk_from_nodes(self, hyperedge: Tuple[int, ...]):
        """
        Simulates updating a hyperedge's stalk based on its incident nodes' features.
        This is a basic aggregation, similar to message passing in GNNs.
        """
        if hyperedge not in self.hyperedge_stalks:
            raise ValueError(f"Hyperedge {hyperedge} not found.")

        incident_node_features: List[torch.Tensor] = [
            self.node_features[node] for node in hyperedge if node in self.node_features
        ]
        
        if incident_node_features:
            # Simple aggregation (e.g., mean) of node features
            aggregated_node_features = torch.mean(torch.stack(incident_node_features), dim=0)
            # Update stalk with some influence from aggregated node features
            self.hyperedge_stalks[hyperedge] = (
                self.hyperedge_stalks[hyperedge] * 0.5 + aggregated_node_features * 0.5
            ).to(DEVICE)
            # print(f"Updated stalk for hyperedge {hyperedge}.") # Remove for cleaner test output

class SheafHypergraphNetwork(nn.Module):
    """
    A conceptual Sheaf Hypergraph Network layer, implemented using PyTorch.
    This demonstrates basic operations on a Sheaf Hypergraph,
    like updating node features based on incident stalks (message passing from hyperedges to nodes).
    A full SHN would involve sophisticated Laplacians and more complex message passing schemes.
    """
    def __init__(self, in_features: int, out_features: int):
        super().__init__()
        self.in_features = in_features
        self.out_features = out_features
        # A simple linear layer to transform aggregated stalk features
        self.linear_transform = nn.Linear(in_features, out_features).to(DEVICE)
        print(f"Initialized SHN layer with input={in_features}, output={out_features} features.")

    def forward(self, hypergraph_instance: SheafHypergraph) -> Dict[int, torch.Tensor]:
        """
        Forward pass: Nodes receive messages from their incident hyperedge stalks.
        """
        # Dictionary to accumulate messages for each node
        new_node_features_sum: Dict[int, torch.Tensor] = {
            node_id: torch.zeros(self.out_features, device=DEVICE) for node_id in hypergraph_instance.node_features
        }
        # Dictionary to count how many hyperedges contribute to each node's update
        node_update_count: Dict[int, int] = {
            node_id: 0 for node_id in hypergraph_instance.node_features
        }

        for he_tuple, stalk in hypergraph_instance.hyperedge_stalks.items():
            # Apply transformation to the stalk
            transformed_stalk = self.linear_transform(stalk)
            
            for node_id in he_tuple:
                if node_id in new_node_features_sum: # Ensure node exists
                    new_node_features_sum[node_id] += transformed_stalk
                    node_update_count[node_id] += 1
        
        # Average the updates for nodes (simple normalization)
        output_node_features: Dict[int, torch.Tensor] = {}
        for node_id in hypergraph_instance.node_features: # Iterate through all original nodes
            if node_update_count[node_id] > 0:
                output_node_features[node_id] = new_node_features_sum[node_id] / node_update_count[node_id]
            else:
                # If a node is not part of any hyperedge, its feature remains its original
                output_node_features[node_id] = hypergraph_instance.node_features[node_id].clone().to(DEVICE) # Ensure it's a clone and on device

        return output_node_features