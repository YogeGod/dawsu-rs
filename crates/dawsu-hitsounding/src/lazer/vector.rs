//! A single-precision 2D vector with the operations osuTK's `Vector2` gives the ported lazer code.
//! `length()` computes in f32 and takes the square root in f64, as osuTK's
//! `(float)Math.Sqrt(X * X + Y * Y)` does.

use std::ops::{Add, Div, Mul, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

impl Vector2 {
    pub const ZERO: Vector2 = Vector2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Vector2 {
        Vector2 { x, y }
    }

    pub fn length_squared(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    pub fn length(self) -> f32 {
        f64::from(self.x * self.x + self.y * self.y).sqrt() as f32
    }

    pub fn dot(a: Vector2, b: Vector2) -> f32 {
        a.x * b.x + a.y * b.y
    }
}

impl Add for Vector2 {
    type Output = Vector2;
    fn add(self, o: Vector2) -> Vector2 {
        Vector2::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vector2 {
    type Output = Vector2;
    fn sub(self, o: Vector2) -> Vector2 {
        Vector2::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f32> for Vector2 {
    type Output = Vector2;
    fn mul(self, s: f32) -> Vector2 {
        Vector2::new(self.x * s, self.y * s)
    }
}

impl Div<f32> for Vector2 {
    type Output = Vector2;
    fn div(self, s: f32) -> Vector2 {
        Vector2::new(self.x / s, self.y / s)
    }
}
