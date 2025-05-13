#!/bin/bash

echo ">>> Root Cargo.toml"
cat Cargo.toml
echo -e "\n\n"

echo ">>> Root README.md"
cat README.md
echo -e "\n\n"

# Iterate over main component directories
for main_dir in astral_hyper_engines developer_ecosystem_astroforge docs ecological_synergy ethical_oversight_ego governance_economy infra knowledge_nexus_isn nodes omniverse_reality_simulation risk_mitigation roadmap_future_vision security_privacy_aegis sgs_foundations synergy_strategies_adoption tests tools trust_obligation_stl_von utils; do
  if [ -d "$main_dir" ]; then
    find "$main_dir" -type f \( -name "Cargo.toml" -o -name "README.md" -o -name "*.rs" \) -print -exec echo ">>> {}" \; -exec cat {} \; -exec echo -e "\n\n" \;
  fi
done

# Specific handling for files directly in sgs_foundations/triad_web (if any beyond subdirs)
if [ -f "sgs_foundations/triad_web/src/lib.rs" ]; then
    echo ">>> sgs_foundations/triad_web/src/lib.rs"
    cat sgs_foundations/triad_web/src/lib.rs
    echo -e "\n\n"
fi
if [ -f "sgs_foundations/triad_web/src/tcp_p2p.rs" ]; then
    echo ">>> sgs_foundations/triad_web/src/tcp_p2p.rs"
    cat sgs_foundations/triad_web/src/tcp_p2p.rs
    echo -e "\n\n"
fi

echo ">>> END OF FILE CONTENTS <<<"