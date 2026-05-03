//! World coordinates and the six-direction enum.
//!
//! Coordinates are signed 32-bit integers per axis. The sparse world has no
//! fixed bounding box, so cells can live anywhere in the `i32` range. In
//! practice the working diameter of a world is `O(∛E_total)`, which for any
//! realistic `E_total` stays well within `i32` limits.
//!
//! The direction order (`xp, xn, yp, yn, zp, zn`) is **load-bearing**:
//! anything in the simulation that traverses neighbors must do so in this
//! order. Programs, snapshot tests, and the UI all depend on it.

/// 3D world coordinate.
///
/// Each axis is a signed 32-bit integer. There is no implicit origin or
/// bounding box; coordinates exist independently of whether a cell is
/// currently allocated at that position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Coord {
    /// X axis component.
    pub x: i32,
    /// Y axis component.
    pub y: i32,
    /// Z axis component.
    pub z: i32,
}

impl Coord {
    /// The world origin: `(0, 0, 0)`. The big bang's initial cell sits here.
    pub const ORIGIN: Self = Self { x: 0, y: 0, z: 0 };

    /// Construct a coordinate from `(x, y, z)`.
    #[must_use]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// Coordinate of the orthogonal neighbor across the given face.
    ///
    /// Uses wrapping arithmetic — at the edges of `i32` this would wrap,
    /// but in practice we never come close.
    #[must_use]
    pub const fn neighbor(self, direction: Direction) -> Self {
        let (dx, dy, dz) = direction.delta();
        Self {
            x: self.x.wrapping_add(dx),
            y: self.y.wrapping_add(dy),
            z: self.z.wrapping_add(dz),
        }
    }
}

/// Direction of energy flow across a cell face. Six in 3D.
///
/// The order is fixed and load-bearing — see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Direction {
    /// `+x`
    Xp = 0,
    /// `-x`
    Xn = 1,
    /// `+y`
    Yp = 2,
    /// `-y`
    Yn = 3,
    /// `+z`
    Zp = 4,
    /// `-z`
    Zn = 5,
}

impl Direction {
    /// Number of directions (`6`).
    pub const COUNT: usize = 6;

    /// All six directions in canonical order.
    pub const ALL: [Self; Self::COUNT] =
        [Self::Xp, Self::Xn, Self::Yp, Self::Yn, Self::Zp, Self::Zn];

    /// The opposite direction. Self-inverse: `d.opposite().opposite() == d`.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Xp => Self::Xn,
            Self::Xn => Self::Xp,
            Self::Yp => Self::Yn,
            Self::Yn => Self::Yp,
            Self::Zp => Self::Zn,
            Self::Zn => Self::Zp,
        }
    }

    /// Delta `(dx, dy, dz)` for moving from a cell to its neighbor across this face.
    #[must_use]
    pub const fn delta(self) -> (i32, i32, i32) {
        match self {
            Self::Xp => (1, 0, 0),
            Self::Xn => (-1, 0, 0),
            Self::Yp => (0, 1, 0),
            Self::Yn => (0, -1, 0),
            Self::Zp => (0, 0, 1),
            Self::Zn => (0, 0, -1),
        }
    }

    /// Index in `[0, 6)`. Useful for indexing into a `[T; 6]`.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}
