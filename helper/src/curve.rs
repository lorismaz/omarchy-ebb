//! ECDH on NIST B-163 (sect163r2): y² + xy = x³ + x² + b over GF(2¹⁶³).
//!
//! Field elements are polynomials over GF(2) packed into three 64-bit words,
//! least significant first. Points use affine coordinates; `None` is the point
//! at infinity. Nothing here is constant-time: the peer is a strap a few
//! metres away, and the private scalar lives for one handshake.

pub const WIRE_LEN: usize = 24; // one coordinate on the wire, little-endian
const DEGREE: usize = 163;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Gf([u64; 3]);

/// x¹⁶³ + x⁷ + x⁶ + x³ + 1
const MODULUS: Gf = Gf([0xC9, 0, 1 << (DEGREE - 128)]);

impl Gf {
    pub const ZERO: Gf = Gf([0; 3]);
    pub const ONE: Gf = Gf([1, 0, 0]);

    pub fn from_hex(hex: &str) -> Gf {
        let mut words = [0u64; 3];
        let digits: Vec<u8> = hex.bytes().rev().collect();
        for (i, d) in digits.iter().enumerate() {
            let v = (*d as char).to_digit(16).expect("hex digit") as u64;
            words[i / 16] |= v << (4 * (i % 16));
        }
        Gf(words)
    }

    /// 24 little-endian bytes; values with bits at or above 163 are rejected.
    pub fn from_wire(bytes: &[u8]) -> Option<Gf> {
        if bytes.len() != WIRE_LEN {
            return None;
        }
        let mut words = [0u64; 3];
        for (i, b) in bytes.iter().enumerate() {
            words[i / 8] |= (*b as u64) << (8 * (i % 8));
        }
        let g = Gf(words);
        (g.degree() < DEGREE as i32).then_some(g)
    }

    pub fn to_wire(self) -> [u8; WIRE_LEN] {
        let mut out = [0u8; WIRE_LEN];
        for (i, b) in out.iter_mut().enumerate() {
            *b = (self.0[i / 8] >> (8 * (i % 8))) as u8;
        }
        out
    }

    pub fn is_zero(self) -> bool {
        self.0 == [0; 3]
    }

    /// Index of the highest set bit, or -1 for zero.
    fn degree(self) -> i32 {
        for w in (0..3).rev() {
            if self.0[w] != 0 {
                return (w as i32) * 64 + 63 - self.0[w].leading_zeros() as i32;
            }
        }
        -1
    }

    fn bit(self, i: usize) -> bool {
        (self.0[i / 64] >> (i % 64)) & 1 == 1
    }

    fn shl(self, n: usize) -> Gf {
        let (words, bits) = (n / 64, n % 64);
        let mut out = [0u64; 3];
        for i in (words..3).rev() {
            let src = i - words;
            out[i] = self.0[src] << bits;
            if bits > 0 && src > 0 {
                out[i] |= self.0[src - 1] >> (64 - bits);
            }
        }
        Gf(out)
    }

    pub fn add(self, o: Gf) -> Gf {
        Gf([self.0[0] ^ o.0[0], self.0[1] ^ o.0[1], self.0[2] ^ o.0[2]])
    }

    pub fn mul(self, o: Gf) -> Gf {
        // Walk o's bits from the top, doubling the accumulator each step.
        let mut acc = Gf::ZERO;
        for i in (0..DEGREE).rev() {
            acc = acc.shl(1);
            if acc.bit(DEGREE) {
                acc = acc.add(MODULUS);
            }
            if o.bit(i) {
                acc = acc.add(self);
            }
        }
        acc
    }

    pub fn square(self) -> Gf {
        self.mul(self)
    }

    /// Multiplicative inverse by the polynomial extended Euclidean algorithm.
    pub fn inverse(self) -> Option<Gf> {
        if self.is_zero() {
            return None;
        }
        // Invariants: r0 ≡ s0·self and r1 ≡ s1·self (mod MODULUS).
        let (mut r0, mut r1) = (self, MODULUS);
        let (mut s0, mut s1) = (Gf::ONE, Gf::ZERO);
        while r0 != Gf::ONE {
            let mut shift = r0.degree() - r1.degree();
            if shift < 0 {
                std::mem::swap(&mut r0, &mut r1);
                std::mem::swap(&mut s0, &mut s1);
                shift = -shift;
            }
            r0 = r0.add(r1.shl(shift as usize));
            s0 = s0.add(s1.shl(shift as usize));
        }
        Some(s0)
    }
}

pub type Point = Option<(Gf, Gf)>;

pub struct Curve {
    pub b: Gf,
    pub g: (Gf, Gf),
    /// Order of the generator, as a bit string in the same packing.
    #[cfg_attr(not(test), allow(dead_code))]
    pub n: Gf,
}

/// The published NIST B-163 parameters.
pub fn b163() -> Curve {
    Curve {
        b: Gf::from_hex("20a601907b8c953ca1481eb10512f78744a3205fd"),
        g: (
            Gf::from_hex("3f0eba16286a2d57ea0991168d4994637e8343e36"),
            Gf::from_hex("0d51fbc6c71a0094fa2cdd545b11c5c0c797324f1"),
        ),
        n: Gf::from_hex("40000000000000000000292fe77e70c12a4234c33"),
    }
}

impl Curve {
    pub fn contains(&self, p: Point) -> bool {
        match p {
            None => true,
            Some((x, y)) => {
                let x2 = x.square();
                y.square().add(x.mul(y)) == x2.mul(x).add(x2).add(self.b)
            }
        }
    }

    pub fn add(&self, p: Point, q: Point) -> Point {
        let (Some((x1, y1)), Some((x2, y2))) = (p, q) else {
            return p.or(q);
        };
        if x1 == x2 {
            // Either the same point (double it) or opposites (-P = (x, x + y)).
            return if y1 == y2 { self.double(p) } else { None };
        }
        let slope = y1.add(y2).mul(x1.add(x2).inverse()?);
        let x3 = slope.square().add(slope).add(x1).add(x2).add(Gf::ONE);
        let y3 = slope.mul(x1.add(x3)).add(x3).add(y1);
        Some((x3, y3))
    }

    pub fn double(&self, p: Point) -> Point {
        let (x, y) = p?;
        if x.is_zero() {
            return None; // the tangent is vertical
        }
        let slope = x.add(y.mul(x.inverse()?));
        let x3 = slope.square().add(slope).add(Gf::ONE);
        let y3 = x.square().add(slope.add(Gf::ONE).mul(x3));
        Some((x3, y3))
    }

    /// k·P, scanning k's bits from the top (double, then add when the bit is set).
    pub fn multiply(&self, k: Gf, p: Point) -> Point {
        let mut acc: Point = None;
        for i in (0..=k.degree().max(0) as usize).rev() {
            acc = self.double(acc);
            if k.bit(i) {
                acc = self.add(acc, p);
            }
        }
        acc
    }

    pub fn encode(p: (Gf, Gf)) -> [u8; 2 * WIRE_LEN] {
        let mut out = [0u8; 2 * WIRE_LEN];
        out[..WIRE_LEN].copy_from_slice(&p.0.to_wire());
        out[WIRE_LEN..].copy_from_slice(&p.1.to_wire());
        out
    }

    /// A peer's public point: 48 bytes, on the curve, not infinity.
    pub fn decode(&self, bytes: &[u8]) -> Option<(Gf, Gf)> {
        if bytes.len() != 2 * WIRE_LEN {
            return None;
        }
        let p = (Gf::from_wire(&bytes[..WIRE_LEN])?, Gf::from_wire(&bytes[WIRE_LEN..])?);
        (self.contains(Some(p)) && !(p.0.is_zero() && p.1.is_zero())).then_some(p)
    }
}

/// A fresh private scalar in [2⁸¹, 2¹⁶²) and its public point.
pub fn keypair(curve: &Curve) -> (Gf, [u8; 2 * WIRE_LEN]) {
    loop {
        let mut raw = [0u8; WIRE_LEN];
        getrandom::fill(&mut raw[..21]).expect("system randomness");
        raw[20] &= 0x03; // keep bits 160 and 161: below 2^162
        let k = Gf::from_wire(&raw).expect("below 2^163");
        if k.degree() >= 81 {
            if let Some(p) = curve.multiply(k, Some(curve.g)) {
                return (k, Curve::encode(p));
            }
        }
    }
}

/// The 48-byte shared point, or None if the peer's point is invalid.
pub fn shared(curve: &Curve, private: Gf, peer: &[u8]) -> Option<[u8; 2 * WIRE_LEN]> {
    let p = curve.decode(peer)?;
    curve.multiply(private, Some(p)).map(Curve::encode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_inverse() {
        let a = Gf::from_hex("123456789abcdef0fedcba9876543210f");
        assert_eq!(a.mul(a.inverse().unwrap()), Gf::ONE);
        assert_eq!(Gf::ZERO.inverse(), None);
    }

    #[test]
    fn wire_round_trip_and_range() {
        let c = b163();
        assert_eq!(Gf::from_wire(&c.g.0.to_wire()), Some(c.g.0));
        assert_eq!(Gf::from_wire(&[0xff; WIRE_LEN]), None);
    }

    #[test]
    fn generator_has_order_n() {
        let c = b163();
        assert!(c.contains(Some(c.g)));
        assert_eq!(c.multiply(c.n, Some(c.g)), None);
    }

    #[test]
    fn diffie_hellman_agrees() {
        let c = b163();
        let (a, pa) = keypair(&c);
        let (b, pb) = keypair(&c);
        let s1 = shared(&c, a, &pb).unwrap();
        assert_eq!(Some(s1), shared(&c, b, &pa));
        // x < 2^163 fits in 21 bytes, so bytes 21..24 of x are always zero
        assert_eq!(&s1[21..24], &[0, 0, 0]);
    }

    #[test]
    fn rejects_bad_points() {
        let c = b163();
        assert!(c.decode(&[1u8; 48]).is_none());
        assert!(c.decode(&[0u8; 48]).is_none());
        assert!(c.decode(&[0u8; 47]).is_none());
    }
}
