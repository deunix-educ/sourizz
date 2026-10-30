//! Small integer-rectangle helper used for damage tracking and monitor layout.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub const fn from_xywh(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x0: x, y0: y, x1: x + w, y1: y + h }
    }

    /// Smallest integer rect containing the float box `(l, t, r, b)`, padded by `pad`.
    pub fn around(l: f32, t: f32, r: f32, b: f32, pad: f32) -> Self {
        Self { x0: (l - pad).floor() as i32, y0: (t - pad).floor() as i32, x1: (r + pad).ceil() as i32, y1: (b + pad).ceil() as i32 }
    }

    pub fn centered(cx: f32, cy: f32, radius: f32) -> Self {
        Self::around(cx, cy, cx, cy, radius)
    }

    pub fn width(&self) -> i32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> i32 {
        self.y1 - self.y0
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn union(self, o: IRect) -> IRect {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        IRect::new(self.x0.min(o.x0), self.y0.min(o.y0), self.x1.max(o.x1), self.y1.max(o.y1))
    }

    pub fn intersect(self, o: IRect) -> IRect {
        IRect::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1))
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 as f32 && x < self.x1 as f32 && y >= self.y0 as f32 && y < self.y1 as f32
    }

    pub fn translate(self, dx: i32, dy: i32) -> IRect {
        IRect::new(self.x0 + dx, self.y0 + dy, self.x1 + dx, self.y1 + dy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_and_intersect() {
        let a = IRect::new(0, 0, 10, 10);
        let b = IRect::new(5, 5, 20, 8);
        assert_eq!(a.union(b), IRect::new(0, 0, 20, 10));
        assert_eq!(a.intersect(b), IRect::new(5, 5, 10, 8));
        assert!(a.intersect(IRect::new(11, 0, 12, 1)).is_empty());
        let empty = IRect::new(0, 0, 0, 0);
        assert_eq!(empty.union(a), a);
        assert_eq!(a.union(empty), a);
    }
}
