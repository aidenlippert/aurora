# ~/aurora_project/core_modules/cess_mesh/tests.py
import torch
import networkx as nx # Import nx
import matplotlib.pyplot as plt # Import plt
# Removed unused imports from typing (Dict, Tuple) as they are not directly used in tests.py
# from typing import Dict, Tuple

# This import should now correctly resolve CESSMesh and DEVICE
from .mesh_simulator import CESSMesh, DEVICE

if __name__ == "__main__":
    print("--- Running CESS Mesh Module Basic Tests ---")

    # 1. Test CUDA availability
    if not torch.cuda.is_available():
        print("WARNING: CUDA not available. Running CESS Mesh tests on CPU.")
    else:
        print(f"CUDA available: {torch.cuda.get_device_name(0)}")
        # Simple CUDA tensor test
        try:
            # Access DEVICE directly as it's imported
            test_tensor: torch.Tensor = torch.randn(5, 5, device=DEVICE) # Explicitly type
            print(f"Test tensor on {DEVICE}: {test_tensor.sum().item()}")
            assert test_tensor.device.type == DEVICE.type # Corrected assertion: compare device.type string ('cuda' or 'cpu')
            print("GPU tensor creation verified.")
        except Exception as e:
            print(f"ERROR: Failed to create GPU tensor: {e}")
            print("Please ensure your CUDA installation and PyTorch setup are correct.")
            # Exit or skip GPU-dependent tests if this fails

    # 2. Initialize the CESS Mesh
    num_nodes: int = 10 # Explicitly type
    # Create CESSMesh instance directly as it's imported
    mesh: CESSMesh = CESSMesh(num_nodes=num_nodes, seed=42)
    print(f"\nInitial Mesh: Nodes={mesh.graph.number_of_nodes()}, Edges={mesh.graph.number_of_edges()}")
    assert mesh.graph.number_of_nodes() == num_nodes
    assert mesh.graph.number_of_edges() > 0 # Should have some initial edges

    # Check node and edge attributes are on the correct device
    for node_id, attrs in mesh.node_attrs.items():
        # CORRECTED: Use .type for comparison
        assert attrs.device.type == DEVICE.type, f"Node {node_id} attributes not on {DEVICE.type}"
    for edge_id, attrs in mesh.edge_attrs.items():
        # CORRECTED: Use .type for comparison
        assert attrs.device.type == DEVICE.type, f"Edge {edge_id} attributes not on {DEVICE.type}"
    print(f"All initial node/edge attributes confirmed on {DEVICE.type}.")


    # 3. Simulate Mesh Evolution
    print("\nSimulating mesh evolution (rewiring and property updates)...")
    mesh.visualize(iteration=0) # Initial state visualization

    for i in range(1, 4): # Perform a few iterations
        print(f"--- Iteration {i} ---")
        rewired: bool = mesh.perform_pachner_move_2_2()
        if rewired:
            print(f"Topology changed in iteration {i}.")
        else:
            print(f"No topology change in iteration {i}.")
        mesh.update_node_properties() # Update node states
        mesh.visualize(iteration=i) # Visualize current state

        # Basic assertions for evolution
        assert mesh.graph.number_of_nodes() == num_nodes # Node count should remain same
        assert len(mesh.node_attrs) == num_nodes
        assert mesh.graph.number_of_edges() >= 0 # Edges might change but should be valid

    print("\n--- CESS Mesh Module Basic Tests Complete ---")