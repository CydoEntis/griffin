// A sample for the highlight tests.
use std::fmt;

/// A point on a grid.
struct Point {
    x: i32,
    y: i32,
}

fn distance(a: &Point, b: &Point) -> f64 {
    let dx = (a.x - b.x) as f64;
    let dy = (a.y - b.y) as f64;
    (dx * dx + dy * dy).sqrt()
}

fn main() {
    let origin = Point { x: 0, y: 0 };
    let label = "origin";
    let scale = 2.5;
    let d = distance(&origin, &origin) * scale;
    println!("{label}: {d}");
}
