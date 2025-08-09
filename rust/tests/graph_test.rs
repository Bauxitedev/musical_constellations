//! This module tests determinism of the graph constellation generation algorithm.
//! Always use `Xoshiro256Plus` to guarantee portability/determinism across all platforms.
//! Also don't use the built-in hash `DefaultHash` or `ahash`, try `HighwayHash` instead (it's fully portable/deterministic).
//! Also watch out for HashMap/HashSet, by default they're randomized.

use musical_constellations_rust::gd::graph::graph_generate::ConstellationGraph;
use rand::Rng;
use serde::Serialize;
#[derive(Serialize)]
pub struct ConstellationGraphSnapshot {
    global_seed: i64,
    num_points: usize,
    max_neighbor_count: usize,
    radius: f32,
    rng_type: String,

    constellation_graph: ConstellationGraph,
}

impl ConstellationGraphSnapshot {
    pub fn new<R: Rng>(
        constellation_graph: ConstellationGraph,
        global_seed: i64,
        radius: f32,
        max_neighbor_count: usize,
        _rng: R,
    ) -> Self {
        Self {
            global_seed,
            num_points: constellation_graph.graph.node_count(),
            constellation_graph,
            radius,
            max_neighbor_count,
            rng_type: std::any::type_name::<R>().to_owned(),
        }
    }
}

// Unit tests
#[cfg(test)]
mod tests {
    use rand::SeedableRng as _;
    use rand_xoshiro::Xoshiro256Plus;

    use super::*; // bring functions from outer scope

    /// This generates a ConstellationGraph with fixed seeds and compares them to the snapshot.
    /// Run this test on many different platforms/targets/profiles, to ensure the graph generation process doesn't suffer from float nondeterminism.
    #[test]
    fn graph_determinism() {
        let mut snapshots = vec![];

        for global_seed in [1_i64, 2, i64::MAX, i64::MIN] {
            let num_points = 30; // Do not use 2000 here, the files become too unwieldy
            let max_neighbor_count = 2 - 1;
            let radius = 5.0;

            let mut seed_bytes = [0u8; 32];
            seed_bytes[0..8].copy_from_slice(&global_seed.to_le_bytes());

            let mut rng = Xoshiro256Plus::from_seed(seed_bytes);
            let constellation_graph =
                ConstellationGraph::new(num_points as usize, radius, max_neighbor_count, &mut rng);
            let snapshot = ConstellationGraphSnapshot::new(
                constellation_graph,
                global_seed,
                radius,
                max_neighbor_count,
                rng,
            );

            snapshots.push(snapshot);
        }

        insta::assert_yaml_snapshot!(snapshots);
    }
}
