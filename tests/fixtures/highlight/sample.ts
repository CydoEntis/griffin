// A sample for the highlight tests.
interface Point {
  x: number;
  label: string;
}

function distance(a: Point, b: Point): number {
  const dx = a.x - b.x;
  return Math.sqrt(dx * dx);
}

const origin: Point = { x: 0, label: "origin" };
console.log(distance(origin, origin));
