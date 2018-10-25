pub use cgmath::{
    Basis2,
    Basis3,
    Deg,
    Euler,
    Point1,
    Point2,
    Point3,
    Quaternion,
    Rad,
    Rotation,
    Vector1,
    Vector2,
    Vector3,
    Vector4,
};
pub use std::f32::consts::PI;

pub const TAU: f32 = ::std::f32::consts::PI * 2.0;

pub trait Clamp {
    fn clamp(self, min: Self, max: Self) -> Self;
}

impl Clamp for f32 {
    fn clamp(self, min: Self, max: Self) -> Self {
        if self < min { return min; } else if self > max { return max; }
        self
    }
}

impl Clamp for f64 {
    fn clamp(self, min: Self, max: Self) -> Self {
        if self < min { return min; } else if self > max { return max; }
        self
    }
}
