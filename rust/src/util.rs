use std::{
    f64::consts::TAU,
    hash::Hash,
    sync::atomic::{AtomicU32, Ordering},
};

use godot::builtin::{Color, Vector3};
use nalgebra::Vector3 as NVector3;
use num_traits::Float;
use ordered_float::OrderedFloat;
use rand::{Rng, SeedableRng as _};
use rand_xoshiro::Xoshiro256Plus;
use sha2::{Digest as _, Sha256};

pub trait Lerp: Sized {
    fn lerp(self, other: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self * (1.0 - t) + other * t
    }
}

impl Lerp for Vector3 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self.lerp(other, t) // This seems to be doing lerp in Rust, not Godot
    }
}
impl Lerp for Color {
    fn lerp(self, other: Self, t: f32) -> Self {
        // Implement your own lerp to avoid calling into Godot
        let r = self.r.lerp(other.r, t);
        let g = self.g.lerp(other.g, t);
        let b = self.b.lerp(other.b, t);
        let a = self.a.lerp(other.a, t);
        Color::from_rgba(r, g, b, a)
    }
}

pub trait LerpSmooth: Sized + Lerp {
    fn lerp_smooth(self, target: Self, lerp_speed: f32, delta: f32) -> Self {
        let t = 1.0 - (-delta * lerp_speed).exp();
        let t_clamped = t.clamp(0.0, 1.0);
        self.lerp(target, t_clamped)
    }
}

impl LerpSmooth for f32 {}
impl LerpSmooth for Vector3 {}
impl LerpSmooth for Color {}

///////////////

/// Atomic float, taken from https://github.com/rust-lang/rust/issues/72353#issuecomment-1093729062
#[derive(Default, Debug)]
pub struct AtomicF32 {
    storage: AtomicU32,
}

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        let as_u64 = value.to_bits();
        Self {
            storage: AtomicU32::new(as_u64),
        }
    }
    pub fn store(&self, value: f32, ordering: Ordering) {
        let as_u32 = value.to_bits();
        self.storage.store(as_u32, ordering)
    }
    pub fn load(&self, ordering: Ordering) -> f32 {
        let as_u32 = self.storage.load(ordering);
        f32::from_bits(as_u32)
    }
}

pub fn round_to_nearest_pow2_f64(n: f64) -> f64 {
    if n <= 0.0 {
        return 1.0;
    }

    let exp = n.log2().round();
    2.0.powf(exp)
}

/// This will fetch the AudioState autoload, get its seed, merge it with the given seed using SHA256, and produce a ChaCha8Rng.
/// ChaCha8Rng is deterministic and portable, so we should get the same results on all platforms given the same seed.
/// Ideally, you only call this ONCE at the start of every `level`, otherwise if the global seed changes during generation, it messes everything up.
/// You can generate sub-rngs from this root-rng and feed them to subnodes, for independent random number generation.
pub fn create_rng_from_seed_and_state(local_seed: u32, global_seed: i64) -> Xoshiro256Plus {
    let mut hasher = Sha256::new();
    hasher.update(global_seed.to_be_bytes());
    hasher.update(local_seed.to_be_bytes());

    let combined_seed = hasher.finalize();

    Xoshiro256Plus::from_seed(combined_seed.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrderedVector3 {
    pub x: OrderedFloat<f32>,
    pub y: OrderedFloat<f32>,
    pub z: OrderedFloat<f32>,
}

impl From<Vector3> for OrderedVector3 {
    fn from(v: Vector3) -> Self {
        OrderedVector3 {
            x: OrderedFloat(v.x),
            y: OrderedFloat(v.y),
            z: OrderedFloat(v.z),
        }
    }
}

impl From<OrderedVector3> for Vector3 {
    fn from(v: OrderedVector3) -> Self {
        Vector3::new(v.x.0, v.y.0, v.z.0)
    }
}
/// Generates a uniformly random unit vector.
/// Source: https://corysimon.github.io/articles/uniformdistn-on-sphere/
/// Source 2: https://medium.com/@all2one/generating-uniformly-distributed-points-on-sphere-1f7125978c4c
/// The distribution should be uniform over the surface of the unit sphere.
/// TODO is this even correct? y is up in Godot right, not z?
pub fn random_unit_axis<R: Rng>(rng: &mut R) -> NVector3<f64> {
    let z = -rng.random_range(-1.0..1.0); // The - here is unfortunately needed to ensure determinism with the snapshot test
    let theta = rng.random_range(0.0..TAU); //Angle on the XY-plane

    let radius = (1.0 - z * z).sqrt(); // Radius of the cross-section, if you slice the unit sphere at height z (x^2+y^2+z^2=1)
    let x = radius * theta.cos();
    let y = radius * theta.sin();
    NVector3::new(x, y, z)
}
