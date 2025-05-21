# ~/aurora_project/core_modules/tgif_flow/router.py
import networkx as nx
import torch
from typing import List, Tuple, Optional, Dict, Any
import matplotlib.pyplot as plt # <--- ADD THIS LINE

# Import necessary components from other AURORA modules
from core_modules.cess_mesh.mesh_simulator import CESSMesh, DEVICE
from .intent import Intent

class TGIFRouter:
    """
    The TGIF (Twistor Geometry Information Fabric) Router.
    Responsible for routing Intent objects across the CESS Mesh.
    Inspired by twistor geometry, this router will prioritize causal correctness
    and leverage the dynamic mesh topology.
    """
    def __init__(self, mesh: CESSMesh):
        self.mesh = mesh
        print(f"TGIF Router initialized, connected to CESS Mesh with {self.mesh.graph.number_of_nodes()} nodes.")

    def get_path(self, source_node_id: int, destination_node_id: int) -> Optional[List[int]]:
        """
        Finds a path between source and destination nodes in the CESS Mesh.
        This is a basic shortest path for now, but will evolve to incorporate
        "conformal correctness" (e.g., path stability, latency, security properties derived from twistor vectors).
        """
        if not self.mesh.graph.has_node(source_node_id) or not self.mesh.graph.has_node(destination_node_id):
            print(f"Routing Error: Source {source_node_id} or Destination {destination_node_id} not in mesh.")
            return None
        
        try:
            # Basic shortest path using NetworkX
            # In a true TGIF Flow, this would be highly optimized, potentially GPU-acceleraccelerated,
            # and influenced by dynamic edge weights (from CESS Mesh attributes).
            path: List[int] = nx.shortest_path(self.mesh.graph, source=source_node_id, target=destination_node_id)
            return path
        except nx.NetworkXNoPath:
            print(f"Routing Error: No path found between {source_node_id} and {destination_node_id}.")
            return None
        except Exception as e:
            print(f"An unexpected routing error occurred: {e}")
            return None

    def route_intent(self, intent: Intent) -> Tuple[bool, Optional[List[int]]]:
        """
        Routes an Intent object through the CESS Mesh.
        Returns a tuple: (success_boolean, path_list_or_None).
        """
        if intent.source_node_id == -1 or intent.destination_node_id == -1:
            print(f"Intent {intent.intent_id[:8]} cannot be routed: source or destination undefined.")
            return False, None

        print(f"Routing Intent {intent.intent_id[:8]} from {intent.source_node_id} to {intent.destination_node_id}...")
        path = self.get_path(intent.source_node_id, intent.destination_node_id)

        if path:
            print(f"Intent {intent.intent_id[:8]} successfully routed. Path: {path}")
            # In a real system, intent would traverse the path, interacting with nodes/edges
            return True, path
        else:
            print(f"Intent {intent.intent_id[:8]} failed to route.")
            return False, None

    def visualize_path(self, path: List[int], iteration: int = 0, title_suffix: str = ""):
        """
        Visualizes a path on the CESS Mesh.
        Re-uses CESSMesh's visualize capability.
        """
        if not path or len(path) < 2:
            print("Cannot visualize path: invalid path.")
            return
        
        # Create a copy of the graph to highlight path
        graph_copy: nx.Graph = self.mesh.graph.copy() # Explicitly type graph_copy
        
        # Color nodes in path
        # Node colors are based on position in original graph.nodes(), assuming int nodes 0-N
        node_colors = ['skyblue'] * graph_copy.number_of_nodes()
        for i, node_id in enumerate(list(graph_copy.nodes())): # Iterate over actual nodes in the graph
            if node_id == path[0]:
                node_colors[node_id] = 'green' # Source
            elif node_id == path[-1]:
                node_colors[node_id] = 'red'   # Destination
            elif node_id in path:
                node_colors[node_id] = 'lightcoral' # Intermediate path node

        # Color edges in path
        edge_colors = ['gray'] * graph_copy.number_of_edges()
        path_edges = list(zip(path[:-1], path[1:]))
        
        for i, edge in enumerate(list(graph_copy.edges())): # Iterate over actual edges in the graph
            # NetworkX edges might store as (u,v) or (v,u), check both directions for path matching
            if (edge[0], edge[1]) in path_edges or (edge[1], edge[0]) in path_edges:
                edge_colors[i] = 'blue'
            
        plt.figure(figsize=(8, 6))
        pos: Dict[Any, Any] = nx.spring_layout(graph_copy, seed=42) # Consistent layout
        nx.draw(graph_copy, pos, with_labels=True, node_color=node_colors, node_size=700, edge_color=edge_colors, font_size=10, width=[2 if ec == 'blue' else 1 for ec in edge_colors])
        plt.title(f"TGIF Flow - Iteration {iteration}{title_suffix}")
        plt.show()