# ~/aurora_project/core_modules/tgif_flow/tests.py
import torch
import networkx as nx
import matplotlib.pyplot as plt # Needed for visualization
from typing import Dict, List, Tuple

# Import the core components
from .intent import Intent
from .router import TGIFRouter

# Import CESS Mesh directly for setup
from core_modules.cess_mesh.mesh_simulator import CESSMesh, DEVICE

if __name__ == "__main__":
    print("--- Running TGIF Flow Module Basic Tests ---")

    # 1. Test CUDA availability (inherited from CESS Mesh)
    if not torch.cuda.is_available():
        print("WARNING: CUDA not available. Running TGIF tests on CPU.")
    else:
        print(f"CUDA available: {torch.cuda.get_device_name(0)}")
        try:
            test_tensor: torch.Tensor = torch.randn(2, 2, device=DEVICE)
            print(f"Test tensor on {DEVICE}: {test_tensor.sum().item()}")
            assert test_tensor.device.type == DEVICE.type
            print("GPU tensor creation verified.")
        except Exception as e:
            print(f"ERROR: Failed to create GPU tensor for TGIF: {e}")
            print("Please ensure your CUDA installation and PyTorch setup are correct.")
            exit(1)

    # 2. Initialize a CESS Mesh (the underlying network topology)
    # We'll use a fixed seed to make the mesh consistent for testing routing
    mesh = CESSMesh(num_nodes=10, seed=100) # Use a specific seed for reproducible mesh
    print(f"\nCESS Mesh for TGIF initialized: Nodes={mesh.graph.number_of_nodes()}, Edges={mesh.graph.number_of_edges()}.")
    # This line now correctly passes title_suffix to the updated visualize method
    mesh.visualize(iteration=0, title_suffix=" (TGIF Base Mesh)") # Visualize the base mesh

    # 3. Initialize the TGIF Router
    router = TGIFRouter(mesh)
    print("\nTGIF Router initialized.")

    # 4. Create and Route Intents
    print("\nAttempting to route intents...")

    # Intent 1: Successful route
    intent1 = Intent(source_node_id=0, destination_node_id=9, payload={"task": "compute_pi"})
    success1, path1 = router.route_intent(intent1)
    assert success1 and path1 is not None and len(path1) > 0
    print(f"Intent 1 Path: {path1}")
    if path1:
        router.visualize_path(path1, iteration=1, title_suffix=" (Intent 1 Path)")

    # Intent 2: Another successful route
    intent2 = Intent(source_node_id=7, destination_node_id=5, payload={"data": "streaming_telemetry"})
    success2, path2 = router.route_intent(intent2)
    assert success2 and path2 is not None and len(path2) > 0
    print(f"Intent 2 Path: {path2}")
    if path2:
        router.visualize_path(path2, iteration=2, title_suffix=" (Intent 2 Path)")

    # Intent 3: Unreachable destination
    unreachable_node = 99 # A node not in our 10-node mesh
    intent3 = Intent(source_node_id=0, destination_node_id=unreachable_node, payload={"request": "query_unreachable"})
    success3, path3 = router.route_intent(intent3)
    assert not success3 and path3 is None
    print(f"Intent 3 (unreachable) routing result: Success={success3}, Path={path3}")

    print("\n--- TGIF Flow Module Basic Tests Complete ---")