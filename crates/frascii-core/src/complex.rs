//! A complex number, in exactly the five operations the kernels need.
//!
//! Hand-rolled rather than pulled from `num-complex`: this crate's manifest
//! allows a numeric dependency, but a struct of two `f64`s and one multiply is
//! not worth one. The hot loops do not use these operators at all — they
//! destructure into raw `f64` locals, because an escape loop written in terms
//! of complex multiplication computes `zr*zr` twice per iteration where the
//! expanded form computes it once.

use core::ops::{Add, Mul, Sub};

/// A point on the complex plane.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Complex {
    /// Real part.
    pub re: f64,
    /// Imaginary part.
    pub im: f64,
}

impl Complex {
    /// The origin.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    /// A point from its parts.
    #[must_use]
    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    /// The squared magnitude.
    ///
    /// Squared, and never `norm()`: escape tests compare against a squared
    /// bailout so the square root never has to be taken, and this runs once per
    /// iteration of every sample.
    #[must_use]
    pub fn norm_sqr(self) -> f64 {
        self.re * self.re + self.im * self.im
    }

    /// Scale both components.
    #[must_use]
    pub fn scale(self, factor: f64) -> Self {
        Self::new(self.re * factor, self.im * factor)
    }
}

impl Add for Complex {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.re + rhs.re, self.im + rhs.im)
    }
}

impl Sub for Complex {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.re - rhs.re, self.im - rhs.im)
    }
}

impl Mul for Complex {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self::new(
            self.re * rhs.re - self.im * rhs.im,
            self.re * rhs.im + self.im * rhs.re,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_sqr_is_the_square_of_the_magnitude() {
        assert!((Complex::new(3.0, 4.0).norm_sqr() - 25.0).abs() < 1e-12);
        assert_eq!(Complex::ZERO.norm_sqr(), 0.0);
    }

    #[test]
    fn multiplication_follows_the_usual_rule() {
        // i * i == -1, the one case worth pinning by hand.
        let i = Complex::new(0.0, 1.0);
        let p = i * i;
        assert!((p.re + 1.0).abs() < 1e-12);
        assert!(p.im.abs() < 1e-12);
    }

    #[test]
    fn addition_and_subtraction_are_componentwise() {
        let a = Complex::new(1.0, 2.0);
        let b = Complex::new(0.25, -0.5);
        assert_eq!(a + b, Complex::new(1.25, 1.5));
        assert_eq!(a - b, Complex::new(0.75, 2.5));
    }

    #[test]
    fn scale_multiplies_both_parts() {
        assert_eq!(Complex::new(2.0, -3.0).scale(0.5), Complex::new(1.0, -1.5));
    }
}
