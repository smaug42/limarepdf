//! 2D affine matrices in the PDF row-vector convention: `p' = p × M`,
//! stored as `[a b c d e f]`. `mul(a, b)` applies `a` first, then `b`.

pub type M = [f32; 6];
pub const ID: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

pub fn mul(a: M, b: M) -> M {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

pub fn inv(m: M) -> Option<M> {
    let det = m[0] * m[3] - m[1] * m[2];
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

pub fn apply(m: M, x: f32, y: f32) -> (f32, f32) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

pub fn translate(x: f32, y: f32) -> M {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

pub fn aabb(points: impl IntoIterator<Item = (f32, f32)>) -> [f32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for (x, y) in points {
        b[0] = b[0].min(x);
        b[1] = b[1].min(y);
        b[2] = b[2].max(x);
        b[3] = b[3].max(y);
    }
    b
}

pub fn transform_rect(m: M, r: [f32; 4]) -> [f32; 4] {
    aabb([(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])].map(|(x, y)| apply(m, x, y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_roundtrip() {
        let m = [2.0, 0.5, -0.3, 1.5, 10.0, -4.0];
        let r = mul(m, inv(m).unwrap());
        for (a, b) in r.iter().zip(ID.iter()) {
            assert!((a - b).abs() < 1e-5, "{r:?}");
        }
        let (x, y) = apply(inv(m).unwrap(), apply(m, 3.0, 7.0).0, apply(m, 3.0, 7.0).1);
        assert!((x - 3.0).abs() < 1e-4 && (y - 7.0).abs() < 1e-4);
    }
}
