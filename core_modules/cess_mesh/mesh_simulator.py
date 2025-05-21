# ~/aurora_project/core_modules/cess_mesh/mesh_simulator.py
import torch
import networkx as nx
import matplotlib.pyplot as plt
from typing import List, Tuple, Dict, Set, Any, Optional
import random

# --- Global Constants / Module-Level Definitions ---
if not torch.cuda.is_available():
    print("WARNING: CUDA is not available. CESS Mesh will run on CPU. Performance will be severely limited.")
    DEVICE: torch.device = torch.device("cpu")
else:
    DEVICE: torch.device = torch.device("cuda")
    print(f"CUDA available. CESS Mesh will use GPU: {torch.cuda.get_device_name(0)}")

# --- CESSMesh Class Definition ---
class CESSMesh:
    """
    A classical simulation of an emergent spacetime mesh, representing AURORA's computational fabric.
    This uses a graph-based approach with dynamic evolution rules inspired by Pachner moves.
    """
    graph: nx.Graph
    node_attrs: Dict[int, torch.Tensor]
    edge_attrs: Dict[Tuple[int, int], torch.Tensor]

    def __init__(self, num_nodes: int = 10, seed: Optional[int] = None):
        if seed is not None:
            random.seed(seed)
            torch.manual_seed(seed)
            if torch.cuda.is_available():
                torch.cuda.manual_seed_all(seed)
        
        self.graph = nx.Graph()
        self.node_attrs = {i: torch.rand(4, device=DEVICE) for i in range(num_nodes)}
        self.edge_attrs = {}

        for _ in range(num_nodes * 2):
            u, v = random.sample(range(num_nodes), 2)
            if not self.graph.has_edge(u, v):
                self.graph.add_edge(u, v)
                self.edge_attrs[(u, v)] = torch.rand(1, device=DEVICE)
                self.edge_attrs[(v, u)] = self.edge_attrs[(u, v)]

        print(f"Initialized CESS Mesh with {self.graph.number_of_nodes()} nodes and {self.graph.number_of_edges()} edges.")

    def _update_edge_attr(self, u: int, v: int, new_attr: torch.Tensor):
        self.edge_attrs[(u, v)] = new_attr
        self.edge_attrs[(v, u)] = new_attr

    def perform_pachner_move_2_2(self) -> bool:
        possible_edges: List[Tuple[Any, Any]] = list(self.graph.edges())
        if not possible_edges:
            return False

        random.shuffle(possible_edges)
        for u, v in possible_edges:
            neighbors_of_u: Set[Any] = set(self.graph.neighbors(u))
            
            if len(neighbors_of_u) > 1:
                other_neighbor_of_u: Any = (neighbors_of_u - {v}).pop()
                
                possible_targets: List[Any] = list(set(self.graph.nodes()) - {u, v, other_neighbor_of_u})
                if not possible_targets:
                    continue

                new_target: Any = random.choice(possible_targets)
                
                if not self.graph.has_edge(u, new_target):
                    self.graph.remove_edge(u, v)
                    self.graph.add_edge(u, new_target)
                    
                    self.edge_attrs.pop((u,v), None)
                    self.edge_attrs.pop((v,u), None)
                    self.edge_attrs[(u,new_target)] = torch.rand(1, device=DEVICE)
                    self.edge_attrs[(new_target,u)] = self.edge_attrs[(u,new_target)]
                    
                    print(f"Rewired edge ({u},{v}) to ({u},{new_target}).")
                    return True
        return False

    def update_node_properties(self):
        for node in self.graph.nodes():
            neighbors: List[Any] = list(self.graph.neighbors(node))
            if neighbors:
                neighbor_states: torch.Tensor = torch.stack([self.node_attrs[n] for n in neighbors])
                new_state: torch.Tensor = torch.mean(neighbor_states, dim=0) + torch.randn_like(self.node_attrs[node]) * 0.1
                self.node_attrs[node] = new_state.to(DEVICE)
        print("Node properties updated based on local interactions.")

    # CORRECTED: Added title_suffix parameter
    def visualize(self, iteration: int = 0, title_suffix: str = ""):
        """Basic visualization of the graph."""
        plt.figure(figsize=(8, 6))
        pos: Dict[Any, Any] = nx.spring_layout(self.graph, seed=42)
        nx.draw(self.graph, pos, with_labels=True, node_color='skyblue', node_size=700, edge_color='gray', font_size=10)
        # Use title_suffix in the plot title
        plt.title(f"CESS Mesh - Iteration {iteration}{title_suffix}")
        plt.show()