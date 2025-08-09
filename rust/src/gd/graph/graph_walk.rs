use std::{collections::BTreeMap, rc::Rc};

use futures::future::join_all;
use godot::{obj::Gd, prelude::*};
use ordered_float::OrderedFloat;
use petgraph::{
    Direction,
    graph::{EdgeIndex, NodeIndex},
};
use rand::Rng;
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::info_span;

use crate::{
    async_node::{AsyncNode as _, wait_for_next_frame},
    gd::{
        autoload::{
            state_main::AudioState,
            state_tick::{TickReceiver, subscribe_to_ticks},
        },
        graph::graph_main::{AudioGraph, DEFAULT_EDGE_TWEEN_PROGRESS, GraphTypedef},
        node_main::AudioNode,
    },
    util::round_to_nearest_pow2_f64,
};

impl AudioGraph {
    pub async fn walk_node<R: Rng + Clone>(
        this: &mut Gd<Self>,
        node_idx: NodeIndex,
        graph: &Rc<GraphTypedef>,
        graph_assoc: &Rc<BTreeMap<NodeIndex, Gd<AudioNode>>>,
        last_diff: Option<Vector3>,
        panic_button_cancel: CancellationToken,
        rng: &mut R,
    ) {
        let mut node = Gd::clone(graph_assoc.get(&node_idx).unwrap());
        let node_pos = graph[node_idx];

        let mut cancelling = false;

        // Check if cancelled before we do anything else
        if node.bind().get_cancelling() {
            node.bind_mut().set_cancelling(false);
            cancelling = true;
        }

        // Play the node without waiting for it (send to "background" (not actually, still on main thread))
        let panic_button_cancel2 = panic_button_cancel.clone();
        this.bind_mut()
            .spawn_local_task(false, info_span!("play"), async move |_this| {
                AudioNode::play(&mut node, 1.0, panic_button_cancel2).await;
            });

        // Find neighbor(s) to move to (this can be multiple neighbors, if the user clicks on a node with a degree of 2 or higher)
        let next_node_idxes = {
            let neighs = graph.neighbors(node_idx).collect::<Vec<_>>();

            if let Some(last_diff) = last_diff {
                let last_dir = last_diff.normalized();

                // If there is only 1 neighbor, we reached end of the graph, so stop instantly.
                if neighs.len() <= 1 {
                    vec![]
                } else {
                    // Else, find the neighbor that is best at preserving the direction of the current walk
                    vec![
                        *neighs
                            .iter()
                            .max_by_key(|neigh_idx| {
                                let dir = (graph[**neigh_idx] - node_pos).normalized();
                                let neigh_dot = last_dir.dot(dir);
                                OrderedFloat::from(neigh_dot)
                            })
                            .unwrap(),
                    ]
                }
            } else {
                neighs
            }
        };

        let reached_end_of_graph = next_node_idxes.is_empty();
        if cancelling || reached_end_of_graph {
            return;
        }

        let mut futures = vec![];

        // Now recurse for every node in next_node_idxes
        for next_node_idx in next_node_idxes {
            let last_diff = graph[next_node_idx] - node_pos;
            let dist_rounded = round_to_nearest_pow2_f64(last_diff.length() as f64 * 8.0)
                .clamp(0.0, 16.0) as usize;
            let edge = graph.find_edge_undirected(node_idx, next_node_idx).unwrap();

            // Every branch gets their own tick receiver to avoid consuming each other's ticks
            let mut ticks = subscribe_to_ticks();

            let mut rng2 = rng.clone();
            let mut this2 = Gd::clone(this);

            let panic_button_cancel = panic_button_cancel.clone();

            futures.push(async move {
                let panic_button_cancel2 = panic_button_cancel.clone();
                let should_continue = Self::wait_for_ticks_and_lerp_edge(
                    &mut this2,
                    dist_rounded,
                    edge,
                    &mut ticks,
                    panic_button_cancel2,
                )
                .await;

                if !should_continue {
                    // Cancelled via panic button, so stop walking
                    tracing::info!("walker cancelled");
                    return;
                }

                Self::walk_node(
                    &mut this2,
                    next_node_idx,
                    graph,
                    graph_assoc,
                    Some(last_diff),
                    panic_button_cancel,
                    &mut rng2,
                )
                .await;
            });
        }

        // Now wait for all recursive calls to complete
        join_all(futures).await;
    }

    pub async fn graph_walk<R>(
        mut this: Gd<Self>,
        mut node: Gd<AudioNode>,
        node_index: NodeIndex,
        graph: Rc<GraphTypedef>,
        graph_assoc: Rc<BTreeMap<NodeIndex, Gd<AudioNode>>>,
        mut ticks: TickReceiver,
        panic_button_cancel: CancellationToken,
        rng: &mut R,
    ) where
        R: Rng + Clone,
    {
        // For the first step, wait until the next beat.
        node.bind_mut().set_pending(true);
        loop {
            let tick = ticks.wait().await;
            if tick.tick == 0 {
                break;
            }
        }

        // Then start the walk.
        Self::walk_node(
            &mut this,
            node_index,
            &graph,
            &graph_assoc,
            None,
            panic_button_cancel,
            rng,
        )
        .await;

        tracing::info!("walker reached end of the graph");
    }

    /// This method waits for ticks and drives the edge-lerping animation. Returns true if successful, false if cancelled.
    pub async fn wait_for_ticks_and_lerp_edge(
        this: &mut Gd<Self>,
        beats: usize,
        (edge_id, edge_dir): (EdgeIndex, Direction),
        ticks: &mut TickReceiver,
        panic_button_cancel: CancellationToken,
    ) -> bool {
        let bpm = AudioState::autoload().bind().get_bpm(); //TODO update this every time you receive a tick, so you can detect tempo changes.
        let ticks_per_beat = 4; //TODO update this every time you receive a tick, so you can detect time signature changes.

        let edge_index = edge_id.index() as i32;

        // Note - we use our own tweening logic here, since we may have to change the tweening speed during the tween, which is not supported with Godot tweens.
        // Also note - this may override other tweens on the same edge.
        let panic_button_cancel2 = panic_button_cancel.clone();
        this.bind_mut().spawn_local_task(
            true,
            info_span!("cylindrical_tween"),
            async move |this| {
                let mut progress = 0.0;

                let mut multi = this
                    .bind()
                    .get_multimesh_instance()
                    .get_multimesh()
                    .unwrap();

                // Drive the edge-lerp animation
                while progress < 1.0 {
                    //Don't forget to check the delta every frame
                    let delta = this.bind().base().get_process_delta_time();
                    let lerp_increment =
                        (ticks_per_beat as f64 / beats as f64) * (bpm / 60.0) * delta; //TODO this will not be accurate if BPM changes during the animation!

                    progress += lerp_increment;

                    let final_progress = match edge_dir {
                        Direction::Outgoing => progress,
                        Direction::Incoming => 1.0 - progress,
                    };

                    multi.set_instance_custom_data(
                        edge_index,
                        Color::from_rgba(final_progress as f32, 0.0, 0.0, 0.0),
                    );

                    let wait = wait_for_next_frame();
                    select! {
                        _ = wait => { /* continue */ }
                        _ = panic_button_cancel2.cancelled() => {
                            tracing::info!("edge tween cancelled");
                            break;
                        }
                    };
                }

                //When done reset progress
                multi.set_instance_custom_data(
                    edge_index,
                    Color::from_rgba(DEFAULT_EDGE_TWEEN_PROGRESS, 0.0, 0.0, 0.0),
                );
            },
        );

        // Wait for next `beats` ticks
        for _ in 0..beats {
            let tick_future = ticks.wait();
            select! {
                _ = tick_future => { /* continue */ }
                _ = panic_button_cancel.cancelled() => { return false; }
            }
        }

        true
    }
}
