use std::{
    collections::{BTreeMap, VecDeque},
    fmt::Debug,
    rc::Rc,
    time::{Duration, Instant},
};

use async_executor::LocalExecutor;
use godot::{
    classes::{
        AudioStreamPlayer, InputEvent, InputEventMouseButton, MeshInstance3D, MultiMesh,
        MultiMeshInstance3D,
    },
    global::MouseButton,
    prelude::*,
};
use itertools::Itertools as _;
use petgraph::{Graph, Undirected, graph::NodeIndex, visit::EdgeRef as _};
use rand::{Rng, SeedableRng, seq::IndexedRandom};
use rand_distr::{Distribution as _, Normal};
use rand_xoshiro::Xoshiro256Plus;
use strum::IntoEnumIterator;
use tokio_util::sync::CancellationToken;
use tracing::{info_span, instrument};

use crate::{
    async_node::{AsyncNode, spawn_rayon_with_result},
    flags::USE_METRONOME,
    format_gdobj,
    gd::{
        autoload::{state_main::AudioState, state_tick::subscribe_to_ticks},
        graph::graph_generate::ConstellationGraph,
        node_main::AudioNode,
        node_stream::Waveform,
    },
    profile,
    util::create_rng_from_seed_and_state,
};

pub type GraphTypedef = Graph<Vector3, (), Undirected>;

#[derive(GodotClass, Debug)]
#[class(init,base=Node3D)]
pub struct AudioGraph {
    #[base]
    base: Base<Node3D>,

    #[export]
    #[init(val = 20)]
    num_points: i32,

    #[init(node = "EdgesMultiMesh")]
    #[var]
    multimesh_instance: OnReady<Gd<MultiMeshInstance3D>>,

    #[init(node = "Metronome")]
    metronome: OnReady<Gd<AudioStreamPlayer>>,

    #[init(node = "IndicatorLoading")]
    indicator_loading: OnReady<Gd<MeshInstance3D>>,

    #[init(val = OnReady::manual())]
    graph: OnReady<Rc<GraphTypedef>>,
    #[init]
    graph_godot_nodes: Rc<BTreeMap<NodeIndex, Gd<AudioNode>>>, //Use BTreeMap instead of HashMap for determinism

    executor: Option<Rc<LocalExecutor<'static>>>,
    is_accepting_input: bool,
    panic_button_cancel: CancellationToken,

    bpm_taps: VecDeque<Instant>,
}

#[godot_api]
impl INode3D for AudioGraph {
    #[cfg_attr(feature = "enable-tracing",  instrument(fields(self = format_gdobj!(self.base()))))]
    fn ready(&mut self) {
        self.start_metronome_task();

        //load() becomes much faster if you call it outside the async executor? Weird...
        let node_scene = profile!(load::<PackedScene>("res://scenes/audio_node.tscn"));

        self.spawn_local_task(
            true,
            info_span!("spawn_all_nodes"),
            async move |mut this| {
                this.bind_mut().indicator_loading.show();
                reset_multimesh(this.bind().multimesh_instance.get_multimesh().unwrap());

                let num_points = this.bind().num_points;

                tracing::info!("audio graph ready, spawning {} points...", num_points);
                let global_seed = AudioState::autoload().bind().get_seed();
                let mut root_rng = create_rng_from_seed_and_state(0xA0A0BE63, global_seed);

                let max_neighbor_count = 3; //3 is good, 2 is sparse, 1 is too sparse (4 is IRL max I think)
                let radius = 5.0; //Warning - if you change the radius it messes up the note timing!
                let mut point_rng = Xoshiro256Plus::from_rng(&mut root_rng); //Forks the rng, so nondeterminism caused by parallellism shouldn't influence the root rng

                let constellation = spawn_rayon_with_result(move || {
                    profile!(
                        "generate_constellation_graph",
                        ConstellationGraph::new(
                            num_points as usize,
                            radius,
                            max_neighbor_count,
                            &mut point_rng
                        )
                    )
                })
                .await
                .expect("generate_points_and_edges panicked");

                let island_count = constellation.islands.len();

                let scc_assoc = {
                    let mut scc_assoc = BTreeMap::<NodeIndex, usize>::default(); //BTreeMap is deterministic now
                    for (island_idx, island) in constellation.islands.iter().enumerate() {
                        for node in island {
                            let inserted = scc_assoc.insert(*node, island_idx);
                            assert_eq!(inserted, None);
                        }
                    }
                    scc_assoc
                };

                let ConstellationGraph {
                    ref graph,
                    ref islands,
                    ref chord,
                    semitone_offset: semitone_offset_base,
                } = constellation;

                tracing::info!(?chord, semitone_offset_base); //Poisson has about ~250 islands, non-poisson about ~90
                tracing::info!(
                    island_count,
                    smallest_island = ?islands.iter().map(|island| island.len()).min().unwrap(), //Should be >=2, I don't want loose points
                    largest_island = ?islands.iter().map(|island| island.len()).max().unwrap()
                );

                ////////////////////

                profile!("setup_multimesh", {
                    let multi = this.bind().multimesh_instance.get_multimesh().unwrap();
                    setup_multimesh(multi, graph);
                });

                this.bind_mut().indicator_loading.hide();

                ////////////////////

                let island_data = Self::generate_island_data(&constellation, &mut root_rng);
                let stats = Self::generate_stats(&constellation, &island_data);
                AudioState::autoload()
                    .bind_mut()
                    .set_graph_debug_str(stats.into());

                let graph_godot_nodes = Self::play_intro_animation(
                    &mut this,
                    &constellation,
                    &island_data,
                    &scc_assoc,
                    node_scene,
                    &mut root_rng,
                )
                .await;

                this.bind_mut().is_accepting_input = true;
                this.bind_mut().graph.init(Rc::new(constellation.graph));
                this.bind_mut().graph_godot_nodes = Rc::new(graph_godot_nodes);
            },
        );
    }

    fn process(&mut self, _delta: f32) {
        self.tick_deferred();
    }

    #[cfg_attr(feature = "enable-tracing", instrument(skip(self)))]
    fn unhandled_input(&mut self, event: Gd<InputEvent>) {
        //Note - while a textbox is selected, unhandled_input triggers anyway, but pressed() is always false.
        //So be sure to use is_action_pressed() instead of is_action().

        if !self.is_accepting_input {
            return;
        }

        if event.is_action_pressed("toggle_metronome") {
            USE_METRONOME.toggle();
        }
        if event.is_action_pressed("panic") {
            //Panic button
            self.panic_button_cancel.cancel();
            self.panic_button_cancel = CancellationToken::new(); //Create a new token, since we can't re-use it after cancelling
        }
        if event.is_action_pressed("stress") {
            //Performance stress test - play the first 256 notes simultaneously
            //A modern PC should easily be able to handle this
            let debug_play_nodes = 256;

            let subnodes = self
                .base_mut()
                .get_children()
                .iter_shared()
                .filter_map(|node| node.try_cast::<AudioNode>().ok())
                .collect::<Vec<_>>();

            for mut node in &mut subnodes.into_iter().take(debug_play_nodes) {
                let panic_button_cancel = self.panic_button_cancel.clone();
                self.spawn_local_task(false, info_span!("play_debug"), async move |_this| {
                    AudioNode::play(&mut node, 20.0, panic_button_cancel).await;
                });
            }
        }
        if event.is_action_pressed("bpm_tap") {
            self.perform_bpm_tap();
        }
    }
}

impl AudioGraph {
    pub fn start_metronome_task(&mut self) {
        tracing::info!("starting metronome task...");
        self.spawn_local_task(false, info_span!("metronome"), async move |mut this| {
            let mut ticks = subscribe_to_ticks();

            tracing::info!("started metronome task");

            loop {
                let tick = ticks.wait().await;

                if USE_METRONOME.get() {
                    let metronome = &mut this.bind_mut().metronome;

                    let volume = if tick.beat == 0 && tick.tick == 0 {
                        1.0
                    } else if tick.tick == 0 {
                        0.3
                    } else {
                        0.1
                    };

                    metronome.set_volume_linear(volume);
                    metronome.play();
                }
            }
        });
    }

    pub fn on_node_input_event(
        &mut self,
        mut node: Gd<AudioNode>,
        node_index: NodeIndex,
        event: Gd<InputEvent>,
    ) {
        if !self.is_accepting_input {
            return;
        }

        match event.try_cast::<InputEventMouseButton>() {
            Ok(mb) if mb.is_pressed() && mb.get_button_index() == MouseButton::RIGHT => {
                node.bind_mut().toggle_cancelling();
            }
            Ok(mb) if mb.is_pressed() && mb.get_button_index() == MouseButton::LEFT => {
                tracing::info!("start playing on node {node_index:?}");

                let ticks = subscribe_to_ticks(); //Call this as early as possible, to improve synchronicity

                let graph = Rc::clone(&self.graph);
                let graph_godot_nodes = Rc::clone(&self.graph_godot_nodes);
                let mut rng = rand::rng(); //Graph walk direction is nondeterministic

                let panic_button_cancel = self.panic_button_cancel.clone();
                self.spawn_local_task(false, info_span!("graph_walk"), async move |this| {
                    Self::graph_walk(
                        this,
                        node,
                        node_index,
                        graph,
                        graph_godot_nodes,
                        ticks,
                        panic_button_cancel,
                        &mut rng,
                    )
                    .await;
                });
            }

            _ => {}
        };
    }

    pub fn perform_bpm_tap(&mut self) {
        let now = Instant::now();

        // Calculate avg bpm
        if self.bpm_taps.len() > 2 {
            let intervals: Vec<Duration> = self
                .bpm_taps
                .iter()
                .tuple_windows()
                .map(|(a, b)| *b - *a)
                .collect();

            let total_secs: f64 = intervals.iter().map(|dur| dur.as_secs_f64()).sum();
            let avg_secs = total_secs as f64 / intervals.len() as f64;
            let new_bpm = (60.0 / avg_secs).clamp(30.0, 300.0);

            tracing::info!("bpm tap set bpm to {new_bpm:.5}");
            AudioState::autoload().bind_mut().set_bpm(new_bpm); // This updates the slider UI as well
        }

        // Add new tap
        self.bpm_taps.push_back(now);

        // Limit tap count
        while self.bpm_taps.len() > 32 {
            self.bpm_taps.pop_front();
        }

        // Delete old taps
        let timeout = 30.0; // seconds
        self.bpm_taps = self
            .bpm_taps
            .iter()
            .cloned()
            .filter(|tap| (now - *tap).as_secs_f64() < timeout)
            .collect();

        tracing::info!("bpm tap count = {}", self.bpm_taps.len());
    }

    pub async fn play_intro_animation<R: Rng>(
        this: &mut Gd<Self>,
        constellation: &ConstellationGraph,
        island_data: &[(Waveform, bool, f64)],
        scc_assoc: &BTreeMap<NodeIndex, usize>,
        node_scene: Gd<PackedScene>,
        root_rng: &mut R,
    ) -> BTreeMap<NodeIndex, Gd<AudioNode>> {
        //Start spawning nodes
        let mut graph_godot_nodes = BTreeMap::default();

        let ConstellationGraph {
            chord,
            semitone_offset: semitone_offset_base,
            graph,
            ..
        } = constellation;

        //Use precise timing here from another thread to evenly spread the node spawning over time, even with low FPS.
        let (tx, rx) = flume::unbounded();
        let node_indices = graph.node_indices().collect::<Vec<_>>();
        tokio::task::spawn_blocking(move || {
            //Don't use rayon here, also don't block inside of a tokio::task::spawn!

            let nodes_per_second = 1000;
            let interval = Duration::from_secs_f64(1.0 / nodes_per_second as f64);
            let mut deadline = Instant::now();

            for i in node_indices {
                deadline += interval;
                spin_sleep::sleep_until(deadline);

                //Send tick
                let Ok(_) = tx.send(i) else {
                    tracing::info!("spawning_start animation cancelled");
                    break; //Important - stop the task if the channel is closed (otherwise it stalls the next animation)
                };
            }
        });

        let spawning_start = Instant::now();
        while let Ok(idx) = rx.recv_async().await {
            let mut node_rng = Xoshiro256Plus::from_rng(root_rng);
            //From now on use node_rng instead of root_rng!

            let pos = graph[idx];
            let instance = node_scene
                .instantiate()
                .expect("failed to instantiate node_scene");

            let mut audionode = instance.cast::<AudioNode>();
            let island_idx = scc_assoc[&idx] as i64;
            let (waveform, is_pad, octave_base) = island_data[usize::try_from(island_idx).unwrap()];

            //Need a little bit of variation of octaves within an island, otherwise it becomes boring
            let octave = (octave_base + Normal::new(0.0_f64, 1.0).unwrap().sample(&mut node_rng))
                .clamp(2.0, 8.0)
                .round() as i32;

            let detune = 0.07; //1.0 = full semitone offset
            let semitone_offset =
                *semitone_offset_base as f32 + node_rng.random_range(-detune..detune);

            {
                let mut audionode = audionode.bind_mut();
                audionode.set_chord(chord.to_godot());
                audionode.set_semitone_offset(semitone_offset);
                audionode.set_octave(octave);
                audionode.set_waveform(waveform.to_godot());

                audionode.set_duration(node_rng.random_range(0.3..1.5));
                audionode.set_node_idx(idx.index().try_into().unwrap());
                audionode.set_is_pad(is_pad);

                audionode.set_rng(node_rng);
            }

            audionode.set_position(pos); //Do this BEFORE add_child! (prevent re-calculating collision BVH twice)
            this.add_child(&audionode);

            graph_godot_nodes.insert(idx, audionode.clone());

            //Animation: make edges gradually visible, at the moment both connected nodes have been spawned
            {
                let others = graph.edges(idx);
                for edge in others {
                    if graph_godot_nodes.contains_key(&edge.target()) {
                        this.bind_mut()
                            .multimesh_instance
                            .get_multimesh()
                            .unwrap()
                            .set_instance_color(
                                edge.id().index() as i32,
                                audionode.bind().get_color(), //NOTE - this may introduce edges that have brightness > 1.0 (breaks MSAA)
                            );
                    }
                }
            }

            //Setup input events
            {
                let mut this = Gd::clone(this); //Clone it so we can move into the closure below
                audionode.signals().input_event().builder().connect_self_gd(
                    move |node, _, event, _, _, _| {
                        this.bind_mut().on_node_input_event(node, idx, event);
                    },
                );
            }
        }

        tracing::info!("`spawning_start` took {:?}", spawning_start.elapsed()); //This should now take 2s exactly, regardless of framerate

        graph_godot_nodes
    }

    pub fn generate_island_data<R: Rng>(
        constellation: &ConstellationGraph,
        root_rng: &mut R,
    ) -> Vec<(Waveform, bool, f64)> {
        let ConstellationGraph { islands, .. } = constellation;

        let island_count = islands.len();

        //Use this to generate stuff for every island, to ensure we remain deterministic, even if the amount of islands changes
        let mut island_rng = Xoshiro256Plus::from_rng(root_rng);

        let island_data: Vec<_> = (0..island_count)
            .map(|_island_idx| {
                let waveform = *Waveform::iter()
                    .collect::<Vec<_>>()
                    .choose_weighted(&mut island_rng, |w| match w {
                        Waveform::Sine => 1.0,
                        Waveform::Triangle => 1.0,
                        Waveform::Saw => 1.0,
                        Waveform::Square => 1.0,
                        Waveform::Noise => 0.25, //Noise is likely than the other waveforms
                    })
                    .unwrap();

                let is_pad = island_rng.random_bool(0.25); //1 in 4 islands is a pad

                //In my experiments with 50 million samples, the minimum is -7 and the max is 14, so gotta clamp it
                let octave_base = Normal::new(3.5_f64, 2.0)
                    .unwrap()
                    .sample(&mut island_rng)
                    .round();

                (waveform, is_pad, octave_base)
            })
            .collect();

        island_data
    }

    pub fn generate_stats(
        constellation: &ConstellationGraph,
        island_data: &[(Waveform, bool, f64)],
    ) -> String {
        let ConstellationGraph {
            chord,
            semitone_offset: semitone_offset_base,
            graph,
            islands,
            ..
        } = constellation;

        let pad_island_count = island_data.iter().filter(|(_, pad, _)| *pad).count();

        let island_sizes = islands
            .iter()
            .map(|island| island.len())
            .collect::<Vec<_>>();

        let island_count = islands.len();

        let waveform_occurrences = [false, true]
            .into_iter()
            .map(|is_pad| {
                let symbol = if is_pad { '▲' } else { '■' };

                Waveform::iter()
                    .map(|wav| {
                        let occurrences = island_data
                            .iter()
                            .filter(|(w, p, _)| *w == wav && *p == is_pad)
                            .count();
                        format!(
                            "[color={}]{symbol}×{occurrences:02}[/color]",
                            wav.as_color().to_html_without_alpha()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .join("\n");

        format!(
            r#"Chord: {chord:?} ({semitone_offset_base:+} semitones)
Vertex/edge count: {}, {}
Island count: {island_count}
Pad island count: {pad_island_count}/{island_count} ({:.1}%)
Waveform occurrences:
{waveform_occurrences}
Island size histogram:
{}"#,
            graph.node_count(),
            graph.edge_count(),
            pad_island_count as f32 / island_count as f32 * 100.0,
            Self::generate_histogram(&island_sizes, island_data)
        )
    }

    pub fn generate_histogram(data: &[usize], extra_data: &[(Waveform, bool, f64)]) -> String {
        //Count occurrences of each number using a BTreeMap (sorted keys)
        let mut counts = BTreeMap::new();
        for (num, extra) in data.iter().zip_eq(extra_data) {
            counts.entry(*num).or_insert(vec![]).push(extra);
        }

        //Now fill up the empty gaps
        let min_bin = 1;
        let max_bin = *counts.last_key_value().expect("BTreemap empty").0;

        for i in min_bin..=max_bin {
            //Ensure every bin is present in the range min_bin..=max_bin, even if it's empty
            counts.entry(i).or_insert(vec![]);
        }

        // Print histogram with counts and bars
        let mut str = String::with_capacity(200);
        for (num, extras) in counts {
            let mut histogram_bar = String::with_capacity(40);

            if extras.is_empty() {
                //Invisible dummy string to avoid the line height changing
                histogram_bar.push_str("[color=transparent]■[/color]");
            } else {
                for (waveform, is_pad, _) in extras {
                    let color = waveform.as_color();

                    //▮█ are both too wide, so use ■ instead
                    histogram_bar.push_str(&format!(
                        "[color={}]{}[/color]",
                        color.to_html_without_alpha(),
                        if *is_pad { '▲' } else { '■' } //Alternatively '△' and '○'
                    ));
                }
            }
            let formatted_num = if num <= 2 {
                format!(
                    //Island too small - give it a red outline
                    "[outline_size=5][outline_color=red]{num:>3}[/outline_color][/outline_size]",
                )
            } else {
                format!("{num:>3}")
            };

            str.push_str(&format!("{formatted_num} {histogram_bar}\n"));
        }

        str
    }
}

pub const DEFAULT_EDGE_TWEEN_PROGRESS: f32 = -999999.0; //Ensures the edge hides the progress indicator in the shader

fn setup_multimesh(mut multi: Gd<MultiMesh>, graph: &GraphTypedef) {
    let edge_count = graph.edge_count();

    multi.set_instance_count(edge_count as i32);

    //NOTE - this will only work correctly if you don't add new edges after deleting them (else edge ids will no longer be consecutive)
    for (i, edge) in graph.edge_references().enumerate() {
        let i = i as i32;
        let a = graph[edge.source()];
        let b = graph[edge.target()];

        let direction = b - a;
        let length = direction.length();
        let midpoint = a + direction * 0.5;

        //Create basis that rotates +Y to the direction vector
        let up = Vector3::UP;
        let axis = up.cross(direction).normalized();
        let angle = up.angle_to(direction);

        let rotation = if angle.abs() < f32::EPSILON {
            Basis::IDENTITY
        } else {
            Basis::from_axis_angle(axis, angle)
        };

        //Stretch the cylinder (assumes original height = 1.0)
        let scale = Vector3::new(1.0, length, 1.0);
        let basis = rotation * Basis::from_scale(scale);
        let transform = Transform3D::new(basis, midpoint);

        multi.set_instance_transform(i, transform);
        multi.set_instance_color(i, Color::BLACK);
        multi.set_instance_custom_data(
            i,
            Color::from_rgba(DEFAULT_EDGE_TWEEN_PROGRESS, 0.0, 0.0, 0.0),
        );
    }
}

fn reset_multimesh(mut multi: Gd<MultiMesh>) {
    multi.set_instance_count(0);
}

impl AsyncNode for AudioGraph {
    fn set_executor(
        &mut self,
        executor: Option<std::rc::Rc<async_executor::LocalExecutor<'static>>>,
    ) {
        self.executor = executor;
    }

    fn get_executor(&self) -> &Option<std::rc::Rc<async_executor::LocalExecutor<'static>>> {
        &self.executor
    }
}
