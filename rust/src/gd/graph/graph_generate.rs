use std::{any::type_name, collections::BTreeMap, f64::consts::TAU, num::NonZero};

use godot::prelude::*;
use kiddo::{NearestNeighbour, SquaredEuclidean, float::kdtree::KdTree};
use nalgebra::{Point3, Unit, UnitQuaternion, Vector3 as NVector3};
use ordered_float::OrderedFloat;
use petgraph::{
    Graph,
    algo::tarjan_scc,
    graph::{NodeIndex, UnGraph},
};
use rand::{Rng, SeedableRng as _, seq::IndexedRandom as _};
use rand_xoshiro::Xoshiro256Plus;
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator as _;
use tracing::instrument;

use crate::{chords::Chord, gd::graph::graph_main::GraphTypedef, profile, util::random_unit_axis};

#[derive(Debug, Serialize, Deserialize)]
pub struct ConstellationGraph {
    pub chord: Chord,
    pub semitone_offset: i32,
    pub graph: GraphTypedef,
    pub islands: Vec<Vec<NodeIndex>>, // Strongly Connected Components (aka "islands")
}

impl ConstellationGraph {
    /// Create the graph and its strongly connected components (islands)
    pub fn new<R: Rng>(n: usize, radius: f32, max_neighbor_count: usize, rng: &mut R) -> Self {
        tracing::info!(rng_type = type_name::<R>(), "generating ConstellationGraph");

        // Generate this first for rng reasons
        let chords = Chord::iter().collect::<Vec<_>>();
        let chord = *chords
            .choose(&mut Xoshiro256Plus::from_rng(rng)) // Making a new rng here to avoid nondeterminism when we change the amount of chords
            .unwrap();
        let semitone_offset_base = rng.random_range(-11..12); // Equal for all notes to avoid dissonance

        let points = Self::generate_points(n, radius as f64, rng);

        let voronoi_rng = Xoshiro256Plus::from_rng(rng);
        let clusters = profile!(
            "cluster_voronoi",
            Self::cluster_voronoi(
                points,
                (n as f64 / 15.0).ceil() as usize, // n / 15 means ~15 nodes per cluster
                voronoi_rng
            )
        );

        let supergraph = Self::connect_clusters_internally(&clusters, max_neighbor_count, rng);
        let scc = tarjan_scc(&supergraph);

        ConstellationGraph {
            chord,
            semitone_offset: semitone_offset_base,
            graph: supergraph,
            islands: scc,
        }
    }

    /// Generate n random points on the unit sphere
    fn generate_points<R: Rng>(n: usize, radius: f64, rng: &mut R) -> Vec<Vector3> {
        // Using a large value of `max_angle` here is safe now due to our new `leniency` algorithm
        let max_angle = TAU * 0.02;
        let points_nalgebra = Self::generate_points_poisson(n, radius, max_angle, rng);

        points_nalgebra
            .into_iter()
            .map(|p| Vector3::from_array(<[f32; 3]>::from(p.cast::<f32>())))
            .collect()
    }

    /// Generates `n` points on the surface of a sphere of radius `radius` using a Poisson-disk-like algorithm.
    /// The points are roughly `max_angle` radians separated from each other, unless the algorithm reaches an iteration limit.
    /// Then it will slowly reduce the angle limit until it succeeds. This is called `leniency`.
    #[cfg_attr(feature = "enable-tracing", instrument(skip(rng)))]
    fn generate_points_poisson<R: Rng>(
        n: usize,
        radius: f64,
        mut max_angle: f64,
        rng: &mut R,
    ) -> Vec<Point3<f64>> {
        pub type KdTreeUsize<A, const K: usize> = KdTree<A, usize, K, 32, u32>;

        let mut points = Vec::with_capacity(n);
        let mut tree = KdTreeUsize::new();

        let epsilon_mult = 0.9; // This is the "leniency factor"

        // Starting point: (0, radius, 0)
        let start = NVector3::new(0.0, radius, 0.0);
        let start_point = Point3::from(start);
        tree.add(&start_point.into(), 0);
        points.push(start_point);

        while points.len() < n {
            let new_point;

            // Random angle per point at 50%..150% of max_angle (this is important to ensure melody rest times are varied - they depend on inter-node distance)
            let angle = rng.random_range(0.5..=1.5) * max_angle;

            // We want chord distance formula here, NOT arc length (we want an underestimation, not an overestimation)
            let mut min_distance = 2.0 * radius * (angle / 2.0).sin() * epsilon_mult;

            let mut iteration = 0;
            loop {
                // Select a previous point as a base
                let parent_idx = rng.random_range(0..points.len());
                let parent = points[parent_idx].coords;

                // If iteration count goes beyond 1k or so, reduce minimum distance (becomes slightly more `lenient`)
                if iteration >= 1_000 {
                    iteration = 0;

                    let leniency_factor = 0.9;
                    min_distance *= leniency_factor;
                    max_angle *= leniency_factor;

                    tracing::warn!(
                        "Reducing poisson min_distance to {:.4} due to reaching iteration limit",
                        min_distance
                    );
                }

                // Apply random axis rotation by fixed angle
                let axis = random_unit_axis(rng);
                let rot = UnitQuaternion::from_axis_angle(&Unit::new_normalize(axis), angle);
                let candidate = rot * parent;

                // Find closest neighbor
                let neighbors = tree.nearest_n_within::<SquaredEuclidean>(
                    &candidate.into(),
                    min_distance * min_distance,
                    NonZero::new(1).unwrap(), //Return up to 1 result
                    false,
                );

                // Reject if too close to neighbor
                if neighbors.is_empty() {
                    let p = Point3::from(candidate);
                    tree.add(&p.into(), points.len());
                    new_point = p;
                    break;
                }

                iteration += 1;
            }

            points.push(new_point);
        }

        points
    }

    /// Cluster the points according to a voronoi-like algorithm, where `k` is the amount of clusters. Returns the clustered points and the cluster centroids.
    #[cfg_attr(feature = "enable-tracing", instrument(skip(points, rng)))]
    fn cluster_voronoi(
        points: Vec<Vector3>,
        k: usize,
        mut rng: Xoshiro256Plus,
    ) -> Vec<(Vec<Vector3>, Vector3)> {
        pub type KdTreeUsize<A, const K: usize> = KdTree<A, usize, K, 32, u32>;
        let mut kdtree = KdTreeUsize::new();

        // Pick k random clusters
        let centroids = points.choose_multiple(&mut rng, k).collect::<Vec<_>>();

        // Add them to kd-tree
        for (i, c) in centroids.iter().enumerate() {
            kdtree.add(&[c.x, c.y, c.z], i);
        }

        let mut clusters = centroids
            .iter()
            .map(|centroid| (vec![], **centroid))
            .collect::<Vec<_>>();

        // Find closest centroid per point and assign the point to that cluster
        for p in points {
            let closest_centroid = kdtree
                .nearest_one::<SquaredEuclidean>(&[p.x, p.y, p.z])
                .item;
            clusters[closest_centroid].0.push(p);
        }

        //Now sort by centroid y (stable sort) for cool animation!
        clusters.sort_by_key(|cluster| OrderedFloat(-cluster.1.y));

        clusters
    }

    fn connect_clusters_internally<R: Rng>(
        clusters: &[(Vec<Vector3>, Vector3)],
        max_neighbor_count: usize,
        rng: &mut R,
    ) -> GraphTypedef {
        pub type KdTreeUsize<A, const K: usize> = KdTree<A, usize, K, 32, u32>;

        let mut supergraph: GraphTypedef = Graph::new_undirected();
        let mut graphs = vec![];

        profile!(
            "connect_clusters_internally", // This is not the bottleneck btw
            for (cluster, _centroid) in clusters.iter() {
                // Build the kd-tree for nearest neighbor search per-cluster
                let mut kdtree = KdTreeUsize::new();
                for (i, p) in cluster.iter().enumerate() {
                    kdtree.add(&[p.x, p.y, p.z], i);
                }

                // Create a graph where each point is a node
                let mut graph: GraphTypedef = Graph::new_undirected();
                let node_indices: Vec<NodeIndex> =
                    cluster.iter().map(|p| graph.add_node(*p)).collect();

                // Add edges between each node and its 1..max_neighbor_count nearest neighbors
                let max_neighbor_count = max_neighbor_count as i32;
                let min_neighbor_count = 1;

                for (i, point) in cluster.iter().enumerate() {
                    let neighbor_count = *(min_neighbor_count..=max_neighbor_count)
                        .collect::<Vec<_>>()
                        .choose_weighted(rng, |w| match w {
                            1 => 0.8,
                            2 => 1.0,
                            3 => 0.3,
                            _ => 0.0,
                        })
                        .unwrap() as usize;

                    // Find neighbors
                    let query = [point.x, point.y, point.z];
                    let neighbors =
                        kdtree.nearest_n::<SquaredEuclidean>(&query, neighbor_count + 1); //+1 since we get the point itself too

                    // Connect vertex i to neighbor j
                    for NearestNeighbour { item: j, .. } in &neighbors {
                        if i != *j {
                            let a = node_indices[i];
                            let b = node_indices[*j];

                            //Only add edge if there isn't one already
                            if !graph.contains_edge(a, b) {
                                graph.add_edge(a, b, ());
                            }
                        }
                    }
                }

                graphs.push(graph);
            }
        );

        Self::merge_undirected_graphs(&mut supergraph, &graphs);
        supergraph
    }

    /// Merges multiple undirected graphs together into one big graph.
    fn merge_undirected_graphs<N: Clone, E: Clone>(
        base: &mut UnGraph<N, E>,
        others: &[UnGraph<N, E>],
    ) {
        for g in others {
            //Use BTreeMap for determinism
            let mut node_map: BTreeMap<NodeIndex, NodeIndex> = BTreeMap::new();

            // Add all nodes from current graph 'g' into 'base'
            for node in g.node_indices() {
                let new_node = base.add_node(g[node].clone());
                node_map.insert(node, new_node);
            }

            // Add all edges, updating node indices to new ones in 'base'
            for edge in g.edge_indices() {
                let (source, target) = g.edge_endpoints(edge).unwrap();
                let weight = g.edge_weight(edge).unwrap().clone();
                base.add_edge(node_map[&source], node_map[&target], weight);
            }
        }
    }
}

impl PartialEq for ConstellationGraph {
    fn eq(&self, other: &Self) -> bool {
        graph_eq(&self.graph, &other.graph) && self.islands == other.islands
    }
}

/// Check if two graphs are "equal". Checks if internal node/edges have the exact same order. Useful for determinism testing.
/// See https://github.com/petgraph/petgraph/issues/199#issuecomment-484077775
fn graph_eq<N, E, Ty, Ix>(
    a: &petgraph::Graph<N, E, Ty, Ix>,
    b: &petgraph::Graph<N, E, Ty, Ix>,
) -> bool
where
    N: PartialEq,
    E: PartialEq,
    Ty: petgraph::EdgeType,
    Ix: petgraph::graph::IndexType + PartialEq,
{
    let a_ns = a.raw_nodes().iter().map(|n| &n.weight);
    let b_ns = b.raw_nodes().iter().map(|n| &n.weight);
    let a_es = a
        .raw_edges()
        .iter()
        .map(|e| (e.source(), e.target(), &e.weight));
    let b_es = b
        .raw_edges()
        .iter()
        .map(|e| (e.source(), e.target(), &e.weight));
    a_ns.eq(b_ns) && a_es.eq(b_es)
}
