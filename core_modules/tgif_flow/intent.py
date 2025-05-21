# ~/aurora_project/core_modules/tgif_flow/intent.py
from dataclasses import dataclass, field
import uuid
from typing import Dict, Any, Optional
import torch

# Import DEVICE from CESS Mesh module, as it's a shared global for GPU usage
from core_modules.cess_mesh.mesh_simulator import DEVICE

@dataclass
class Intent:
    """
    Represents a computational 'intent' or message in AURORA.
    This is an abstract representation of a desired operation, state change, or query.
    Inspired by twistor theory, its core can be a high-dimensional vector.
    """
    # Unique identifier for this intent
    intent_id: str = field(default_factory=lambda: str(uuid.uuid4()))

    # Source and Destination nodes (represented by integer IDs in the CESS Mesh)
    source_node_id: int = -1 # -1 implies undefined or external source initially
    destination_node_id: int = -1 # -1 implies undefined or broadcast initially

    # Payload: The actual data or computational request
    payload: Dict[str, Any] = field(default_factory=dict)

    # Contextual metadata: can include timestamps, priority, security flags, etc.
    metadata: Dict[str, Any] = field(default_factory=dict)

    # Twistor-inspired Vector: A high-dimensional representation of the intent's 'nature'
    # This vector could encode properties like 'causality', 'conformal symmetry', 'priority', 'security level'.
    # For now, it's a random tensor, but later it would be generated intelligently.
    intent_vector: torch.Tensor = field(init=False)
    vector_dim: int = 16 # Default dimension for the intent vector

    def __post_init__(self):
        # Initialize intent_vector on the specified device
        self.intent_vector = torch.randn(self.vector_dim, device=DEVICE)

    def __str__(self) -> str:
        return f"Intent(ID={self.intent_id[:8]}, Src={self.source_node_id}, Dest={self.destination_node_id}, PayloadKeys={list(self.payload.keys())}, VecShape={tuple(self.intent_vector.shape)})"

    def __repr__(self) -> str:
        return self.__str__()